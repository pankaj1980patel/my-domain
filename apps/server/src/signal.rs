// Signaling relay: a device posts a typed signal addressed to one of its OWN
// sibling devices. The server delivers it over the target's WebSocket when
// that's connected, else pushes it as an FCM data message to the target's
// token. Both `from` and `to` must belong to the caller's user (anti-spoof).
//
// Limits (security review M21), all per account and in memory:
// - every `/signal` and `/signal/selfping` request: `SIGNALS_PER_MIN`
//   (bursts of a fifth of it), checked before the database is asked;
// - a signal body at most `MAX_SIGNAL_BYTES`, its push payload at most
//   `PUSH_MAX_BYTES` (FCM refuses data over 4 KB);
// - pushes: `PUSH_PER_MIN` per account (bursts of half), `PUSH_PER_TARGET_PER_MIN`
//   per target device (bursts of a third) and `PUSH_PER_DAY` per account;
// - the same push (account, target, type, payload) again within
//   `PUSH_DEDUPE_SECS` is answered ok without sending it twice.
// Over a limit: 429 with `Retry-After`.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::{extract::State, Json};
use mongodb::bson::doc;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::fcm::FcmError;
use crate::limits::{env_or, Buckets, Recent, Windows};
use crate::models::Device;
use crate::state::AppState;

/// Largest signal body field (`data` as JSON); ones over the push cap still
/// go over a WebSocket.
pub const MAX_SIGNAL_BYTES: usize = 16 * 1024;
/// Longest signal `type`.
const MAX_KIND: usize = 64;
/// Most accounts / targets tracked by each limiter.
const MAX_TRACKED: usize = 100_000;

/// Signal and push limits (see the module doc).
#[derive(Clone, Copy, Debug)]
pub struct PushConfig {
    pub signals_per_min: f64,
    pub per_min: f64,
    pub per_target_per_min: f64,
    pub per_day: u32,
    pub max_bytes: usize,
    pub dedupe: Duration,
}

impl Default for PushConfig {
    fn default() -> Self {
        PushConfig {
            signals_per_min: 600.0,
            per_min: 60.0,
            per_target_per_min: 30.0,
            per_day: 5000,
            max_bytes: 3584,
            dedupe: Duration::from_secs(10),
        }
    }
}

impl PushConfig {
    pub fn from_env() -> Self {
        let d = PushConfig::default();
        PushConfig {
            signals_per_min: env_or("SIGNALS_PER_MIN", d.signals_per_min),
            per_min: env_or("PUSH_PER_MIN", d.per_min),
            per_target_per_min: env_or("PUSH_PER_TARGET_PER_MIN", d.per_target_per_min),
            per_day: env_or("PUSH_PER_DAY", d.per_day),
            max_bytes: env_or("PUSH_MAX_BYTES", d.max_bytes),
            dedupe: Duration::from_secs(env_or("PUSH_DEDUPE_SECS", d.dedupe.as_secs())),
        }
    }
}

/// What to do with a push that passed the limits.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Send,
    /// The same push went out moments ago.
    Duplicate,
}

struct PushInner {
    signals: Buckets<String>,
    account: Buckets<String>,
    target: Buckets<(String, String)>,
    daily: Windows<String>,
    recent: Recent<u64>,
}

/// Per-account signal and push limiter, shared by the handlers.
pub struct PushLimiter {
    cfg: PushConfig,
    hasher: RandomState,
    inner: Mutex<PushInner>,
}

impl PushLimiter {
    pub fn new(cfg: PushConfig) -> Self {
        PushLimiter {
            cfg,
            hasher: RandomState::new(),
            inner: Mutex::new(PushInner {
                signals: Buckets::per_minute(
                    cfg.signals_per_min,
                    cfg.signals_per_min / 5.0,
                    MAX_TRACKED,
                ),
                account: Buckets::per_minute(cfg.per_min, cfg.per_min / 2.0, MAX_TRACKED),
                target: Buckets::per_minute(
                    cfg.per_target_per_min,
                    cfg.per_target_per_min / 3.0,
                    MAX_TRACKED,
                ),
                daily: Windows::new(Duration::from_secs(24 * 3600), MAX_TRACKED),
                recent: Recent::new(cfg.dedupe, MAX_TRACKED),
            }),
        }
    }

