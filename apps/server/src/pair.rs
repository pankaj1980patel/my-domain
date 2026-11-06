//! Add a device without typing: a signed-in device asks for a short-lived,
//! single-use pairing, shows it as a QR code (and a 6-digit code), and the
//! new device trades it for a session of the same account. No password is
//! involved and none is ever sent.
//!
//!   POST /pair/create (signed in) {"claim": "<64 hex>", "code": "123456"?, "code_box": "<opaque>"?}
//!        -> {"code": "123456", "expires_in": 300}
//!   POST /pair/claim  {"claim": "<64 hex>"} or {"code": "123456"}, plus {"name": "Pixel 8"}
//!        -> {"token": "<session>", "username": "...", "code_box": "<opaque>"?}
//!   GET  /pair/status (signed in) -> {"state": "none" | "waiting" | "joined", "name"?}
//!   POST /pair/cancel (signed in)
//!
//! `claim` is a hash the creating device made from a secret that only goes
//! into the QR code; the server never sees that secret, nor the device key,
//! which travels inside the QR sealed under it. `code_box` is the device key
//! sealed under the 6-digit code (Argon2id), kept here only so a device that
//! can't scan can type the code instead; it is opaque to the server and is
//! dropped with the pairing. The device picks the code itself (it seals the
//! box under it) and makes another if the server says it's taken (409);
//! without one the server picks it.
//!
//! Each account has at most one pairing; making another replaces it. A
//! pairing lives `PAIR_TTL` (5 minutes) and is used once: a claim removes the
//! claim and the code at once, leaving only "joined" (with the new device's
//! name) for the creator's next status check. Failed claims are limited per
//! client address and server-wide, so a million codes can't be walked.
//! Pairings live in memory: a restart just cancels them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{make_token, verify_token, AuthUser};
use crate::devices::client_ip;
use crate::error::AppError;

/// How long a pairing can be claimed.
pub const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
/// Failed claims allowed per client address in `FAIL_WINDOW`.
const FAILS_PER_CLIENT: u32 = 10;
/// Failed claims allowed server-wide in `FAIL_WINDOW`: with one pairing
/// waiting, guessing its code by walking codes from many addresses has at
/// most a 1-in-3,000 chance over its 5 minutes.
const FAILS_GLOBAL: u32 = 60;
const FAIL_WINDOW: Duration = Duration::from_secs(60);
/// Longest `code_box` accepted (a sealed 32-byte key is far less).
const MAX_BOX: usize = 512;

struct Pairing {
    username: String,
    /// Cleared once claimed, so neither can be used again.
    claim: Option<String>,
    code: Option<String>,
    code_box: Option<String>,
    expires: Instant,
    /// The name of the device that joined, for the creator's next status.
    joined: Option<String>,
}

/// A count of failures since `since`.
#[derive(Clone, Copy)]
struct Fails {
    since: Instant,
    n: u32,
}

impl Fails {
    fn current(&mut self, now: Instant) -> &mut u32 {
        if now.saturating_duration_since(self.since) >= FAIL_WINDOW {
            *self = Fails { since: now, n: 0 };
        }
        &mut self.n
    }
}

#[derive(Default)]
struct Inner {
    /// user id -> that account's pairing.
    pairings: HashMap<String, Pairing>,
    fails: HashMap<String, Fails>,
    global: Option<Fails>,
}

#[derive(Clone)]
pub struct PairState {
    jwt_secret: String,
    ttl: Duration,
    inner: Arc<Mutex<Inner>>,
}

impl PairState {
    pub fn new(jwt_secret: String) -> Self {
        Self::with_ttl(jwt_secret, PAIR_TTL)
    }

    pub fn with_ttl(jwt_secret: String, ttl: Duration) -> Self {
        PairState {
            jwt_secret,
            ttl,
            inner: Arc::default(),
        }
    }
}

pub fn router(state: PairState) -> Router {
    Router::new()
        .route("/pair/create", post(create))
        .route("/pair/claim", post(claim))
        .route("/pair/status", get(status))
        .route("/pair/cancel", post(cancel))
        .with_state(state)
}

fn signed_in(s: &PairState, headers: &HeaderMap) -> Result<AuthUser, AppError> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;
    verify_token(&s.jwt_secret, token).ok_or(AppError::Unauthorized)
}

