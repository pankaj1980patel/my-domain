//! Account security: changing the account password, "Sign out all
//! devices", the signed-in sessions list, and noticing that the server no
//! longer takes this device's session ("signed out elsewhere").
//!
//! The account password (the server's) is not the encryption passphrase:
//! that one never leaves your devices (`features::rekey` changes it). A
//! sign-out by the server drops only the session token; the key stays, so
//! signing in again with the password is enough (`sign_in_again`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use super::*;
use crate::registry::{
    change_password_call, logout_all_call, revoke_session_call, sessions_call, AccountSession,
};

/// kv flag: the server signed this device out (shown again after a restart).
pub(super) const SIGNED_OUT_KEY: &str = "signed_out_elsewhere";
/// Shortest account password the server takes.
const MIN_PASSWORD: usize = 6;

/// Forget the session token and tell the apps (the event and the kv flag).
pub(super) fn mark_signed_out<P: Platform>(platform: &P, sink: &dyn EventSink) {
    platform.secret_set(super::session::SECRET_TOKEN, None);
    platform.kv_set(SIGNED_OUT_KEY, "1");
    let username = platform.kv_get("username").unwrap_or_default();
    sink.emit(CoreEvent::SignedOut { username });
}

impl<P: Platform> Engine<P> {
    /// Watch for the server refusing this engine's token (any call, or the
    /// relay): forget it and emit `SignedOut`. Not while this device itself
    /// is changing its session (`auth_busy`): the old token is refused then
    /// on purpose.
    pub(super) fn watch_sign_out(&self) {
        let token: Weak<Mutex<Option<String>>> = Arc::downgrade(&self.token);
        let platform: Weak<P> = Arc::downgrade(&self.ctx.platform);
        let sink: Weak<dyn EventSink> = Arc::downgrade(&self.ctx.sink);
        let shutdown = Arc::downgrade(&self.ctx.shutdown);
        let busy: Weak<AtomicBool> = Arc::downgrade(&self.auth_busy);
        crate::registry::on_token_refused(Arc::new(move |refused: &str| {
            let (Some(token), Some(platform), Some(sink), Some(shutdown), Some(busy)) = (
                token.upgrade(),
                platform.upgrade(),
                sink.upgrade(),
                shutdown.upgrade(),
                busy.upgrade(),
            ) else {
                return false;
            };
            if shutdown.load(Ordering::Relaxed) {
                return false;
            }
            if busy.load(Ordering::Relaxed) {
                return true;
            }
            {
                let mut t = token.lock();
                if t.as_deref() != Some(refused) {
                    return true; // another engine's, or an old one of ours
                }
                *t = None;
            }
            mark_signed_out(&*platform, &*sink);
            true
        }));
    }

    /// The server signed this device out (and it hasn't signed in again):
    /// the apps show "Sign in again" instead of the app.
    pub fn signed_out_elsewhere(&self) -> bool {
        self.ctx.platform.kv_get(SIGNED_OUT_KEY).as_deref() == Some("1")
    }

    /// Sign in again after a sign-out by the server, with the account
    /// password only: the server and username are the saved ones and the
    /// encryption key is still here. `NoKey` when it isn't (sign in fully).
    pub fn sign_in_again(&self, password: &str) -> crate::Result<()> {
        let p = &self.ctx.platform;
        let (Some(url), Some(user)) = (p.kv_get("server_url"), p.kv_get("username")) else {
            return Err(CoreError::NotLoggedIn);
        };
        if self.ctx.key.lock().is_none() {
            let key = p
                .secret_get(super::session::SECRET_KEY)
                .and_then(|k| super::session::unhex32(&k))
                .ok_or(CoreError::NoKey)?;
            *self.ctx.key.lock() = Some(key);
        }
        let resp = auth_call(&url, "/auth/login", &user, password)?;
        self.adopt_session(&url, resp.token, resp.username);
        self.refresh_in_background();
        Ok(())
    }

    /// Change the account password (Settings → Account). The server signs
    /// out every other session; this device gets a new token at once.
    pub fn change_account_password(&self, current: &str, new_password: &str) -> crate::Result<()> {
        if new_password.chars().count() < MIN_PASSWORD || new_password.len() > 256 {
            return Err(CoreError::InvalidInput(
                "the new password must be 6 to 256 characters".into(),
            ));
        }
        if current == new_password {
            return Err(CoreError::InvalidInput(
                "the new password is the same as the current one".into(),
            ));
        }
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let _busy = Busy::set(&self.auth_busy);
        let resp = change_password_call(&url, &token, current, new_password)?;
        self.adopt_session(&url, resp.token, resp.username);
        self.refresh_in_background();
        Ok(())
    }

    /// Sign out every device of the account, this one too. With the
    /// password (checked first: a wrong one signs nobody out), this device
    /// signs straight back in; without it (or if that fails) it's signed
    /// out and `SignedOut` asks for it. True when signed back in.
    pub fn sign_out_all_devices(&self, password: &str) -> crate::Result<bool> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let user = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        if !password.is_empty() && !verify_password_call(&url, &user, password)? {
            return Err(crate::registry::WRONG_PASSWORD.into());
        }
        let busy = Busy::set(&self.auth_busy);
        logout_all_call(&url, &token)?;
        let again = if password.is_empty() {
            None
        } else {
            auth_call(&url, "/auth/login", &user, password).ok()
        };
        let back = match again {
            Some(resp) => {
                self.adopt_session(&url, resp.token, resp.username);
                self.refresh_in_background();
                true
            }
            None => {
                *self.token.lock() = None;
                mark_signed_out(&*self.ctx.platform, &*self.ctx.sink);
                false
            }
        };
        drop(busy);
        Ok(back)
    }

    /// The account's signed-in sessions, this device's first.
    pub fn account_sessions(&self) -> crate::Result<Vec<AccountSession>> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        Ok(sessions_call(&url, &token)?)
    }

    /// Sign out one session from the list. If the server signed out every
    /// session instead (its list of signed-out ones was full), this device
    /// is signed out too.
    pub fn sign_out_session(&self, id: &str) -> crate::Result<()> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        if revoke_session_call(&url, &token, id)? {
            *self.token.lock() = None;
            mark_signed_out(&*self.ctx.platform, &*self.ctx.sink);
        }
        Ok(())
    }

    /// Re-register and refresh the devices with the new token (off this
    /// thread).
    fn refresh_in_background(&self) {
        let h = self.handles();
        std::thread::spawn(move || {
            let session = h.server_url.lock().clone().zip(h.token.lock().clone());
            if let Some((url, token)) = session {
                let _ = peers::refresh(&h.ctx, &url, &token);
            }
        });
    }
}

/// `auth_busy` set while alive.
struct Busy<'a>(&'a AtomicBool);

impl<'a> Busy<'a> {
    fn set(flag: &'a AtomicBool) -> Self {
        flag.store(true, Ordering::Relaxed);
        Busy(flag)
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}