    /// One signal request from `user` (before anything else is done).
    pub fn signal(&self, user: &str, now: Instant) -> Result<(), AppError> {
        self.inner
            .lock()
            .unwrap()
            .signals
            .try_take(&user.to_string(), now)
            .map_err(|w| AppError::RateLimited(w, "too many signals, slow down".into()))
    }

    /// The push payload as FCM will get it, if it fits.
    pub fn sized(&self, data: serde_json::Value) -> Result<serde_json::Value, AppError> {
        if data.to_string().len() > self.cfg.max_bytes {
            return Err(AppError::TooLarge(format!(
                "push payload over {} bytes",
                self.cfg.max_bytes
            )));
        }
        Ok(data)
    }

    /// A key for "the same push": account, target and payload.
    pub fn digest(&self, user: &str, to: &str, data: &serde_json::Value) -> u64 {
        self.hasher.hash_one((user, to, data.to_string()))
    }

    /// Whether `user` may push to `to` now; charges the limits when it may.
    pub fn push(&self, user: &str, to: &str, digest: u64, now: Instant) -> Result<Admit, AppError> {
        let mut g = self.inner.lock().unwrap();
        if g.recent.seen(&digest, now) {
            return Ok(Admit::Duplicate);
        }
        let user = user.to_string();
        let target = (user.clone(), to.to_string());
        let mut wait = g.account.wait(&user, now).max(g.target.wait(&target, now));
        if g.daily.count(&user, now) >= self.cfg.per_day {
            wait = wait.max(g.daily.resets_in(&user, now));
        }
        if !wait.is_zero() {
            return Err(AppError::RateLimited(
                wait,
                "too many pushes, slow down".into(),
            ));
        }
        g.account.take(&user, now);
        g.target.take(&target, now);
        g.daily.add(&user, now);
        Ok(Admit::Send)
    }

    /// The push went out: a repeat within the dedupe window is dropped.
    pub fn sent(&self, digest: u64, now: Instant) {
        self.inner.lock().unwrap().recent.insert(digest, now);
    }
}

fn check_len(name: &str, v: &str, max: usize) -> Result<(), AppError> {
    if v.is_empty() || v.len() > max {
        return Err(AppError::BadRequest(format!(
            "{name} must be 1-{max} bytes"
        )));
    }
    Ok(())
}

/// Send `data` to `token` within the push limits; `Ok(true)` if it was a
/// duplicate and wasn't sent again.
async fn limited_push(
    s: &AppState,
    user: &str,
    to: &str,
    token: &str,
    data: serde_json::Value,
) -> Result<bool, FcmPushError> {
    let fcm = s
        .fcm
        .as_ref()
        .ok_or(AppError::Internal("FCM not configured on server".into()))?;
    let data = s.push.sized(data)?;
    let digest = s.push.digest(user, to, &data);
    if s.push.push(user, to, digest, Instant::now())? == Admit::Duplicate {
        return Ok(true);
    }
    fcm.send_data(token, data).await?;
    s.push.sent(digest, Instant::now());
    Ok(false)
}

/// Why `limited_push` didn't send.
enum FcmPushError {
    App(AppError),
    Fcm(FcmError),
}

impl From<AppError> for FcmPushError {
    fn from(e: AppError) -> Self {
        FcmPushError::App(e)
    }
}

impl From<FcmError> for FcmPushError {
    fn from(e: FcmError) -> Self {
        FcmPushError::Fcm(e)
    }
}

#[derive(Deserialize)]
pub struct SignalReq {
    /// Caller's own node_id (verified to belong to the caller).
    pub from: String,
    /// Target sibling node_id.
    pub to: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Type-specific payload (candidates, session id, timing, …).
    #[serde(default)]
    pub data: serde_json::Value,
}

