//! Removed devices: a signed revocation list shared by your devices.
//!
//! Every device of an account shares one key, so a device you remove from
//! the list but that still has the passphrase would be found again on the
//! network (mDNS, the LAN beacon, the server's list) and linked again. So
//! removing a device (`Engine::remove_device`) writes a revocation: the
//! removed node id, the device key pinned for it (`device_key`), its name,
//! when, and which of your devices did it, signed by that device's own key
//! (Ed25519, over [`signed_bytes`]). It goes to all your devices, which
//! keep it (kv `revocations`) and from then on refuse that node id, and
//! that key under any node id:
//! - its links: QUIC (after the handshake), the relay, Bluetooth;
//! - its messages (`transport::open_envelope`) and media records;
//! - its LAN beacons and mDNS announcements, and its entry in the server's
//!   list (it isn't added back as a peer);
//! - its signals (pushes asking for a link).
//!
//! "Allow again" (`Engine::allow_device_again`) writes a newer entry for the
//! same node id saying it's allowed, signed the same way: the device is
//! found again as before. Entries are last-writer-wins per node id by
//! (`at`, `by`, revoked), so every device ends with the same list whatever
//! order they came in; an allowed entry stays (a tombstone) so an older
//! revocation never comes back.
//!
//!   revocations { from, list: { <node>: {key, name, revoked, at, by,
//!                                          by_key, sig} } }
//!
//! An entry is taken only when:
//! - it's signed by `by_key`, and `by_key` is the key this device pinned
//!   for `by` (or, for a device of yours it lists but never met directly,
//!   taken on first use as QUIC does), or this device's own key;
//! - `by` isn't revoked here, isn't the device it names, and the entry
//!   doesn't name this device (a device never takes its own removal: it
//!   would have nothing to do with it);
//! - its time isn't far ahead of this device's clock.
//!
//! The list goes (it's small) as labels do: when a link comes up to a
//! device that said `revoke: 1` (in its status or `addr_update`), and to
//! every linked device when it changes here. A device that has something
//! newer answers once.
//!
//! Limits: a device holding the passphrase can still join again as a new
//! device (a new node id and key, as a reinstall or a QR / code join
//! would): you see it in your list and can remove it again. Changing the
//! passphrase is what keeps it out for good. And a removed device that
//! sends its own revocation of another device first (before the one
//! removing it is heard) is believed until its own removal arrives;
//! "Allow again" undoes that.

use std::collections::{BTreeMap, HashMap};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying a device keeps the list.
pub const FLAG: &str = "revoke";
const VERSION: u64 = 1;
/// The most entries kept (allowed-again ones included).
pub const MAX_ENTRIES: usize = 256;
const MAX_NODE: usize = 128;
const MAX_NAME: usize = 80;

const MAP_KEY: &str = "revocations";
const CAPS_PREFIX: &str = "revoke_caps:";

/// One device's entry, as it goes on the wire and into `kv`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The device key pinned for it (base64), empty when it never
    /// connected with one.
    #[serde(default)]
    pub key: String,
    /// Its name when it was removed (for the list).
    #[serde(default)]
    pub name: String,
    /// Removed (true), or allowed again (false).
    #[serde(default)]
    pub revoked: bool,
    /// When (unix ms).
    #[serde(default)]
    pub at: u64,
    /// The device that did it, and its key (base64).
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub by_key: String,
    /// `by_key`'s signature of [`signed_bytes`], base64.
    #[serde(default)]
    pub sig: String,
}

/// Node id -> its entry.
pub type RevocationMap = BTreeMap<String, Entry>;

impl Entry {
    fn order(&self) -> (u64, &str, bool) {
        (self.at, &self.by, self.revoked)
    }

    /// This entry replaces `other` in a merge.
    pub fn wins_over(&self, other: &Entry) -> bool {
        self.order() > other.order()
    }
}

