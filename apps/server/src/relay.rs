//! WebSocket relay: the last rung of the connect ladder, for networks where
//! no direct path works (e.g. UDP blocked, symmetric NAT on both sides).
//!
//! Protocol (JSON text frames):
//!   client -> server, first frame:  {"auth": "<session token>", "node_id": "<id>"}
//!   server -> client:               {"ok": true, "bin": 1}  (or {"error": …} + close)
//!   client -> server:               {"to": "<node_id>", "data": "<opaque>"}
//!   server -> client:               {"from": "<node_id>", "data": "<opaque>"}
//!   server -> client (target gone): {"error": "offline", "to": "<node_id>"}
//!
//! Binary frames (`"bin": 1` in the welcome says this server forwards them;
//! an older one closes the socket on one, so clients send them only after
//! seeing it):
//!   client -> server: [version u8 = 1][to_len u8][to: node id, UTF-8][opaque payload]
//!   server -> client: [version u8 = 1][from_len u8][from: node id][the same payload]
//! Same rules as text: forwarded only to a device of the SAME user, `from`
//! is the sender's authenticated node id (never what it claims), charged to
//! the same per-account budget, `{"error":"offline"}` (text) when the target
//! isn't connected. At most `MAX_BIN_FRAME` bytes; a bigger one, another
//! version, or a malformed one is dropped (counted in `dropped`). Clients
//! use them for screen and audio records (sealed end to end, see mdcore's
//! `transport::media`); a client that never sends one sees no change.
//!
//! `data` (and a binary payload) is never parsed here. It carries the same end-to-end-encrypted
//! frames the peer-to-peer links carry, so the server only ever sees
//! ciphertext. Frames are forwarded only between devices of the SAME user.
//!
//! Fair use: each account gets a token bucket (`RELAY_BYTES_PER_SEC`, default
//! 4 MiB/s, bursts up to 4x). A sender over budget is SLOWED (we stop reading
//! its socket for a moment, so TCP pushes back), never dropped: relayed
//! frames aren't retransmitted, so dropping one would lose a message.
//! `GET /relay/stats` reports aggregate counters (no per-user data).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::session::Sessions;

/// Largest relayed frame (matches the clients' 4 MiB link cap, plus JSON
/// wrapping overhead).
const MAX_FRAME: usize = 5 * 1024 * 1024;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// The binary frame version this server forwards (said as `"bin"` in the
/// welcome).
const BIN_VERSION: u8 = 1;
/// Largest binary frame: a screen snapshot (at most 600 KB) and its
/// framing, with room to spare. Clients send bigger ones as text.
pub const MAX_BIN_FRAME: usize = 1024 * 1024;

/// What goes out on a device's socket.
#[derive(Debug)]
enum Out {
    Text(String),
    Bin(Vec<u8>),
    /// The session was signed out: close the socket.
    Close,
}

impl Out {
    fn len(&self) -> usize {
        match self {
            Out::Text(t) => t.len(),
            Out::Bin(b) => b.len(),
            Out::Close => 0,
        }
    }
}

/// Split a client's binary frame into (target node id, payload): `None` for
/// another version or a malformed one.
fn parse_bin(frame: &[u8]) -> Option<(&str, &[u8])> {
    let (&ver, rest) = frame.split_first()?;
    let (&n, rest) = rest.split_first()?;
    if ver != BIN_VERSION || n == 0 || rest.len() < usize::from(n) {
        return None;
    }
    let (to, payload) = rest.split_at(usize::from(n));
    Some((std::str::from_utf8(to).ok()?, payload))
}

