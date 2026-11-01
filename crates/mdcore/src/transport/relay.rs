//! Relay link: the last connect-ladder rung. Both devices hold a WebSocket to
//! the registry server's `/relay`, and the server forwards frames between
//! them (only between devices of the same account). Works through firewalls
//! that block UDP entirely, because it's an ordinary outgoing web connection.
//!
//! What the server sees: `{"to": peer, "data": <frame>}`, where `<frame>` is
//! the same JSON `WsFrame` the peer-to-peer links carry. `Msg` frames hold
//! end-to-end-encrypted envelopes, so the server only ever sees ciphertext.
//!
//! One relay socket per engine, shared by every relayed peer. Each relayed
//! peer gets an entry in `ctx.ws_conns` whose queue wraps frames as
//! `{"to": peer, "data": frame}`, so the rest of the engine treats a relayed
//! peer exactly like a direct link.
//!
//! Liveness handshake: a peer is marked live only after its `Hello` arrives
//! through the relay (proof it's connected). Each side answers the FIRST
//! `Hello` it gets from a peer with its own, so both sides confirm even if
//! one joined after the other's first `Hello` was dropped as "offline".

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::Message as WsMessage;

use crate::engine::NetCtx;
use crate::model::{Envelope, WsFrame};
use crate::platform::Platform;

use super::{drop_live, handle_incoming, mark_live, TransportKind};

static RELAY_CONN_SEQ: AtomicU64 = AtomicU64::new(4_000_000);

/// This engine's relay connection, if open.
pub struct RelayConn {
    id: u64,
    /// Raw text frames to send to the server.
    tx: mpsc::Sender<String>,
    /// Peers reached through the relay: node_id -> their `ws_conns` conn id.
    peers: HashMap<String, u64>,
    /// Peers whose `Hello` we've received (and answered once).
    confirmed: HashSet<String>,
}

/// `ws://host:port/relay` for a registry base URL (`http://host:port`).
/// `https` servers need TLS on the WebSocket, which isn't built in yet.
pub fn relay_url_for(server_base: &str) -> Result<String, String> {
    if let Some(rest) = server_base.strip_prefix("http://") {
        Ok(format!("ws://{}/relay", rest.trim_end_matches('/')))
    } else if server_base.starts_with("https://") {
        Err("relay over TLS (wss://) is not supported yet".into())
    } else {
        Err(format!("unsupported server URL for relay: {server_base}"))
    }
}

/// Open the relay connection if it isn't already. Blocks for the WebSocket
/// handshake and the server's auth reply (a few seconds at most).
pub fn ensure_relay<P: Platform>(ctx: &NetCtx<P>, url: &str, token: &str) -> Result<(), String> {
    if ctx.relay.lock().is_some() {
        return Ok(());
    }
    let hostport = url
        .strip_prefix("ws://")
        .and_then(|r| r.split('/').next())
        .ok_or_else(|| format!("bad relay url: {url}"))?;
    let addr: SocketAddr = hostport
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("relay host did not resolve")?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(4))
        .map_err(|e| format!("relay connect failed: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let (mut ws, _) =
        tungstenite::client(url, stream).map_err(|e| format!("relay handshake failed: {e}"))?;

    let me = ctx.identity.lock().node_id.clone();
    ws.send(WsMessage::Text(
        json!({"auth": token, "node_id": me}).to_string(),
    ))
    .map_err(|e| e.to_string())?;
    match ws.read() {
        Ok(WsMessage::Text(t)) if parse(&t)["ok"] == true => {}
        Ok(WsMessage::Text(t)) => return Err(format!("relay refused: {t}")),
        other => return Err(format!("relay auth failed: {other:?}")),
    }
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)));

    let id = RELAY_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel::<String>();
    {
        let mut slot = ctx.relay.lock();
        if slot.is_some() {
            return Ok(()); // another thread won the race; drop ours
        }
        *slot = Some(RelayConn {
            id,
            tx,
            peers: HashMap::new(),
            confirmed: HashSet::new(),
        });
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || relay_loop(ws, rx, ctx, id));
    Ok(())
}