fn is_claim(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Inner {
    fn purge(&mut self, now: Instant) {
        self.pairings.retain(|_, p| p.expires > now);
    }

    fn code_in_use(&self, code: &str) -> bool {
        self.pairings
            .values()
            .any(|p| p.code.as_deref() == Some(code))
    }
}

/// A random 6-digit code (no modulo bias).
fn random_code() -> String {
    loop {
        let n = OsRng.next_u32();
        // 4_294_000_000 is the largest multiple of 1_000_000 below 2^32.
        if n < 4_294_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

#[derive(Deserialize)]
struct CreateReq {
    claim: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    code_box: Option<String>,
}

async fn create(
    State(s): State<PairState>,
    headers: HeaderMap,
    Json(req): Json<CreateReq>,
) -> Result<Json<Value>, AppError> {
    let user = signed_in(&s, &headers)?;
    let claim = req.claim.to_ascii_lowercase();
    if !is_claim(&claim) {
        return Err(AppError::BadRequest("claim must be 64 hex digits".into()));
    }
    if req.code_box.as_ref().is_some_and(|b| b.len() > MAX_BOX) {
        return Err(AppError::BadRequest("code_box too long".into()));
    }
    let now = Instant::now();
    let mut inner = s.inner.lock().unwrap();
    inner.purge(now);
    // Replaces this account's earlier pairing (so its code is free again).
    inner.pairings.remove(&user.user_id);
    if inner
        .pairings
        .values()
        .any(|p| p.claim.as_ref() == Some(&claim))
    {
        return Err(AppError::Conflict("claim already in use".into()));
    }
    let code = match req.code {
        Some(c) if c.len() != 6 || !c.bytes().all(|b| b.is_ascii_digit()) => {
            return Err(AppError::BadRequest("code must be 6 digits".into()));
        }
        Some(c) if inner.code_in_use(&c) => {
            return Err(AppError::Conflict("code in use".into()));
        }
        Some(c) => c,
        None => loop {
            let c = random_code();
            if !inner.code_in_use(&c) {
                break c;
            }
        },
    };
    inner.pairings.insert(
        user.user_id,
        Pairing {
            username: user.username,
            claim: Some(claim),
            code: Some(code.clone()),
            code_box: req.code_box,
            expires: now + s.ttl,
            joined: None,
        },
    );
    Ok(Json(json!({ "code": code, "expires_in": s.ttl.as_secs() })))
}

#[derive(Deserialize)]
struct ClaimReq {
    #[serde(default)]
    claim: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    name: String,
}

async fn claim(
    State(s): State<PairState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<ClaimReq>,
) -> Result<Json<Value>, AppError> {
    // Per address (the one Caddy saw, see `client_ip`).
    let client = client_ip(peer, &headers).to_string();
    let now = Instant::now();
    let mut inner = s.inner.lock().unwrap();
    inner.purge(now);
    let client_fails = inner
        .fails
        .get_mut(&client)
        .map(|f| *f.current(now))
        .unwrap_or(0);
    let global_fails = inner.global.as_mut().map(|f| *f.current(now)).unwrap_or(0);
    if client_fails >= FAILS_PER_CLIENT || global_fails >= FAILS_GLOBAL {
        return Err(AppError::TooMany);
    }

    let claim = req.claim.map(|c| c.trim().to_ascii_lowercase());
    let code: Option<String> = req
        .code
        .map(|c| c.chars().filter(|c| c.is_ascii_digit()).collect());
    let found = inner.pairings.iter().find_map(|(uid, p)| {
        let by_claim = claim.is_some() && p.claim.is_some() && p.claim == claim;
        let by_code = code.as_ref().is_some_and(|c| c.len() == 6) && p.code == code;
        (by_claim || by_code).then(|| (uid.clone(), by_code))
    });
    let Some((uid, by_code)) = found else {
        inner
            .fails
            .retain(|_, f| now.saturating_duration_since(f.since) < FAIL_WINDOW);
        *inner
            .fails
            .entry(client)
            .or_insert(Fails { since: now, n: 0 })
            .current(now) += 1;
        *inner
            .global
            .get_or_insert(Fails { since: now, n: 0 })
            .current(now) += 1;
        return Err(AppError::Gone(
            "this join code has expired or was already used".into(),
        ));
    };
    let p = inner.pairings.get_mut(&uid).expect("found above");
    let name = req.name.trim().chars().take(64).collect::<String>();
    let name = if name.is_empty() {
        "A new device".to_string()
    } else {
        name
    };
    let token = make_token(&s.jwt_secret, &uid, &p.username)?;
    // Single use: neither the claim nor the code works again.
    p.claim = None;
    p.code = None;
    p.joined = Some(name);
    let code_box = p.code_box.take().filter(|_| by_code);
    Ok(Json(json!({
        "token": token,
        "username": p.username,
        "code_box": code_box,
    })))
}

async fn status(State(s): State<PairState>, headers: HeaderMap) -> Result<Json<Value>, AppError> {
    let user = signed_in(&s, &headers)?;
    let mut inner = s.inner.lock().unwrap();
    inner.purge(Instant::now());
    let out = match inner.pairings.get(&user.user_id) {
        None => json!({ "state": "none" }),
        Some(Pairing {
            joined: Some(name), ..
        }) => {
            let name = name.clone();
            // Told once; the pairing is spent.
            inner.pairings.remove(&user.user_id);
            json!({ "state": "joined", "name": name })
        }
        Some(_) => json!({ "state": "waiting" }),
    };
    Ok(Json(out))
}

async fn cancel(State(s): State<PairState>, headers: HeaderMap) -> Result<StatusCode, AppError> {
    let user = signed_in(&s, &headers)?;
    s.inner.lock().unwrap().pairings.remove(&user.user_id);
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::issue_token;

    const SECRET: &str = "pair-test-secret";

    async fn start(state: PairState) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        format!("http://{addr}")
    }

    fn tok(user: &str) -> String {
        issue_token(SECRET, &format!("id-{user}"), user).unwrap()
    }

    fn claim_hex(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    async fn post(base: &str, path: &str, auth: Option<&str>, body: Value) -> (u16, Value) {
        let mut r = reqwest::Client::new()
            .post(format!("{base}{path}"))
            .json(&body);
        if let Some(t) = auth {
            r = r.bearer_auth(t);
        }
        let resp = r.send().await.unwrap();
        let code = resp.status().as_u16();
        (code, resp.json().await.unwrap_or(Value::Null))
    }

    async fn status_of(base: &str, auth: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{base}/pair/status"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_needs_a_session_and_a_proper_claim() {
        let base = start(PairState::new(SECRET.into())).await;
        let (c, _) = post(&base, "/pair/create", None, json!({"claim": claim_hex(1)})).await;
        assert_eq!(c, 401);
        let (c, _) = post(
            &base,
            "/pair/create",
            Some("nope"),
            json!({"claim": claim_hex(1)}),
        )
        .await;
        assert_eq!(c, 401);
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": "short"}),
        )
        .await;
        assert_eq!(c, 400);
        let (c, v) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(1)}),
        )
        .await;
        assert_eq!(c, 200);
        let code = v["code"].as_str().unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(v["expires_in"], 300);
        assert_eq!(status_of(&base, &tok("ann")).await["state"], "waiting");
    }

    #[tokio::test]
    async fn the_qr_claim_signs_in_the_same_account_once() {
        let base = start(PairState::new(SECRET.into())).await;
        let ann = tok("ann");
        post(
            &base,
            "/pair/create",
            Some(&ann),
            json!({"claim": claim_hex(7), "code_box": "BOX"}),
        )
        .await;
        let (c, v) = post(
            &base,
            "/pair/claim",
            None,
            json!({"claim": claim_hex(7), "name": "Pixel"}),
        )
        .await;
        assert_eq!(c, 200);
        assert_eq!(v["username"], "ann");
        // The code box is only for a typed code.
        assert!(v["code_box"].is_null());
        let session = verify_token(SECRET, v["token"].as_str().unwrap()).unwrap();
        assert_eq!(session.user_id, "id-ann");
        // Single use.
        let (c, _) = post(
            &base,
            "/pair/claim",
            None,
            json!({"claim": claim_hex(7), "name": "Eve"}),
        )
        .await;
        assert_eq!(c, 410);
        // The creator hears who joined, once.
        let s = status_of(&base, &ann).await;
        assert_eq!(s["state"], "joined");
        assert_eq!(s["name"], "Pixel");
        assert_eq!(status_of(&base, &ann).await["state"], "none");
    }

    #[tokio::test]
    async fn the_typed_code_works_once_and_hands_over_the_box() {
        let base = start(PairState::new(SECRET.into())).await;
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(2), "code_box": "BOX"}),
        )
        .await;
        let code = v["code"].as_str().unwrap().to_string();
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        let (c, v) = post(
            &base,
            "/pair/claim",
            None,
            json!({"code": spaced, "name": "Laptop"}),
        )
        .await;
        assert_eq!(c, 200);
        assert_eq!(v["code_box"], "BOX");
        let (c, _) = post(
            &base,
            "/pair/claim",
            None,
            json!({"code": code, "name": "Laptop"}),
        )
        .await;
        assert_eq!(c, 410);
        // The QR claim of the same pairing is spent too.
        let (c, _) = post(&base, "/pair/claim", None, json!({"claim": claim_hex(2)})).await;
        assert_eq!(c, 410);
    }

    #[tokio::test]
    async fn the_device_may_pick_the_code_but_not_one_in_use() {
        let base = start(PairState::new(SECRET.into())).await;
        let (c, v) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(10), "code": "424242"}),
        )
        .await;
        assert_eq!(c, 200);
        assert_eq!(v["code"], "424242");
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("bob")),
            json!({"claim": claim_hex(11), "code": "424242"}),
        )
        .await;
        assert_eq!(c, 409);
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("bob")),
            json!({"claim": claim_hex(11), "code": "4242"}),
        )
        .await;
        assert_eq!(c, 400);
        // Ann replacing her own pairing may keep the code.
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(12), "code": "424242"}),
        )
        .await;
        assert_eq!(c, 200);
    }

    #[tokio::test]
    async fn an_expired_pairing_cannot_be_claimed() {
        let base = start(PairState::with_ttl(
            SECRET.into(),
            Duration::from_millis(150),
        ))
        .await;
        let ann = tok("ann");
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&ann),
            json!({"claim": claim_hex(3)}),
        )
        .await;
        assert_eq!(v["expires_in"], 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (c, _) = post(&base, "/pair/claim", None, json!({"claim": claim_hex(3)})).await;
        assert_eq!(c, 410);
        assert_eq!(status_of(&base, &ann).await["state"], "none");
    }

    #[tokio::test]
    async fn another_account_cannot_see_or_cancel_it_and_a_new_one_replaces_it() {
        let base = start(PairState::new(SECRET.into())).await;
        let (ann, bob) = (tok("ann"), tok("bob"));
        post(
            &base,
            "/pair/create",
            Some(&ann),
            json!({"claim": claim_hex(4)}),
        )
        .await;
        assert_eq!(status_of(&base, &bob).await["state"], "none");
        let (c, _) = post(&base, "/pair/cancel", Some(&bob), json!({})).await;
        assert_eq!(c, 204);
        assert_eq!(status_of(&base, &ann).await["state"], "waiting");
        // Bob can't reuse Ann's claim for his own pairing.
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&bob),
            json!({"claim": claim_hex(4)}),
        )
        .await;
        assert_eq!(c, 409);
        // Ann's second pairing replaces her first.
        post(
            &base,
            "/pair/create",
            Some(&ann),
            json!({"claim": claim_hex(5)}),
        )
        .await;
        let (c, _) = post(&base, "/pair/claim", None, json!({"claim": claim_hex(4)})).await;
        assert_eq!(c, 410);
        // Cancelled: gone.
        post(&base, "/pair/cancel", Some(&ann), json!({})).await;
        let (c, _) = post(&base, "/pair/claim", None, json!({"claim": claim_hex(5)})).await;
        assert_eq!(c, 410);
    }

    #[tokio::test]
    async fn failed_claims_are_rate_limited() {
        let base = start(PairState::new(SECRET.into())).await;
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(6)}),
        )
        .await;
        let code = v["code"].as_str().unwrap().to_string();
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..FAILS_PER_CLIENT {
            let (c, _) = post(&base, "/pair/claim", None, json!({"code": wrong})).await;
            assert_eq!(c, 410);
        }
        // Now even the right code is refused from this address...
        let (c, _) = post(&base, "/pair/claim", None, json!({"code": code})).await;
        assert_eq!(c, 429);
        // ...but not from another one.
        let resp = reqwest::Client::new()
            .post(format!("{base}/pair/claim"))
            .header("x-forwarded-for", "203.0.113.9")
            .json(&json!({"code": code, "name": "Pixel"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn too_many_failures_server_wide_stop_all_claims() {
        let base = start(PairState::new(SECRET.into())).await;
        post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(8)}),
        )
        .await;
        let client = reqwest::Client::new();
        for i in 0..FAILS_GLOBAL {
            let resp = client
                .post(format!("{base}/pair/claim"))
                .header("x-forwarded-for", format!("198.51.100.{}", i % 250))
                .json(&json!({"claim": claim_hex(9)}))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status().as_u16(), 410);
        }
        let (c, _) = post(&base, "/pair/claim", None, json!({"claim": claim_hex(8)})).await;
        assert_eq!(c, 429);
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let c = random_code();
            assert_eq!(c.len(), 6);
            assert!(c.bytes().all(|b| b.is_ascii_digit()));
        }
    }
}