/// The frame forwarded to the target: `[version][from_len][from][payload]`.
/// `None` if `from` doesn't fit a length byte.
fn bin_out(from: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let n = u8::try_from(from.len()).ok().filter(|n| *n > 0)?;
    let mut out = Vec::with_capacity(2 + from.len() + payload.len());
    out.extend_from_slice(&[BIN_VERSION, n]);
    out.extend_from_slice(from.as_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

type Key = (String, String); // (user_id, node_id)
/// Connected devices: (user, node) -> (connection id, outgoing queue, when
/// we last heard from it in ms since `RelayState::epoch`).
type Conns = Arc<Mutex<HashMap<Key, (u64, Queue, Arc<AtomicU64>)>>>;

/// Most bytes waiting for one device. A device that stops reading (stuck,
/// or on purpose) must not make the server hold frames without end: past
/// this the sender is told the target is offline, as if it had gone.
pub const MAX_QUEUED: usize = 16 * 1024 * 1024;
/// Longest node id accepted (clients use short ids).
const MAX_NODE_ID: usize = 128;
/// Most relay connections one account may hold at once.
pub const MAX_CONNS_PER_USER: usize = 16;
/// A socket that says nothing (not even a ping; clients ping every 20 s)
/// for this long is closed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// One device's outgoing queue, with the bytes in it counted.
#[derive(Clone)]
struct Queue {
    tx: mpsc::UnboundedSender<Out>,
    bytes: Arc<AtomicUsize>,
    /// The session id of the token it signed in with (empty for an old
    /// token), so signing out that session closes it.
    sid: Arc<str>,
}

impl Queue {
    /// Queue `out` unless the device already has `MAX_QUEUED` bytes waiting.
    fn push(&self, out: Out) -> bool {
        let n = out.len();
        let prev = self.bytes.fetch_add(n, Ordering::AcqRel);
        if prev.saturating_add(n) > MAX_QUEUED {
            self.bytes.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        if self.tx.send(out).is_err() {
            self.bytes.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        true
    }
}

/// How often idle (refilled) account buckets are dropped.
const BUCKET_SWEEP: Duration = Duration::from_secs(60);

/// Clients ping every 20 s. A socket silent for longer than this belongs to
/// a device that went to sleep (a phone with the screen off) or vanished:
/// a signal sent over it would be lost, so it goes by push instead.
const SIGNAL_FRESH: Duration = Duration::from_secs(30);

/// Per-account token bucket: `rate` bytes/s, holding at most `burst` bytes.
pub struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(burst: f64, now: Instant) -> Self {
        Bucket {
            tokens: burst,
            last: now,
        }
    }

    /// Whether the bucket has refilled to `burst` by `now` (then it's the
    /// same as a new one and can be dropped).
    fn full(&self, rate: f64, burst: f64, now: Instant) -> bool {
        self.tokens + now.saturating_duration_since(self.last).as_secs_f64() * rate >= burst
    }

    /// Take `n` bytes; returns how long the sender must wait first (zero if
    /// it's within budget). The debt is paid off by future refill.
    pub fn take(&mut self, n: usize, rate: f64, burst: f64, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * rate).min(burst) - n as f64;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / rate)
        }
    }
}

/// Aggregate relay counters (served at `GET /relay/stats`).
#[derive(Default)]
pub struct Stats {
    frames: AtomicU64,
    bytes: AtomicU64,
    throttled: AtomicU64,
    /// Binary frames dropped: too big, another version, or malformed.
    dropped: AtomicU64,
}

#[derive(Clone)]
pub struct RelayState {
    sessions: Arc<Sessions>,
    conns: Conns,
    next_id: Arc<AtomicU64>,
    /// Bytes per second per account.
    rate: f64,
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
    /// When `buckets` next drops its full ones (ms since `epoch`).
    next_sweep: Arc<AtomicU64>,
    stats: Arc<Stats>,
    epoch: Instant,
    /// See `SIGNAL_FRESH` (shorter in tests).
    signal_fresh: Duration,
}

