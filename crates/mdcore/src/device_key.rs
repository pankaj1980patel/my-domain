//! A key of this device's own (Phase 4, ADR-0005), and the keys it has seen
//! other devices use.
//!
//! Each install creates an Ed25519 key once and keeps it: in the OS secure
//! store (Keychain, Android Keystore, Secret Service) when the platform has
//! one, else in its kv store (mdctl). Signing out keeps it: it belongs to
//! the install, not the account. Its QUIC certificate is made
//! from that key, and both ends of a QUIC connection present one, so the
//! handshake proves which key the other side holds.
//!
//! Trust on first use: the first time a device connects, its key is
//! remembered under its node id. From then on, a connection claiming that
//! node id with another key, or with none, is refused. A device from before
//! device keys (a throwaway certificate) is still let in, and isn't
//! remembered, until it updates.
//!
//! A device that's reinstalled (or has its data cleared) comes back with a
//! new key. Its connections are still refused (it could be someone else
//! using its id), but the refusal is recorded (`Change`: the key it had,
//! the one it came with, when, how often) and the user is told once per new
//! key. Only the user can trust it (`trust`), and only the exact key
//! recorded is then remembered: a third key is refused again. Records are
//! kept across restarts (kv `peer_key_changes`), at most `MAX_CHANGES`.
//!
//! Verifying a device: both show their own key's short fingerprint
//! (`fingerprint`) and, per pair, a safety number made from both keys
//! (`safety_number`, the same on both devices) and a QR code
//! (`VerifyCode`). When the user says they match, or this device scans
//! the other's code and it checks out, the device is marked verified: the
//! key verified is kept next to its pin (kv `peer_keys_verified`), so it
//! counts only while that key is the one remembered. Trusting a new key
//! clears it; a key change says whether the old key had been verified.

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use parking_lot::Mutex;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use crate::platform::Platform;

/// Where this device's key is kept (PKCS#8, base64).
const KEY_KV: &str = "identity_key";
/// kv key holding our public key: says a key was made before, so a secure
/// store that can't be read (a locked keyring) doesn't get it replaced.
const PUBLIC_KV: &str = "identity_key_public";
/// Where the first build (2026-11-03) kept it by mistake: the name the
/// session uses for the passphrase key (`engine::session`), which it
/// overwrote.
const MISPLACED_KV: &str = "device_key";
/// kv key holding the keys seen per device (JSON: node id -> base64 key).
const PINS_KV: &str = "peer_keys";
/// kv key holding the refused key changes (JSON: node id -> `Change`).
const CHANGES_KV: &str = "peer_key_changes";
/// kv key holding the keys the user verified (JSON: node id -> base64 key).
const VERIFIED_KV: &str = "peer_keys_verified";
/// What a verify QR code starts with (opened by the phone's camera too).
pub const VERIFY_PREFIX: &str = "mydomain://verify?";
/// The most key changes kept (the one heard of longest ago goes first).
pub const MAX_CHANGES: usize = 32;

/// The DER header of an Ed25519 public key in a certificate
/// (SubjectPublicKeyInfo: algorithm 1.3.101.112, then a 32-byte bit string).
const ED25519_SPKI: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

pub struct DeviceKey {
    pkcs8: Vec<u8>,
    public: Vec<u8>,
}

impl DeviceKey {
    /// This device's key: the stored one, or a new one (then stored).
    pub fn load_or_create(store: &dyn Store) -> DeviceKey {
        Self::rescue_misplaced(store);
        if let Some(key) = store
            .secure_get(KEY_KV)
            .or_else(|| store.get(KEY_KV))
            .and_then(|s| B64.decode(s).ok())
            .and_then(Self::from_pkcs8)
        {
            return key;
        }
        let key = Self::generate();
        if store.get(PUBLIC_KV).is_some() {
            // We have a key, but its store can't be read right now. Use this
            // one for this run only (peers will refuse it) and keep the real
            // one for the next start.
            return key;
        }
        let value = B64.encode(&key.pkcs8);
        store.secure_set(KEY_KV, &value);
        // No secure store here (it kept nothing): the kv store, then.
        if store.secure_get(KEY_KV).as_deref() != Some(value.as_str()) {
            store.set(KEY_KV, &value);
        }
        store.set(PUBLIC_KV, &key.public());
        key
    }

