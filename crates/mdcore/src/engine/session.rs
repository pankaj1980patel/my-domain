//! Account session and the encryption key: login/register (server), offline
//! dev login, setting/rotating the key, logout.

use super::*;

impl<P: Platform> Engine<P> {
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
        let url = base(server_url);
        *self.server_url.lock() = Some(url.clone());
        *self.token.lock() = Some(resp.token);
        *self.username.lock() = Some(resp.username.clone());
        self.ctx.platform.kv_set("server_url", &url);
        self.ctx.platform.kv_set("username", &resp.username);
        Ok(())
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
        Ok(())
    }

    pub fn set_encryption_key(&self, passphrase: &str) -> crate::Result<()> {
        let username = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        if passphrase.trim().is_empty() {
            return Err("encryption key is required".into());
        }
        let key = derive_key(passphrase, &username).ok_or("failed to derive key")?;
        *self.ctx.key.lock() = Some(key);
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
        *self.ctx.key.lock() = Some(key);
        Ok(())
    }

    pub fn generate_key(&self) -> String {
        crate::crypto::generate_key()
    }

    pub fn logout(&self) {
        *self.token.lock() = None;
        *self.ctx.key.lock() = None;
    }
}
