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
//! name) for the creator's next status check. Pairings live in memory: a
//! restart just cancels them.
//!
//! Guessing limits (security review M18), none of which can stop a right
//! claim from an address that hasn't been failing:
//! - Per client address (an IPv6 /64 counts as one): `fails_per_client`
//!   wrong claims in a minute lock that address out, for a minute the first
//!   time and twice as long each time after (up to an hour; forgotten after
//!   an hour of quiet). Answers are 429 with `Retry-After`. A lockout is
//!   saved in the store when it starts and read back at startup, so a
//!   restart neither lifts it nor resets the doubling.
//! - Per pairing: every wrong code is a guess at each live code, so a
//!   pairing's code stops working after `code_lock_after` wrong codes were
//!   tried while it was live (at most a 0.1% chance of a hit by default,
//!   however many addresses guess). Its QR claim keeps working and the
//!   creator can make a new code; `/pair/status` says `code_locked`.
//! - Server-wide, soft: past `fails_global` wrong codes a minute, each
//!   address's allowance halves for every multiple of it (down to 1), so a
//!   spread-out walk is slowed further. Nobody is blocked by it.
//! - A QR claim (64 hex) can't be guessed: a wrong one counts only against
//!   its own address, never server-wide or against a pairing.

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

use crate::auth::AuthUser;
use crate::devices::client_ip;
use crate::error::AppError;
use crate::limits::{client_key, from_wall, to_wall, wall_ms};
use crate::session::Sessions;
use crate::store::LimitRec;

/// How long a pairing can be claimed.
pub const PAIR_TTL: Duration = Duration::from_secs(5 * 60);
/// Failed claims allowed per client address in `FAIL_WINDOW`
/// (`PAIR_FAILS_PER_CLIENT`).
const FAILS_PER_CLIENT: u32 = 10;
/// Wrong codes a minute server-wide before each address's allowance starts
/// halving (`PAIR_FAILS_GLOBAL`).
const FAILS_GLOBAL: u32 = 120;
/// Wrong codes tried while a pairing is live before its code stops working
/// (`PAIR_CODE_LOCK_AFTER`): at most a 1-in-1,000 chance of a hit.
const CODE_LOCK_AFTER: u64 = 1000;
const FAIL_WINDOW: Duration = Duration::from_secs(60);
/// Longest lockout of one address (it doubles per lockout up to this).
const MAX_LOCKOUT: Duration = Duration::from_secs(60 * 60);
/// An address quiet this long starts again from a one-minute lockout.
const STRIKE_MEMORY: Duration = Duration::from_secs(60 * 60);
/// Most client addresses tracked (the ones heard from longest ago go first).
const MAX_CLIENTS: usize = 100_000;
/// Largest request body on the pairing routes.
const MAX_BODY: usize = 8 * 1024;
/// Pairings with a code the device chose, per account in `FAIL_WINDOW`.
/// "Code in use" answers say a code is live somewhere, so making pairings
/// with chosen codes is itself a way to walk the codes: an honest device
/// makes one (and very rarely a second after a clash).
const CHOSEN_CODES_PER_USER: u32 = 5;
/// The same, server-wide: with these caps a walk of chosen codes over a
/// pairing's 5 minutes sees at most 600 of the million codes.
const CHOSEN_CODES_GLOBAL: u32 = 120;
/// Most pairings waiting at once, server-wide.
const MAX_PAIRINGS: usize = 10_000;
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
    /// `Inner::code_fails_total` when it was made (see `code_locked`).
    fails_base: u64,
}

/// Guessing limits (see the module doc), from the environment by default.
#[derive(Clone, Copy, Debug)]
pub struct PairLimits {
    pub fails_per_client: u32,
    pub fails_global: u32,
    pub code_lock_after: u64,
}