impl RelayState {
    pub fn new(sessions: Arc<Sessions>) -> Self {
        let rate = std::env::var("RELAY_BYTES_PER_SEC")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|r| *r > 0.0)
            .unwrap_or(4.0 * 1024.0 * 1024.0);
        Self::with_rate(sessions, rate)
    }

    pub fn with_rate(sessions: Arc<Sessions>, rate: f64) -> Self {
        RelayState {
            sessions,
            conns: Arc::default(),
            next_id: Arc::default(),
            rate,
            buckets: Arc::default(),
            next_sweep: Arc::default(),
            stats: Arc::default(),
            epoch: Instant::now(),
            signal_fresh: SIGNAL_FRESH,
        }
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn burst(&self) -> f64 {
        self.rate * 4.0
    }

    /// Deliver a signal to one of `user_id`'s devices over its WebSocket, if
    /// it's connected and was heard from lately (`SIGNAL_FRESH`). `false` =
    /// not reachable that way (the caller falls back to FCM).
    pub fn deliver_signal(&self, user_id: &str, to: &str, frame: String) -> bool {
        let now = self.now_ms();
        let fresh = self.signal_fresh.as_millis() as u64;
        let target = self
            .conns
            .lock()
            .unwrap()
            .get(&(user_id.to_string(), to.to_string()))
            .filter(|(_, _, heard)| now.saturating_sub(heard.load(Ordering::Relaxed)) <= fresh)
            .map(|(_, q, _)| q.clone());
        target.is_some_and(|q| q.push(Out::Text(frame)))
    }

    /// Close `user_id`'s sockets that signed in with session `sid` (all of
    /// them for `None`): the session was signed out.
    pub fn sign_out(&self, user_id: &str, sid: Option<&str>) {
        for ((u, _), (_, q, _)) in self.conns.lock().unwrap().iter() {
            if u == user_id && sid.is_none_or(|s| *q.sid == *s) {
                let _ = q.tx.send(Out::Close);
            }
        }
    }

    /// Open relay sockets (for `/admin/limits`).
    pub fn connections(&self) -> usize {
        self.conns.lock().unwrap().len()
    }

    /// Charge `n` bytes to `user`; how long to pause before forwarding.
    fn charge(&self, user: &str, n: usize) -> Duration {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
        // Accounts that went quiet would otherwise keep their bucket for
        // ever (security review L24): a full bucket is dropped.
        let ms = self.now_ms();
        if ms >= self.next_sweep.load(Ordering::Relaxed) {
            self.next_sweep
                .store(ms + BUCKET_SWEEP.as_millis() as u64, Ordering::Relaxed);
            let (rate, burst) = (self.rate, self.burst());
            buckets.retain(|_, b| !b.full(rate, burst, now));
        }
        let b = buckets
            .entry(user.to_string())
            .or_insert_with(|| Bucket::new(self.burst(), now));
        b.take(n, self.rate, self.burst(), now)
    }
}

/// `GET /relay` (WebSocket upgrade). Has its own state, so it can be mounted
/// and tested without MongoDB or FCM.
pub fn router(state: RelayState) -> Router {
    Router::new()
        .route("/relay", get(upgrade))
        .route("/relay/stats", get(stats))
        .with_state(state)
}

async fn upgrade(ws: WebSocketUpgrade, State(s): State<RelayState>) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME)
        .on_upgrade(move |socket| serve(socket, s))
}

async fn stats(State(s): State<RelayState>) -> Json<Value> {
    let connections = s.conns.lock().unwrap().len();
    Json(json!({
        "connections": connections,
        "frames": s.stats.frames.load(Ordering::Relaxed),
        "bytes": s.stats.bytes.load(Ordering::Relaxed),
        "throttled": s.stats.throttled.load(Ordering::Relaxed),
        "dropped": s.stats.dropped.load(Ordering::Relaxed),
    }))
}

