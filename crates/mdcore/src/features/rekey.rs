//! Changing the passphrase on all your devices (key rotation).
//!
//! Every device of an account shares one key, derived from the passphrase
//! (`crypto::derive_key`: Argon2id, salted with the username, so the new
//! passphrase typed on a device later gives the same key). A device you
//! removed (`features::revoke`) is refused by node id and device key, but
//! if it still knows the passphrase it can come back as a new install.
//! Changing the passphrase is what keeps it out: the new key goes only to
//! your trusted devices, and the old one stops working.
//!
//! 1. Start (any device, Settings → Security): the new passphrase gives the
//!    new key. Each of your devices that isn't removed and whose device key
//!    is pinned here (`device_key`) is a target. For each, an offer,
//!    `rekey_offer { from, o: { id, by, by_key, to, to_key, at, expires,
//!    members, epk, ct, sig } }`.
//!    `ct` is the new key sealed to that device alone: X25519 between a
//!    fresh ephemeral key (`epk`) and the target's Ed25519 device key in
//!    Montgomery form (`device_key::x25519_public`), hashed with both
//!    public keys and the id into an XChaCha20-Poly1305 key; the header is
//!    the associated data. `sig` is the initiator's device key over every
//!    field. Offers to devices that are offline wait (and are sent when a
//!    link to them comes up) until `expires` (7 days,
//!    `CoreConfig::key_rotation_ttl`). A device with no pinned key can't
//!    get one: it's listed, and needs the new passphrase typed on it.
//! 2. A target takes an offer only when it's addressed to it (its node id
//!    and its own device key), signed by the key pinned here for `by`
//!    (never one learned from the offer), `by` and its key aren't removed,
//!    it came over `by`'s own link, and it hasn't expired. It keeps the new
//!    key as "next" (beside the current one) and answers
//!    `rekey_ack { id, phase: "ready", proof }`; `proof` is a hash of the
//!    new key, the id, its node id and the phase, so only a device holding
//!    the key can answer.
//! 3. Finish (on the initiator, any time before the offers expire): the
//!    initiator switches (the old key becomes "previous") and sends a signed
//!    `rekey_commit` to the devices that are ready (and to the others as
//!    they answer). Each switches and answers `phase: "switched"`. Cancel,
//!    before Finish, sends a signed `rekey_cancel`: "next" is dropped.
//! 4. While the change is under way, a device opens messages with the
//!    current key, then "next", then "previous" (the last two only from the
//!    devices in the change, `members`); it seals with the new key for the
//!    devices that hold it (ready or switched, from the commit or seen
//!    using it) and the old one for the others. When every target has
//!    switched, the initiator sends a signed `rekey_done` and everyone
//!    drops the old key. At `expires` it's dropped anyway.
//!
//! A device that missed it (offline until after it expired, or with no
//! pinned key) still has the old key. Its devices' envelopes are signed
//! (`transport::envelope`, `env: 3`) with keys it pinned, so when they
//! don't open under any key it holds, it knows the account key changed:
//! [`KeyRotation::needs_passphrase`], and the apps ask for the new
//! passphrase. Typing it (`Engine::set_encryption_key`) gives the new key.
//!
//! The server takes no part: it never sees the passphrase or any key, and
//! the account password (sign-in) is separate and unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use parking_lot::Mutex;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::device_key::{x25519, x25519_base, x25519_public, DeviceKey};
use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// How long offers wait for a device that's offline (the default).
pub const TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// The longest an offer may say it lasts.
const MAX_TTL_MS: u64 = 31 * 24 * 3600 * 1000;
/// The most devices in one change.
pub const MAX_MEMBERS: usize = 64;
/// The shortest passphrase taken.
pub const MIN_LEN: usize = 8;

const STATE_KV: &str = "rekey_state";
const MISMATCH_KV: &str = "rekey_mismatch";
const NEXT_SECRET: &str = "device_key_next";
const PREV_SECRET: &str = "device_key_prev";
/// Where the session keeps the account key (`engine::session`).
const CURRENT_SECRET: &str = "device_key";

// --- wire ------------------------------------------------------------------

/// The new key, sealed to one device and signed by the one changing it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub by: String,
    pub by_key: String,
    pub to: String,
    pub to_key: String,
    pub at: u64,
    pub expires: u64,
    #[serde(default)]
    pub members: Vec<String>,
    pub epk: String,
    pub ct: String,
    #[serde(default)]
    pub sig: String,
}

/// A signed commit, cancel or done from the device changing it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    pub kind: String,
    pub id: String,
    pub by: String,
    pub at: u64,
    /// Commit: the devices holding the new key now.
    #[serde(default)]
    pub switched: Vec<String>,
    #[serde(default)]
    pub sig: String,
}

fn lp(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(s);
}

/// What the initiator's device key signs in an offer: every field but the
/// signature, each with its length.
pub fn offer_bytes(o: &Offer) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    out.extend_from_slice(b"my-domain rekey offer v1\0");
    for s in [&o.id, &o.by, &o.by_key, &o.to, &o.to_key, &o.epk, &o.ct] {
        lp(&mut out, s.as_bytes());
    }
    out.extend_from_slice(&o.at.to_be_bytes());
    out.extend_from_slice(&o.expires.to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(o.members.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for m in &o.members {
        lp(&mut out, m.as_bytes());
    }
    out
}

/// The associated data of the sealed key: who, to whom, which change, when.
fn seal_aad(o: &Offer) -> Vec<u8> {
    let mut out = Vec::with_capacity(160);
    out.extend_from_slice(b"my-domain rekey key v1\0");
    for s in [&o.id, &o.by, &o.by_key, &o.to, &o.to_key] {
        lp(&mut out, s.as_bytes());
    }
    out.extend_from_slice(&o.at.to_be_bytes());
    out.extend_from_slice(&o.expires.to_be_bytes());
    out
}

/// The symmetric key the new key is sealed under.
fn wrap_key(shared: &[u8; 32], epk: &[u8; 32], to_x: &[u8; 32], id: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"my-domain rekey wrap v1\0");
    h.update(shared);
    h.update(epk);
    h.update(to_x);
    h.update(id.as_bytes());
    h.finalize().into()
}

