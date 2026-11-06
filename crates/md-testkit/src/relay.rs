//! A fake of the server's WebSocket `/relay` (apps/server/src/relay.rs):
//! same protocol, on a loopback port. Devices are grouped by their auth
//! token. `FakeRegistry` issues one token per user, so "same token" means "same
//! user", which is the real server's forwarding rule.
//!
//! Binary frames as the real server forwards them (`[1][to_len][to][payload]`
//! in, `[1][from_len][from][payload]` out), said with `"bin": 1` in the
//! welcome. `start_old` plays a server from before: no `"bin"`, and a binary
//! frame closes the socket, as the old server's did.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::Message;

type Key = (String, String); // (token, node_id)

/// (token, node) -> (connection id, outgoing queue).
pub(crate) type RelayConns = Arc<Mutex<HashMap<Key, (u64, mpsc::Sender<Message>)>>>;
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// The real server's binary frame cap.
const MAX_BIN_FRAME: usize = 1024 * 1024;

/// What the relay saw.
#[derive(Default)]
struct Seen {
    /// Every `{to, data}` text frame forwarded.
    forwarded: Mutex<Vec<Value>>,
    /// Binary frames forwarded.
    binary: AtomicU64,
    /// node -> bytes of every frame it sent (text and binary, as received).
    bytes_from: Mutex<HashMap<String, u64>>,
}

pub struct FakeRelay {
    url: String,
    seen: Arc<Seen>,
    pub(crate) conns: RelayConns,
}

impl FakeRelay {
    pub fn start() -> Self {
        Self::start_on("127.0.0.1:0")
    }

    /// A relay as deployed before binary frames: says no `"bin"`, and drops
    /// a device that sends one.
    pub fn start_old() -> Self {
        Self::serve_on("127.0.0.1:0", false)
    }

    /// Serve on `bind` (`ip:port`; port 0 picks one), e.g. a simulated
    /// network's server address.
    pub fn start_on(bind: &str) -> Self {
        Self::serve_on(bind, true)
    }

    fn serve_on(bind: &str, bin: bool) -> Self {
        let listener = TcpListener::bind(bind).unwrap();
        let local = listener.local_addr().unwrap();
        let conns: RelayConns = Arc::default();
        let seen: Arc<Seen> = Arc::default();
        {
            let seen = seen.clone();
            let conns = conns.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (conns, seen) = (conns.clone(), seen.clone());
                    std::thread::spawn(move || serve(stream, conns, seen, bin));
                }
            });
        }
        FakeRelay {
            url: format!("ws://{local}/relay"),
            seen,
            conns,
        }
    }

    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Whether a device is connected (any account).
    pub fn connected(&self, node_id: &str) -> bool {
        self.conns.lock().unwrap().keys().any(|(_, n)| n == node_id)
    }

    /// Every `{to, data}` text frame the relay forwarded, in order (what the
    /// server sees).
    pub fn forwarded(&self) -> Vec<Value> {
        self.seen.forwarded.lock().unwrap().clone()
    }

    /// Play a hostile server: hand `to` a frame (`data`) as if `from` sent
    /// it, e.g. a copy of one it forwarded before. False if `to` isn't
    /// connected.
    pub fn inject(&self, to: &str, from: &str, data: &str) -> bool {
        let q = self
            .conns
            .lock()
            .unwrap()
            .iter()
            .find(|((_, n), _)| n == to)
            .map(|(_, (_, q))| q.clone());
        let out = json!({"from": from, "data": data}).to_string();
        q.is_some_and(|q| q.send(Message::Text(out)).is_ok())
    }

    /// How many binary frames the relay forwarded.
    pub fn binary_frames(&self) -> u64 {
        self.seen.binary.load(Ordering::Relaxed)
    }

    /// Bytes `node_id` sent the relay so far (every frame, text or binary,
    /// as the server received it; WebSocket headers not counted).
    pub fn bytes_from(&self, node_id: &str) -> u64 {
        self.seen
            .bytes_from
            .lock()
            .unwrap()
            .get(node_id)
            .copied()
            .unwrap_or(0)
    }
}

