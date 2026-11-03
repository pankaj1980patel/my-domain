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

use std::collections::HashMap;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use parking_lot::Mutex;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

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

/// Whether a connection from `node_id` holding `key` may go ahead, and
/// remember the key the first time.
pub fn accept(pins: &Pins, store: &dyn Store, node_id: &str, key: Option<&str>) -> bool {
    let mut map = pins.lock();
    let map = map.get_or_insert_with(|| {
        store
            .get(PINS_KV)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    match (map.get(node_id), key) {
        (Some(known), Some(key)) => known == key,
        (Some(_), None) => false, // it had a key before: no downgrade
        (None, Some(key)) => {
            map.insert(node_id.to_string(), key.to_string());
            if let Ok(json) = serde_json::to_string(&*map) {
                store.set(PINS_KV, &json);
            }
            true
        }
        (None, None) => true, // not updated yet
    }
}

/// The remembered keys, loaded from the kv store on first use.
pub type Pins = Arc<Mutex<Option<HashMap<String, String>>>>;

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
}