/// What the signer's key signs: a label, then every field but the
/// signature, each with its length.
pub fn signed_bytes(node: &str, e: &Entry) -> Vec<u8> {
    let mut out = Vec::with_capacity(160);
    out.extend_from_slice(b"my-domain revocation v1\0");
    for s in [node, &e.key, &e.name, &e.by, &e.by_key] {
        out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out.push(u8::from(e.revoked));
    out.extend_from_slice(&e.at.to_be_bytes());
    out
}

/// Sign `e` (about `node`) as this device: `by`, `by_key` and `sig` set.
pub fn sign(node: &str, e: &mut Entry, me: &str, key: &crate::device_key::DeviceKey) {
    e.by = me.to_string();
    e.by_key = key.public();
    e.sig = B64.encode(key.sign(&signed_bytes(node, e)));
}

/// `e`'s signature is `by_key`'s.
pub fn signature_ok(node: &str, e: &Entry) -> bool {
    let Ok(sig) = B64.decode(&e.sig) else {
        return false;
    };
    crate::device_key::verify(&e.by_key, &signed_bytes(node, e), &sig)
}

fn valid_node(node: &str) -> bool {
    !node.trim().is_empty() && node.len() <= MAX_NODE && !node.chars().any(char::is_control)
}

/// Well formed (before any key is looked at).
fn well_formed(node: &str, e: &Entry) -> bool {
    valid_node(node)
        && valid_node(&e.by)
        && node != e.by
        && e.name.chars().count() <= MAX_NAME
        && e.key.len() <= 64
        && e.by_key.len() <= 64
        && e.sig.len() <= 128
        && !node.starts_with("manual:")
}

/// The list, and which devices keep it.
#[derive(Default)]
pub struct Revocations {
    map: Mutex<RevocationMap>,
    caps: Mutex<HashMap<String, bool>>,
}

impl Revocations {
    /// The list kept last run.
    pub fn load<P: Platform + ?Sized>(platform: &P) -> Revocations {
        let map = platform
            .kv_get(MAP_KEY)
            .and_then(|j| serde_json::from_str::<RevocationMap>(&j).ok())
            .unwrap_or_default();
        Revocations {
            map: Mutex::new(map),
            caps: Mutex::default(),
        }
    }

    /// `node` was removed (and not allowed again).
    pub fn is_revoked(&self, node: &str) -> bool {
        self.map.lock().get(node).is_some_and(|e| e.revoked)
    }

    /// `key` (base64) belongs to a removed device.
    pub fn key_revoked(&self, key: &str) -> bool {
        !key.is_empty() && self.map.lock().values().any(|e| e.revoked && e.key == key)
    }

    /// A link from `node` holding `key` must be refused.
    pub fn refuses(&self, node: &str, key: Option<&str>) -> bool {
        self.is_revoked(node) || key.is_some_and(|k| self.key_revoked(k))
    }

    /// Every entry (allowed-again ones too).
    pub fn entries(&self) -> RevocationMap {
        self.map.lock().clone()
    }

    fn save<P: Platform + ?Sized>(&self, platform: &P, map: &RevocationMap) {
        if let Ok(json) = serde_json::to_string(map) {
            platform.kv_set(MAP_KEY, &json);
        }
    }
}

fn now_ms() -> u64 {
    crate::transport::envelope::now_ms()
}

/// May `e` (about `node`, arrived from another device) be taken? Its
/// signer must be one of your devices, not removed, holding the key it
/// signed with (see the module docs).
fn trusted<P: Platform>(ctx: &NetCtx<P>, node: &str, e: &Entry) -> bool {
    let me = ctx.identity.lock().node_id.clone();
    if !well_formed(node, e) || node == me || !super::not_ahead(e.at, now_ms()) {
        return false;
    }
    if ctx.revoked.is_revoked(&e.by) || !signature_ok(node, e) {
        return false;
    }
    if e.by == me {
        return e.by_key == ctx.device_key.public();
    }
    match crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, &e.by) {
        Some(pin) => pin == e.by_key,
        // Never met directly: one of the devices it lists, on first use.
        None => {
            super::known(ctx, &e.by)
                && crate::device_key::accept(&ctx.peer_keys, &*ctx.platform, &e.by, Some(&e.by_key))
        }
    }
}

/// Merge entries that arrived; returns the nodes whose entry changed.
fn merge<P: Platform>(ctx: &NetCtx<P>, incoming: RevocationMap) -> Vec<String> {
    let candidates: Vec<(String, Entry)> = {
        let map = ctx.revoked.map.lock();
        incoming
            .into_iter()
            .filter(|(n, e)| map.get(n).is_none_or(|cur| e.wins_over(cur)))
            .collect()
    };
    let mut changed = Vec::new();
    for (node, e) in candidates {
        // Checked one by one: an entry taken may revoke the signer of the
        // next one.
        if !trusted(ctx, &node, &e) {
            continue;
        }
        let mut map = ctx.revoked.map.lock();
        if !map.get(&node).is_none_or(|cur| e.wins_over(cur)) {
            continue;
        }
        if !map.contains_key(&node) && map.len() >= MAX_ENTRIES && !make_room(&mut map) {
            continue;
        }
        map.insert(node.clone(), e);
        ctx.revoked.save(&*ctx.platform, &map);
        changed.push(node);
    }
    changed
}

/// Room for one more: drop the oldest allowed-again entry.
fn make_room(map: &mut RevocationMap) -> bool {
    let oldest = map
        .iter()
        .filter(|(_, e)| !e.revoked)
        .min_by_key(|(_, e)| e.at)
        .map(|(n, _)| n.clone());
    oldest.is_some_and(|n| map.remove(&n).is_some())
}