pub fn notice_bytes(n: &Notice) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(b"my-domain rekey notice v1\0");
    for s in [&n.kind, &n.id, &n.by] {
        lp(&mut out, s.as_bytes());
    }
    out.extend_from_slice(&n.at.to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(n.switched.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for m in &n.switched {
        lp(&mut out, m.as_bytes());
    }
    out
}

fn sign_notice(n: &mut Notice, key: &DeviceKey) {
    n.sig = B64.encode(key.sign(&notice_bytes(n)));
}

/// `n`'s signature is `key`'s.
pub fn notice_ok(n: &Notice, key: &str) -> bool {
    B64.decode(&n.sig)
        .is_ok_and(|sig| crate::device_key::verify(key, &notice_bytes(n), &sig))
}

/// Proof that a device holds `key`, for an ack.
pub fn proof(key: &[u8; 32], id: &str, node: &str, phase: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"my-domain rekey ack v1\0");
    h.update(key);
    for s in [id, node, phase] {
        h.update(u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
        h.update(s.as_bytes());
    }
    hex(&h.finalize())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_node(node: &str) -> bool {
    !node.trim().is_empty()
        && node.len() <= 128
        && !node.chars().any(char::is_control)
        && !node.starts_with("manual:")
}

/// Seal `key` to the device `to` (holding `to_key`), signed as `me`.
#[allow(clippy::too_many_arguments)]
pub fn make_offer(
    signer: &DeviceKey,
    me: &str,
    to: &str,
    to_key: &str,
    id: &str,
    at: u64,
    expires: u64,
    members: &[String],
    key: &[u8; 32],
) -> Option<Offer> {
    let to_x = x25519_public(to_key)?;
    let mut e = [0u8; 32];
    OsRng.fill_bytes(&mut e);
    let epk = x25519_base(e);
    let shared = x25519(e, to_x);
    if shared == [0u8; 32] {
        return None;
    }
    let mut o = Offer {
        id: id.to_string(),
        by: me.to_string(),
        by_key: signer.public(),
        to: to.to_string(),
        to_key: to_key.to_string(),
        at,
        expires,
        members: members.to_vec(),
        epk: B64.encode(epk),
        ..Default::default()
    };
    let wk = wrap_key(&shared, &epk, &to_x, id);
    o.ct = B64.encode(crate::crypto::seal(&wk, &seal_aad(&o), key)?);
    o.sig = B64.encode(signer.sign(&offer_bytes(&o)));
    Some(o)
}

/// Why an offer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Malformed,
    NotForMe,
    NotPinned,
    BadSignature,
    Expired,
    Undecryptable,
}

/// Check an offer that came to this device (`me`, holding `own`) and open
/// it: the new key. `pinned_by`: the key pinned here for `o.by`.
pub fn open_offer(
    o: &Offer,
    own: &DeviceKey,
    me: &str,
    pinned_by: Option<&str>,
    now: u64,
) -> Result<[u8; 32], Refused> {
    if !valid_id(&o.id)
        || !valid_node(&o.by)
        || o.by == me
        || o.members.len() > MAX_MEMBERS
        || o.members.iter().any(|m| !valid_node(m))
        || o.by_key.len() > 64
        || o.epk.len() > 64
        || o.ct.len() > 256
    {
        return Err(Refused::Malformed);
    }
    if o.to != me || o.to_key != own.public() {
        return Err(Refused::NotForMe);
    }
    if pinned_by != Some(o.by_key.as_str()) {
        return Err(Refused::NotPinned);
    }
    let sig_ok = B64
        .decode(&o.sig)
        .is_ok_and(|sig| crate::device_key::verify(&o.by_key, &offer_bytes(o), &sig));
    if !sig_ok {
        return Err(Refused::BadSignature);
    }
    if o.expires <= now
        || o.expires <= o.at
        || o.expires - o.at > MAX_TTL_MS
        || !super::not_ahead(o.at, now)
    {
        return Err(Refused::Expired);
    }
    let sk = own.x25519_secret().ok_or(Refused::Undecryptable)?;
    let epk: [u8; 32] = B64
        .decode(&o.epk)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(Refused::Malformed)?;
    let shared = x25519(sk, epk);
    if shared == [0u8; 32] {
        return Err(Refused::Undecryptable);
    }
    let wk = wrap_key(&shared, &epk, &x25519_base(sk), &o.id);
    let ct = B64.decode(&o.ct).map_err(|_| Refused::Malformed)?;
    crate::crypto::open(&wk, &seal_aad(o), &ct)
        .and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
        .ok_or(Refused::Undecryptable)
}

// --- state -----------------------------------------------------------------

/// A target device's progress (the initiator's list).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The offer hasn't been answered (offline, or on its way).
    Waiting,
    /// It holds the new key (not using it yet).
    Ready,
    /// It switched to the new key.
    Updated,
    /// It missed it: the offer expired (or the change ended) first.
    Expired,
    /// It has no device key pinned here: the new passphrase must be typed
    /// on it.
    NoKey,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Waiting => "waiting",
            Status::Ready => "ready",
            Status::Updated => "updated",
            Status::Expired => "expired",
            Status::NoKey => "no_key",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Target {
    name: String,
    status: Status,
    #[serde(default)]
    offer: Option<Offer>,
}

/// One change, as this device knows it (kv `rekey_state`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Saved {
    id: String,
    by: String,
    by_key: String,
    #[serde(default)]
    by_name: String,
    at: u64,
    expires: u64,
    committed: bool,
    /// "" while under way, then "done", "expired" or "cancelled".
    #[serde(default)]
    ended: String,
    #[serde(default)]
    members: Vec<String>,
    /// Devices known to hold the new key (sealed to with it).
    #[serde(default)]
    switched: BTreeSet<String>,
    /// The initiator's list.
    #[serde(default)]
    targets: BTreeMap<String, Target>,
    /// Ended and cleared from Settings: kept (not shown) so a device that
    /// missed it is still told apart from this one having missed a change.
    #[serde(default)]
    hidden: bool,
}

