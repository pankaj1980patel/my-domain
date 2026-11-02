//! A fake of the server's WebSocket `/relay` (apps/server/src/relay.rs):
//! same JSON protocol, on a loopback port. Devices are grouped by their auth
//! token. `FakeRegistry` issues one token per user, so "same token" means "same
//! user", which is the real server's forwarding rule.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tungstenite::Message;

type Key = (String, String); // (token, node_id)

pub(crate) type RelayConns = Arc<Mutex<HashMap<Key, mpsc::Sender<String>>>>;

pub struct FakeRelay {
    url: String,
    forwarded: Arc<Mutex<Vec<Value>>>,
    pub(crate) conns: RelayConns,
}

impl FakeRelay {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let conns: RelayConns = Arc::default();
        let forwarded: Arc<Mutex<Vec<Value>>> = Arc::default();
        {
            let forwarded = forwarded.clone();
            let conns = conns.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (conns, forwarded) = (conns.clone(), forwarded.clone());
                    std::thread::spawn(move || serve(stream, conns, forwarded));
                }
            });
        }
        FakeRelay {
            url: format!("ws://127.0.0.1:{port}/relay"),
            forwarded,
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

    /// Every `{to, data}` the relay forwarded, in order (what the server sees).
    pub fn forwarded(&self) -> Vec<Value> {
        self.forwarded.lock().unwrap().clone()
    }
}

fn serve(
    stream: std::net::TcpStream,
    conns: Arc<Mutex<HashMap<Key, mpsc::Sender<String>>>>,
    forwarded: Arc<Mutex<Vec<Value>>>,
) {
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
    let (tx, rx) = mpsc::channel::<String>();
    conns.lock().unwrap().insert(key.clone(), tx);
    let _ = ws.send(Message::Text(json!({"ok": true}).to_string()));
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(50)));

    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                if let (Some(to), Some(data)) = (v["to"].as_str(), v.get("data")) {
                    forwarded.lock().unwrap().push(v.clone());
                    let target = conns
                        .lock()
                        .unwrap()
                        .get(&(key.0.clone(), to.to_string()))
                        .cloned();
                    let out = json!({"from": key.1, "data": data}).to_string();
                    if target.map(|q| q.send(out).is_ok()) != Some(true) {
                        let _ = ws.send(Message::Text(
                            json!({"error": "offline", "to": to}).to_string(),
                        ));
                    }
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
        while let Ok(out) = rx.try_recv() {
            if ws.send(Message::Text(out)).is_err() {
                return;
            }
        }
    }
    conns.lock().unwrap().remove(&key);
}
