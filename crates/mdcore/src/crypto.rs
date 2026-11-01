//! End-to-end encryption: a user passphrase → Argon2id → XChaCha20-Poly1305.
//! The server never sees plaintext or keys.

use base64::{engine::general_purpose::STANDARD, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};

use crate::model::Envelope;

/// Derive the 32-byte message key from the user's passphrase. The salt is bound
/// to the username so the same passphrase yields the same key on all of the
/// user's devices (and differs between users).
pub fn derive_key(passphrase: &str, username: &str) -> Option<[u8; 32]> {
    let salt = format!("my-domain-e2ee:{username}");
    let mut key = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), &mut key)
        .ok()?;
    Some(key)
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Option<Envelope> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).ok()?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher.encrypt(XNonce::from_slice(&nonce), plaintext).ok()?;
    Some(Envelope {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ct),
    })
}

pub fn decrypt(key: &[u8; 32], env: &Envelope) -> Option<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).ok()?;
    let nonce = STANDARD.decode(&env.nonce).ok()?;
    if nonce.len() != 24 {
        return None;
    }
    let ct = STANDARD.decode(&env.ciphertext).ok()?;
    cipher.decrypt(XNonce::from_slice(&nonce), ct.as_ref()).ok()
}

/// Generate a random 24-byte key, base64-encoded (UI "Generate" helper).
pub fn generate_key() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = derive_key("pass", "alice").unwrap();
        let env = encrypt(&key, b"hello").unwrap();
        assert_eq!(decrypt(&key, &env).unwrap(), b"hello");
    }

    #[test]
    fn same_passphrase_and_user_give_same_key_on_every_device() {
        assert_eq!(derive_key("pass", "alice"), derive_key("pass", "alice"));
    }

    #[test]
    fn username_is_part_of_the_salt() {
        assert_ne!(derive_key("pass", "alice"), derive_key("pass", "bob"));
    }

    #[test]
    fn wrong_key_fails_to_decrypt() {
        let env = encrypt(&derive_key("pass", "alice").unwrap(), b"secret").unwrap();
        assert!(decrypt(&derive_key("other", "alice").unwrap(), &env).is_none());
    }

    #[test]
    fn every_encryption_uses_a_fresh_nonce() {
        let key = [7u8; 32];
        let a = encrypt(&key, b"same").unwrap();
        let b = encrypt(&key, b"same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = [7u8; 32];
        let mut env = encrypt(&key, b"pay 10").unwrap();
        let mut raw = STANDARD.decode(&env.ciphertext).unwrap();
        raw[0] ^= 1; // flip one bit
        env.ciphertext = STANDARD.encode(raw);
        assert!(decrypt(&key, &env).is_none(), "Poly1305 tag must catch it");
    }

    #[test]
    fn malformed_envelopes_are_rejected_not_panicking() {
        let key = [7u8; 32];
        let bad = [
            Envelope {
                nonce: "not base64!".into(),
                ciphertext: "AAAA".into(),
            },
            Envelope {
                nonce: STANDARD.encode([0u8; 12]),
                ciphertext: "AAAA".into(),
            },
            Envelope {
                nonce: STANDARD.encode([0u8; 24]),
                ciphertext: String::new(),
            },
        ];
        for env in &bad {
            assert!(decrypt(&key, env).is_none());
        }
    }
}