/// Route `peer` through the relay: register its outgoing queue in
/// `ws_conns` (unless it already has a link). Does NOT mark it live; that
/// happens when its `Hello` arrives.
pub fn add_relay_link<P: Platform>(ctx: &NetCtx<P>, peer: &str) -> bool {
    let relay_tx = {
        let relay = ctx.relay.lock();
        let Some(r) = relay.as_ref() else {
            return false;
        };
        if r.peers.contains_key(peer) {
            return true;
        }
        r.tx.clone()
    };
    let conn_id = RELAY_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (ptx, prx) = mpsc::channel::<String>();
    {
        let mut conns = ctx.ws_conns.lock();
        if conns.contains_key(peer) {
            return false; // a direct link exists; prefer it
        }
        conns.insert(peer.to_string(), (conn_id, ptx));
    }
    if let Some(r) = ctx.relay.lock().as_mut() {
        r.peers.insert(peer.to_string(), conn_id);
    }
    // Wrap this peer's frames for the server. Ends when the link is removed
    // (sender dropped) or the relay socket is gone.
    let to = peer.to_string();
    std::thread::spawn(move || {
        while let Ok(frame) = prx.recv() {
            let wrapped = json!({"to": to, "data": frame}).to_string();
            if relay_tx.send(wrapped).is_err() {
                break;
            }
        }
    });
    true
}

/// Send our `Hello` to `peer` through the relay.
pub fn send_hello<P: Platform>(ctx: &NetCtx<P>, peer: &str) {
    let hello = {
        let id = ctx.identity.lock();
        serde_json::to_string(&WsFrame::Hello {
            node_id: id.node_id.clone(),
            name: id.name.clone(),
        })
        .unwrap_or_default()
    };
    if let Some(r) = ctx.relay.lock().as_ref() {
        let _ = r.tx.send(json!({"to": peer, "data": hello}).to_string());
    }
}

fn relay_loop<P: Platform>(
    mut ws: tungstenite::WebSocket<TcpStream>,
    rx: mpsc::Receiver<String>,
    ctx: NetCtx<P>,
    my_id: u64,
) {
    let mut last_ping = Instant::now();
    loop {
        match ws.read() {
            Ok(WsMessage::Text(t)) => on_server_frame(&ctx, &parse(&t)),
            Ok(WsMessage::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
        let mut dead = false;
        while let Ok(out) = rx.try_recv() {
            if ws.send(WsMessage::Text(out)).is_err() {
                dead = true;
                break;
            }
        }
        // Keep idle proxies/NATs from dropping the connection.
        if last_ping.elapsed() > Duration::from_secs(20) {
            dead |= ws.send(WsMessage::Ping(vec![])).is_err();
            last_ping = Instant::now();
        }
        if dead || ctx.shutdown.load(Ordering::Relaxed) {
            break;
        }
    }
    // Tear down: forget the relay and every peer that was reached through it.
    let peers = {
        let mut slot = ctx.relay.lock();
        match slot.as_ref() {
            Some(r) if r.id == my_id => slot.take().map(|r| r.peers).unwrap_or_default(),
            _ => HashMap::new(),
        }
    };
    for (peer, conn_id) in peers {
        let ours = {
            let mut conns = ctx.ws_conns.lock();
            let ours = conns
                .get(&peer)
                .map(|(id, _)| *id == conn_id)
                .unwrap_or(false);
            if ours {
                conns.remove(&peer);
            }
            ours
        };
        if ours {
            drop_live(&ctx, &peer);
        }
    }
}

fn on_server_frame<P: Platform>(ctx: &NetCtx<P>, v: &Value) {
    let (Some(from), Some(data)) = (v["from"].as_str(), v["data"].as_str()) else {
        return; // e.g. {"error":"offline"}: the ladder's timeout handles it
    };
    match serde_json::from_str::<WsFrame>(data) {
        Ok(WsFrame::Hello { .. }) => {
            add_relay_link(ctx, from);
            let first = ctx
                .relay
                .lock()
                .as_mut()
                .map(|r| r.confirmed.insert(from.to_string()))
                .unwrap_or(false);
            if first {
                send_hello(ctx, from);
            }
            mark_live(ctx, from, TransportKind::Relay, None);
        }
        Ok(WsFrame::Msg { nonce, ciphertext }) => {
            let env = Envelope { nonce, ciphertext };
            handle_incoming(ctx, &env, String::new(), "RELAY");
        }
        Err(_) => {}
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}