async fn serve(mut socket: WebSocket, s: RelayState) {
    // 1. Authenticate with the first frame.
    let Some((user_id, sid, node_id)) = authenticate(&mut socket, &s).await else {
        let _ = socket
            .send(Message::Text(json!({"error": "unauthorized"}).to_string()))
            .await;
        return;
    };
    let key: Key = (user_id.clone(), node_id.clone());
    let my_id = s.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let queued = Arc::new(AtomicUsize::new(0));
    let queue = Queue {
        tx,
        bytes: Arc::clone(&queued),
        sid: sid.into(),
    };
    let heard = Arc::new(AtomicU64::new(s.now_ms()));
    let admitted = {
        let mut conns = s.conns.lock().unwrap();
        // A reconnecting device replaces its old connection; a new one must
        // fit under the account's cap.
        let others = conns
            .keys()
            .filter(|(u, n)| *u == user_id && *n != node_id)
            .count();
        let ok = others < MAX_CONNS_PER_USER;
        if ok {
            conns.insert(key.clone(), (my_id, queue, Arc::clone(&heard)));
        }
        ok
    };
    if !admitted {
        let _ = socket
            .send(Message::Text(
                json!({"error": "too many connections"}).to_string(),
            ))
            .await;
        return;
    }
    if socket
        .send(Message::Text(
            json!({"ok": true, "bin": BIN_VERSION}).to_string(),
        ))
        .await
        .is_err()
    {
        remove_if_ours(&s, &key, my_id);
        return;
    }
    tracing::info!(user = %user_id, node = %node_id, "relay: connected");

    // 2. Pump: frames from this device go to their target; frames queued for
    //    this device go out on its socket.
    loop {
        tokio::select! {
            incoming = tokio::time::timeout(IDLE_TIMEOUT, socket.recv()) => {
                let Ok(incoming) = incoming else { break }; // silent too long
                heard.store(s.now_ms(), Ordering::Relaxed);
                let (len, to, out) = match incoming {
                    Some(Ok(Message::Text(text))) => {
                        // Charged before parsing, as everything read is.
                        throttle(&s, &user_id, text.len()).await;
                        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                        let (Some(to), Some(data)) = (v["to"].as_str(), v.get("data")) else { continue };
                        let out = json!({"from": node_id, "data": data}).to_string();
                        (text.len(), to.to_string(), Out::Text(out))
                    }
                    Some(Ok(Message::Binary(frame))) => {
                        throttle(&s, &user_id, frame.len()).await;
                        let parsed = (frame.len() <= MAX_BIN_FRAME)
                            .then(|| parse_bin(&frame))
                            .flatten()
                            .and_then(|(to, payload)| Some((to.to_string(), bin_out(&node_id, payload)?)));
                        let Some((to, out)) = parsed else {
                            s.stats.dropped.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        (frame.len(), to, Out::Bin(out))
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    _ => break, // closed or error
                };
                // Same account only: the target is looked up under the
                // sender's own user id.
                let target = s.conns.lock().unwrap()
                    .get(&(user_id.clone(), to.clone()))
                    .map(|(_, q, _)| q.clone());
                let delivered = target.map(|q| q.push(out)).unwrap_or(false);
                if delivered {
                    s.stats.frames.fetch_add(1, Ordering::Relaxed);
                    s.stats.bytes.fetch_add(len as u64, Ordering::Relaxed);
                } else {
                    let reply = json!({"error": "offline", "to": to}).to_string();
                    if socket.send(Message::Text(reply)).await.is_err() { break; }
                }
            }
            Some(out) = rx.recv() => {
                queued.fetch_sub(out.len(), Ordering::AcqRel);
                let msg = match out {
                    Out::Text(t) => Message::Text(t),
                    Out::Bin(b) => Message::Binary(b),
                    Out::Close => {
                        let bye = json!({"error": "signed out"}).to_string();
                        let _ = socket.send(Message::Text(bye)).await;
                        break;
                    }
                };
                if socket.send(msg).await.is_err() { break; }
            }
        }
    }
    remove_if_ours(&s, &key, my_id);
    tracing::info!(user = %user_id, node = %node_id, "relay: disconnected");
}

/// Fair use: over budget -> pause reading this socket (TCP backpressure
/// slows the sender); never drop the frame.
async fn throttle(s: &RelayState, user: &str, n: usize) {
    let wait = s.charge(user, n);
    if !wait.is_zero() {
        s.stats.throttled.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(wait).await;
    }
}

/// (user id, session id, node id) from the first frame, if its token is good.
async fn authenticate(socket: &mut WebSocket, s: &RelayState) -> Option<(String, String, String)> {
    let first = tokio::time::timeout(AUTH_TIMEOUT, socket.recv())
        .await
        .ok()??
        .ok()?;
    let Message::Text(text) = first else {
        return None;
    };
    let v: Value = serde_json::from_str(&text).ok()?;
    let user = s.sessions.verify(v["auth"].as_str()?).await.ok()?;
    let node_id = v["node_id"].as_str()?.trim().to_string();
    (!node_id.is_empty() && node_id.len() <= MAX_NODE_ID).then_some((
        user.user_id,
        user.sid,
        node_id,
    ))
}

/// Remove the registration only if it's still this connection's (a newer one
/// from the same device may have replaced it).
fn remove_if_ours(s: &RelayState, key: &Key, my_id: u64) {
    let mut conns = s.conns.lock().unwrap();
    if conns
        .get(key)
        .map(|(id, _, _)| *id == my_id)
        .unwrap_or(false)
    {
        conns.remove(key);
    }
}

#[cfg(test)]
mod tests {
    //! Run the relay router on a real port and talk to it with a plain
    //! (blocking) WebSocket client on ordinary threads.
    use super::*;
    use crate::auth::issue_token;
    use tungstenite::Message as M;

    const SECRET: &str = "test-secret";

    fn start() -> u16 {
        start_with(RelayState::new(Sessions::test(SECRET)))
    }

    fn start_with(state: RelayState) -> u16 {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap().port()).unwrap();
                axum::serve(listener, router(state)).await.unwrap();
            });
        });
        rx.recv().unwrap()
    }

    fn get_stats(port: u16) -> Value {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "GET /relay/stats HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        serde_json::from_str(resp.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    type Client = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

    fn client(port: u16, token: &str, node: &str) -> Client {
        let (mut ws, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/relay")).unwrap();
        ws.send(M::Text(json!({"auth": token, "node_id": node}).to_string()))
            .unwrap();
        ws
    }

    fn next_json(ws: &mut Client) -> Value {
        loop {
            if let M::Text(t) = ws.read().unwrap() {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }

    fn tok(user: &str) -> String {
        issue_token(SECRET, user, user).unwrap()
    }

    #[test]
    fn forwards_between_devices_of_the_same_user() {
        let port = start();
        let mut laptop = client(port, &tok("alice"), "laptop");
        assert_eq!(next_json(&mut laptop)["ok"], true);
        let mut phone = client(port, &tok("alice"), "phone");
        assert_eq!(next_json(&mut phone)["ok"], true);

        laptop
            .send(M::Text(
                json!({"to": "phone", "data": "ciphertext"}).to_string(),
            ))
            .unwrap();
        assert_eq!(
            next_json(&mut phone),
            json!({"from": "laptop", "data": "ciphertext"})
        );
    }

    #[test]
    fn never_forwards_to_another_users_device() {
        let port = start();
        let mut alice = client(port, &tok("alice"), "a-laptop");
        next_json(&mut alice);
        let mut bob = client(port, &tok("bob"), "b-phone");
        next_json(&mut bob);

        alice
            .send(M::Text(json!({"to": "b-phone", "data": "x"}).to_string()))
            .unwrap();
        // Alice is told the target is offline (from her account's view)...
        assert_eq!(next_json(&mut alice)["error"], "offline");
    }

    #[test]
    fn bucket_allows_bursts_then_paces_to_the_rate() {
        let t0 = Instant::now();
        let mut b = Bucket::new(4000.0, t0);
        // A burst within the bucket passes at once.
        assert_eq!(b.take(4000, 1000.0, 4000.0, t0), Duration::ZERO);
        // The next 1000 bytes owe a full second at 1000 B/s.
        assert_eq!(b.take(1000, 1000.0, 4000.0, t0), Duration::from_secs(1));
        // After 3 s the debt is repaid and 2000 bytes are banked again.
        assert_eq!(
            b.take(2000, 1000.0, 4000.0, t0 + Duration::from_secs(3)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_heavy_sender_is_slowed_not_dropped_and_stats_count_it() {
        // 2 KB/s per account, so a 30 KB burst must take several seconds.
        let port = start_with(RelayState::with_rate(Sessions::test(SECRET), 2000.0));
        let mut laptop = client(port, &tok("alice"), "laptop");
        next_json(&mut laptop);
        let mut phone = client(port, &tok("alice"), "phone");
        next_json(&mut phone);

        let started = Instant::now();
        let chunk = "x".repeat(3000);
        for _ in 0..10 {
            laptop
                .send(M::Text(json!({"to": "phone", "data": chunk}).to_string()))
                .unwrap();
        }
        for _ in 0..10 {
            assert_eq!(next_json(&mut phone)["data"], chunk.as_str()); // every frame arrives
        }
        assert!(
            started.elapsed() >= Duration::from_secs(5),
            "30 KB at 2 KB/s (8 KB burst) can't finish in {:?}",
            started.elapsed()
        );
        let st = get_stats(port);
        assert_eq!(st["frames"], 10);
        assert!(st["throttled"].as_u64().unwrap() > 0);
        assert_eq!(st["connections"], 2);
    }

    #[test]
    fn signals_go_to_a_connected_device_and_report_when_it_is_not() {
        let state = RelayState::new(Sessions::test(SECRET));
        let port = start_with(state.clone());
        let mut phone = client(port, &tok("alice"), "phone");
        next_json(&mut phone);

        let frame = json!({"signal": {"type": "signal", "sender": "laptop", "payload": "{}"}});
        assert!(state.deliver_signal("alice", "phone", frame.to_string()));
        assert_eq!(next_json(&mut phone), frame);
        assert!(
            !state.deliver_signal("alice", "tablet", "{}".into()),
            "not connected"
        );
        assert!(
            !state.deliver_signal("bob", "phone", "{}".into()),
            "another user's device"
        );
    }

    #[test]
    fn signing_out_a_session_closes_its_socket_and_it_cannot_come_back() {
        let sessions = Sessions::test(SECRET);
        let state = RelayState::new(sessions.clone());
        let port = start_with(state.clone());
        let (t1, t2) = (tok("alice"), tok("alice"));
        let mut laptop = client(port, &t1, "laptop");
        assert_eq!(next_json(&mut laptop)["ok"], true);
        let mut phone = client(port, &t2, "phone");
        assert_eq!(next_json(&mut phone)["ok"], true);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let user = rt.block_on(sessions.verify(&t1)).unwrap();
        assert!(!rt.block_on(sessions.logout(&user)).unwrap());
        state.sign_out("alice", Some(&user.sid));
        assert_eq!(next_json(&mut laptop), json!({"error": "signed out"}));
        // The other session is still connected.
        assert!(state.deliver_signal("alice", "phone", "{}".into()));
        assert_eq!(next_json(&mut phone), json!({}));
        // The signed-out token can't sign in again.
        let mut again = client(port, &t1, "laptop");
        assert_eq!(next_json(&mut again), json!({"error": "unauthorized"}));

        // Signing out everything closes the rest.
        rt.block_on(sessions.logout_all("alice")).unwrap();
        state.sign_out("alice", None);
        assert_eq!(next_json(&mut phone), json!({"error": "signed out"}));
    }

    #[test]
    fn a_silent_socket_gets_no_signals_until_it_speaks_again() {
        let mut state = RelayState::new(Sessions::test(SECRET));
        state.signal_fresh = Duration::from_millis(300);
        let port = start_with(state.clone());
        let mut phone = client(port, &tok("alice"), "phone");
        next_json(&mut phone);
        assert!(state.deliver_signal("alice", "phone", "{}".into()), "fresh");
        assert_eq!(next_json(&mut phone), json!({}));

        // Asleep: no pings for longer than `signal_fresh`. Push instead.
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            !state.deliver_signal("alice", "phone", "{}".into()),
            "stale"
        );

        // Awake again: its next ping makes it reachable.
        phone.send(M::Ping(vec![])).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !state.deliver_signal("alice", "phone", "{}".into()) {
            assert!(Instant::now() < deadline, "never fresh again");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn bin(to: &str, payload: &[u8]) -> M {
        let mut f = vec![1, to.len() as u8];
        f.extend_from_slice(to.as_bytes());
        f.extend_from_slice(payload);
        M::Binary(f)
    }

    fn next_bin(ws: &mut Client) -> Vec<u8> {
        loop {
            if let M::Binary(b) = ws.read().unwrap() {
                return b;
            }
        }
    }

    /// Two of alice's devices, past the welcome.
    fn pair_of(port: u16) -> (Client, Client) {
        let mut laptop = client(port, &tok("alice"), "laptop");
        assert_eq!(next_json(&mut laptop), json!({"ok": true, "bin": 1}));
        let mut phone = client(port, &tok("alice"), "phone");
        next_json(&mut phone);
        (laptop, phone)
    }

    #[test]
    fn forwards_binary_frames_with_the_senders_own_id() {
        let port = start();
        let (mut laptop, mut phone) = pair_of(port);
        let payload: Vec<u8> = (0..70_000u32).map(|i| (i % 256) as u8).collect();
        laptop.send(bin("phone", &payload)).unwrap();
        let got = next_bin(&mut phone);
        assert_eq!(&got[..8], b"\x01\x06laptop");
        assert_eq!(&got[8..], &payload[..]);
        // And back, mixed with text: both kinds keep working on one socket.
        phone.send(bin("laptop", b"pong")).unwrap();
        phone
            .send(M::Text(json!({"to": "laptop", "data": "t"}).to_string()))
            .unwrap();
        assert_eq!(next_bin(&mut laptop), b"\x01\x05phonepong");
        assert_eq!(next_json(&mut laptop)["data"], "t");
        assert_eq!(get_stats(port)["frames"], 3);
    }

    #[test]
    fn never_forwards_binary_to_another_users_device() {
        let port = start();
        let mut alice = client(port, &tok("alice"), "a-laptop");
        next_json(&mut alice);
        let mut bob = client(port, &tok("bob"), "b-phone");
        next_json(&mut bob);
        alice.send(bin("b-phone", b"secret")).unwrap();
        assert_eq!(
            next_json(&mut alice),
            json!({"error": "offline", "to": "b-phone"})
        );
        // Bob got nothing: the next thing he sees is his own reply.
        bob.send(M::Text(json!({"to": "nobody", "data": "x"}).to_string()))
            .unwrap();
        assert_eq!(next_json(&mut bob)["error"], "offline");
    }

    #[test]
    fn oversized_or_malformed_binary_is_dropped_and_the_socket_lives() {
        let port = start();
        let (mut laptop, mut phone) = pair_of(port);
        laptop
            .send(bin("phone", &vec![0u8; MAX_BIN_FRAME]))
            .unwrap(); // header + payload > the cap
        laptop
            .send(M::Binary(vec![2, 5, b'p', b'h', b'o', b'n', b'e']))
            .unwrap(); // version 2
        laptop.send(M::Binary(vec![1, 9, b'p'])).unwrap(); // short id
        laptop.send(M::Binary(vec![1, 0, 1, 2])).unwrap(); // empty id
        laptop.send(M::Binary(vec![1, 2, 0xff, 0xfe, 0])).unwrap(); // not UTF-8
        laptop.send(M::Binary(vec![])).unwrap();
        // Still connected, and the next good frame is the first one through.
        laptop.send(bin("phone", b"ok")).unwrap();
        assert_eq!(next_bin(&mut phone), b"\x01\x06laptopok");
        let st = get_stats(port);
        assert_eq!(st["dropped"], 6);
        assert_eq!(st["frames"], 1);
        // Exactly at the cap still goes.
        let pad = MAX_BIN_FRAME - 2 - "phone".len();
        laptop.send(bin("phone", &vec![7u8; pad])).unwrap();
        assert_eq!(next_bin(&mut phone).len(), 2 + "laptop".len() + pad);
    }

    #[test]
    fn binary_frames_share_the_accounts_budget() {
        let port = start_with(RelayState::with_rate(Sessions::test(SECRET), 2000.0));
        let (mut laptop, mut phone) = pair_of(port);
        let started = Instant::now();
        for _ in 0..10 {
            laptop.send(bin("phone", &[1u8; 3000])).unwrap();
        }
        for _ in 0..10 {
            assert_eq!(next_bin(&mut phone).len(), 8 + 3000);
        }
        assert!(
            started.elapsed() >= Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert!(get_stats(port)["throttled"].as_u64().unwrap() > 0);
    }

    #[test]
    fn rejects_a_bad_token() {
        let port = start();
        let mut ws = client(port, "not-a-jwt", "x");
        assert_eq!(next_json(&mut ws)["error"], "unauthorized");
    }

    #[test]
    fn a_device_that_stops_reading_cannot_make_the_server_hold_frames_without_end() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let q = Queue {
            tx,
            bytes: Arc::default(),
            sid: "".into(),
        };
        let chunk = vec![0u8; 1024 * 1024];
        let mut taken = 0;
        while q.push(Out::Bin(chunk.clone())) {
            taken += 1;
            assert!(taken <= MAX_QUEUED / chunk.len(), "queue never filled");
        }
        assert_eq!(taken, MAX_QUEUED / chunk.len());
        assert!(!q.push(Out::Text("x".into())));
        // Draining (as the socket pump does) makes room again.
        let out = rx.try_recv().unwrap();
        q.bytes.fetch_sub(out.len(), Ordering::AcqRel);
        assert!(q.push(Out::Bin(chunk)));
    }

    #[test]
    fn rejects_an_overlong_node_id() {
        let port = start();
        let mut ws = client(port, &tok("alice"), &"n".repeat(MAX_NODE_ID + 1));
        assert_eq!(next_json(&mut ws)["error"], "unauthorized");
    }

    #[test]
    fn idle_account_buckets_are_dropped() {
        let s = RelayState::with_rate(Sessions::test(SECRET), 1000.0);
        s.charge("first", 1); // the first sweep
        let t0 = Instant::now();
        {
            let mut b = s.buckets.lock().unwrap();
            b.insert(
                "gone".into(),
                Bucket {
                    tokens: 0.0,
                    last: t0 - Duration::from_secs(10),
                },
            );
            b.insert(
                "busy".into(),
                Bucket {
                    tokens: -1.0e9,
                    last: t0,
                },
            );
        }
        // Not due yet: nothing is dropped.
        s.charge("new", 1);
        assert_eq!(s.buckets.lock().unwrap().len(), 4);
        // Due: the refilled ones go, the one in debt stays.
        std::thread::sleep(Duration::from_millis(20));
        s.next_sweep.store(0, Ordering::Relaxed);
        s.charge("new", 1);
        let mut keys: Vec<String> = s.buckets.lock().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, vec!["busy".to_string(), "new".to_string()]);
    }

    #[test]
    fn caps_connections_per_account() {
        let port = start();
        let t = tok("alice");
        let mut held = Vec::new();
        for i in 0..MAX_CONNS_PER_USER {
            let mut ws = client(port, &t, &format!("dev-{i}"));
            assert_eq!(next_json(&mut ws)["ok"], true);
            held.push(ws);
        }
        let mut extra = client(port, &t, "one-too-many");
        assert_eq!(next_json(&mut extra)["error"], "too many connections");
        // A device reconnecting under its own id still gets in.
        let mut again = client(port, &t, "dev-0");
        assert_eq!(next_json(&mut again)["ok"], true);
        // Another account is unaffected.
        let mut bob = client(port, &tok("bob"), "dev-0");
        assert_eq!(next_json(&mut bob)["ok"], true);
    }
}
