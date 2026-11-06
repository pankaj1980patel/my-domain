//! Adding a device by scanning a QR code (or typing a 6-digit code).
//!
//! The signed-in device makes a one-time **join link**:
//!
//! ```text
//! mydomain://join?v=1&s=<server>&u=<username>&t=<secret>&k=<sealed key>
//! ```
//!
//! * `t` — a random 128-bit secret, made on that device. It never reaches the
//!   server: the server is given only [`claim_id`] (a SHA-256 of it), which
//!   the new device presents to get a session of the account.
//! * `k` — the device key (the 32 bytes derived from the passphrase), sealed
//!   with XChaCha20-Poly1305 under a key hashed from the same secret. Only
//!   someone holding the link can open it; the server never sees it.
//! * No password: the link signs in through the server's single-use, 5-minute
//!   pairing (`/pair/*`), not with the account password.
//!
//! The **6-digit code** shown under the QR is the fallback for a device that
//! can't scan. It has to bring the key too, so the creating device leaves a
//! [`seal_for_code`] box with the server: the device key sealed under an
//! Argon2id hash of the code. The server holds that box for at most
//! 5 minutes (until the code is used), and could only open it by trying
//! every code. Scanning keeps the key off the server entirely.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine as _};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::crypto::{open, seal};

pub const LINK_PREFIX: &str = "mydomain://join?";
/// Authenticated with every sealed key, so a box can't be reused elsewhere.
const AAD_LINK: &[u8] = b"my-domain join link v1";
const AAD_CODE: &[u8] = b"my-domain join code v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinLink {
    /// The registry server (`scheme://host[:port]`).
    pub server: String,
    pub username: String,
    pub secret: [u8; 16],
    /// The device key, sealed under [`wrap_key`] of `secret`.
    pub sealed_key: Vec<u8>,
}

fn sha(label: &[u8], secret: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(label);
    h.update(secret);
    h.finalize().into()
}

/// What the server is told: a one-way hash of the link's secret (64 hex).
pub fn claim_id(secret: &[u8; 16]) -> String {
    sha(b"my-domain join claim v1:", secret)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The key the device key is sealed under in a link. A different hash from
/// [`claim_id`], so the server can't compute it from what it's told.
fn wrap_key(secret: &[u8; 16]) -> [u8; 32] {
    sha(b"my-domain join wrap v1:", secret)
}

impl JoinLink {
    /// A fresh link for this account and device key.
    pub fn new(server: &str, username: &str, key: &[u8; 32]) -> Option<JoinLink> {
        let mut secret = [0u8; 16];
        OsRng.fill_bytes(&mut secret);
        Some(JoinLink {
            server: crate::registry::base(server),
            username: username.to_string(),
            secret,
            sealed_key: seal(&wrap_key(&secret), AAD_LINK, key)?,
        })
    }

    pub fn claim(&self) -> String {
        claim_id(&self.secret)
    }

    /// The device key, if the link is intact.
    pub fn open_key(&self) -> Option<[u8; 32]> {
        open(&wrap_key(&self.secret), AAD_LINK, &self.sealed_key)?
            .try_into()
            .ok()
    }

    pub fn to_uri(&self) -> String {
        format!(
            "{LINK_PREFIX}v=1&s={}&u={}&t={}&k={}",
            pct_encode(&self.server),
            pct_encode(&self.username),
            B64.encode(self.secret),
            B64.encode(&self.sealed_key),
        )
    }

    /// Read a link (as scanned, or pasted with stray spaces around it).
    pub fn parse(uri: &str) -> Option<JoinLink> {
        let uri = uri.trim();
        let query = uri
            .get(..LINK_PREFIX.len())
            .filter(|p| p.eq_ignore_ascii_case(LINK_PREFIX))
            .map(|_| &uri[LINK_PREFIX.len()..])?;
        let (mut v, mut s, mut u, mut t, mut k) = (None, None, None, None, None);
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=')?;
            let value = pct_decode(value)?;
            match name {
                "v" => v = Some(value),
                "s" => s = Some(value),
                "u" => u = Some(value),
                "t" => t = Some(value),
                "k" => k = Some(value),
                _ => {} // newer fields are ignored
            }
        }
        if v.as_deref() != Some("1") {
            return None;
        }
        let server = s.filter(|s| s.starts_with("https://") || s.starts_with("http://"))?;
        let username = u.filter(|u| !u.trim().is_empty())?;
        let secret: [u8; 16] = B64.decode(t?).ok()?.try_into().ok()?;
        let sealed_key = B64.decode(k?).ok()?;
        Some(JoinLink {
            server,
            username,
            secret,
            sealed_key,
        })
    }
}

/// Percent-encode a query value: everything but unreserved characters and
/// `:` `/` (allowed in a query, and keeps the server URL short).
pub(crate) fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub(crate) fn pct_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Just the digits of a typed code ("123 456" -> "123456"); `None` unless
/// that's exactly 6 of them.
pub fn normalize_code(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    (digits.len() == 6
        && code
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '-'))
    .then_some(digits)
}