impl Saved {
    fn active(&self) -> bool {
        self.ended.is_empty()
    }
    fn is_member(&self, node: &str) -> bool {
        self.members.iter().any(|m| m == node)
    }
}

/// A device of yours seen using another account key.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Mismatch {
    node: String,
    name: String,
}

#[derive(Default)]
struct State {
    saved: Option<Saved>,
    next: Option<[u8; 32]>,
    prev: Option<[u8; 32]>,
    mismatch: Option<Mismatch>,
}

/// This device's part in a passphrase change (`NetCtx::rekey`).
#[derive(Default)]
pub struct Rekey {
    st: Mutex<State>,
}

impl Rekey {
    /// What was kept last run.
    pub fn load<P: Platform + ?Sized>(platform: &P) -> Rekey {
        let saved = platform
            .kv_get(STATE_KV)
            .and_then(|j| serde_json::from_str::<Saved>(&j).ok());
        let mismatch = platform
            .kv_get(MISMATCH_KV)
            .and_then(|j| serde_json::from_str::<Mismatch>(&j).ok());
        let active = saved.as_ref().is_some_and(Saved::active);
        let committed = saved.as_ref().is_some_and(|s| s.committed);
        let next = platform
            .secret_get(NEXT_SECRET)
            .and_then(|k| unhex32(&k))
            .filter(|_| active && !committed);
        let prev = platform
            .secret_get(PREV_SECRET)
            .and_then(|k| unhex32(&k))
            .filter(|_| active && committed);
        Rekey {
            st: Mutex::new(State {
                saved,
                next,
                prev,
                mismatch,
            }),
        }
    }
}

fn now_ms() -> u64 {
    crate::transport::envelope::now_ms()
}

fn persist<P: Platform>(ctx: &NetCtx<P>, st: &State) {
    let p = &*ctx.platform;
    let json = st
        .saved
        .as_ref()
        .and_then(|s| serde_json::to_string(s).ok())
        .unwrap_or_default();
    p.kv_set(STATE_KV, &json);
    let mm = st
        .mismatch
        .as_ref()
        .and_then(|m| serde_json::to_string(m).ok())
        .unwrap_or_default();
    p.kv_set(MISMATCH_KV, &mm);
    p.secret_set(NEXT_SECRET, st.next.map(|k| hex(&k)).as_deref());
    p.secret_set(PREV_SECRET, st.prev.map(|k| hex(&k)).as_deref());
}

/// Make `key` the account key (here and in the secret store).
fn use_key<P: Platform>(ctx: &NetCtx<P>, key: [u8; 32]) {
    ctx.platform.secret_set(CURRENT_SECRET, Some(&hex(&key)));
    *ctx.key.lock() = Some(key);
}

fn changed<P: Platform>(ctx: &NetCtx<P>) {
    let state = info(ctx).state;
    ctx.sink.emit(CoreEvent::KeyRotationChanged { state });
}

fn peer_name<P: Platform>(ctx: &NetCtx<P>, node: &str) -> String {
    ctx.peers
        .lock()
        .get(node)
        .map(|p| p.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| node.to_string())
}

// --- the transport's side --------------------------------------------------

/// Which of this device's keys opened something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Current,
    Next,
    Previous,
}

/// The key to seal with for `node`: the old one while a change is under
/// way and `node` (one of its devices) isn't known to hold the new one.
pub fn send_key<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<[u8; 32]> {
    let cur = *ctx.key.lock();
    let st = ctx.rekey.st.lock();
    if let (Some(prev), Some(s)) = (st.prev, st.saved.as_ref()) {
        if s.is_member(node) && !s.switched.contains(node) {
            return Some(prev);
        }
    }
    cur
}

/// The keys to try on something from `from`: the current one, then (from
/// the devices in a change under way) the next and the previous.
pub fn open_keys<P: Platform>(ctx: &NetCtx<P>, from: Option<&str>) -> Vec<([u8; 32], Which)> {
    let mut out = Vec::with_capacity(2);
    if let Some(k) = *ctx.key.lock() {
        out.push((k, Which::Current));
    }
    let st = ctx.rekey.st.lock();
    let member = match (from, st.saved.as_ref()) {
        (Some(f), Some(s)) => s.active() && s.is_member(f),
        _ => false,
    };
    if member {
        if let Some(k) = st.next {
            out.push((k, Which::Next));
        }
        if let Some(k) = st.prev {
            out.push((k, Which::Previous));
        }
    }
    out
}

