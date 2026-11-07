//! What the server keeps beyond accounts and devices: each account's
//! session state (token version and revoked sessions), limiter records that
//! must outlive a restart (lockouts, daily counts) and invite codes.
//!
//! `MongoStore` is the real one (the server's MongoDB; records expire by a
//! TTL index on `expires`). `MemStore` stands in for it in tests, where it can
//! be shared by two server instances to play a restart.

use std::sync::Arc;

use mongodb::bson::{doc, oid::ObjectId, DateTime, Document, Regex};
use mongodb::{Collection, Database};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::USERS_COLL;

/// Limiter records (`md_limits`) and invite codes (`md_invites`).
pub const LIMITS_COLL: &str = "md_limits";
pub const INVITES_COLL: &str = "md_invites";

/// A session signed out by itself (its token's `sid`), kept until the token
/// would have expired anyway (`exp`, seconds since the epoch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revoked {
    pub sid: String,
    pub exp: i64,
}

/// A signed-in session, for the account's "Signed-in sessions" list
/// (`GET /auth/sessions`). Kept until its token expires (`exp`); an account
/// lists at most `MAX_SESSIONS`, the least recently seen dropped first
/// (dropping one from the list doesn't sign it out). Times are seconds
/// since the epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRec {
    pub sid: String,
    pub created: i64,
    pub last_seen: i64,
    pub exp: i64,
    /// The device that registered with this session, once it has.
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub name: String,
}

/// Sessions an account lists at once.
pub const MAX_SESSIONS: usize = 50;

/// One account's session state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    /// Tokens carry the version they were issued under; one below this is
    /// signed out ("sign out all devices", password change).
    pub ver: i64,
    pub revoked: Vec<Revoked>,
    /// Its signed-in sessions (some may since have been revoked or expired:
    /// `Sessions::list` leaves those out).
    pub sessions: Vec<SessionRec>,
}

/// `list` with `rec` merged in: an existing session (same `sid`) gets its
/// `last_seen` (and device, when given), a new one is added; expired ones go,
/// and past `MAX_SESSIONS` the least recently seen.
/// (What `MongoStore::put_session` does in the database; used as is by
/// `MemStore`.)
#[cfg(test)]
pub fn merge_session(list: &mut Vec<SessionRec>, rec: SessionRec, now: i64) {
    list.retain(|r| r.exp >= now);
    match list.iter_mut().find(|r| r.sid == rec.sid) {
        Some(r) => {
            r.last_seen = r.last_seen.max(rec.last_seen);
            if !rec.node_id.is_empty() {
                r.node_id = rec.node_id;
                r.name = rec.name;
            }
        }
        None => list.push(rec),
    }
    if list.len() > MAX_SESSIONS {
        list.sort_by_key(|r| std::cmp::Reverse(r.last_seen));
        list.truncate(MAX_SESSIONS);
    }
}

/// A limiter record: a counted window (`since_ms`, `n`) or a lockout
/// (`strikes`, `until_ms`), gone after `expires_ms`. Times are wall-clock ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitRec {
    /// With its kind as a prefix ("push:", "reg:", "pair:").
    pub key: String,
    pub since_ms: i64,
    pub n: u32,
    pub strikes: u32,
    pub until_ms: i64,
    pub expires_ms: i64,
}

// async_trait's expansion trips clippy::double_must_use on newer Rust.
#[allow(clippy::double_must_use)]
#[axum::async_trait]
pub trait Store: Send + Sync {
    /// The account's session state; `None` if there is no such account.
    async fn account(&self, user_id: &str) -> Result<Option<Account>, AppError>;
    /// Sign out every session of the account (and forget its revoked list,
    /// which the new version covers).
    async fn bump_version(&self, user_id: &str) -> Result<(), AppError>;
    /// Sign out one session; expired entries are dropped on the way. The
    /// session leaves the account's list too.
    async fn revoke(&self, user_id: &str, sid: &str, exp: i64) -> Result<(), AppError>;
    /// Add a session to the account's list, or note that it was seen (and
    /// which device it is): see `merge_session`.
    async fn put_session(&self, user_id: &str, rec: SessionRec) -> Result<(), AppError>;
    /// Save a limiter record in the background (never blocks a request).
    /// Records with the same key and `since_ms` are one record, keeping the
    /// larger `n`.
    fn put_limit(&self, rec: LimitRec);
    /// Unexpired records whose key starts with `prefix`.
    async fn load_limits(&self, prefix: &str) -> Result<Vec<LimitRec>, AppError>;
    /// A new invite code (by its hash), good for `uses` sign-ups.
    async fn add_invite(&self, hash: &str, uses: u32, expires_ms: i64) -> Result<(), AppError>;
    /// Use one sign-up of an invite; false if it's unknown, used up or expired.
    async fn take_invite(&self, hash: &str) -> Result<bool, AppError>;
    /// Give back a use taken by a sign-up that then failed.
    async fn return_invite(&self, hash: &str) -> Result<(), AppError>;
    /// Invite codes with uses left (for `/admin/limits`).
    async fn invites_left(&self) -> Result<u64, AppError>;
}

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    pub fn shared(db: Database) -> Arc<dyn Store> {
        Arc::new(MongoStore { db })
    }

    fn users(&self) -> Collection<Document> {
        self.db.collection(USERS_COLL)
    }

    fn limits(&self) -> Collection<Document> {
        self.db.collection(LIMITS_COLL)
    }

    fn invites(&self) -> Collection<Document> {
        self.db.collection(INVITES_COLL)
    }
}

