//! Sessions that can be signed out (security review L23).
//!
//! Every token carries a random session id (`sid`) and the account's token
//! version (`ver`) when it was issued. A token is good while it's unexpired,
//! its version is the account's current one and its `sid` is not on the
//! account's revoked list. `POST /auth/logout` revokes the caller's `sid`;
//! `POST /auth/logout_all` and a password change bump the version, which
//! signs out every session at once (and empties the revoked list).
//!
//! Each session is listed for the account (`GET /auth/sessions`) from the
//! first time its token is used, with when it was last seen (written at most
//! every `TOUCH_EVERY`) and the device that registered with it; any one can
//! be signed out from the list (`POST /auth/sessions/revoke`).
//!
//! The account's state is read once and cached for `CACHE_TTL`; a change made
//! through this server updates the cache at once, so the TTL matters only
//! when the database was changed from elsewhere. Tokens from before this
//! (no `sid`, no `ver`) count as version 0 until the account's first bump.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::store::{Account, SessionRec, Store};

pub const TOKEN_TTL_SECS: u64 = 7 * 24 * 3600;
/// How long an account's session state is trusted without asking the store.
const CACHE_TTL: Duration = Duration::from_secs(30);
/// Most accounts cached at once.
const MAX_CACHED: usize = 100_000;
/// Sessions signed out one by one that an account keeps on its list (each
/// until its token expires). Past this a logout signs out every session.
pub const MAX_REVOKED: usize = 64;
/// How often a session's "last seen" is written while it's in use.
const TOUCH_EVERY: Duration = Duration::from_secs(10 * 60);

/// One session in `GET /auth/sessions`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionOut {
    pub id: String,
    /// The device that registered with it (empty until one has).
    pub name: String,
    pub node_id: String,
    /// Seconds since the epoch.
    pub created: i64,
    pub last_seen: i64,
    /// The session this request came with.
    pub current: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user id (hex)
    pub username: String,
    pub exp: usize,
    #[serde(default)]
    pub ver: i64,
    #[serde(default)]
    pub sid: String,
}