/// Something from `from` opened under `which`: a device of the change
/// using the new key holds it; a device seen with another key works again.
pub fn opened<P: Platform>(ctx: &NetCtx<P>, from: Option<&str>, which: Which) {
    let Some(from) = from else {
        return;
    };
    let mut emit = false;
    {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let mut dirty = false;
        if which == Which::Current && st.prev.is_some() {
            if let Some(s) = st.saved.as_mut() {
                if s.is_member(from) && s.switched.insert(from.to_string()) {
                    dirty = true;
                }
            }
        }
        if which == Which::Current && st.mismatch.as_ref().is_some_and(|m| m.node == from) {
            st.mismatch = None;
            dirty = true;
            emit = true;
        }
        if dirty {
            persist(ctx, st);
        }
    }
    if emit {
        changed(ctx);
    }
}

/// An envelope from `from` didn't open under any key here, yet it's signed
/// by the key pinned for `from` (one of your devices): it uses another
/// account key. Either it missed a change (we know of it: listed as
/// expired) or this device did (the user is asked for the new passphrase).
pub fn undecryptable<P: Platform>(ctx: &NetCtx<P>, env: &crate::model::Envelope, from: &str) {
    if from.is_empty()
        || env.sig.is_empty()
        || env.src != from
        || ctx.revoked.is_revoked(from)
        || ctx.key.lock().is_none()
    {
        return;
    }
    let Some(pin) = crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, from) else {
        return;
    };
    if !crate::transport::envelope::signature_ok(env, &pin) {
        return;
    }
    let me = ctx.identity.lock().node_id.clone();
    let name = peer_name(ctx, from);
    {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        if let Some(s) = st.saved.as_mut() {
            // A change under way: keys differ for a while.
            if s.active() {
                return;
            }
            // It was in our last change and never took it: it's behind.
            if s.is_member(from) && !s.switched.contains(from) {
                if s.by == me {
                    if let Some(t) = s.targets.get_mut(from) {
                        if t.status != Status::Expired {
                            t.status = Status::Expired;
                            persist(ctx, st);
                        }
                    }
                }
                return;
            }
        }
        if st.mismatch.is_some() {
            return;
        }
        st.mismatch = Some(Mismatch {
            node: from.to_string(),
            name,
        });
        persist(ctx, st);
    }
    eprintln!("[rekey] {from} uses another account key: asking for the passphrase");
    changed(ctx);
}

// --- what the apps show ----------------------------------------------------

/// One device in a change (the initiator's list).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RotationDevice {
    pub node_id: String,
    pub name: String,
    /// "waiting", "ready", "updated", "expired" or "no_key".
    pub status: String,
}

/// The passphrase change, for Settings → Security.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KeyRotation {
    /// "none"; "preparing" (the new key is going out, nothing switched:
    /// can be cancelled); "switching" (switched, waiting for the rest);
    /// then "done", "expired" or "cancelled".
    pub state: String,
    /// Started on this device (the list and Finish / Cancel are here).
    pub by_me: bool,
    /// The device that started it.
    pub by_name: String,
    /// The devices it goes to (on the device that started it).
    pub devices: Vec<RotationDevice>,
    /// Seconds until offers to offline devices expire (under way only).
    pub expires_in_s: u32,
    /// A device of yours uses another passphrase: this one needs the new
    /// one typed in.
    pub needs_passphrase: bool,
    /// Which device (its name).
    pub mismatch_name: String,
}

pub fn info<P: Platform>(ctx: &NetCtx<P>) -> KeyRotation {
    let me = ctx.identity.lock().node_id.clone();
    let peers = ctx.peers.lock().clone();
    let st = ctx.rekey.st.lock();
    let mut out = KeyRotation {
        state: "none".into(),
        by_me: false,
        by_name: String::new(),
        devices: Vec::new(),
        expires_in_s: 0,
        needs_passphrase: st.mismatch.is_some(),
        mismatch_name: st
            .mismatch
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_default(),
    };
    let Some(s) = st.saved.as_ref().filter(|s| !s.hidden) else {
        return out;
    };
    out.state = if !s.ended.is_empty() {
        s.ended.clone()
    } else if s.committed {
        "switching".into()
    } else {
        "preparing".into()
    };
    out.by_me = s.by == me;
    out.by_name = peers
        .get(&s.by)
        .map(|p| p.name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| Some(s.by_name.clone()).filter(|n| !n.is_empty()))
        .unwrap_or_else(|| s.by.clone());
    if s.active() {
        out.expires_in_s =
            u32::try_from(s.expires.saturating_sub(now_ms()) / 1000).unwrap_or(u32::MAX);
    }
    let mut devices: Vec<RotationDevice> = s
        .targets
        .iter()
        .map(|(node, t)| RotationDevice {
            node_id: node.clone(),
            name: peers
                .get(node)
                .map(|p| p.name.clone())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| t.name.clone()),
            status: t.status.as_str().into(),
        })
        .collect();
    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.node_id.cmp(&b.node_id)));
    out.devices = devices;
    out
}