pub async fn relay(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<SignalReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    s.push.signal(&user.user_id, Instant::now())?;
    check_len("from", &req.from, crate::devices::MAX_NODE_ID)?;
    check_len("to", &req.to, crate::devices::MAX_NODE_ID)?;
    check_len("type", &req.kind, MAX_KIND)?;
    if req.data.to_string().len() > MAX_SIGNAL_BYTES {
        return Err(AppError::TooLarge(format!(
            "signal data over {MAX_SIGNAL_BYTES} bytes"
        )));
    }
    if req.from == req.to {
        return Err(AppError::BadRequest("cannot signal self".into()));
    }
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);

    // Anti-spoof: the claimed sender must be one of the caller's devices.
    if coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.from })
        .await?
        .is_none()
    {
        return Err(AppError::BadRequest("unknown 'from' device".into()));
    }
    let target = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.to })
        .await?
        .ok_or(AppError::BadRequest("unknown 'to' device".into()))?;

    // 1. The target's WebSocket, if it's connected: instant, and needs no
    //    push service (desktops are always on; phones while the app runs).
    let frame = serde_json::json!({
        "signal": { "type": req.kind, "sender": req.from, "payload": req.data.to_string() }
    });
    if s.relay
        .deliver_signal(&user.user_id, &req.to, frame.to_string())
    {
        return Ok(Json(serde_json::json!({ "ok": true, "via": "ws" })));
    }
    // 2. Otherwise an FCM push, which can wake a sleeping phone.
    let token = target.fcm_token.clone().ok_or(AppError::Conflict(
        "target has no FCM token (offline)".into(),
    ))?;

    // FCM data payloads are string→string; the typed body is stringified.
    // NB: `from` is an FCM-reserved data key (rejected with INVALID_ARGUMENT),
    // so the sender node id travels as `sender`.
    let data = serde_json::json!({
        "type": req.kind,
        "sender": req.from,
        "to": req.to,
        "payload": req.data.to_string(),
    });

    match limited_push(&s, &user.user_id, &req.to, &token, data).await {
        Ok(false) => Ok(Json(serde_json::json!({ "ok": true }))),
        Ok(true) => Ok(Json(serde_json::json!({ "ok": true, "dup": true }))),
        Err(FcmPushError::App(e)) => Err(e),
        Err(FcmPushError::Fcm(FcmError::Unregistered)) => {
            let _ = coll
                .update_one(
                    doc! { "user_id": &user.user_id, "node_id": &req.to },
                    doc! { "$unset": { "fcm_token": "" } },
                )
                .await;
            Err(AppError::Conflict(
                "target FCM token invalid (cleared)".into(),
            ))
        }
        Err(FcmPushError::Fcm(e)) => Err(AppError::Internal(format!("FCM send failed: {e:?}"))),
    }
}

#[derive(Deserialize)]
pub struct SelfPingReq {
    /// Caller's own node_id (verified to belong to the caller).
    pub node_id: String,
    /// Correlation id echoed back in the pushed payload.
    #[serde(default)]
    pub sid: String,
}