impl Default for PairLimits {
    fn default() -> Self {
        PairLimits {
            fails_per_client: FAILS_PER_CLIENT,
            fails_global: FAILS_GLOBAL,
            code_lock_after: CODE_LOCK_AFTER,
        }
    }
}

impl PairLimits {
    pub fn from_env() -> Self {
        use crate::limits::env_or;
        PairLimits {
            fails_per_client: env_or("PAIR_FAILS_PER_CLIENT", FAILS_PER_CLIENT),
            fails_global: env_or("PAIR_FAILS_GLOBAL", FAILS_GLOBAL),
            code_lock_after: env_or("PAIR_CODE_LOCK_AFTER", CODE_LOCK_AFTER),
        }
    }
}

/// One client address's failed claims and lockouts.
struct Client {
    /// Failures in the window that started at `since`.
    since: Instant,
    n: u32,
    /// Lockouts so far (each doubles the next).
    strikes: u32,
    locked_until: Option<Instant>,
    last: Instant,
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
    /// Client address (IPv6 by /64) -> its failures.
    clients: HashMap<String, Client>,
    clients_swept: Option<Instant>,
    /// Wrong codes server-wide in the current minute.
    code_fails: Option<Fails>,
    /// Wrong codes ever (a pairing's code locks `code_lock_after` past
    /// the value when it was made).
    code_fails_total: u64,
    /// user id -> pairings made with a chosen code lately.
    chosen: HashMap<String, Fails>,
    chosen_global: Option<Fails>,
}

#[derive(Clone)]
pub struct PairState {
    sessions: Arc<Sessions>,
    ttl: Duration,
    limits: PairLimits,
    inner: Arc<Mutex<Inner>>,
}

impl PairState {
    pub fn new(sessions: Arc<Sessions>) -> Self {
        Self::with(sessions, PAIR_TTL, PairLimits::from_env())
    }

    #[cfg(test)]
    pub fn with_ttl(sessions: Arc<Sessions>, ttl: Duration) -> Self {
        Self::with(sessions, ttl, PairLimits::default())
    }

    pub fn with(sessions: Arc<Sessions>, ttl: Duration, limits: PairLimits) -> Self {
        PairState {
            sessions,
            ttl,
            limits,
            inner: Arc::default(),
        }
    }

    /// Read back the lockouts saved before a restart; how many.
    pub async fn restore(&self) -> Result<usize, AppError> {
        let recs = self.sessions.store().load_limits(LOCKOUT_PREFIX).await?;
        let (now, now_ms) = (Instant::now(), wall_ms());
        let mut inner = self.inner.lock().unwrap();
        let mut n = 0;
        for r in recs {
            let Some(key) = r.key.strip_prefix(LOCKOUT_PREFIX) else {
                continue;
            };
            let last_ms = r.expires_ms - STRIKE_MEMORY.as_millis() as i64;
            let (Some(last), until) = (
                from_wall(last_ms, now, now_ms),
                from_wall(r.until_ms, now, now_ms),
            ) else {
                continue;
            };
            let keep_newer = inner
                .clients
                .get(key)
                .is_some_and(|c| c.strikes >= r.strikes);
            if keep_newer {
                continue;
            }
            inner.clients.insert(
                key.to_string(),
                Client {
                    since: now,
                    n: 0,
                    strikes: r.strikes,
                    locked_until: until.filter(|u| *u > now),
                    last,
                },
            );
            n += 1;
        }
        Ok(n)
    }

    /// Counts only (for `/admin/limits`).
    pub fn stats(&self) -> Value {
        let now = Instant::now();
        let mut inner = self.inner.lock().unwrap();
        inner.purge(now);
        let locked = inner
            .clients
            .values()
            .filter(|c| c.locked_until.is_some_and(|u| u > now))
            .count();
        json!({
            "pairings": inner.pairings.len(),
            "addresses_tracked": inner.clients.len(),
            "addresses_locked_out": locked,
            "wrong_codes_total": inner.code_fails_total,
        })
    }
}