fn now_secs() -> i64 {
    crate::limits::wall_ms() / 1000
}

/// `^prefix` as a regex, every non-alphanumeric character escaped.
fn prefix_regex(prefix: &str) -> Regex {
    let mut pattern = String::from("^");
    for c in prefix.chars() {
        if !c.is_ascii_alphanumeric() {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    Regex {
        pattern,
        options: String::new(),
    }
}

#[axum::async_trait]
impl Store for MongoStore {
    async fn account(&self, user_id: &str) -> Result<Option<Account>, AppError> {
        let Ok(oid) = ObjectId::parse_str(user_id) else {
            return Ok(None);
        };
        let Some(d) = self
            .users()
            .find_one(doc! { "_id": oid })
            .projection(doc! { "token_ver": 1, "revoked": 1, "sessions": 1 })
            .await?
        else {
            return Ok(None);
        };
        let ver = match d.get("token_ver") {
            Some(mongodb::bson::Bson::Int64(v)) => *v,
            Some(mongodb::bson::Bson::Int32(v)) => i64::from(*v),
            _ => 0,
        };
        let revoked = d
            .get_array("revoked")
            .map(|a| {
                a.iter()
                    .filter_map(|b| mongodb::bson::from_bson::<Revoked>(b.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let sessions = d
            .get_array("sessions")
            .map(|a| {
                a.iter()
                    .filter_map(|b| mongodb::bson::from_bson::<SessionRec>(b.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(Account {
            ver,
            revoked,
            sessions,
        }))
    }

    async fn bump_version(&self, user_id: &str) -> Result<(), AppError> {
        let oid = ObjectId::parse_str(user_id).map_err(|_| AppError::Unauthorized)?;
        self.users()
            .update_one(
                doc! { "_id": oid },
                doc! {
                    "$inc": { "token_ver": 1_i64 },
                    "$set": { "revoked": [], "sessions": [] },
                },
            )
            .await?;
        Ok(())
    }

    async fn revoke(&self, user_id: &str, sid: &str, exp: i64) -> Result<(), AppError> {
        let oid = ObjectId::parse_str(user_id).map_err(|_| AppError::Unauthorized)?;
        self.users()
            .update_one(
                doc! { "_id": oid },
                doc! { "$pull": { "revoked": { "exp": { "$lt": now_secs() } } } },
            )
            .await?;
        self.users()
            .update_one(
                doc! { "_id": oid },
                doc! {
                    "$push": { "revoked": { "sid": sid, "exp": exp } },
                    "$pull": { "sessions": { "sid": sid } },
                },
            )
            .await?;
        Ok(())
    }

    async fn put_session(&self, user_id: &str, rec: SessionRec) -> Result<(), AppError> {
        let oid = ObjectId::parse_str(user_id).map_err(|_| AppError::Unauthorized)?;
        // Seen again: just the time (and the device, once known).
        let mut set = doc! { "sessions.$.last_seen": rec.last_seen };
        if !rec.node_id.is_empty() {
            set.insert("sessions.$.node_id", &rec.node_id);
            set.insert("sessions.$.name", &rec.name);
        }
        let r = self
            .users()
            .update_one(
                doc! { "_id": oid, "sessions.sid": &rec.sid },
                doc! { "$set": set },
            )
            .await?;
        if r.matched_count > 0 {
            return Ok(());
        }
        // New: drop the expired, then add it, keeping the most recently seen.
        self.users()
            .update_one(
                doc! { "_id": oid },
                doc! { "$pull": { "sessions": { "exp": { "$lt": now_secs() } } } },
            )
            .await?;
        let bson = mongodb::bson::to_bson(&rec).map_err(|e| AppError::Internal(e.to_string()))?;
        self.users()
            .update_one(
                doc! { "_id": oid },
                doc! { "$push": { "sessions": {
                    "$each": [bson],
                    "$sort": { "last_seen": 1 },
                    "$slice": -(MAX_SESSIONS as i64),
                } } },
            )
            .await?;
        Ok(())
    }

    fn put_limit(&self, rec: LimitRec) {
        let coll = self.limits();
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        rt.spawn(async move {
            let id = format!("{}@{}", rec.key, rec.since_ms);
            let r = coll
                .update_one(
                    doc! { "_id": id },
                    doc! {
                        "$max": { "n": i64::from(rec.n) },
                        "$set": {
                            "key": &rec.key,
                            "since_ms": rec.since_ms,
                            "strikes": i64::from(rec.strikes),
                            "until_ms": rec.until_ms,
                            "expires": DateTime::from_millis(rec.expires_ms),
                        },
                    },
                )
                .upsert(true)
                .await;
            if let Err(e) = r {
                tracing::warn!("saving a limit record failed: {e}");
            }
        });
    }

    async fn load_limits(&self, prefix: &str) -> Result<Vec<LimitRec>, AppError> {
        use futures::TryStreamExt;
        let now = DateTime::from_millis(crate::limits::wall_ms());
        let docs: Vec<Document> = self
            .limits()
            .find(doc! { "key": prefix_regex(prefix), "expires": { "$gt": now } })
            .await?
            .try_collect()
            .await?;
        let int = |d: &Document, k: &str| -> i64 {
            match d.get(k) {
                Some(mongodb::bson::Bson::Int64(v)) => *v,
                Some(mongodb::bson::Bson::Int32(v)) => i64::from(*v),
                _ => 0,
            }
        };
        Ok(docs
            .iter()
            .filter_map(|d| {
                Some(LimitRec {
                    key: d.get_str("key").ok()?.to_string(),
                    since_ms: int(d, "since_ms"),
                    n: u32::try_from(int(d, "n")).unwrap_or(u32::MAX),
                    strikes: u32::try_from(int(d, "strikes")).unwrap_or(0),
                    until_ms: int(d, "until_ms"),
                    expires_ms: d.get_datetime("expires").ok()?.timestamp_millis(),
                })
            })
            .collect())
    }

    async fn add_invite(&self, hash: &str, uses: u32, expires_ms: i64) -> Result<(), AppError> {
        self.invites()
            .insert_one(doc! {
                "_id": hash,
                "left": i64::from(uses),
                "expires": DateTime::from_millis(expires_ms),
            })
            .await?;
        Ok(())
    }

    async fn take_invite(&self, hash: &str) -> Result<bool, AppError> {
        let now = DateTime::from_millis(crate::limits::wall_ms());
        Ok(self
            .invites()
            .find_one_and_update(
                doc! { "_id": hash, "left": { "$gt": 0 }, "expires": { "$gt": now } },
                doc! { "$inc": { "left": -1_i64 } },
            )
            .await?
            .is_some())
    }

    async fn return_invite(&self, hash: &str) -> Result<(), AppError> {
        self.invites()
            .update_one(doc! { "_id": hash }, doc! { "$inc": { "left": 1_i64 } })
            .await?;
        Ok(())
    }

    async fn invites_left(&self) -> Result<u64, AppError> {
        let now = DateTime::from_millis(crate::limits::wall_ms());
        Ok(self
            .invites()
            .count_documents(doc! { "left": { "$gt": 0 }, "expires": { "$gt": now } })
            .await?)
    }
}

/// An in-memory `Store` for tests. Any account id exists (version 0) until
/// it's changed; `remove_account` makes one disappear.
#[cfg(test)]
#[derive(Default)]
pub struct MemStore {
    inner: std::sync::Mutex<MemInner>,
}

#[cfg(test)]
#[derive(Default)]
struct MemInner {
    accounts: std::collections::HashMap<String, Account>,
    gone: std::collections::HashSet<String>,
    limits: std::collections::HashMap<String, LimitRec>,
    invites: std::collections::HashMap<String, (u32, i64)>,
}

#[cfg(test)]
impl MemStore {
    pub fn new() -> Arc<MemStore> {
        Arc::new(MemStore::default())
    }

    pub fn remove_account(&self, user_id: &str) {
        self.inner.lock().unwrap().gone.insert(user_id.to_string());
    }

    pub fn limit_records(&self) -> usize {
        self.inner.lock().unwrap().limits.len()
    }
}

#[cfg(test)]
#[axum::async_trait]
impl Store for MemStore {
    async fn account(&self, user_id: &str) -> Result<Option<Account>, AppError> {
        let g = self.inner.lock().unwrap();
        if g.gone.contains(user_id) {
            return Ok(None);
        }
        Ok(Some(g.accounts.get(user_id).cloned().unwrap_or_default()))
    }

    async fn bump_version(&self, user_id: &str) -> Result<(), AppError> {
        let mut g = self.inner.lock().unwrap();
        let a = g.accounts.entry(user_id.to_string()).or_default();
        a.ver += 1;
        a.revoked.clear();
        a.sessions.clear();
        Ok(())
    }

    async fn revoke(&self, user_id: &str, sid: &str, exp: i64) -> Result<(), AppError> {
        let mut g = self.inner.lock().unwrap();
        let a = g.accounts.entry(user_id.to_string()).or_default();
        let now = now_secs();
        a.revoked.retain(|r| r.exp >= now);
        a.revoked.push(Revoked {
            sid: sid.to_string(),
            exp,
        });
        a.sessions.retain(|r| r.sid != sid);
        Ok(())
    }

    async fn put_session(&self, user_id: &str, rec: SessionRec) -> Result<(), AppError> {
        let mut g = self.inner.lock().unwrap();
        let a = g.accounts.entry(user_id.to_string()).or_default();
        merge_session(&mut a.sessions, rec, now_secs());
        Ok(())
    }

    fn put_limit(&self, rec: LimitRec) {
        let mut g = self.inner.lock().unwrap();
        let id = format!("{}@{}", rec.key, rec.since_ms);
        let n = g.limits.get(&id).map_or(rec.n, |old| old.n.max(rec.n));
        g.limits.insert(id, LimitRec { n, ..rec });
    }

    async fn load_limits(&self, prefix: &str) -> Result<Vec<LimitRec>, AppError> {
        let now = crate::limits::wall_ms();
        Ok(self
            .inner
            .lock()
            .unwrap()
            .limits
            .values()
            .filter(|r| r.key.starts_with(prefix) && r.expires_ms > now)
            .cloned()
            .collect())
    }

    async fn add_invite(&self, hash: &str, uses: u32, expires_ms: i64) -> Result<(), AppError> {
        self.inner
            .lock()
            .unwrap()
            .invites
            .insert(hash.to_string(), (uses, expires_ms));
        Ok(())
    }

    async fn take_invite(&self, hash: &str) -> Result<bool, AppError> {
        let now = crate::limits::wall_ms();
        let mut g = self.inner.lock().unwrap();
        match g.invites.get_mut(hash) {
            Some((left, exp)) if *left > 0 && *exp > now => {
                *left -= 1;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn return_invite(&self, hash: &str) -> Result<(), AppError> {
        if let Some((left, _)) = self.inner.lock().unwrap().invites.get_mut(hash) {
            *left += 1;
        }
        Ok(())
    }

    async fn invites_left(&self) -> Result<u64, AppError> {
        let now = crate::limits::wall_ms();
        Ok(self
            .inner
            .lock()
            .unwrap()
            .invites
            .values()
            .filter(|(left, exp)| *left > 0 && *exp > now)
            .count() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(sid: &str, seen: i64, exp: i64) -> SessionRec {
        SessionRec {
            sid: sid.into(),
            created: seen,
            last_seen: seen,
            exp,
            node_id: String::new(),
            name: String::new(),
        }
    }

    #[test]
    fn sessions_merge_by_id_drop_the_expired_and_keep_the_most_recent() {
        let mut list = vec![rec("old", 1, 50), rec("a", 10, 1000)];
        merge_session(&mut list, rec("b", 20, 1000), 100);
        let ids: Vec<&str> = list.iter().map(|r| r.sid.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "the expired one is dropped");
        // Seen again, now from a device: updated in place.
        let mut again = rec("a", 30, 1000);
        again.node_id = "n1".into();
        again.name = "Pixel".into();
        merge_session(&mut list, again, 100);
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].last_seen, list[0].name.as_str()), (30, "Pixel"));
        assert_eq!(list[0].created, 10, "created stays");
        // Seen again without a device: the device stays.
        merge_session(&mut list, rec("a", 40, 1000), 100);
        assert_eq!(list[0].node_id, "n1");

        for i in 0..MAX_SESSIONS {
            merge_session(&mut list, rec(&format!("s{i}"), 100 + i as i64, 1000), 100);
        }
        assert_eq!(list.len(), MAX_SESSIONS);
        assert!(
            !list.iter().any(|r| r.sid == "a"),
            "least recently seen goes"
        );
    }

    #[test]
    fn prefixes_are_matched_literally() {
        assert_eq!(prefix_regex("push:").pattern, "^push\\:");
        assert_eq!(prefix_regex("a.b").pattern, "^a\\.b");
    }
}