    /// A key the first build put in the passphrase key's place: move it to
    /// its own name (peers may have remembered it) and clear that place, so
    /// the app asks for the passphrase again instead of failing to read it.
    fn rescue_misplaced(store: &dyn Store) {
        let is_ours = |v: &String| B64.decode(v).ok().and_then(Self::from_pkcs8).is_some();
        if store.secure_get(KEY_KV).is_none() {
            if let Some(v) = store.secure_get(MISPLACED_KV).filter(is_ours) {
                store.secure_set(KEY_KV, &v);
                if store.secure_get(KEY_KV).as_deref() == Some(v.as_str()) {
                    store.secure_clear(MISPLACED_KV);
                }
            }
        }
        if store.get(KEY_KV).is_none() {
            if let Some(v) = store.get(MISPLACED_KV).filter(is_ours) {
                store.set(KEY_KV, &v);
            }
        }
    }

    fn generate() -> DeviceKey {
        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .unwrap_or_else(|e| panic!("ed25519 key generation failed: {e}"));
        DeviceKey {
            pkcs8: pair.serialize_der(),
            public: pair.public_key_raw().to_vec(),
        }
    }

    fn from_pkcs8(der: Vec<u8>) -> Option<DeviceKey> {
        let pair = rcgen::KeyPair::try_from(der.as_slice()).ok()?;
        Some(DeviceKey {
            public: pair.public_key_raw().to_vec(),
            pkcs8: der,
        })
    }

    /// The public key, base64: what other devices remember us by.
    pub fn public(&self) -> String {
        B64.encode(&self.public)
    }

    /// A self-signed certificate for this key (for QUIC, both directions).
    pub fn certificate(&self) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), String> {
        let pair = rcgen::KeyPair::try_from(self.pkcs8.as_slice()).map_err(|e| e.to_string())?;
        let cert = rcgen::CertificateParams::new(vec!["mydomain".into()])
            .and_then(|p| p.self_signed(&pair))
            .map_err(|e| e.to_string())?;
        let key = PrivatePkcs8KeyDer::from(self.pkcs8.clone());
        Ok((cert.der().clone(), key.into()))
    }
}

/// The Ed25519 key in a certificate, base64; `None` for any other kind
/// (a device from before device keys).
pub fn key_in_cert(cert: &[u8]) -> Option<String> {
    let at = cert
        .windows(ED25519_SPKI.len())
        .position(|w| w == ED25519_SPKI)?;
    let key = cert.get(at + ED25519_SPKI.len()..at + ED25519_SPKI.len() + 32)?;
    Some(B64.encode(key))
}

/// What to do with a connection from a device (`check`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Go ahead (the key it had, a first meeting, or a device from before
    /// device keys).
    Accept,
    /// Refused: it had a key and came with none (no downgrade).
    Refuse,
    /// Refused: it came with another key than the one remembered, now
    /// recorded. `first`: the first time this key turned up (tell the user).
    Changed { first: bool },
}

/// A refused key change, as kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Change {
    /// The key remembered (base64).
    old: String,
    /// The key it came with (base64): the only one `trust` will remember.
    new: String,
    first_at: u64,
    last_at: u64,
    count: u32,
    /// "Not now": keep refusing, stop asking (until another key turns up).
    #[serde(default)]
    dismissed: bool,
}

/// A device whose key changed, for the apps (`Engine::key_changes`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KeyChange {
    pub node_id: String,
    /// Its name as this device knows it (the node id when it doesn't).
    pub name: String,
    /// Short fingerprints of the key it had and the one it came with.
    pub old_fingerprint: String,
    pub new_fingerprint: String,
    /// Seconds since it first, and last, came with the new key.
    pub first_ago_s: u32,
    pub last_ago_s: u32,
    /// Connections refused with that key.
    pub attempts: u32,
    /// The user said "Not now" (still refused, not asked again).
    pub dismissed: bool,
    /// The key it had was verified by the user: be careful.
    pub was_verified: bool,
}

/// The keys other devices used, and the changes refused (both loaded from
/// the kv store on first use).
#[derive(Default)]
pub struct PeerKeys {
    pins: Mutex<Option<HashMap<String, String>>>,
    changes: Mutex<Option<HashMap<String, Change>>>,
    verified: Mutex<Option<HashMap<String, String>>>,
}

/// The remembered keys (shared by the engine and its threads).
pub type Pins = Arc<PeerKeys>;