/// A rough strength for a passphrase: 0 (too short) to 4 (strong).
pub fn strength(p: &str) -> u8 {
    let p = p.trim();
    let len = p.chars().count();
    if len < MIN_LEN {
        return 0;
    }
    let classes = [
        p.chars().any(char::is_lowercase),
        p.chars().any(char::is_uppercase),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    let words = p.split_whitespace().count();
    let mut score = 1u8;
    if len >= 12 {
        score += 1;
    }
    if len >= 16 {
        score += 1;
    }
    if classes >= 3 || words >= 4 {
        score += 1;
    }
    score.min(4)
}

// --- the initiator -----------------------------------------------------------

/// Start changing the passphrase to `passphrase` (the account `username`):
/// offers go to every trusted device of yours.
pub fn start<P: Platform>(ctx: &NetCtx<P>, username: &str, passphrase: &str) -> crate::Result<()> {
    if passphrase.trim().chars().count() < MIN_LEN {
        return Err(CoreError::InvalidInput(format!(
            "use at least {MIN_LEN} characters"
        )));
    }
    let cur = (*ctx.key.lock()).ok_or(CoreError::NoKey)?;
    let new = crate::crypto::derive_key(passphrase, username).ok_or("failed to derive key")?;
    if new == cur {
        return Err(CoreError::InvalidInput(
            "that's the passphrase in use now".into(),
        ));
    }
    tick(ctx);
    if ctx
        .rekey
        .st
        .lock()
        .saved
        .as_ref()
        .is_some_and(Saved::active)
    {
        return Err(CoreError::InvalidInput(
            "a passphrase change is already under way".into(),
        ));
    }
    let (me, my_name) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.name.clone())
    };
    // Every device of yours: the ones listed now that aren't removed.
    let peers: Vec<(String, String)> = ctx
        .peers
        .lock()
        .values()
        .filter(|p| p.node_id != me && valid_node(&p.node_id))
        .map(|p| (p.node_id.clone(), p.name.clone()))
        .collect();
    let mut keyed: Vec<(String, String, String)> = Vec::new();
    let mut targets = BTreeMap::new();
    for (node, name) in peers {
        if ctx.revoked.is_revoked(&node) {
            continue;
        }
        match crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, &node) {
            Some(k) if !ctx.revoked.key_revoked(&k) && keyed.len() < MAX_MEMBERS - 1 => {
                keyed.push((node, name, k));
            }
            Some(_) => {}
            None => {
                targets.insert(
                    node,
                    Target {
                        name,
                        status: Status::NoKey,
                        offer: None,
                    },
                );
            }
        }
    }
    let mut idb = [0u8; 16];
    OsRng.fill_bytes(&mut idb);
    let id = hex(&idb);
    let at = now_ms();
    let ttl = u64::try_from(ctx.config.key_rotation_ttl.as_millis()).unwrap_or(u64::MAX);
    let expires = at.saturating_add(ttl.clamp(1, MAX_TTL_MS));
    let mut members: Vec<String> = vec![me.clone()];
    members.extend(keyed.iter().map(|(n, _, _)| n.clone()));
    for (node, name, key) in &keyed {
        let offer = make_offer(
            &ctx.device_key,
            &me,
            node,
            key,
            &id,
            at,
            expires,
            &members,
            &new,
        )
        .ok_or("could not seal the new key")?;
        targets.insert(
            node.clone(),
            Target {
                name: name.clone(),
                status: Status::Waiting,
                offer: Some(offer),
            },
        );
    }
    {
        let mut st = ctx.rekey.st.lock();
        st.saved = Some(Saved {
            id,
            by: me.clone(),
            by_key: ctx.device_key.public(),
            by_name: my_name,
            at,
            expires,
            committed: false,
            ended: String::new(),
            members,
            switched: BTreeSet::new(),
            targets,
            hidden: false,
        });
        st.next = Some(new);
        st.prev = None;
        st.mismatch = None;
        persist(ctx, &st);
    }
    for (node, _, _) in &keyed {
        send_offer(ctx, node);
    }
    changed(ctx);
    Ok(())
}

fn send_offer<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let offer = {
        let st = ctx.rekey.st.lock();
        st.saved
            .as_ref()
            .filter(|s| s.active())
            .and_then(|s| s.targets.get(node))
            .filter(|t| t.status == Status::Waiting)
            .and_then(|t| t.offer.clone())
    };
    if let Some(o) = offer {
        let me = ctx.identity.lock().node_id.clone();
        let msg = json!({"type": "rekey_offer", "from": me, "o": o}).to_string();
        send_control_to(ctx, node, &msg);
    }
}

/// A signed notice of `kind` for the current change (the initiator's).
fn notice<P: Platform>(ctx: &NetCtx<P>, kind: &str, s: &Saved) -> String {
    let mut n = Notice {
        kind: kind.into(),
        id: s.id.clone(),
        by: s.by.clone(),
        at: now_ms(),
        switched: if kind == "commit" {
            s.switched.iter().cloned().collect()
        } else {
            Vec::new()
        },
        sig: String::new(),
    };
    sign_notice(&mut n, &ctx.device_key);
    json!({"type": format!("rekey_{kind}"), "from": s.by, "n": n}).to_string()
}

fn send_to_all<P: Platform>(ctx: &NetCtx<P>, nodes: &[String], msg: &str) {
    for node in nodes {
        send_control_to(ctx, node, msg);
    }
}

/// Finish: before the switch, switch (the ready devices follow, the rest
/// as they answer); after it, stop waiting for the devices that haven't
/// switched (they'll need the new passphrase typed in).
pub fn finish<P: Platform>(ctx: &NetCtx<P>) -> crate::Result<()> {
    let me = ctx.identity.lock().node_id.clone();
    let cur = *ctx.key.lock();
    let committed = {
        let st = ctx.rekey.st.lock();
        match st.saved.as_ref().filter(|s| s.active() && s.by == me) {
            Some(s) => s.committed,
            None => {
                return Err(CoreError::InvalidInput(
                    "no passphrase change started here".into(),
                ))
            }
        }
    };
    if committed {
        end(ctx, "done");
        return Ok(());
    }
    let (msg, to) = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let Some(s) = st
            .saved
            .as_mut()
            .filter(|s| s.active() && s.by == me && !s.committed)
        else {
            return Err(CoreError::InvalidInput(
                "no passphrase change started here".into(),
            ));
        };
        let next = st.next.take().ok_or("the new key is missing")?;
        s.committed = true;
        let ready: Vec<String> = s
            .targets
            .iter()
            .filter(|(_, t)| t.status == Status::Ready)
            .map(|(n, _)| n.clone())
            .collect();
        s.switched = ready.iter().cloned().collect();
        s.switched.insert(me.clone());
        st.prev = cur;
        use_key(ctx, next);
        persist(ctx, st);
        let msg = notice(ctx, "commit", st.saved.as_ref().ok_or("gone")?);
        (msg, ready)
    };
    send_to_all(ctx, &to, &msg);
    changed(ctx);
    maybe_done(ctx);
    Ok(())
}