/// Round-trip FCM self-test: push a `ping` data message to the caller's OWN
/// device token. Unlike `relay`, self-addressing is allowed here — that's the
/// whole point: the device verifies it can actually receive its own pushes.
pub async fn selfping(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<SelfPingReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    s.push.signal(&user.user_id, Instant::now())?;
    check_len("node_id", &req.node_id, crate::devices::MAX_NODE_ID)?;
    if req.sid.len() > MAX_KIND {
        return Err(AppError::BadRequest(format!(
            "sid must be at most {MAX_KIND} bytes"
        )));
    }
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    let device = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?
        .ok_or(AppError::BadRequest("unknown device".into()))?;

    let token = device.fcm_token.clone().ok_or(AppError::Conflict(
        "device has no FCM token (offline)".into(),
    ))?;

    let data = serde_json::json!({
        "type": "ping",
        "sender": req.node_id,
        "to": req.node_id,
        "payload": format!("selfping:{}", req.sid),
    });

    match limited_push(&s, &user.user_id, &req.node_id, &token, data).await {
        Ok(_) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(FcmPushError::App(e)) => Err(e),
        Err(FcmPushError::Fcm(FcmError::Unregistered)) => {
            let _ = coll
                .update_one(
                    doc! { "user_id": &user.user_id, "node_id": &req.node_id },
                    doc! { "$unset": { "fcm_token": "" } },
                )
                .await;
            Err(AppError::Conflict("FCM token invalid (cleared)".into()))
        }
        Err(FcmPushError::Fcm(e)) => Err(AppError::Internal(format!("FCM send failed: {e:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn small() -> PushConfig {
        PushConfig {
            signals_per_min: 600.0,
            per_min: 60.0,
            per_target_per_min: 30.0,
            per_day: 50,
            max_bytes: 3584,
            dedupe: Duration::from_secs(10),
        }
    }

    fn retry(e: AppError) -> Duration {
        match e {
            AppError::RateLimited(w, _) => w,
            _ => panic!("not a rate limit"),
        }
    }

    #[test]
    fn pushes_per_target_then_per_account_are_limited_with_a_retry_time() {
        let l = PushLimiter::new(small());
        let t0 = Instant::now();
        // Per target: a burst of 10, then one every 2 s.
        for i in 0..10u64 {
            assert_eq!(l.push("ann", "phone", i, t0).unwrap(), Admit::Send);
        }
        let w = retry(l.push("ann", "phone", 99, t0).unwrap_err());
        assert!(
            w > Duration::from_millis(1900) && w <= Duration::from_secs(2),
            "{w:?}"
        );
        // Other targets share the account's burst of 30.
        for i in 0..20u64 {
            assert_eq!(
                l.push("ann", &format!("pc{i}"), 100 + i, t0).unwrap(),
                Admit::Send
            );
        }
        let w = retry(l.push("ann", "laptop", 200, t0).unwrap_err());
        assert!(w <= Duration::from_secs(1), "{w:?}");
        // Another account is untouched; after waiting, ann may push again.
        assert!(l.push("bob", "phone", 300, t0).is_ok());
        assert!(l
            .push("ann", "laptop", 201, t0 + Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn an_account_has_a_daily_ceiling() {
        let l = PushLimiter::new(small());
        let mut t = Instant::now();
        for i in 0..50u64 {
            assert!(l.push("ann", &format!("d{}", i % 5), i, t).is_ok(), "{i}");
            t += Duration::from_secs(3);
        }
        let w = retry(l.push("ann", "d0", 999, t).unwrap_err());
        assert!(w > Duration::from_secs(23 * 3600), "{w:?}");
        assert!(l.push("ann", "d0", 999, t + w).is_ok());
    }

    #[test]
    fn the_same_push_within_the_window_is_sent_once() {
        let l = PushLimiter::new(small());
        let t0 = Instant::now();
        let data = json!({"type": "signal", "payload": "x"});
        let d = l.digest("ann", "phone", &data);
        assert_eq!(l.push("ann", "phone", d, t0).unwrap(), Admit::Send);
        l.sent(d, t0);
        assert_eq!(
            l.push("ann", "phone", d, t0 + Duration::from_secs(5))
                .unwrap(),
            Admit::Duplicate
        );
        assert_eq!(
            l.push("ann", "phone", d, t0 + Duration::from_secs(11))
                .unwrap(),
            Admit::Send
        );
        // Another target, account or payload is a different push.
        assert_ne!(d, l.digest("ann", "pc", &data));
        assert_ne!(d, l.digest("bob", "phone", &data));
        assert_ne!(
            d,
            l.digest("ann", "phone", &json!({"type": "signal", "payload": "y"}))
        );
        // A push that failed to send isn't remembered, so a retry goes.
        let d2 = l.digest("ann", "phone", &json!({"n": 2}));
        assert_eq!(l.push("ann", "phone", d2, t0).unwrap(), Admit::Send);
        assert_eq!(l.push("ann", "phone", d2, t0).unwrap(), Admit::Send);
    }

    #[test]
    fn oversized_push_payloads_are_refused() {
        let l = PushLimiter::new(small());
        assert!(l.sized(json!({"payload": "x".repeat(3000)})).is_ok());
        assert!(matches!(
            l.sized(json!({"payload": "x".repeat(3600)})),
            Err(AppError::TooLarge(_))
        ));
    }

    #[test]
    fn signal_requests_are_limited_per_account() {
        let l = PushLimiter::new(PushConfig {
            signals_per_min: 60.0,
            ..small()
        });
        let t0 = Instant::now();
        for _ in 0..12 {
            assert!(l.signal("ann", t0).is_ok());
        }
        assert!(retry(l.signal("ann", t0).unwrap_err()) <= Duration::from_secs(1));
        assert!(l.signal("bob", t0).is_ok());
    }
}
