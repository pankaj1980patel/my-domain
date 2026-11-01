//! WebSocket relay: the last rung of the connect ladder, for networks where
//! no direct path works (e.g. UDP blocked, symmetric NAT on both sides).
//!
//! Protocol (JSON text frames):
//!   client -> server, first frame:  {"auth": "<session token>", "node_id": "<id>"}
//!   server -> client:               {"ok": true}            (or {"error": …} + close)
//!   client -> server:               {"to": "<node_id>", "data": "<opaque>"}
//!   server -> client:               {"from": "<node_id>", "data": "<opaque>"}
//!   server -> client (target gone): {"error": "offline", "to": "<node_id>"}
//!
//! `data` is never parsed here. It carries the same end-to-end-encrypted
//! frames the peer-to-peer links carry, so the server only ever sees
//! ciphertext. Frames are forwarded only between devices of the SAME user.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::auth::verify_token;

/// Largest relayed frame (matches the clients' 4 MiB link cap, plus JSON
/// wrapping overhead).
const MAX_FRAME: usize = 5 * 1024 * 1024;
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);

type Key = (String, String); // (user_id, node_id)
/// Connected devices: (user, node) -> (connection id, outgoing queue).
type Conns = Arc<Mutex<HashMap<Key, (u64, mpsc::UnboundedSender<String>)>>>;

#[derive(Clone)]
pub struct RelayState {
    jwt_secret: String,
    conns: Conns,
    next_id: Arc<AtomicU64>,
}

impl RelayState {
    pub fn new(jwt_secret: String) -> Self {
        RelayState {
            jwt_secret,
            conns: Arc::default(),
            next_id: Arc::default(),
        }
    }
}

/// `GET /relay` (WebSocket upgrade). Has its own state, so it can be mounted
/// and tested without MongoDB or FCM.
pub fn router(state: RelayState) -> Router {
    Router::new()
        .route("/relay", get(upgrade))
        .with_state(state)
}

async fn upgrade(ws: WebSocketUpgrade, State(s): State<RelayState>) -> impl IntoResponse {
    ws.max_message_size(MAX_FRAME)
        .on_upgrade(move |socket| serve(socket, s))
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
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    // A reconnecting device replaces its old connection.
    s.conns.lock().unwrap().insert(key.clone(), (my_id, tx));
    if socket
        .send(Message::Text(json!({"ok": true}).to_string()))
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
                let Some(Ok(Message::Text(text))) = incoming else {
                    match incoming {
                        Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                        _ => break, // closed, error, or binary (unsupported)
                    }
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                let (Some(to), Some(data)) = (v["to"].as_str(), v.get("data")) else { continue };
                let target = s.conns.lock().unwrap()
                    .get(&(user_id.clone(), to.to_string()))
                    .map(|(_, q)| q.clone());
                let out = json!({"from": node_id, "data": data}).to_string();
                let delivered = target.map(|q| q.send(out).is_ok()).unwrap_or(false);
                if !delivered {
                    let reply = json!({"error": "offline", "to": to}).to_string();
                    if socket.send(Message::Text(reply)).await.is_err() { break; }
                }
            }
            Some(out) = rx.recv() => {
                if socket.send(Message::Text(out)).await.is_err() { break; }
            }
        }
    }
    remove_if_ours(&s, &key, my_id);
    tracing::info!(user = %user_id, node = %node_id, "relay: disconnected");
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
    if conns.get(key).map(|(id, _)| *id == my_id).unwrap_or(false) {
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
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap().port()).unwrap();
                axum::serve(listener, router(RelayState::new(SECRET.into())))
                    .await
                    .unwrap();
            });
        });
        rx.recv().unwrap()
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
    fn rejects_a_bad_token() {
        let port = start();
        let mut ws = client(port, "not-a-jwt", "x");
        assert_eq!(next_json(&mut ws)["error"], "unauthorized");
    }
}