/// Cancel before the switch: every device drops the new key.
pub fn cancel<P: Platform>(ctx: &NetCtx<P>) -> crate::Result<()> {
    let me = ctx.identity.lock().node_id.clone();
    let (msg, to) = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let Some(s) = st.saved.as_mut().filter(|s| s.active() && s.by == me) else {
            return Err(CoreError::InvalidInput(
                "no passphrase change started here".into(),
            ));
        };
        if s.committed {
            return Err(CoreError::InvalidInput(
                "already switched: it can't be cancelled now".into(),
            ));
        }
        s.ended = "cancelled".into();
        let to: Vec<String> = s
            .targets
            .iter()
            .filter(|(_, t)| t.offer.is_some())
            .map(|(n, _)| n.clone())
            .collect();
        for t in s.targets.values_mut() {
            t.offer = None;
        }
        st.next = None;
        persist(ctx, st);
        (notice(ctx, "cancel", st.saved.as_ref().ok_or("gone")?), to)
    };
    send_to_all(ctx, &to, &msg);
    changed(ctx);
    Ok(())
}

/// Every target switched: tell them, and drop the old key.
fn maybe_done<P: Platform>(ctx: &NetCtx<P>) {
    let me = ctx.identity.lock().node_id.clone();
    let all = {
        let st = ctx.rekey.st.lock();
        st.saved.as_ref().is_some_and(|s| {
            s.active()
                && s.by == me
                && s.committed
                && s.targets
                    .values()
                    .all(|t| matches!(t.status, Status::Updated | Status::NoKey))
        })
    };
    if all {
        end(ctx, "done");
    }
}

/// End the change here (the initiator: tell the devices that switched).
fn end<P: Platform>(ctx: &NetCtx<P>, how: &str) {
    let me = ctx.identity.lock().node_id.clone();
    let send = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let Some(s) = st.saved.as_mut().filter(|s| s.active()) else {
            return;
        };
        s.ended = how.into();
        for t in s.targets.values_mut() {
            if matches!(t.status, Status::Waiting | Status::Ready) {
                t.status = Status::Expired;
            }
            t.offer = None;
        }
        let mine = s.by == me;
        let committed = s.committed;
        st.next = None;
        st.prev = None;
        persist(ctx, st);
        let s = st.saved.as_ref();
        match s {
            Some(s) if mine => {
                let kind = if committed { "done" } else { "cancel" };
                let to: Vec<String> = s
                    .members
                    .iter()
                    .filter(|m| **m != me && (!committed || s.switched.contains(*m)))
                    .cloned()
                    .collect();
                Some((notice(ctx, kind, s), to))
            }
            _ => None,
        }
    };
    if let Some((msg, to)) = send {
        send_to_all(ctx, &to, &msg);
    }
    changed(ctx);
}

/// A change under way that passed its expiry ends: the devices that
/// didn't switch missed it.
pub fn tick<P: Platform>(ctx: &NetCtx<P>) {
    let due = ctx
        .rekey
        .st
        .lock()
        .saved
        .as_ref()
        .is_some_and(|s| s.active() && now_ms() >= s.expires);
    if due {
        end(ctx, "expired");
    }
}

/// Check for expiry now and then (more often when it's short, in tests).
pub fn spawn<P: Platform>(ctx: &NetCtx<P>) {
    let ctx = ctx.clone();
    let every = (ctx.config.key_rotation_ttl / 8)
        .clamp(Duration::from_millis(250), Duration::from_secs(60));
    std::thread::spawn(move || {
        while ctx.shutdown.wait(every) {
            tick(&ctx);
        }
    });
}

/// A link to `node` came up: what waited for it.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    tick(ctx);
    let me = ctx.identity.lock().node_id.clone();
    let commit = {
        let st = ctx.rekey.st.lock();
        let Some(s) = st.saved.as_ref().filter(|s| s.active() && s.by == me) else {
            return;
        };
        match s.targets.get(node).map(|t| t.status) {
            Some(Status::Waiting) => None,
            Some(Status::Ready) if s.committed => Some(notice(ctx, "commit", s)),
            _ => return,
        }
    };
    match commit {
        Some(msg) => send_control_to(ctx, node, &msg),
        None => send_offer(ctx, node),
    }
}

/// Forget a finished change and a "needs the passphrase" notice (the user
/// typed it, or dismissed the list).
pub fn dismiss<P: Platform>(ctx: &NetCtx<P>) {
    {
        let mut st = ctx.rekey.st.lock();
        if let Some(s) = st.saved.as_mut().filter(|s| !s.active()) {
            s.hidden = true;
        }
        st.mismatch = None;
        persist(ctx, &st);
    }
    changed(ctx);
}

/// The account key was set by hand (the passphrase typed, or signed out):
/// any change under way here is dropped.
pub fn reset<P: Platform>(ctx: &NetCtx<P>) {
    {
        let mut st = ctx.rekey.st.lock();
        if st.saved.is_none() && st.mismatch.is_none() && st.next.is_none() && st.prev.is_none() {
            return;
        }
        *st = State::default();
        persist(ctx, &st);
    }
    changed(ctx);
}