/// Lockout records in the store are keyed `pair:<address>`.
const LOCKOUT_PREFIX: &str = "pair:";

pub fn router(state: PairState) -> Router {
    Router::new()
        .route("/pair/create", post(create))
        .route("/pair/claim", post(claim))
        .route("/pair/status", get(status))
        .route("/pair/cancel", post(cancel))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .with_state(state)
}

async fn signed_in(s: &PairState, headers: &HeaderMap) -> Result<AuthUser, AppError> {
    let token = crate::auth::bearer(headers).ok_or(AppError::Unauthorized)?;
    s.sessions.verify(token).await
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

    fn code_locked(&self, p: &Pairing, lim: &PairLimits) -> bool {
        self.code_fails_total.saturating_sub(p.fails_base) >= lim.code_lock_after
    }

    /// How many times over the soft server-wide ceiling wrong codes are this
    /// minute (0 = under it); each level halves every address's allowance.
    fn pressure(&mut self, now: Instant, lim: &PairLimits) -> u32 {
        let n = self
            .code_fails
            .as_mut()
            .map(|f| *f.current(now))
            .unwrap_or(0);
        (n / lim.fails_global.max(1)).min(8)
    }

    /// Failures one address may have this minute.
    fn allowance(&mut self, now: Instant, lim: &PairLimits) -> u32 {
        (lim.fails_per_client >> self.pressure(now, lim)).max(1)
    }

    /// How long `key` must wait before claiming (zero = it may).
    fn client_wait(&mut self, key: &str, now: Instant, lim: &PairLimits) -> Duration {
        let allowance = self.allowance(now, lim);
        let Some(c) = self.clients.get(key) else {
            return Duration::ZERO;
        };
        if let Some(until) = c.locked_until.filter(|u| *u > now) {
            return until - now;
        }
        let in_window = now.saturating_duration_since(c.since) < FAIL_WINDOW;
        if in_window && c.n >= allowance {
            // The allowance shrank under pressure after these failures.
            return (c.since + FAIL_WINDOW).saturating_duration_since(now);
        }
        Duration::ZERO
    }

    /// Count a failed claim from `key`; `code` = it was a typed code (a
    /// guess at every live code) rather than a QR claim. A lockout that
    /// starts is returned, to be saved.
    fn failed(
        &mut self,
        key: String,
        code: bool,
        now: Instant,
        lim: &PairLimits,
    ) -> Option<LimitRec> {
        if code {
            *self
                .code_fails
                .get_or_insert(Fails { since: now, n: 0 })
                .current(now) += 1;
            self.code_fails_total += 1;
        }
        let allowance = self.allowance(now, lim);
        self.sweep_clients(now);
        let c = self.clients.entry(key.clone()).or_insert(Client {
            since: now,
            n: 0,
            strikes: 0,
            locked_until: None,
            last: now,
        });
        if now.saturating_duration_since(c.last) >= STRIKE_MEMORY {
            c.strikes = 0;
        }
        if now.saturating_duration_since(c.since) >= FAIL_WINDOW {
            c.since = now;
            c.n = 0;
        }
        c.last = now;
        c.n += 1;
        if c.n < allowance {
            return None;
        }
        // Locked out: one window, doubled per lockout before it.
        let lockout = FAIL_WINDOW
            .saturating_mul(1u32 << c.strikes.min(10))
            .min(MAX_LOCKOUT);
        c.strikes += 1;
        c.locked_until = Some(now + lockout);
        c.since = now;
        c.n = 0;
        // Saved until the strikes would be forgotten.
        Some(LimitRec {
            key: format!("{LOCKOUT_PREFIX}{key}"),
            since_ms: 0,
            n: 0,
            strikes: c.strikes,
            until_ms: to_wall(now + lockout),
            expires_ms: to_wall(now + STRIKE_MEMORY.max(lockout)),
        })
    }

    fn sweep_clients(&mut self, now: Instant) {
        let due = self
            .clients_swept
            .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(30));
        if !due && self.clients.len() < MAX_CLIENTS {
            return;
        }
        self.clients_swept = Some(now);
        self.clients.retain(|_, c| {
            c.locked_until.is_some_and(|u| u > now)
                || now.saturating_duration_since(c.last) < STRIKE_MEMORY
        });
        if self.clients.len() >= MAX_CLIENTS {
            crate::limits::evict_oldest(&mut self.clients, |c| c.last);
        }
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
    let user = signed_in(&s, &headers).await?;
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
    // A chosen code is a guess at the codes in use: limited per account and
    // server-wide, and counted before it's checked.
    if req.code.is_some() {
        inner
            .chosen
            .retain(|_, f| now.saturating_duration_since(f.since) < FAIL_WINDOW);
        let mine = *inner
            .chosen
            .entry(user.user_id.clone())
            .or_insert(Fails { since: now, n: 0 })
            .current(now);
        let all = *inner
            .chosen_global
            .get_or_insert(Fails { since: now, n: 0 })
            .current(now);
        if mine >= CHOSEN_CODES_PER_USER || all >= CHOSEN_CODES_GLOBAL {
            return Err(AppError::TooMany);
        }
        if let Some(f) = inner.chosen.get_mut(&user.user_id) {
            *f.current(now) += 1;
        }
        if let Some(f) = inner.chosen_global.as_mut() {
            *f.current(now) += 1;
        }
    }
    // Replaces this account's earlier pairing (so its code is free again).
    inner.pairings.remove(&user.user_id);
    if inner.pairings.len() >= MAX_PAIRINGS {
        return Err(AppError::TooMany);
    }
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
    let fails_base = inner.code_fails_total;
    inner.pairings.insert(
        user.user_id,
        Pairing {
            username: user.username,
            claim: Some(claim),
            code: Some(code.clone()),
            code_box: req.code_box,
            expires: now + s.ttl,
            joined: None,
            fails_base,
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
    let client = client_key(client_ip(peer, &headers));
    let lim = s.limits;
    let now = Instant::now();
    let claim = req
        .claim
        .map(|c| c.trim().to_ascii_lowercase())
        .filter(|c| is_claim(c));
    let code: Option<String> = req
        .code
        .map(|c| c.chars().filter(|c| c.is_ascii_digit()).collect());
    // The pairing it matches, if any (`by_code`: matched by the code).
    let matches = |inner: &Inner, uid: &str, by_code: bool| -> bool {
        inner.pairings.get(uid).is_some_and(|p| {
            if by_code {
                code.as_ref().is_some_and(|c| c.len() == 6)
                    && p.code == code
                    && !inner.code_locked(p, &lim)
            } else {
                claim.is_some() && p.claim == claim
            }
        })
    };
    let (uid, by_code, username) = {
        let mut inner = s.inner.lock().unwrap();
        inner.purge(now);
        let wait = inner.client_wait(&client, now, &lim);
        if !wait.is_zero() {
            return Err(AppError::RateLimited(
                wait,
                "too many wrong tries from here, wait a while".into(),
            ));
        }
        let found = inner.pairings.iter().find_map(|(uid, p)| {
            let by_claim = matches(&inner, uid, false);
            let by_code = matches(&inner, uid, true);
            (by_claim || by_code).then(|| (uid.clone(), by_code, p.username.clone()))
        });
        let Some(found) = found else {
            // A wrong QR claim guesses nothing (2^256), so it counts only
            // against its own address; anything else is a guess at the codes.
            let guessed_code = code.as_ref().is_some_and(|c| c.len() == 6) || claim.is_none();
            if let Some(rec) = inner.failed(client, guessed_code, now, &lim) {
                s.sessions.store().put_limit(rec);
            }
            return Err(AppError::Gone(
                "this join code has expired or was already used".into(),
            ));
        };
        found
    };
    // Issued at the account's token version (a store read, so outside the
    // lock), then the pairing is checked again before it's spent.
    let token = s.sessions.issue(&uid, &username).await?;
    let mut inner = s.inner.lock().unwrap();
    if !matches(&inner, &uid, by_code) {
        return Err(AppError::Gone(
            "this join code has expired or was already used".into(),
        ));
    }
    let p = inner.pairings.get_mut(&uid).expect("checked above");
    let name = req.name.trim().chars().take(64).collect::<String>();
    let name = if name.is_empty() {
        "A new device".to_string()
    } else {
        name
    };
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
    let user = signed_in(&s, &headers).await?;
    let mut inner = s.inner.lock().unwrap();
    inner.purge(Instant::now());
    let locked = inner
        .pairings
        .get(&user.user_id)
        .is_some_and(|p| inner.code_locked(p, &s.limits));
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
        Some(_) => json!({ "state": "waiting", "code_locked": locked }),
    };
    Ok(Json(out))
}

async fn cancel(State(s): State<PairState>, headers: HeaderMap) -> Result<StatusCode, AppError> {
    let user = signed_in(&s, &headers).await?;
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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
        let session = crate::session::decode_token(SECRET, v["token"].as_str().unwrap()).unwrap();
        assert_eq!(session.sub, "id-ann");
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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
    async fn chosen_codes_cannot_be_walked_to_find_another_accounts_pairing() {
        let base = start(PairState::new(Sessions::test(SECRET))).await;
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(20), "code": "777777"}),
        )
        .await;
        assert_eq!(c, 200);
        // Mallory walks codes: a few tries, then she is refused before the
        // code is even looked at (so a later hit tells her nothing).
        let mut answers = Vec::new();
        for i in 0..8u32 {
            let (c, _) = post(
                &base,
                "/pair/create",
                Some(&tok("mallory")),
                json!({"claim": claim_hex(21), "code": format!("{:06}", 777770 + i)}),
            )
            .await;
            answers.push(c);
        }
        assert_eq!(&answers[..5], &[200; 5]);
        assert!(answers[5..].iter().all(|c| *c == 429), "{answers:?}");
        // A pairing with a server-picked code is not limited this way.
        let (c, _) = post(
            &base,
            "/pair/create",
            Some(&tok("mallory")),
            json!({"claim": claim_hex(22)}),
        )
        .await;
        assert_eq!(c, 200);
    }

    #[tokio::test]
    async fn an_expired_pairing_cannot_be_claimed() {
        let base = start(PairState::with_ttl(
            Sessions::test(SECRET),
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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
        let base = start(PairState::new(Sessions::test(SECRET))).await;
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

    /// A claim from `ip` (the test client talks from loopback, so the
    /// forwarded address is trusted as the proxy's).
    async fn claim_from(base: &str, ip: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{base}/pair/claim"))
            .header("x-forwarded-for", ip)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// A 6-digit code that is none of `taken`.
    fn wrong_code(i: u32, taken: &[&str]) -> String {
        (i..)
            .map(|n| format!("{:06}", n % 1_000_000))
            .find(|c| !taken.contains(&c.as_str()))
            .unwrap()
    }

    #[tokio::test]
    async fn a_lockout_says_when_to_retry() {
        let base = start(PairState::new(Sessions::test(SECRET))).await;
        for _ in 0..FAILS_PER_CLIENT {
            let r = claim_from(&base, "198.51.100.1", json!({"code": "000000"})).await;
            assert_eq!(r.status().as_u16(), 410);
        }
        let r = claim_from(&base, "198.51.100.1", json!({"code": "000000"})).await;
        assert_eq!(r.status().as_u16(), 429);
        let secs: u64 = r.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60).contains(&secs), "{secs}");
    }

    #[tokio::test]
    async fn a_flood_of_wrong_claims_does_not_stop_right_claims_for_other_codes() {
        // The old hard server-wide limit (M18): 60 bad claims a minute put
        // every claim on the server at 429. Now nothing blocks a right claim
        // from an address that hasn't been failing.
        let lim = PairLimits {
            fails_per_client: 10,
            fails_global: 20,
            code_lock_after: 1000,
        };
        let base = start(PairState::with(Sessions::test(SECRET), PAIR_TTL, lim)).await;
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&tok("ann")),
            json!({"claim": claim_hex(8)}),
        )
        .await;
        let ann_code = v["code"].as_str().unwrap().to_string();
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&tok("bob")),
            json!({"claim": claim_hex(9)}),
        )
        .await;
        let bob_code = v["code"].as_str().unwrap().to_string();
        let taken = [ann_code.as_str(), bob_code.as_str()];
        // 300 wrong codes and 100 wrong QR claims from 150 addresses: 15x
        // the soft ceiling.
        for i in 0..300u32 {
            let ip = format!("198.51.{}.{}", 100 + i % 3, i % 50);
            let r = claim_from(&base, &ip, json!({"code": wrong_code(i * 7, &taken)})).await;
            assert!(matches!(r.status().as_u16(), 410 | 429));
        }
        for i in 0..100u32 {
            let ip = format!("203.0.113.{}", i % 100);
            let r = claim_from(&base, &ip, json!({"claim": claim_hex(200)})).await;
            assert_eq!(r.status().as_u16(), 410);
        }
        // Under pressure an address that already failed is out after one.
        let r = claim_from(&base, "198.51.100.1", json!({"code": ann_code})).await;
        assert_eq!(r.status().as_u16(), 429);
        // A fresh address with the right code, and a QR claim, both work.
        let r = claim_from(
            &base,
            "192.0.2.10",
            json!({"code": ann_code, "name": "Pixel"}),
        )
        .await;
        assert_eq!(r.status().as_u16(), 200);
        let r = claim_from(&base, "192.0.2.11", json!({"claim": claim_hex(9)})).await;
        assert_eq!(r.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn a_code_locks_after_enough_wrong_codes_but_its_qr_claim_still_works() {
        let lim = PairLimits {
            fails_per_client: 1000,
            fails_global: 1000,
            code_lock_after: 30,
        };
        let base = start(PairState::with(Sessions::test(SECRET), PAIR_TTL, lim)).await;
        let ann = tok("ann");
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&ann),
            json!({"claim": claim_hex(30), "code_box": "BOX"}),
        )
        .await;
        let code = v["code"].as_str().unwrap().to_string();
        // Wrong QR claims don't count against a code.
        for _ in 0..40 {
            let r = claim_from(&base, "198.51.100.7", json!({"claim": claim_hex(31)})).await;
            assert_eq!(r.status().as_u16(), 410);
        }
        assert_eq!(status_of(&base, &ann).await["code_locked"], false);
        for i in 0..30u32 {
            let r = claim_from(
                &base,
                "198.51.100.8",
                json!({"code": wrong_code(i, &[&code])}),
            )
            .await;
            assert_eq!(r.status().as_u16(), 410);
        }
        let s = status_of(&base, &ann).await;
        assert_eq!(s["state"], "waiting");
        assert_eq!(s["code_locked"], true);
        // The right code no longer works (answered like any wrong one)...
        let r = claim_from(&base, "192.0.2.1", json!({"code": code})).await;
        assert_eq!(r.status().as_u16(), 410);
        // ...the QR does, and a pairing made after the flood takes codes.
        let r = claim_from(&base, "192.0.2.2", json!({"claim": claim_hex(30)})).await;
        assert_eq!(r.status().as_u16(), 200);
        let (_, v) = post(
            &base,
            "/pair/create",
            Some(&tok("bob")),
            json!({"claim": claim_hex(32)}),
        )
        .await;
        let r = claim_from(&base, "192.0.2.3", json!({"code": v["code"]})).await;
        assert_eq!(r.status().as_u16(), 200);
    }

    #[test]
    fn lockouts_double_per_strike_and_are_forgotten_after_quiet() {
        let lim = PairLimits {
            fails_per_client: 3,
            fails_global: 1_000_000,
            code_lock_after: 1_000_000,
        };
        let mut inner = Inner::default();
        let mut t = Instant::now();
        let mut lockouts = Vec::new();
        for _ in 0..4 {
            for _ in 0..3 {
                assert!(inner.client_wait("a", t, &lim).is_zero());
                inner.failed("a".into(), true, t, &lim);
            }
            let w = inner.client_wait("a", t, &lim);
            lockouts.push(w.as_secs());
            t += w;
        }
        assert_eq!(lockouts, vec![60, 120, 240, 480]);
        assert!(inner.client_wait("b", t, &lim).is_zero(), "per address");
        // An hour of quiet: back to a one-minute lockout, and the entry is
        // dropped from memory at the next sweep.
        t += STRIKE_MEMORY + Duration::from_secs(1);
        for _ in 0..3 {
            inner.failed("a".into(), true, t, &lim);
        }
        assert_eq!(inner.client_wait("a", t, &lim).as_secs(), 60);
        t += STRIKE_MEMORY + Duration::from_secs(61);
        inner.failed("c".into(), false, t, &lim);
        assert_eq!(inner.clients.len(), 1);
    }

    #[tokio::test]
    async fn a_lockout_and_its_strikes_survive_a_restart() {
        let lim = PairLimits {
            fails_per_client: 3,
            fails_global: 1_000_000,
            code_lock_after: 1_000_000,
        };
        let sessions = Sessions::test(SECRET);
        let before = PairState::with(sessions.clone(), PAIR_TTL, lim);
        let base = start(before.clone()).await;
        for _ in 0..3 {
            let r = claim_from(&base, "198.51.100.20", json!({"code": "123456"})).await;
            assert_eq!(r.status().as_u16(), 410);
        }
        let r = claim_from(&base, "198.51.100.20", json!({"code": "123456"})).await;
        assert_eq!(r.status().as_u16(), 429);
        assert_eq!(before.stats()["addresses_locked_out"], 1);

        // A new server on the same store.
        let after = PairState::with(sessions, PAIR_TTL, lim);
        assert_eq!(after.restore().await.unwrap(), 1);
        let base = start(after.clone()).await;
        let r = claim_from(&base, "198.51.100.20", json!({"code": "123456"})).await;
        assert_eq!(r.status().as_u16(), 429, "still locked out");
        let secs: u64 = r.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((55..=60).contains(&secs), "{secs}");
        let r = claim_from(&base, "198.51.100.21", json!({"code": "123456"})).await;
        assert_eq!(r.status().as_u16(), 410, "another address isn't");
        // The strike count came back too: the next lockout is two minutes.
        let key = client_key("198.51.100.20".parse().unwrap());
        let mut inner = after.inner.lock().unwrap();
        assert_eq!(inner.clients[&key].strikes, 1);
        let t = Instant::now() + Duration::from_secs(61);
        for _ in 0..3 {
            inner.failed(key.clone(), true, t, &lim);
        }
        assert_eq!(inner.client_wait(&key, t, &lim).as_secs(), 120);
    }

    #[test]
    fn ipv6_clients_count_by_their_64() {
        let k = |s: &str| client_key(s.parse().unwrap());
        assert_eq!(k("2001:db8:1:2::1"), k("2001:db8:1:2:ffff::9"));
        assert_ne!(k("2001:db8:1:2::1"), k("2001:db8:1:3::1"));
        assert_eq!(k("::ffff:198.51.100.4"), "198.51.100.4");
        assert_eq!(k("198.51.100.4"), "198.51.100.4");
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