/// `node` is revoked now: drop it from the list of devices and close its
/// links (whatever they are).
fn enforce<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let had = ctx.peers.lock().remove(node).is_some();
    // Paired again, it starts with nothing allowed.
    super::perms::forget(ctx, node);
    crate::transport::close_link(ctx, node, "manual");
    // A Bluetooth or relay route goes with it.
    ctx.ws_conns.lock().remove(node);
    crate::transport::drop_live(ctx, node);
    if had {
        crate::engine::emit_peers(ctx);
    }
}

/// What changed, for the apps.
fn announce<P: Platform>(ctx: &NetCtx<P>, changed: &[String]) {
    if changed.is_empty() {
        return;
    }
    for node in changed {
        if ctx.revoked.is_revoked(node) {
            enforce(ctx, node);
        }
    }
    ctx.sink.emit(CoreEvent::RevocationsChanged {
        node_ids: changed.to_vec(),
    });
}

/// A removed device, for the apps' list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RemovedDevice {
    pub node_id: String,
    /// Its name when it was removed (the node id if none).
    pub name: String,
    /// Its key's short fingerprint ("" when it never connected with one).
    pub fingerprint: String,
    /// Seconds since it was removed.
    pub removed_ago_s: u32,
    /// Which device removed it: this one, or another (by its name here).
    pub by_me: bool,
    pub by_name: String,
}

/// The removed devices, the most recently removed first.
pub fn removed<P: Platform>(ctx: &NetCtx<P>) -> Vec<RemovedDevice> {
    let me = ctx.identity.lock().node_id.clone();
    let now = now_ms();
    let peers = ctx.peers.lock().clone();
    let mut list: Vec<(u64, RemovedDevice)> = ctx
        .revoked
        .entries()
        .into_iter()
        .filter(|(_, e)| e.revoked)
        .map(|(node, e)| {
            let by_name = peers
                .get(&e.by)
                .map(|p| p.name.clone())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| e.by.clone());
            (
                e.at,
                RemovedDevice {
                    name: if e.name.trim().is_empty() {
                        node.clone()
                    } else {
                        e.name.clone()
                    },
                    fingerprint: if e.key.is_empty() {
                        String::new()
                    } else {
                        crate::device_key::fingerprint(&e.key)
                    },
                    removed_ago_s: u32::try_from(now.saturating_sub(e.at) / 1000)
                        .unwrap_or(u32::MAX),
                    by_me: e.by == me,
                    by_name,
                    node_id: node,
                },
            )
        })
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.node_id.cmp(&b.1.node_id)));
    list.into_iter().map(|(_, d)| d).collect()
}

/// Write and sign an entry for `node` here (`revoked`: removed, or allowed
/// again), newer than any it had.
fn put<P: Platform>(ctx: &NetCtx<P>, node: &str, revoked: bool) -> crate::Result<()> {
    let me = ctx.identity.lock().node_id.clone();
    if !valid_node(node) || node.starts_with("manual:") {
        return Err(CoreError::InvalidInput("which device?".into()));
    }
    if node == me {
        return Err(CoreError::InvalidInput(
            "this device can't remove itself".into(),
        ));
    }
    let prev = ctx.revoked.map.lock().get(node).cloned();
    if !revoked && !prev.as_ref().is_some_and(|e| e.revoked) {
        return Err(CoreError::InvalidInput("that device isn't removed".into()));
    }
    let key = crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, node)
        .or_else(|| {
            prev.as_ref()
                .map(|e| e.key.clone())
                .filter(|k| !k.is_empty())
        })
        .unwrap_or_default();
    let name = ctx
        .peers
        .lock()
        .get(node)
        .map(|p| p.name.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| prev.as_ref().map(|e| e.name.clone()))
        .unwrap_or_default()
        .chars()
        .take(MAX_NAME)
        .collect();
    let mut e = Entry {
        key,
        name,
        revoked,
        at: now_ms().max(prev.as_ref().map_or(0, |e| e.at.saturating_add(1))),
        ..Default::default()
    };
    sign(node, &mut e, &me, &ctx.device_key);
    {
        let mut map = ctx.revoked.map.lock();
        if !map.contains_key(node) && map.len() >= MAX_ENTRIES && !make_room(&mut map) {
            return Err(CoreError::InvalidInput(format!(
                "at most {MAX_ENTRIES} removed devices"
            )));
        }
        map.insert(node.to_string(), e);
        ctx.revoked.save(&*ctx.platform, &map);
    }
    Ok(())
}

/// The user removes `node`: revoked here and on every device of yours that
/// hears it (now, or at its next link).
pub fn remove<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    put(ctx, node, true)?;
    // The others hear it before its links go (it's left out).
    send_all(ctx);
    announce(ctx, &[node.to_string()]);
    Ok(())
}

