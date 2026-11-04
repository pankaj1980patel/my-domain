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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::auth::verify_token;

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
type Conns = Arc<Mutex<HashMap<Key, (u64, mpsc::UnboundedSender<Out>, Arc<AtomicU64>)>>>;

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
    jwt_secret: String,
    conns: Conns,
    next_id: Arc<AtomicU64>,
    /// Bytes per second per account.
    rate: f64,
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
    stats: Arc<Stats>,
    epoch: Instant,
    /// See `SIGNAL_FRESH` (shorter in tests).
    signal_fresh: Duration,
}

impl RelayState {
    pub fn new(jwt_secret: String) -> Self {
        let rate = std::env::var("RELAY_BYTES_PER_SEC")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|r| *r > 0.0)
            .unwrap_or(4.0 * 1024.0 * 1024.0);
        Self::with_rate(jwt_secret, rate)
    }

    pub fn with_rate(jwt_secret: String, rate: f64) -> Self {
        RelayState {
            jwt_secret,
            conns: Arc::default(),
            next_id: Arc::default(),
            rate,
            buckets: Arc::default(),
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
        target.is_some_and(|q| q.send(Out::Text(frame)).is_ok())
    }

    /// Charge `n` bytes to `user`; how long to pause before forwarding.
    fn charge(&self, user: &str, n: usize) -> Duration {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap();
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
    let Some((user_id, node_id)) = authenticate(&mut socket, &s).await else {
        let _ = socket
            .send(Message::Text(json!({"error": "unauthorized"}).to_string()))
            .await;
        return;
    };
    let key: Key = (user_id.clone(), node_id.clone());
    let my_id = s.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let heard = Arc::new(AtomicU64::new(s.now_ms()));
    // A reconnecting device replaces its old connection.
    s.conns
        .lock()
        .unwrap()
        .insert(key.clone(), (my_id, tx, Arc::clone(&heard)));
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
            incoming = socket.recv() => {
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
                let delivered = target.map(|q| q.send(out).is_ok()).unwrap_or(false);
                if delivered {
                    s.stats.frames.fetch_add(1, Ordering::Relaxed);
                    s.stats.bytes.fetch_add(len as u64, Ordering::Relaxed);
                } else {
                    let reply = json!({"error": "offline", "to": to}).to_string();
                    if socket.send(Message::Text(reply)).await.is_err() { break; }
                }
            }
            Some(out) = rx.recv() => {
                let msg = match out {
                    Out::Text(t) => Message::Text(t),
                    Out::Bin(b) => Message::Binary(b),
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

async fn authenticate(socket: &mut WebSocket, s: &RelayState) -> Option<(String, String)> {
    let first = tokio::time::timeout(AUTH_TIMEOUT, socket.recv())
        .await
        .ok()??
        .ok()?;
    let Message::Text(text) = first else {
        return None;
    };
    let v: Value = serde_json::from_str(&text).ok()?;
    let user = verify_token(&s.jwt_secret, v["auth"].as_str()?)?;
    let node_id = v["node_id"].as_str()?.trim().to_string();
    (!node_id.is_empty()).then_some((user.user_id, node_id))
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
        start_with(RelayState::new(SECRET.into()))
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
        let port = start_with(RelayState::with_rate(SECRET.into(), 2000.0));
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
        let state = RelayState::new(SECRET.into());
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
    fn a_silent_socket_gets_no_signals_until_it_speaks_again() {
        let mut state = RelayState::new(SECRET.into());
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
        let port = start_with(RelayState::with_rate(SECRET.into(), 2000.0));
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
}
