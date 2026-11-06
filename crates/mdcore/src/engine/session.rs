//! Account session and the encryption key: login/register (server), offline
//! dev login, setting/rotating the key, logout.

use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

/// Secret-store keys (see `Platform::secret_get`).
const SECRET_TOKEN: &str = "session_token";
const SECRET_KEY: &str = "device_key";

/// A restored token must stay valid at least this long, or we ask the user
/// to sign in again rather than fail later.
const MIN_TOKEN_LIFE_SECS: u64 = 3600;

/// Seconds since the epoch at which a JWT expires (its `exp` claim), read
/// without checking the signature: the server does that.
fn token_expiry(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["exp"].as_u64()
}

fn token_usable(token: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    token_expiry(token).is_some_and(|exp| exp > now + MIN_TOKEN_LIFE_SECS)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if s.len() != 64 {
        return None;
    }
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

impl<P: Platform> Engine<P> {
    /// Sign back in from the secret store: the saved token (if it has at
    /// least an hour left) and the derived device key. True when restored;
    /// the caller then refreshes from the server in the background.
    pub(crate) fn restore_session(&self) -> bool {
        let p = &self.ctx.platform;
        let (Some(url), Some(user)) = (p.kv_get("server_url"), p.kv_get("username")) else {
            return false;
        };
        let token = p.secret_get(SECRET_TOKEN).filter(|t| token_usable(t));
        let key = p.secret_get(SECRET_KEY).and_then(|k| unhex32(&k));
        let (Some(token), Some(key)) = (token, key) else {
            return false;
        };
        *self.server_url.lock() = Some(url);
        *self.username.lock() = Some(user);
        *self.token.lock() = Some(token);
        *self.ctx.key.lock() = Some(key);
        true
    }

    /// The key (and so our LAN tag) just changed: tell the LAN, so our other
    /// devices there list us. Only with automatic LAN discovery on (tests
    /// turn it off and scan explicitly).
    fn announce_with_key(&self) {
        if self.ctx.config.mdns {
            netchange::announce(&self.ctx, &self.disco_send);
        }
    }

    fn save_key(&self, key: &[u8; 32]) {
        self.ctx.platform.secret_set(SECRET_KEY, Some(&hex(key)));
    }

    // --- auth / key ---

    fn do_auth(
        &self,
        path: &str,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> crate::Result<()> {
        if server_url.trim().is_empty() {
            return Err("server URL is required".into());
        }
        let resp = auth_call(server_url, path, username.trim(), password)?;
        self.adopt_session(server_url, resp.token, resp.username);
        Ok(())
    }

    /// Signed in: keep the session (and where) for this run and the next.
    pub(super) fn adopt_session(&self, server_url: &str, token: String, username: String) {
        let url = base(server_url);
        *self.server_url.lock() = Some(url.clone());
        self.ctx.platform.secret_set(SECRET_TOKEN, Some(&token));
        *self.token.lock() = Some(token);
        self.ctx.platform.kv_set("server_url", &url);
        self.ctx.platform.kv_set("username", &username);
        *self.username.lock() = Some(username);
    }

    /// Use (and keep) an already-derived device key, e.g. one handed over
    /// by a join link.
    pub(super) fn adopt_key(&self, key: [u8; 32]) {
        self.save_key(&key);
        *self.ctx.key.lock() = Some(key);
        self.announce_with_key();
    }

    pub fn auth_login(
        &self,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> crate::Result<()> {
        self.do_auth("/auth/login", server_url, username, password)
    }

    pub fn auth_register(
        &self,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> crate::Result<()> {
        self.do_auth("/auth/register", server_url, username, password)
    }

    /// DEV / LAN-only: skip the registry. Set a local username + key so LAN
    /// discovery and messaging work between devices sharing the same credentials.
    pub fn dev_login(&self, username: &str, passphrase: &str) -> crate::Result<()> {
        let username = username.trim();
        if username.is_empty() || passphrase.trim().is_empty() {
            return Err("username and encryption key required".into());
        }
        let key = derive_key(passphrase, username).ok_or("failed to derive key")?;
        *self.username.lock() = Some(username.to_string());
        *self.ctx.key.lock() = Some(key);
        self.announce_with_key();
        Ok(())
    }

    pub fn set_encryption_key(&self, passphrase: &str) -> crate::Result<()> {
        let username = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        if passphrase.trim().is_empty() {
            return Err("encryption key is required".into());
        }
        let key = derive_key(passphrase, &username).ok_or("failed to derive key")?;
        self.save_key(&key);
        *self.ctx.key.lock() = Some(key);
        self.announce_with_key();
        Ok(())
    }

    pub fn update_encryption_key(&self, new_passphrase: &str, password: &str) -> crate::Result<()> {
        let username = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let url = self.server_url.lock().clone().ok_or("no server")?;
        if !verify_password_call(&url, &username, password)? {
            return Err("incorrect password".into());
        }
        if new_passphrase.trim().is_empty() {
            return Err("new encryption key is required".into());
        }
        let key = derive_key(new_passphrase, &username).ok_or("failed to derive key")?;
        self.save_key(&key);
        *self.ctx.key.lock() = Some(key);
        self.announce_with_key();
        Ok(())
    }

    pub fn generate_key(&self) -> String {
        crate::crypto::generate_key()
    }

    pub fn logout(&self) {
        *self.token.lock() = None;
        *self.ctx.key.lock() = None;
        self.ctx.platform.secret_set(SECRET_TOKEN, None);
        self.ctx.platform.secret_set(SECRET_KEY, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(exp: u64) -> String {
        let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"sub":"u","exp":{exp}}}"#));
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
    }

    #[test]
    fn reads_the_expiry_and_refuses_tokens_about_to_expire() {
        assert_eq!(token_expiry(&jwt(42)), Some(42));
        assert_eq!(token_expiry("not-a-jwt"), None);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(token_usable(&jwt(now + 7 * 24 * 3600)));
        assert!(!token_usable(&jwt(now + 60)));
    }

    #[test]
    fn keys_round_trip_through_hex() {
        let key = [7u8; 32];
        assert_eq!(unhex32(&hex(&key)), Some(key));
        assert_eq!(unhex32("abc"), None);
    }
}