/// Tests: the offer waiting for `node`, as it goes on the wire.
pub fn offer_message<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<String> {
    let st = ctx.rekey.st.lock();
    let s = st.saved.as_ref()?;
    let o = s.targets.get(node)?.offer.clone()?;
    Some(json!({"type": "rekey_offer", "from": s.by, "o": o}).to_string())
}

// --- the other devices -------------------------------------------------------

fn on_offer<P: Platform>(ctx: &NetCtx<P>, from: &str, v: &Value) {
    let Ok(o) = serde_json::from_value::<Offer>(v.clone()) else {
        return;
    };
    // From the device that signed it, over its own link; not a removed one.
    if o.by != from || ctx.revoked.refuses(&o.by, Some(&o.by_key)) {
        eprintln!("[rekey] offer from {from} refused (sender)");
        return;
    }
    let me = ctx.identity.lock().node_id.clone();
    let pin = crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, &o.by);
    let key = match open_offer(&o, &ctx.device_key, &me, pin.as_deref(), now_ms()) {
        Ok(k) => k,
        Err(why) => {
            eprintln!("[rekey] offer from {from} refused ({why:?})");
            return;
        }
    };
    tick(ctx);
    let cur = *ctx.key.lock();
    let phase = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        if cur == Some(key) {
            "switched"
        } else {
            let take = match st.saved.as_ref() {
                Some(s) if s.id == o.id => s.active() && !s.committed && st.next == Some(key),
                // Busy switching for another change: that ends first.
                Some(s) if s.active() && s.committed => false,
                // Two at once: the later one wins, everywhere.
                Some(s) if s.active() && (s.at, s.id.as_str()) > (o.at, o.id.as_str()) => false,
                _ => {
                    st.saved = Some(Saved {
                        id: o.id.clone(),
                        by: o.by.clone(),
                        by_key: o.by_key.clone(),
                        by_name: peer_name(ctx, &o.by),
                        at: o.at,
                        expires: o.expires,
                        committed: false,
                        ended: String::new(),
                        members: o.members.clone(),
                        switched: BTreeSet::new(),
                        targets: BTreeMap::new(),
                        hidden: false,
                    });
                    st.next = Some(key);
                    st.prev = None;
                    st.mismatch = None;
                    persist(ctx, st);
                    true
                }
            };
            if !take {
                return;
            }
            "ready"
        }
    };
    ack(ctx, from, &o.id, phase, &key);
    changed(ctx);
}

fn ack<P: Platform>(ctx: &NetCtx<P>, to: &str, id: &str, phase: &str, key: &[u8; 32]) {
    let me = ctx.identity.lock().node_id.clone();
    let msg = json!({
        "type": "rekey_ack", "from": me, "id": id, "phase": phase,
        "proof": proof(key, id, &me, phase),
    })
    .to_string();
    send_control_to(ctx, to, &msg);
}

fn on_ack<P: Platform>(ctx: &NetCtx<P>, from: &str, msg: &ControlMsg) {
    let (id, phase, got) = (msg.str("id"), msg.str("phase"), msg.str("proof"));
    if phase != "ready" && phase != "switched" {
        return;
    }
    let me = ctx.identity.lock().node_id.clone();
    let cur = *ctx.key.lock();
    let commit = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let new = if st.saved.as_ref().is_some_and(|s| s.committed) {
            cur
        } else {
            st.next
        };
        let Some(new) = new else {
            return;
        };
        let Some(s) = st
            .saved
            .as_mut()
            .filter(|s| s.active() && s.by == me && s.id == id)
        else {
            return;
        };
        if got != proof(&new, &id, from, &phase) {
            eprintln!("[rekey] ack from {from} refused (proof)");
            return;
        }
        let Some(t) = s.targets.get_mut(from).filter(|t| t.offer.is_some()) else {
            return;
        };
        let status = if phase == "switched" {
            Status::Updated
        } else {
            Status::Ready
        };
        if t.status == Status::Updated || t.status == status {
            None
        } else {
            t.status = status;
            s.switched.insert(from.to_string());
            let commit = (s.committed && status == Status::Ready).then(|| notice(ctx, "commit", s));
            persist(ctx, st);
            Some(commit)
        }
    };
    let Some(commit) = commit else {
        return;
    };
    if let Some(m) = commit {
        send_control_to(ctx, from, &m);
    }
    changed(ctx);
    maybe_done(ctx);
}

fn on_notice<P: Platform>(ctx: &NetCtx<P>, from: &str, v: &Value) {
    let Ok(n) = serde_json::from_value::<Notice>(v.clone()) else {
        return;
    };
    let me = ctx.identity.lock().node_id.clone();
    if n.by != from || n.by == me || n.switched.len() > MAX_MEMBERS {
        return;
    }
    let cur = *ctx.key.lock();
    let reply = {
        let mut st = ctx.rekey.st.lock();
        let st = &mut *st;
        let Some(s) = st
            .saved
            .as_mut()
            .filter(|s| s.active() && s.id == n.id && s.by == n.by)
        else {
            return;
        };
        if ctx.revoked.refuses(&s.by, Some(&s.by_key)) || !notice_ok(&n, &s.by_key) {
            eprintln!("[rekey] {} from {from} refused (signature)", n.kind);
            return;
        }
        match n.kind.as_str() {
            "commit" => {
                if !s.committed {
                    let Some(next) = st.next.take() else {
                        return;
                    };
                    s.committed = true;
                    st.prev = cur;
                    use_key(ctx, next);
                }
                let known: Vec<String> = n
                    .switched
                    .iter()
                    .filter(|m| s.is_member(m))
                    .cloned()
                    .collect();
                s.switched.extend(known);
                s.switched.insert(s.by.clone());
                let reply = (*ctx.key.lock()).map(|k| (s.id.clone(), k));
                persist(ctx, st);
                reply
            }
            "cancel" if !s.committed => {
                s.ended = "cancelled".into();
                st.next = None;
                persist(ctx, st);
                None
            }
            "done" if s.committed => {
                s.ended = "done".into();
                st.prev = None;
                persist(ctx, st);
                None
            }
            _ => return,
        }
    };
    if let Some((id, key)) = reply {
        ack(ctx, from, &id, "switched", &key);
    }
    changed(ctx);
}