/// "Allow again": `node` may be found and linked again.
pub fn allow<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    put(ctx, node, false)?;
    ctx.quic_failed.lock().remove(node);
    send_all(ctx);
    announce(ctx, &[node.to_string()]);
    Ok(())
}

/// Say in this device's status that it keeps the list.
pub fn extend_status<P: Platform>(_ctx: &NetCtx<P>, status: &mut Value) {
    status[FLAG] = VERSION.into();
}

/// `node` said (`flag`) whether it keeps the list; newly heard, it gets it.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    if set_caps(ctx, node, on) && on {
        send_to(ctx, node);
    }
}

fn set_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> bool {
    if node.is_empty() || supports(ctx, node) == on {
        return false;
    }
    ctx.revoked.caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
    true
}

/// `node` said it keeps the list.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if let Some(on) = ctx.revoked.caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    ctx.revoked.caps.lock().insert(node.to_string(), on);
    on
}

/// A link to `node` came up: send it the list (when there's one).
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if supports(ctx, node) {
        send_to(ctx, node);
    }
}

fn send_to<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if node.starts_with("manual:") || ctx.revoked.is_revoked(node) || !supports(ctx, node) {
        return;
    }
    let list = ctx.revoked.entries();
    if list.is_empty() {
        return;
    }
    let me = ctx.identity.lock().node_id.clone();
    let msg = json!({ "type": "revocations", "from": me, "list": list }).to_string();
    send_control_to(ctx, node, &msg);
}

fn send_all<P: Platform>(ctx: &NetCtx<P>) {
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    for node in linked {
        send_to(ctx, &node);
    }
}

/// A list as it arrived; entries that can't be read are left out.
fn parse(v: &Value) -> RevocationMap {
    let Some(obj) = v.as_object() else {
        return RevocationMap::new();
    };
    obj.iter()
        .filter_map(|(node, e)| {
            let e = serde_json::from_value::<Entry>(e.clone()).ok()?;
            well_formed(node, &e).then(|| (node.clone(), e))
        })
        .take(MAX_ENTRIES * 2)
        .collect()
}

pub struct RevokeHandler;

impl<P: Platform> ControlHandler<P> for RevokeHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["revocations"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !super::known(ctx, &from) {
            return; // only your own devices
        }
        set_caps(ctx, &from, true);
        let theirs = parse(&msg.json("list"));
        let changed = merge(ctx, theirs.clone());
        announce(ctx, &changed);
        // Anything newer here (or that it lacks): answer once.
        let answer = {
            let ours = ctx.revoked.map.lock();
            ours.iter()
                .any(|(n, e)| theirs.get(n).is_none_or(|t| e.wins_over(t)))
        };
        if answer && !ctx.revoked.is_revoked(&from) {
            send_to(ctx, &from);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_key::DeviceKey;

    fn entry(revoked: bool, at: u64) -> Entry {
        Entry {
            key: "k".into(),
            name: "Old phone".into(),
            revoked,
            at,
            ..Default::default()
        }
    }

    #[test]
    fn a_signed_entry_checks_out_and_any_change_breaks_it() {
        let key = DeviceKey::ephemeral();
        let mut e = entry(true, 10);
        sign("gone", &mut e, "me", &key);
        assert!(signature_ok("gone", &e));
        assert!(!signature_ok("other", &e), "another node");
        let tweak: [fn(&mut Entry); 5] = [
            |e| e.revoked = false,
            |e| e.at += 1,
            |e| e.key = "x".into(),
            |e| e.name = "x".into(),
            |e| e.by = "x".into(),
        ];
        for t in tweak {
            let mut f = e.clone();
            t(&mut f);
            assert!(!signature_ok("gone", &f), "{f:?}");
        }
        // Signed by another key than it names.
        let mut f = e.clone();
        f.by_key = DeviceKey::ephemeral().public();
        assert!(!signature_ok("gone", &f));
    }

    #[test]
    fn the_newest_entry_wins() {
        let removed = entry(true, 10);
        let allowed = entry(false, 20);
        assert!(allowed.wins_over(&removed));
        assert!(!removed.wins_over(&allowed));
        // The same time: removed wins (and every device agrees).
        assert!(entry(true, 20).wins_over(&allowed));
    }

    #[test]
    fn badly_formed_entries_are_left_out() {
        let mut e = entry(true, 1);
        e.by = "a".into();
        assert!(well_formed("b", &e));
        assert!(!well_formed("a", &e), "names its signer");
        assert!(!well_formed("", &e));
        assert!(!well_formed("manual:1.2.3.4", &e));
        e.name = "x".repeat(MAX_NAME + 1);
        assert!(!well_formed("b", &e));
    }
}