/// A random 6-digit code.
pub fn random_code() -> String {
    loop {
        let n = OsRng.next_u32();
        // The largest multiple of 1,000,000 below 2^32: no modulo bias.
        if n < 4_294_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

fn code_key(code: &str, salt: &[u8]) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(code.as_bytes(), salt, &mut out)
        .ok()?;
    Some(out)
}

/// The device key sealed under a 6-digit code (base64url of
/// `salt(16) || sealed`), for the server to hand to whoever types the code.
pub fn seal_for_code(code: &str, key: &[u8; 32]) -> Option<String> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut out = salt.to_vec();
    out.extend(seal(&code_key(code, &salt)?, AAD_CODE, key)?);
    Some(B64.encode(out))
}

/// Open a [`seal_for_code`] box with the code that was typed.
pub fn open_code_box(code: &str, sealed: &str) -> Option<[u8; 32]> {
    let raw = B64.decode(sealed.trim()).ok()?;
    if raw.len() < 16 {
        return None;
    }
    let (salt, rest) = raw.split_at(16);
    open(&code_key(code, salt)?, AAD_CODE, rest)?
        .try_into()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [9u8; 32];

    #[test]
    fn a_link_round_trips_and_opens_the_key() {
        let link = JoinLink::new("https://mydomain.servehttp.com/auth/login", "ann", &KEY).unwrap();
        assert_eq!(link.server, "https://mydomain.servehttp.com");
        let uri = link.to_uri();
        assert!(uri.starts_with("mydomain://join?v=1&s=https://mydomain.servehttp.com&u=ann&t="));
        let back = JoinLink::parse(&format!("  {uri}\n")).unwrap();
        assert_eq!(back, link);
        assert_eq!(back.open_key(), Some(KEY));
        // Small enough for a comfortable QR code.
        assert!(uri.len() < 200, "{} bytes", uri.len());
        assert!(crate::qr::QrCode::encode(uri.as_bytes()).unwrap().version <= 10);
    }

    #[test]
    fn the_link_holds_no_password_and_the_server_learns_nothing_from_the_claim() {
        let link = JoinLink::new("https://x.example", "ann", &KEY).unwrap();
        let uri = link.to_uri();
        assert!(!uri.contains("password") && !uri.contains("&p="));
        // The claim is a hash: neither the secret nor the wrap key.
        let claim = link.claim();
        assert_eq!(claim.len(), 64);
        assert_ne!(claim, B64.encode(link.secret));
        let wrap_hex: String = wrap_key(&link.secret)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_ne!(claim, wrap_hex);
        // Each link is new.
        let other = JoinLink::new("https://x.example", "ann", &KEY).unwrap();
        assert_ne!(other.secret, link.secret);
        assert_ne!(other.claim(), claim);
    }

    #[test]
    fn odd_usernames_and_servers_survive_the_round_trip() {
        let link = JoinLink::new("http://10.0.0.5:6969", "ann & bob=ok+%é", &KEY).unwrap();
        let back = JoinLink::parse(&link.to_uri()).unwrap();
        assert_eq!(back.username, "ann & bob=ok+%é");
        assert_eq!(back.server, "http://10.0.0.5:6969");
    }

    #[test]
    fn broken_or_foreign_links_are_refused() {
        let link = JoinLink::new("https://x.example", "ann", &KEY).unwrap();
        let uri = link.to_uri();
        assert!(JoinLink::parse("https://x.example/join?v=1").is_none());
        assert!(JoinLink::parse(&uri.replace("v=1", "v=2")).is_none());
        assert!(JoinLink::parse(&uri.replace("s=https://", "s=ftp://")).is_none());
        assert!(JoinLink::parse(&uri.replace("&u=ann", "")).is_none());
        assert!(JoinLink::parse(&uri.replace("&t=", "&t=AA")).is_none());
        assert!(JoinLink::parse("mydomain://join?v=1&s=%ZZ").is_none());
        // Case of the prefix doesn't matter (some scanners upper-case it).
        assert!(JoinLink::parse(&uri.replacen("mydomain://join", "MYDOMAIN://JOIN", 1)).is_some());
        // A tampered key, or another secret, doesn't open.
        let mut bad = link.clone();
        bad.sealed_key[30] ^= 1;
        assert_eq!(bad.open_key(), None);
        let mut other = link.clone();
        other.secret[0] ^= 1;
        assert_eq!(other.open_key(), None);
    }

    #[test]
    fn the_code_box_opens_only_with_its_code() {
        let code = random_code();
        let sealed = seal_for_code(&code, &KEY).unwrap();
        assert_eq!(open_code_box(&code, &sealed), Some(KEY));
        let wrong = if code == "000000" { "000001" } else { "000000" };
        assert_eq!(open_code_box(wrong, &sealed), None);
        assert_eq!(open_code_box(&code, "AAAA"), None);
        assert_eq!(open_code_box(&code, "not base64!"), None);
        // Fresh salt every time.
        assert_ne!(seal_for_code(&code, &KEY).unwrap(), sealed);
    }

    #[test]
    fn codes_are_six_digits_and_typing_is_forgiving() {
        for _ in 0..100 {
            let c = random_code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
        }
        assert_eq!(normalize_code(" 123 456 ").as_deref(), Some("123456"));
        assert_eq!(normalize_code("123-456").as_deref(), Some("123456"));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("1234567"), None);
        assert_eq!(normalize_code("12a456"), None);
    }
}