pub struct RekeyHandler;

impl<P: Platform> ControlHandler<P> for RekeyHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "rekey_offer",
            "rekey_ack",
            "rekey_commit",
            "rekey_cancel",
            "rekey_done",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        // Only over a link (it names the sender), from a device that isn't
        // removed.
        let Some(from) = msg.node.filter(|n| !n.is_empty()).map(str::to_string) else {
            return;
        };
        if ctx.revoked.is_revoked(&from) {
            return;
        }
        match msg.kind() {
            "rekey_offer" => on_offer(ctx, &from, &msg.json("o")),
            "rekey_ack" => on_ack(ctx, &from, msg),
            _ => {
                let n = msg.json("n");
                let kind = n["kind"].as_str().unwrap_or_default();
                if msg.kind() == format!("rekey_{kind}") {
                    on_notice(ctx, &from, &n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "00112233445566778899aabbccddeeff";

    fn offer(by: &DeviceKey, to: &DeviceKey, key: &[u8; 32], now: u64) -> Offer {
        make_offer(
            by,
            "a",
            "b",
            &to.public(),
            ID,
            now,
            now + 60_000,
            &["a".into(), "b".into()],
            key,
        )
        .unwrap()
    }

    #[test]
    fn an_offer_opens_only_on_the_device_it_was_sealed_to() {
        let (a, b, c) = (
            DeviceKey::ephemeral(),
            DeviceKey::ephemeral(),
            DeviceKey::ephemeral(),
        );
        let now = now_ms();
        let key = [9u8; 32];
        let o = offer(&a, &b, &key, now);
        let pin = a.public();
        assert_eq!(open_offer(&o, &b, "b", Some(&pin), now), Ok(key));
        // Another device (even claiming the node id) can't.
        assert_eq!(
            open_offer(&o, &c, "b", Some(&pin), now),
            Err(Refused::NotForMe)
        );
        let mut forged = o.clone();
        forged.to_key = c.public();
        assert_eq!(
            open_offer(&forged, &c, "b", Some(&pin), now),
            Err(Refused::BadSignature)
        );
        assert_eq!(
            open_offer(&o, &b, "c", Some(&pin), now),
            Err(Refused::NotForMe)
        );
    }

    #[test]
    fn a_tampered_or_forged_offer_is_refused() {
        let (a, b, m) = (
            DeviceKey::ephemeral(),
            DeviceKey::ephemeral(),
            DeviceKey::ephemeral(),
        );
        let now = now_ms();
        let o = offer(&a, &b, &[9u8; 32], now);
        let pin = a.public();
        let tweak: [fn(&mut Offer); 6] = [
            |o| o.expires += 1,
            |o| o.at -= 1,
            |o| o.members.push("x".into()),
            |o| o.ct = B64.encode([0u8; 48]),
            |o| o.epk = B64.encode([1u8; 32]),
            |o| o.id = "ffeeddccbbaa99887766554433221100".into(),
        ];
        for t in tweak {
            let mut f = o.clone();
            t(&mut f);
            assert_eq!(
                open_offer(&f, &b, "b", Some(&pin), now),
                Err(Refused::BadSignature),
                "{f:?}"
            );
        }
        // Signed by another key (a device posing as `a`): not the pinned one.
        let forged = make_offer(
            &m,
            "a",
            "b",
            &b.public(),
            ID,
            now,
            now + 60_000,
            &[],
            &[1; 32],
        )
        .unwrap();
        assert_eq!(
            open_offer(&forged, &b, "b", Some(&pin), now),
            Err(Refused::NotPinned)
        );
        // Nobody pinned for the signer: refused too.
        assert_eq!(open_offer(&o, &b, "b", None, now), Err(Refused::NotPinned));
        // Expired.
        assert_eq!(
            open_offer(&o, &b, "b", Some(&pin), now + 61_000),
            Err(Refused::Expired)
        );
    }

    #[test]
    fn notices_are_signed_and_proofs_bind_the_key() {
        let a = DeviceKey::ephemeral();
        let mut n = Notice {
            kind: "commit".into(),
            id: ID.into(),
            by: "a".into(),
            at: 5,
            switched: vec!["b".into()],
            sig: String::new(),
        };
        sign_notice(&mut n, &a);
        assert!(notice_ok(&n, &a.public()));
        assert!(!notice_ok(&n, &DeviceKey::ephemeral().public()));
        let mut f = n.clone();
        f.kind = "cancel".into();
        assert!(!notice_ok(&f, &a.public()));
        let mut f = n.clone();
        f.switched.push("evil".into());
        assert!(!notice_ok(&f, &a.public()));
        assert_ne!(
            proof(&[1; 32], ID, "b", "ready"),
            proof(&[2; 32], ID, "b", "ready")
        );
        assert_ne!(
            proof(&[1; 32], ID, "b", "ready"),
            proof(&[1; 32], ID, "c", "ready")
        );
    }

    #[test]
    fn passphrase_strength_grows_with_length_and_variety() {
        assert_eq!(strength("short"), 0);
        assert_eq!(strength("abcdefgh"), 1);
        assert!(strength("correct horse battery staple") >= 3);
        assert_eq!(strength("Tr0ub4dor&3-and-more!"), 4);
    }
}