fn load<T: serde::de::DeserializeOwned + Default>(store: &dyn Store, key: &str) -> T {
    store
        .get(key)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(store: &dyn Store, key: &str, value: &T) {
    if let Ok(json) = serde_json::to_string(value) {
        store.set(key, &json);
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A short fingerprint of a key (base64) for people to compare: the first
/// 8 bytes of its SHA-256, as four groups of four hex digits.
pub fn fingerprint(key: &str) -> String {
    let raw = B64.decode(key).unwrap_or_else(|_| key.as_bytes().to_vec());
    let hash = Sha256::digest(&raw);
    hash[..8]
        .chunks(2)
        .map(|c| format!("{:02X}{:02X}", c[0], c[1]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a connection from `node_id` holding `key` may go ahead (and
/// remember the key the first time).
pub fn accept(pins: &Pins, store: &dyn Store, node_id: &str, key: Option<&str>) -> bool {
    check(pins, store, node_id, key) == Verdict::Accept
}

/// What to do with a connection from `node_id` holding `key`: remember the
/// key the first time, record another key (see `Change`).
pub fn check(pins: &Pins, store: &dyn Store, node_id: &str, key: Option<&str>) -> Verdict {
    let known = {
        let mut map = pins.pins.lock();
        let map = map.get_or_insert_with(|| load(store, PINS_KV));
        match (map.get(node_id), key) {
            (Some(known), Some(key)) if known == key => return Verdict::Accept,
            (Some(known), Some(_)) => known.clone(),
            (Some(_), None) => return Verdict::Refuse, // it had a key before: no downgrade
            (None, Some(key)) => {
                map.insert(node_id.to_string(), key.to_string());
                save(store, PINS_KV, &*map);
                return Verdict::Accept;
            }
            (None, None) => return Verdict::Accept, // not updated yet
        }
    };
    let Some(key) = key else {
        return Verdict::Refuse;
    };
    let now = now_secs();
    let mut changes = pins.changes.lock();
    let changes = changes.get_or_insert_with(|| load(store, CHANGES_KV));
    let first = match changes.get_mut(node_id) {
        Some(c) if c.new == key && c.old == known => {
            c.last_at = now;
            c.count = c.count.saturating_add(1);
            false
        }
        _ => {
            if !changes.contains_key(node_id) && changes.len() >= MAX_CHANGES {
                let oldest = changes
                    .iter()
                    .min_by_key(|(_, c)| c.last_at)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    changes.remove(&k);
                }
            }
            changes.insert(
                node_id.to_string(),
                Change {
                    old: known,
                    new: key.to_string(),
                    first_at: now,
                    last_at: now,
                    count: 1,
                    dismissed: false,
                },
            );
            true
        }
    };
    save(store, CHANGES_KV, &*changes);
    Verdict::Changed { first }
}

/// The key changes refused so far, newest first; `name` names a node id.
pub fn changes(pins: &Pins, store: &dyn Store, name: impl Fn(&str) -> String) -> Vec<KeyChange> {
    let now = now_secs();
    let verified = pins
        .verified
        .lock()
        .get_or_insert_with(|| load(store, VERIFIED_KV))
        .clone();
    let ago = |t: u64| u32::try_from(now.saturating_sub(t)).unwrap_or(u32::MAX);
    let mut list: Vec<(u64, KeyChange)> = pins
        .changes
        .lock()
        .get_or_insert_with(|| load(store, CHANGES_KV))
        .iter()
        .map(|(node, c)| {
            (
                c.last_at,
                KeyChange {
                    node_id: node.clone(),
                    name: name(node),
                    old_fingerprint: fingerprint(&c.old),
                    new_fingerprint: fingerprint(&c.new),
                    first_ago_s: ago(c.first_at),
                    last_ago_s: ago(c.last_at),
                    attempts: c.count,
                    dismissed: c.dismissed,
                    was_verified: verified.get(node) == Some(&c.old),
                },
            )
        })
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.node_id.cmp(&b.1.node_id)));
    list.into_iter().map(|(_, c)| c).collect()
}

/// `node_id` has a key change recorded (waiting, or dismissed): its
/// connections are being refused for it.
pub fn has_change(pins: &Pins, store: &dyn Store, node_id: &str) -> bool {
    pins.changes
        .lock()
        .get_or_insert_with(|| load(store, CHANGES_KV))
        .contains_key(node_id)
}

/// The user trusts `node_id`'s new key: remember the key recorded for it
/// (only that one) in place of the old. `new_fingerprint` is the one the
/// user was shown: when another key turned up since, nothing changes.
pub fn trust(
    pins: &Pins,
    store: &dyn Store,
    node_id: &str,
    new_fingerprint: &str,
) -> Result<(), String> {
    let mut changes = pins.changes.lock();
    let changes = changes.get_or_insert_with(|| load(store, CHANGES_KV));
    let Some(change) = changes.get(node_id) else {
        return Err("no key change for this device".into());
    };
    if fingerprint(&change.new) != new_fingerprint.trim() {
        return Err("its key changed again: check the new fingerprint".into());
    }
    let new = change.new.clone();
    {
        let mut map = pins.pins.lock();
        let map = map.get_or_insert_with(|| load(store, PINS_KV));
        map.insert(node_id.to_string(), new);
        save(store, PINS_KV, &*map);
    }
    changes.remove(node_id);
    save(store, CHANGES_KV, &*changes);
    // A new key isn't verified: the user compares again.
    let mut verified = pins.verified.lock();
    let verified = verified.get_or_insert_with(|| load(store, VERIFIED_KV));
    if verified.remove(node_id).is_some() {
        save(store, VERIFIED_KV, &*verified);
    }
    Ok(())
}

/// The key remembered for `node_id` (base64), if it connected with one.
pub fn pinned(pins: &Pins, store: &dyn Store, node_id: &str) -> Option<String> {
    pins.pins
        .lock()
        .get_or_insert_with(|| load(store, PINS_KV))
        .get(node_id)
        .cloned()
}

/// The user verified `node_id`'s key, and it's still the one remembered.
pub fn is_verified(pins: &Pins, store: &dyn Store, node_id: &str) -> bool {
    let Some(pin) = pinned(pins, store, node_id) else {
        return false;
    };
    pins.verified
        .lock()
        .get_or_insert_with(|| load(store, VERIFIED_KV))
        .get(node_id)
        == Some(&pin)
}

/// Mark `node_id` verified (its remembered key: only while that one stays
/// remembered) or not. Fails when no key is remembered for it, or its key
/// changed and that isn't settled.
pub fn set_verified(pins: &Pins, store: &dyn Store, node_id: &str, on: bool) -> Result<(), String> {
    // Checked before taking the verified lock (`trust` takes the locks the
    // other way round).
    let pin = if on {
        if has_change(pins, store, node_id) {
            return Err("its security key changed: settle that first".into());
        }
        let Some(pin) = pinned(pins, store, node_id) else {
            return Err("it hasn't connected with a security key yet".into());
        };
        Some(pin)
    } else {
        None
    };
    let mut verified = pins.verified.lock();
    let verified = verified.get_or_insert_with(|| load(store, VERIFIED_KV));
    match pin {
        Some(pin) => {
            verified.insert(node_id.to_string(), pin);
        }
        None if verified.remove(node_id).is_none() => return Ok(()),
        None => {}
    }
    save(store, VERIFIED_KV, &*verified);
    Ok(())
}

/// The safety number of two devices' keys (base64): 12 groups of 5 digits,
/// the same whichever device works it out (the keys are sorted first).
/// Equal on both devices only when each holds the other's real key.
pub fn safety_number(a: &str, b: &str) -> String {
    let raw = |k: &str| B64.decode(k).unwrap_or_else(|_| k.as_bytes().to_vec());
    let (mut lo, mut hi) = (raw(a), raw(b));
    if hi < lo {
        std::mem::swap(&mut lo, &mut hi);
    }
    let mut h = Sha512::new();
    h.update(b"my-domain safety number v1");
    for k in [&lo, &hi] {
        h.update(u32::try_from(k.len()).unwrap_or(u32::MAX).to_be_bytes());
        h.update(k);
    }
    let hash = h.finalize();
    hash[..60]
        .chunks(5)
        .map(|c| {
            let n = c.iter().fold(0u64, |n, &b| (n << 8) | u64::from(b));
            format!("{:05}", n % 100_000)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A verify QR code: the device showing it (`from`, its key) and the one
/// meant to scan it (`to`, the key the shower remembers for it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyCode {
    pub from: String,
    pub from_key: String,
    pub to: String,
    pub to_key: String,
}

impl VerifyCode {
    pub fn to_uri(&self) -> String {
        use crate::join::pct_encode as e;
        format!(
            "{VERIFY_PREFIX}v=1&f={}&fk={}&t={}&tk={}",
            e(&self.from),
            e(&self.from_key),
            e(&self.to),
            e(&self.to_key)
        )
    }

    /// Read a scanned code; `None` when it isn't one (or is damaged).
    pub fn parse(uri: &str) -> Option<VerifyCode> {
        let uri = uri.trim();
        let query = uri
            .get(..VERIFY_PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(VERIFY_PREFIX))
            .map(|_| &uri[VERIFY_PREFIX.len()..])?;
        let (mut v, mut f, mut fk, mut t, mut tk) = (None, None, None, None, None);
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=')?;
            let value = crate::join::pct_decode(value)?;
            match name {
                "v" => v = Some(value),
                "f" => f = Some(value),
                "fk" => fk = Some(value),
                "t" => t = Some(value),
                "tk" => tk = Some(value),
                _ => {} // newer fields are ignored
            }
        }
        if v.as_deref() != Some("1") {
            return None;
        }
        let key = |k: Option<String>| k.filter(|k| B64.decode(k).is_ok_and(|r| r.len() == 32));
        let id = |n: Option<String>| n.filter(|n| !n.trim().is_empty());
        Some(VerifyCode {
            from: id(f)?,
            from_key: key(fk)?,
            to: id(t)?,
            to_key: key(tk)?,
        })
    }
}

/// What came of scanning a verify code (`Engine::verify_scanned`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VerifyScan {
    /// The device now verified (none: nothing verified, see `problem`).
    pub node_id: Option<String>,
    /// "not_a_code" (not a verify code), "not_for_this_device" (made for
    /// another device), "unknown_device" (not one of yours, or it hasn't
    /// connected with a key yet), "key_changed" (its key change isn't
    /// settled), "mismatch" (the keys don't match what each device holds:
    /// don't trust it).
    pub problem: Option<String>,
}

/// Check a scanned verify code against this device (`me`, `my_key`) and
/// the keys remembered, and mark its device verified when it all matches.
pub fn check_scan(
    pins: &Pins,
    store: &dyn Store,
    me: &str,
    my_key: &str,
    text: &str,
) -> VerifyScan {
    let fail = |p: &str| VerifyScan {
        node_id: None,
        problem: Some(p.to_string()),
    };
    let Some(code) = VerifyCode::parse(text) else {
        return fail("not_a_code");
    };
    if code.to != me {
        return fail("not_for_this_device");
    }
    if has_change(pins, store, &code.from) {
        return fail("key_changed");
    }
    let Some(pin) = pinned(pins, store, &code.from) else {
        return fail("unknown_device");
    };
    if pin != code.from_key || code.to_key != my_key {
        return fail("mismatch");
    }
    match set_verified(pins, store, &code.from, true) {
        Ok(()) => VerifyScan {
            node_id: Some(code.from),
            problem: None,
        },
        Err(_) => fail("key_changed"),
    }
}

/// "Not now": keep refusing `node_id`'s new key and stop asking about it
/// (another key would ask again). False: no change recorded for it.
pub fn dismiss(pins: &Pins, store: &dyn Store, node_id: &str) -> bool {
    let mut changes = pins.changes.lock();
    let changes = changes.get_or_insert_with(|| load(store, CHANGES_KV));
    let Some(c) = changes.get_mut(node_id) else {
        return false;
    };
    c.dismissed = true;
    save(store, CHANGES_KV, &*changes);
    true
}

/// The storage this module needs (a `Platform` is one).
pub trait Store {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str);
    /// The OS secure store; keeps nothing where there is none.
    fn secure_get(&self, _key: &str) -> Option<String> {
        None
    }
    fn secure_set(&self, _key: &str, _value: &str) {}
    fn secure_clear(&self, _key: &str) {}
}

impl<P: Platform> Store for P {
    fn get(&self, key: &str) -> Option<String> {
        self.kv_get(key)
    }
    fn set(&self, key: &str, value: &str) {
        self.kv_set(key, value)
    }
    fn secure_get(&self, key: &str) -> Option<String> {
        self.secret_get(key)
    }
    fn secure_set(&self, key: &str, value: &str) {
        self.secret_set(key, Some(value))
    }
    fn secure_clear(&self, key: &str) {
        self.secret_set(key, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// kv only (like mdctl).
    #[derive(Default)]
    struct Mem(Mutex<HashMap<String, String>>);
    impl Store for Mem {
        fn get(&self, key: &str) -> Option<String> {
            self.0.lock().get(key).cloned()
        }
        fn set(&self, key: &str, value: &str) {
            self.0.lock().insert(key.into(), value.into());
        }
    }

    /// kv plus a secure store (like the apps).
    #[derive(Default)]
    struct WithSecrets(Mem, Mem);
    impl Store for WithSecrets {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key)
        }
        fn set(&self, key: &str, value: &str) {
            self.0.set(key, value)
        }
        fn secure_get(&self, key: &str) -> Option<String> {
            self.1.get(key)
        }
        fn secure_set(&self, key: &str, value: &str) {
            self.1.set(key, value)
        }
        fn secure_clear(&self, key: &str) {
            self.1 .0.lock().remove(key);
        }
    }

    #[test]
    fn the_session_key_is_left_alone() {
        let store = WithSecrets::default();
        let session_key = "ab".repeat(32); // how the session stores it (hex)
        store.secure_set(MISPLACED_KV, &session_key);
        let key = DeviceKey::load_or_create(&store);
        assert_eq!(store.secure_get(MISPLACED_KV), Some(session_key));
        assert_eq!(DeviceKey::load_or_create(&store).public(), key.public());
    }

    #[test]
    fn a_misplaced_key_is_moved_and_its_place_cleared() {
        let store = WithSecrets::default();
        let old = DeviceKey::generate();
        store.secure_set(MISPLACED_KV, &B64.encode(&old.pkcs8));
        let key = DeviceKey::load_or_create(&store);
        assert_eq!(key.public(), old.public(), "peers remembered this one");
        assert_eq!(store.secure_get(MISPLACED_KV), None);
    }

    #[test]
    fn the_key_goes_to_the_secure_store_when_there_is_one() {
        let store = WithSecrets::default();
        let key = DeviceKey::load_or_create(&store);
        assert!(store.1.get(KEY_KV).is_some());
        assert!(store.0.get(KEY_KV).is_none(), "not in plain kv");
        assert_eq!(DeviceKey::load_or_create(&store).public(), key.public());
    }

    #[test]
    fn an_unreadable_secure_store_doesnt_replace_the_key() {
        let store = WithSecrets::default();
        let key = DeviceKey::load_or_create(&store);
        let stored = store.1.get(KEY_KV);
        // Locked keyring: nothing readable.
        store.1 .0.lock().clear();
        let this_run = DeviceKey::load_or_create(&store);
        assert_ne!(this_run.public(), key.public());
        assert!(store.1.get(KEY_KV).is_none(), "not overwritten");
        // Unlocked again: the real key is back.
        store.1.set(KEY_KV, stored.as_deref().unwrap());
        assert_eq!(DeviceKey::load_or_create(&store).public(), key.public());
    }

    #[test]
    fn the_key_is_kept_and_its_certificate_carries_it() {
        let kv = Mem::default();
        let a = DeviceKey::load_or_create(&kv);
        let again = DeviceKey::load_or_create(&kv);
        assert_eq!(a.public(), again.public());
        let (cert, _) = a.certificate().unwrap();
        assert_eq!(
            key_in_cert(cert.as_ref()).as_deref(),
            Some(a.public().as_str())
        );
        let other = DeviceKey::load_or_create(&Mem::default());
        assert_ne!(a.public(), other.public());
    }

    #[test]
    fn an_old_style_certificate_has_no_device_key() {
        let cert = rcgen::generate_simple_self_signed(vec!["mydomain".into()]).unwrap();
        assert_eq!(key_in_cert(cert.cert.der()), None);
    }

    #[test]
    fn the_first_key_is_remembered_and_others_are_refused() {
        let (kv, pins) = (Mem::default(), Pins::default());
        assert!(accept(&pins, &kv, "phone", None), "not updated yet");
        assert!(accept(&pins, &kv, "phone", Some("k1")));
        assert!(accept(&pins, &kv, "phone", Some("k1")));
        assert!(!accept(&pins, &kv, "phone", Some("k2")), "another key");
        assert!(!accept(&pins, &kv, "phone", None), "no downgrade");
        // Remembered across restarts (fresh pins, same store).
        assert!(!accept(&Pins::default(), &kv, "phone", Some("k2")));
        assert!(accept(&Pins::default(), &kv, "phone", Some("k1")));
    }

    #[test]
    fn a_new_key_is_recorded_once_and_only_it_can_be_trusted() {
        let (kv, pins) = (Mem::default(), Pins::default());
        assert_eq!(check(&pins, &kv, "phone", Some("k1")), Verdict::Accept);
        assert_eq!(
            check(&pins, &kv, "phone", Some("k2")),
            Verdict::Changed { first: true }
        );
        assert_eq!(
            check(&pins, &kv, "phone", Some("k2")),
            Verdict::Changed { first: false },
            "once per key, not per attempt"
        );
        let list = changes(&pins, &kv, |n| n.to_uppercase());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "PHONE");
        assert_eq!(list[0].attempts, 2);
        assert_eq!(list[0].old_fingerprint, fingerprint("k1"));
        assert_eq!(list[0].new_fingerprint, fingerprint("k2"));
        // Kept across restarts.
        let again = Pins::default();
        assert_eq!(changes(&again, &kv, str::to_string).len(), 1);
        assert_eq!(
            check(&again, &kv, "phone", Some("k2")),
            Verdict::Changed { first: false }
        );

        // Not now: still refused, not asked again.
        assert!(dismiss(&pins, &kv, "phone"));
        assert!(changes(&pins, &kv, str::to_string)[0].dismissed);
        assert_eq!(
            check(&pins, &kv, "phone", Some("k2")),
            Verdict::Changed { first: false }
        );
        // A third key asks again, and trusting needs the fingerprint shown.
        assert_eq!(
            check(&pins, &kv, "phone", Some("k3")),
            Verdict::Changed { first: true }
        );
        assert!(trust(&pins, &kv, "phone", &fingerprint("k2")).is_err());
        assert!(!accept(&pins, &kv, "phone", Some("k3")));
        trust(&pins, &kv, "phone", &fingerprint("k3")).unwrap();
        assert!(changes(&pins, &kv, str::to_string).is_empty());
        assert!(accept(&Pins::default(), &kv, "phone", Some("k3")));
        assert!(
            !accept(&pins, &kv, "phone", Some("k1")),
            "the old one is gone"
        );
        // ... and is a new change to ask about, from the trusted key.
        assert_eq!(
            changes(&pins, &kv, str::to_string)[0].old_fingerprint,
            fingerprint("k3")
        );
        assert!(!dismiss(&pins, &kv, "laptop"));
    }

    #[test]
    fn key_changes_are_capped() {
        let (kv, pins) = (Mem::default(), Pins::default());
        for i in 0..MAX_CHANGES + 5 {
            let node = format!("n{i}");
            assert!(accept(&pins, &kv, &node, Some("a")));
            assert!(!accept(&pins, &kv, &node, Some("b")));
        }
        assert_eq!(changes(&pins, &kv, str::to_string).len(), MAX_CHANGES);
    }

    fn b64(seed: u8) -> String {
        B64.encode([seed; 32])
    }

    #[test]
    fn the_safety_number_is_symmetric_and_stable() {
        let (a, b) = (b64(1), b64(2));
        let n = safety_number(&a, &b);
        assert_eq!(n, safety_number(&b, &a), "the same on both devices");
        let groups: Vec<&str> = n.split(' ').collect();
        assert_eq!(groups.len(), 12);
        assert!(groups
            .iter()
            .all(|g| g.len() == 5 && g.bytes().all(|c| c.is_ascii_digit())));
        assert_ne!(n, safety_number(&a, &b64(3)), "another key, another number");
        assert_ne!(n, safety_number(&b64(3), &b));
        // Fixed: an update must never change what users compare.
        assert_eq!(n, SAFETY_1_2);
    }

    /// `safety_number(b64(1), b64(2))`, as first released.
    const SAFETY_1_2: &str =
        "83209 04899 17154 39681 79009 07377 50615 33206 92703 06688 20318 89074";

    #[test]
    fn verified_counts_only_for_the_key_verified() {
        let (kv, pins) = (Mem::default(), Pins::default());
        assert!(
            set_verified(&pins, &kv, "phone", true).is_err(),
            "no key yet"
        );
        assert!(accept(&pins, &kv, "phone", Some("k1")));
        assert!(!is_verified(&pins, &kv, "phone"));
        set_verified(&pins, &kv, "phone", true).unwrap();
        assert!(is_verified(&pins, &kv, "phone"));
        assert!(is_verified(&Pins::default(), &kv, "phone"), "kept");

        // Its key changes: the change says it was verified, and it can't be
        // verified again until that's settled.
        assert!(!accept(&pins, &kv, "phone", Some("k2")));
        assert!(changes(&pins, &kv, str::to_string)[0].was_verified);
        assert!(set_verified(&pins, &kv, "phone", true).is_err());
        trust(&pins, &kv, "phone", &fingerprint("k2")).unwrap();
        assert!(
            !is_verified(&pins, &kv, "phone"),
            "cleared with the new key"
        );
        assert!(!is_verified(&Pins::default(), &kv, "phone"));
        // A change from a key never verified says so.
        assert!(!accept(&pins, &kv, "phone", Some("k3")));
        assert!(!changes(&pins, &kv, str::to_string)[0].was_verified);

        assert!(accept(&pins, &kv, "laptop", Some("k1")));
        set_verified(&pins, &kv, "laptop", true).unwrap();
        set_verified(&pins, &kv, "laptop", false).unwrap();
        assert!(!is_verified(&pins, &kv, "laptop"));
        set_verified(&pins, &kv, "laptop", false).unwrap();
    }

    #[test]
    fn a_verify_code_reads_back() {
        let code = VerifyCode {
            from: "desk top+1".into(),
            from_key: b64(1),
            to: "phone".into(),
            to_key: b64(251),
        };
        let uri = code.to_uri();
        assert!(uri.starts_with(VERIFY_PREFIX));
        assert_eq!(VerifyCode::parse(&format!(" {uri}\n")), Some(code.clone()));
        assert_eq!(VerifyCode::parse("mydomain://join?v=1"), None);
        assert_eq!(VerifyCode::parse(&uri.replace("v=1", "v=2")), None);
        assert_eq!(VerifyCode::parse(&uri.replace("&t=phone", "")), None);
        let short = uri.replace(&crate::join::pct_encode(&b64(251)), "AAAA");
        assert_eq!(VerifyCode::parse(&short), None, "not a key");
    }

    #[test]
    fn a_scanned_code_verifies_only_the_right_device() {
        let (kv, pins) = (Mem::default(), Pins::default());
        let (desk, phone) = (b64(1), b64(2));
        assert!(accept(&pins, &kv, "desk", Some(&desk)));
        let code = |from: &str, fk: &str, to: &str, tk: &str| {
            VerifyCode {
                from: from.into(),
                from_key: fk.into(),
                to: to.into(),
                to_key: tk.into(),
            }
            .to_uri()
        };
        let scan = |text: &str| check_scan(&pins, &kv, "phone", &phone, text);
        let problem = |text: &str| scan(text).problem.unwrap_or_default();
        assert_eq!(problem("hello"), "not_a_code");
        assert_eq!(
            problem(&code("desk", &desk, "tablet", &phone)),
            "not_for_this_device"
        );
        assert_eq!(
            problem(&code("other", &desk, "phone", &phone)),
            "unknown_device"
        );
        assert_eq!(problem(&code("desk", &b64(9), "phone", &phone)), "mismatch");
        assert_eq!(problem(&code("desk", &desk, "phone", &b64(9))), "mismatch");
        assert!(!is_verified(&pins, &kv, "desk"));
        let ok = scan(&code("desk", &desk, "phone", &phone));
        assert_eq!(ok.node_id.as_deref(), Some("desk"));
        assert_eq!(ok.problem, None);
        assert!(is_verified(&pins, &kv, "desk"));
        // A key change waiting: no verifying until it's settled.
        assert!(!accept(&pins, &kv, "desk", Some(&b64(5))));
        assert_eq!(
            problem(&code("desk", &b64(5), "phone", &phone)),
            "key_changed"
        );
    }

    #[test]
    fn fingerprints_are_short_and_stable() {
        let fp = fingerprint("AAAA");
        assert_eq!(fp.len(), 19);
        assert_eq!(fp, fingerprint("AAAA"));
        assert_ne!(fp, fingerprint("AAAB"));
    }
}