fn serve(stream: std::net::TcpStream, conns: RelayConns, seen: Arc<Seen>, bin: bool) {
    let Ok(mut ws) = tungstenite::accept(stream) else {
        return;
    };
    let Ok(Message::Text(first)) = ws.read() else {
        return;
    };
    let auth: Value = serde_json::from_str(&first).unwrap_or(Value::Null);
    let (Some(token), Some(node)) = (auth["auth"].as_str(), auth["node_id"].as_str()) else {
        let _ = ws.send(Message::Text(json!({"error": "unauthorized"}).to_string()));
        return;
    };
    let key: Key = (token.to_string(), node.to_string());
    let (tx, rx) = mpsc::channel::<Message>();
    let my_id = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    conns.lock().unwrap().insert(key.clone(), (my_id, tx));
    let welcome = if bin {
        json!({"ok": true, "bin": 1})
    } else {
        json!({"ok": true})
    };
    let _ = ws.send(Message::Text(welcome.to_string()));
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(50)));
    let count = |n: usize| {
        *seen
            .bytes_from
            .lock()
            .unwrap()
            .entry(key.1.clone())
            .or_default() += n as u64;
    };
    let target = |to: &str| {
        conns
            .lock()
            .unwrap()
            .get(&(key.0.clone(), to.to_string()))
            .map(|(_, q)| q.clone())
    };

    loop {
        // (target, frame to forward), or nothing to forward.
        let forward: Option<(String, Message)> = match ws.read() {
            Ok(Message::Text(t)) => {
                count(t.len());
                let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                match (v["to"].as_str(), v.get("data")) {
                    (Some(to), Some(data)) => {
                        seen.forwarded.lock().unwrap().push(v.clone());
                        let out = json!({"from": key.1, "data": data}).to_string();
                        Some((to.to_string(), Message::Text(out)))
                    }
                    _ => None,
                }
            }
            // The old server closed the socket on a binary frame.
            Ok(Message::Binary(_)) if !bin => break,
            Ok(Message::Binary(f)) => {
                count(f.len());
                parse_bin(&f, &key.1).map(|(to, out)| {
                    seen.binary.fetch_add(1, Ordering::Relaxed);
                    (to, Message::Binary(out))
                })
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => None,
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
            {
                None
            }
            Err(_) => break,
        };
        if let Some((to, out)) = forward {
            if target(&to).map(|q| q.send(out).is_ok()) != Some(true) {
                let _ = ws.send(Message::Text(
                    json!({"error": "offline", "to": to}).to_string(),
                ));
            }
        }
        while let Ok(out) = rx.try_recv() {
            if ws.send(out).is_err() {
                return;
            }
        }
    }
    // Only our own registration: the device may have reconnected already
    // (the real server checks the same way).
    let mut conns = conns.lock().unwrap();
    if conns.get(&key).is_some_and(|(id, _)| *id == my_id) {
        conns.remove(&key);
    }
}

/// A device's binary frame -> (target, the frame to forward from `from`),
/// as the real server checks it.
fn parse_bin(f: &[u8], from: &str) -> Option<(String, Vec<u8>)> {
    if f.len() > MAX_BIN_FRAME {
        return None;
    }
    let (&ver, rest) = f.split_first()?;
    let (&n, rest) = rest.split_first()?;
    if ver != 1 || n == 0 || rest.len() < usize::from(n) {
        return None;
    }
    let (to, payload) = rest.split_at(usize::from(n));
    let to = std::str::from_utf8(to).ok()?.to_string();
    let mut out = vec![1, u8::try_from(from.len()).ok()?];
    out.extend_from_slice(from.as_bytes());
    out.extend_from_slice(payload);
    Some((to, out))
}