/// Who a request is from (a checked token).
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: String,
    pub username: String,
    /// Empty for a token from before session ids.
    pub sid: String,
    pub exp: i64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_sid() -> String {
    let mut b = [0u8; 16];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A signed token for `user_id` under token version `ver`, with a new
/// session id.
pub fn make_token(
    secret: &str,
    user_id: &str,
    username: &str,
    ver: i64,
) -> Result<String, AppError> {
    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        exp: (now_secs() + TOKEN_TTL_SECS) as usize,
        ver,
        sid: random_sid(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

/// The token's claims if its signature and expiry check out (nothing else).
pub fn decode_token(secret: &str, token: &str) -> Option<Claims> {
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|d| d.claims)
}

pub struct Sessions {
    secret: String,
    store: Arc<dyn Store>,
    cache: Mutex<HashMap<String, (Instant, Arc<Account>)>>,
    /// Session id -> when its "last seen" was last written and the device
    /// it was written with.
    touched: Mutex<HashMap<String, (Instant, String)>>,
}

impl Sessions {
    pub fn new(secret: String, store: Arc<dyn Store>) -> Arc<Sessions> {
        Arc::new(Sessions {
            secret,
            store,
            cache: Mutex::default(),
            touched: Mutex::default(),
        })
    }

    /// Sessions over an in-memory store (tests).
    #[cfg(test)]
    pub fn test(secret: &str) -> Arc<Sessions> {
        Self::new(secret.into(), crate::store::MemStore::new())
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    /// The account's session state, from the cache when fresh. Unknown
    /// accounts are `Unauthorized`.
    async fn account(&self, user_id: &str, fresh: bool) -> Result<Arc<Account>, AppError> {
        let now = Instant::now();
        if !fresh {
            if let Some((at, a)) = self.cache.lock().unwrap().get(user_id) {
                if now.saturating_duration_since(*at) < CACHE_TTL {
                    return Ok(Arc::clone(a));
                }
            }
        }
        let a = Arc::new(
            self.store
                .account(user_id)
                .await?
                .ok_or(AppError::Unauthorized)?,
        );
        let mut c = self.cache.lock().unwrap();
        if c.len() >= MAX_CACHED {
            c.retain(|_, (at, _)| now.saturating_duration_since(*at) < CACHE_TTL);
            if c.len() >= MAX_CACHED {
                crate::limits::evict_oldest(&mut c, |(at, _)| *at);
            }
        }
        c.insert(user_id.to_string(), (now, Arc::clone(&a)));
        Ok(a)
    }

    /// Drop the cached state (after changing it in the store directly).
    pub fn forget(&self, user_id: &str) {
        self.cache.lock().unwrap().remove(user_id);
    }

    /// A new session for the account (at its current token version).
    pub async fn issue(&self, user_id: &str, username: &str) -> Result<String, AppError> {
        let a = self.account(user_id, false).await?;
        make_token(&self.secret, user_id, username, a.ver)
    }

    /// A new session at token version `ver`, read by the caller with the
    /// account (login, sign-up).
    pub fn issue_at(&self, user_id: &str, username: &str, ver: i64) -> Result<String, AppError> {
        make_token(&self.secret, user_id, username, ver)
    }

    /// Check a token: signature, expiry, version and revocation. `Internal`
    /// when the store can't be reached (a token is never let through
    /// unchecked).
    pub async fn verify(&self, token: &str) -> Result<AuthUser, AppError> {
        let c = decode_token(&self.secret, token).ok_or(AppError::Unauthorized)?;
        let mut a = self.account(&c.sub, false).await?;
        if c.ver > a.ver {
            // Issued after what the cache holds (another server, or a
            // bump in the database): look again.
            a = self.account(&c.sub, true).await?;
        }
        if c.ver != a.ver || (!c.sid.is_empty() && a.revoked.iter().any(|r| r.sid == c.sid)) {
            return Err(AppError::Unauthorized);
        }
        let user = AuthUser {
            user_id: c.sub,
            username: c.username,
            sid: c.sid,
            exp: c.exp as i64,
        };
        self.touch(&user, None).await;
        Ok(user)
    }

    /// Note that `user`'s session was seen now (from device `node`: id,
    /// name), at most every `TOUCH_EVERY` unless the device is new. Best
    /// effort: a store error only skips it.
    async fn touch(&self, user: &AuthUser, node: Option<(&str, &str)>) {
        if user.sid.is_empty() {
            return;
        }
        let now = Instant::now();
        {
            let mut t = self.touched.lock().unwrap();
            if let Some((at, n)) = t.get(&user.sid) {
                let same_device = node.is_none_or(|(id, _)| id == n);
                if same_device && now.saturating_duration_since(*at) < TOUCH_EVERY {
                    return;
                }
            }
            if t.len() >= MAX_CACHED {
                crate::limits::evict_oldest(&mut t, |(at, _)| *at);
            }
            let known = t.get(&user.sid).map(|(_, n)| n.clone()).unwrap_or_default();
            let n = node.map_or(known, |(id, _)| id.to_string());
            t.insert(user.sid.clone(), (now, n));
        }
        let secs = now_secs() as i64;
        let (node_id, name) = node.unwrap_or_default();
        let rec = SessionRec {
            sid: user.sid.clone(),
            // Tokens don't say when they were issued; their expiry does.
            created: (user.exp - TOKEN_TTL_SECS as i64).min(secs),
            last_seen: secs,
            exp: user.exp,
            node_id: node_id.to_string(),
            name: name.to_string(),
        };
        if let Err(e) = self.store.put_session(&user.user_id, rec).await {
            tracing::warn!("noting a session failed: {e:?}");
            self.touched.lock().unwrap().remove(&user.sid);
        }
    }

    /// The device `node_id` (named `name`) registered with `user`'s session.
    pub async fn note_device(&self, user: &AuthUser, node_id: &str, name: &str) {
        self.touch(user, Some((node_id, name))).await;
    }

    /// The account's signed-in sessions, this one first, then the most
    /// recently seen (signed-out and expired ones left out).
    pub async fn list(&self, user: &AuthUser) -> Result<Vec<SessionOut>, AppError> {
        let a = self.account(&user.user_id, true).await?;
        let now = now_secs() as i64;
        let mut out: Vec<SessionOut> = a
            .sessions
            .iter()
            .filter(|r| r.exp >= now && !a.revoked.iter().any(|x| x.sid == r.sid))
            .map(|r| SessionOut {
                id: r.sid.clone(),
                name: r.name.clone(),
                node_id: r.node_id.clone(),
                created: r.created,
                last_seen: r.last_seen,
                current: r.sid == user.sid,
            })
            .collect();
        out.sort_by_key(|s| (!s.current, std::cmp::Reverse(s.last_seen)));
        Ok(out)
    }

    /// Sign out the account's session `id` (from the list). `None` when the
    /// account lists no such session; else whether every session was signed
    /// out instead (see `logout`).
    pub async fn revoke_session(
        &self,
        user: &AuthUser,
        id: &str,
    ) -> Result<Option<bool>, AppError> {
        if id == user.sid {
            return self.logout(user).await.map(Some);
        }
        let a = self.account(&user.user_id, true).await?;
        let Some(rec) = a.sessions.iter().find(|r| r.sid == id) else {
            return Ok(None);
        };
        let target = AuthUser {
            user_id: user.user_id.clone(),
            username: user.username.clone(),
            sid: rec.sid.clone(),
            exp: rec.exp,
        };
        self.logout(&target).await.map(Some)
    }

    /// Sign out the session `user` came with. A token from before session
    /// ids, or an account whose revoked list is full, signs out every
    /// session instead. True when every session was signed out.
    pub async fn logout(&self, user: &AuthUser) -> Result<bool, AppError> {
        let a = self.account(&user.user_id, true).await?;
        let now = now_secs() as i64;
        let live = a.revoked.iter().filter(|r| r.exp >= now).count();
        if user.sid.is_empty() || live >= MAX_REVOKED {
            return self.logout_all(&user.user_id).await.map(|()| true);
        }
        let r = self.store.revoke(&user.user_id, &user.sid, user.exp).await;
        self.forget(&user.user_id);
        self.touched.lock().unwrap().remove(&user.sid);
        r.map(|()| false)
    }

    /// Sign out every session of the account.
    pub async fn logout_all(&self, user_id: &str) -> Result<(), AppError> {
        let r = self.store.bump_version(user_id).await;
        self.forget(user_id);
        r
    }

    /// Accounts in the cache (for `/admin/limits`).
    pub fn cached(&self) -> usize {
        self.cache.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;

    #[tokio::test]
    async fn logout_signs_out_one_session_and_logout_all_every_one() {
        let s = Sessions::test("secret");
        let a = s.issue("u1", "ann").await.unwrap();
        let b = s.issue("u1", "ann").await.unwrap();
        let other = s.issue("u2", "bob").await.unwrap();
        let ua = s.verify(&a).await.unwrap();
        assert!(s.verify(&b).await.is_ok());

        s.logout(&ua).await.unwrap();
        assert!(matches!(s.verify(&a).await, Err(AppError::Unauthorized)));
        assert!(s.verify(&b).await.is_ok(), "the other session stays");

        s.logout_all("u1").await.unwrap();
        assert!(s.verify(&b).await.is_err());
        assert!(
            s.verify(&other).await.is_ok(),
            "other accounts are untouched"
        );
        // A new sign-in after that works.
        let c = s.issue("u1", "ann").await.unwrap();
        assert!(s.verify(&c).await.is_ok());
    }

    #[tokio::test]
    async fn sessions_are_listed_with_their_device_and_can_be_signed_out_one_by_one() {
        let s = Sessions::test("secret");
        let a = s.issue("u1", "ann").await.unwrap();
        let b = s.issue("u1", "ann").await.unwrap();
        let unused = s.issue("u1", "ann").await.unwrap();
        let ua = s.verify(&a).await.unwrap();
        let ub = s.verify(&b).await.unwrap();
        s.note_device(&ub, "node-b", "Pixel").await;
        let _ = unused; // never used: not listed

        let list = s.list(&ua).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].current && list[0].id == ua.sid, "this one first");
        assert_eq!(
            (list[1].name.as_str(), list[1].node_id.as_str()),
            ("Pixel", "node-b")
        );
        assert!(!list[1].current);
        assert!(list[1].created <= list[1].last_seen);

        // Signed out from the list: gone from it and refused.
        assert_eq!(s.revoke_session(&ua, &ub.sid).await.unwrap(), Some(false));
        assert!(s.verify(&b).await.is_err());
        let list = s.list(&ua).await.unwrap();
        assert_eq!(list.iter().map(|x| &x.id).collect::<Vec<_>>(), [&ua.sid]);
        assert_eq!(s.revoke_session(&ua, "nope").await.unwrap(), None);
        // Other accounts' sessions can't be named.
        let other = s.issue("u2", "bob").await.unwrap();
        let uo = s.verify(&other).await.unwrap();
        assert_eq!(s.revoke_session(&uo, &ua.sid).await.unwrap(), None);
        assert!(s.verify(&a).await.is_ok());

        // "Sign out all devices" empties the list.
        s.logout_all("u1").await.unwrap();
        let fresh = s.issue("u1", "ann").await.unwrap();
        let uf = s.verify(&fresh).await.unwrap();
        let list = s.list(&uf).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].current);
    }

    #[tokio::test]
    async fn tokens_from_before_session_ids_work_until_the_first_bump() {
        let s = Sessions::test("secret");
        #[derive(Serialize)]
        struct Old<'a> {
            sub: &'a str,
            username: &'a str,
            exp: usize,
        }
        let old = encode(
            &Header::default(),
            &Old {
                sub: "u1",
                username: "ann",
                exp: (now_secs() + 3600) as usize,
            },
            &EncodingKey::from_secret(b"secret"),
        )
        .unwrap();
        let u = s.verify(&old).await.unwrap();
        assert!(u.sid.is_empty());
        // Its logout can't name the session: every session goes.
        let fresh = s.issue("u1", "ann").await.unwrap();
        s.logout(&u).await.unwrap();
        assert!(s.verify(&old).await.is_err());
        assert!(s.verify(&fresh).await.is_err());
    }

    #[tokio::test]
    async fn a_change_made_elsewhere_is_seen_after_the_cache_and_unknown_accounts_fail() {
        let store = MemStore::new();
        let s = Sessions::new("secret".into(), store.clone());
        let t = s.issue("u1", "ann").await.unwrap();
        assert!(s.verify(&t).await.is_ok());
        // Bumped behind this server's back (another instance): a token it
        // issues after the bump is checked against a fresh read even with
        // a stale cache, and that read signs out the old one here too.
        let s2 = Sessions::new("secret".into(), store.clone());
        s2.logout_all("u1").await.unwrap();
        let newer = s2.issue("u1", "ann").await.unwrap();
        assert!(s.verify(&newer).await.is_ok());
        assert!(s.verify(&t).await.is_err());

        store.remove_account("u3");
        let gone = make_token("secret", "u3", "cy", 0).unwrap();
        assert!(s.verify(&gone).await.is_err());
        assert!(s.verify("garbage").await.is_err());
    }

    #[tokio::test]
    async fn a_full_revoked_list_signs_out_everything_instead() {
        let s = Sessions::test("secret");
        let mut toks = Vec::new();
        for _ in 0..=MAX_REVOKED {
            toks.push(s.issue("u1", "ann").await.unwrap());
        }
        let keep = s.issue("u1", "ann").await.unwrap();
        for t in &toks[..MAX_REVOKED] {
            let u = s.verify(t).await.unwrap();
            s.logout(&u).await.unwrap();
        }
        assert!(s.verify(&keep).await.is_ok());
        let u = s.verify(&toks[MAX_REVOKED]).await.unwrap();
        s.logout(&u).await.unwrap();
        assert!(s.verify(&keep).await.is_err(), "list full: all signed out");
    }
}
