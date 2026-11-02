//! Direct messaging transports (UDP / TCP) plus the shared incoming-message
//! pipeline and the clipboard control-message handling. WebSocket lives in the
//! `ws` submodule.
//!
//! Wire format here is protocol v1 (JSON `Envelope` carrying `Plaintext`); the
//! binary multiplexed framing (v2) is layered on later in `wire::`.

pub mod punch;
pub mod quality;
pub mod quic;
pub mod relay;
#[cfg(feature = "webrtc")]
pub mod rtc;
pub mod ws;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::crypto::{decrypt, encrypt};
use crate::discovery::PeerMap;
use crate::engine::{KeyHolder, NetCtx, WsConns};
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{Envelope, IncomingMessage, Plaintext, WsFrame};
use crate::platform::Platform;

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Unified live-connection model — the single source of truth for "who is
// connected, over what transport". The UI green dot reads ONLY this.
// ---------------------------------------------------------------------------

/// Which transport a live P2P link is using. Serializes lowercase for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    Ws,
    Udp,
    Punch,
    WebRtc,
    Quic,
    Relay,
}

/// A live peer-to-peer link. `last_seen` drives heartbeat staleness for the
/// connectionless transports (UDP/Punch/WebRtc); WS is event-driven.
#[derive(Clone)]
pub struct P2pConn {
    pub kind: TransportKind,
    pub last_seen: u64,
    pub addr: Option<SocketAddr>,
}

/// node_id -> live link.
pub type P2pConns = Arc<Mutex<HashMap<String, P2pConn>>>;

/// The peer has a live link that isn't through the relay.
pub fn is_direct<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    ctx.p2p_conns
        .lock()
        .get(node_id)
        .is_some_and(|c| c.kind != TransportKind::Relay)
}

/// Mark a peer live (first time) or refresh an existing link. The transport is
/// fixed at establishment by whoever created the link (ws/punch/webrtc); a
/// refresh (e.g. a heartbeat ping) only bumps `last_seen` and never downgrades
/// the kind. Emits `PeerConnected` only on first appearance. Never emits under
/// the lock.
pub fn mark_live<P: Platform>(
    ctx: &NetCtx<P>,
    node_id: &str,
    kind: TransportKind,
    addr: Option<SocketAddr>,
) {
    let newly = {
        let mut m = ctx.p2p_conns.lock();
        match m.get_mut(node_id) {
            // A relayed peer that now has a direct link is UPGRADED (and the
            // UI told, with the new transport). Never the other way round.
            Some(c) if c.kind == TransportKind::Relay && kind != TransportKind::Relay => {
                *c = P2pConn {
                    kind,
                    last_seen: now_secs(),
                    addr,
                };
                true
            }
            Some(c) => {
                c.last_seen = now_secs();
                if addr.is_some() {
                    c.addr = addr;
                }
                false
            }
            None => {
                m.insert(
                    node_id.to_string(),
                    P2pConn {
                        kind,
                        last_seen: now_secs(),
                        addr,
                    },
                );
                true
            }
        }
    };
    if newly {
        ctx.sink.emit(CoreEvent::PeerConnected {
            node_id: node_id.to_string(),
            transport: kind,
        });
    }
}

/// Refresh `last_seen` for an existing live peer (e.g. on pong/keepalive).
pub fn touch_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    if let Some(c) = ctx.p2p_conns.lock().get_mut(node_id) {
        c.last_seen = now_secs();
    }
}

/// Drop a peer's live link. Emits `PeerDisconnected` if it was present.
pub fn drop_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    let removed = ctx.p2p_conns.lock().remove(node_id).is_some();
    if removed {
        ctx.sink.emit(CoreEvent::PeerDisconnected {
            node_id: node_id.to_string(),
        });
    }
}

/// Snapshot of live peers and their transports.
pub fn live_peers<P: Platform>(
    ctx: &NetCtx<P>,
) -> Vec<(String, TransportKind, Option<SocketAddr>)> {
    ctx.p2p_conns
        .lock()
        .iter()
        .map(|(k, v)| (k.clone(), v.kind, v.addr))
        .collect()
}

/// Snapshot of live peers with their `last_seen` — for heartbeat staleness checks.
pub fn live_snapshot<P: Platform>(ctx: &NetCtx<P>) -> Vec<(String, TransportKind, u64)> {
    ctx.p2p_conns
        .lock()
        .iter()
        .map(|(k, v)| (k.clone(), v.kind, v.last_seen))
        .collect()
}

/// Decrypt an envelope into a UI message; undecryptable messages are surfaced
/// (not silently dropped).
/// How often one sender IP may produce a visible "could not decrypt" notice.
const UNDECRYPTABLE_NOTICE_EVERY_SECS: u64 = 60;
/// Cap on tracked sender IPs, so a flood from many addresses can't grow memory.
const UNDECRYPTABLE_MAX_TRACKED: usize = 1024;

/// An envelope from `ip` failed to decrypt. Count it, and surface at most one
/// notice per sender IP per minute: enough to tell a user their passphrase
/// doesn't match, without letting any LAN host flood the chat with junk.
fn note_undecryptable<P: Platform>(ctx: &NetCtx<P>, ip: String, protocol: &str) {
    ctx.decrypt_fail_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = now_secs();
    {
        let mut last = ctx.decrypt_fail_last.lock();
        if let Some(t) = last.get(&ip) {
            if now.saturating_sub(*t) < UNDECRYPTABLE_NOTICE_EVERY_SECS {
                return;
            }
        }
        if last.len() >= UNDECRYPTABLE_MAX_TRACKED {
            last.clear();
        }
        last.insert(ip.clone(), now);
    }
    ctx.sink.emit(CoreEvent::MessageReceived(IncomingMessage {
        from: "(unknown)".into(),
        ip,
        protocol: protocol.into(),
        text: "🔒 message could not be decrypted (wrong encryption key)".into(),
        ts: now,
        ok: false,
    }));
}

/// How a message leaves this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendPath {
    /// Over the peer's live persistent link. WebSocket, UDP hole punch and
    /// WebRTC all register in `ws_conns` and share one framed send path.
    Link,
    /// One short TCP connection per message.
    Tcp,
    /// One UDP datagram.
    Udp,
}

impl SendPath {
    /// Label reported to the UI (the strings the apps have always shown).
    pub fn label(self) -> &'static str {
        match self {
            SendPath::Link => "WS",
            SendPath::Tcp => "TCP",
            SendPath::Udp => "UDP",
        }
    }
}

/// The user's choice for peers with no live link (settings: "UDP" / "TCP").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Directed {
    #[default]
    Udp,
    Tcp,
}

impl Directed {
    /// "tcp" (any case, trimmed) selects TCP; anything else is UDP.
    pub fn parse(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("tcp") {
            Directed::Tcp
        } else {
            Directed::Udp
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Directed::Udp => "UDP",
            Directed::Tcp => "TCP",
        }
    }
}

/// Pick how to send to `node_id`: a live persistent link always wins (so
/// exchanges ride the active connection); otherwise the directed choice. The
/// single chooser used by chat and every feature.
pub fn best_path(ws_conns: &WsConns, directed: Directed, node_id: &str) -> SendPath {
    if ws_conns.lock().contains_key(node_id) {
        SendPath::Link
    } else {
        match directed {
            Directed::Tcp => SendPath::Tcp,
            Directed::Udp => SendPath::Udp,
        }
    }
}

/// `best_path` using a context's directed-transport setting.
pub(crate) fn path_for<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> SendPath {
    let directed = *ctx.directed_transport.lock();
    best_path(&ctx.ws_conns, directed, node_id)
}

/// Send an already-built control-message JSON to one peer over the active
/// connection (live WS/punch/WebRTC), else the directed transport. Shared by
/// every discrete feature.
pub(crate) fn send_control_to<P: Platform>(ctx: &NetCtx<P>, node_id: &str, text: &str) {
    let from = ctx.identity.lock().name.clone();
    let path = path_for(ctx, node_id);
    let _ = send_payload(
        &ctx.peers,
        &ctx.ws_conns,
        &ctx.key,
        &from,
        node_id,
        path,
        text,
    );
}

/// Broadcast a control-message JSON to all known peers.
pub fn broadcast_control<P: Platform>(ctx: &NetCtx<P>, text: &str) {
    let ids: Vec<String> = ctx.peers.lock().keys().cloned().collect();
    for id in ids {
        send_control_to(ctx, &id, text);
    }
}

/// Decrypt and dispatch an incoming envelope. Clipboard control messages are
/// handled in-band (never shown as chat); everything else is emitted as a
/// normal `MessageReceived`.
pub fn handle_incoming<P: Platform>(ctx: &NetCtx<P>, env: &Envelope, ip: String, protocol: &str) {
    let pt = {
        let guard = ctx.key.lock();
        guard.as_ref().and_then(|k| decrypt(k, env))
    };
    let Some(pt) = pt else {
        note_undecryptable(ctx, ip, protocol);
        return;
    };
    let Ok(msg) = serde_json::from_slice::<Plaintext>(&pt) else {
        return;
    };

    // Discrete-feature control message? (clipboard / notification / call / …)
    let control = ctx.control.clone();
    if control.dispatch(ctx, &msg.from, &ip, protocol, &msg.text) {
        return;
    }

    ctx.sink.emit(CoreEvent::MessageReceived(IncomingMessage {
        from: msg.from,
        ip,
        protocol: protocol.into(),
        text: msg.text,
        ts: now_secs(),
        ok: true,
    }));
}

/// Encrypt `text` and send it to `node_id` over the given transport. Shared by
/// the chat send command and the clipboard helpers.
pub fn send_payload(
    peers: &PeerMap,
    ws_conns: &WsConns,
    key: &KeyHolder,
    from: &str,
    node_id: &str,
    path: SendPath,
    text: &str,
) -> crate::Result<()> {
    let k = key.lock().ok_or(CoreError::NoKey)?;
    let plaintext = serde_json::to_vec(&Plaintext {
        from: from.to_string(),
        text: text.to_string(),
    })
    .map_err(|e| e.to_string())?;
    let env = encrypt(&k, &plaintext).ok_or("encryption failed")?;
    let body = serde_json::to_vec(&env).map_err(|e| e.to_string())?;

    if path == SendPath::Link {
        let frame = serde_json::to_string(&WsFrame::Msg {
            nonce: env.nonce.clone(),
            ciphertext: env.ciphertext.clone(),
        })
        .map_err(|e| e.to_string())?;
        let sender = ws_conns
            .lock()
            .get(node_id)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| {
                CoreError::NoLink("no WebSocket connection — trigger 'Connect (WS)' first".into())
            })?;
        sender
            .send(frame)
            .map_err(|_| CoreError::NoLink("WebSocket connection closed".into()))?;
        return Ok(());
    }

    let peer = peers
        .lock()
        .get(node_id)
        .cloned()
        .ok_or(CoreError::PeerNotFound)?;
    let ip: Ipv4Addr = peer.ip.parse().map_err(|_| "bad peer ip")?;
    match path {
        SendPath::Tcp => {
            let addr = SocketAddr::new(ip.into(), peer.tcp_port);
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
                .map_err(|e| format!("TCP connect failed: {e}"))?;
            stream
                .write_all(&body)
                .map_err(|e| format!("TCP send failed: {e}"))?;
            stream
                .shutdown(std::net::Shutdown::Write)
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        SendPath::Udp => {
            let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            socket
                .send_to(&body, SocketAddr::new(ip.into(), peer.udp_port))
                .map_err(|e| format!("UDP send failed: {e}"))?;
            Ok(())
        }
        // Returned early above; kept so the match stays exhaustive without a panic.
        SendPath::Link => Err(CoreError::NoLink("no live link".into())),
    }
}

/// Largest TCP message body we accept. A message is one JSON envelope (base64
/// ciphertext), so 4 MiB leaves room for a ~2.9 MiB plaintext.
pub(crate) const MAX_TCP_MESSAGE: u64 = 4 * 1024 * 1024;
/// A sender must deliver its whole message within this time.
const TCP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// At most this many inbound TCP messages are read concurrently; extra
/// connections are closed immediately instead of each getting a thread.
const MAX_TCP_IN_FLIGHT: usize = 128;

/// Accept loop for directed-TCP messages: one short-lived connection per
/// message (the sender writes one envelope, then closes).
///
/// Defences (each used to be missing): a size cap on the body, a read timeout
/// so an idle connection can't hold a thread forever, and a cap on concurrent
/// readers so a connection flood can't exhaust threads.
pub fn tcp_recv_loop<P: Platform>(listener: TcpListener, ctx: NetCtx<P>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let in_flight = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            // e.g. EMFILE (out of file descriptors): back off instead of spinning.
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        };
        if in_flight.fetch_add(1, Ordering::AcqRel) >= MAX_TCP_IN_FLIGHT {
            in_flight.fetch_sub(1, Ordering::AcqRel);
            continue; // dropping `stream` closes it
        }
        let ctx = ctx.clone();
        let in_flight = in_flight.clone();
        std::thread::spawn(move || {
            read_tcp_message(&mut stream, &ctx);
            in_flight.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

fn read_tcp_message<P: Platform>(stream: &mut TcpStream, ctx: &NetCtx<P>) {
    let ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    if stream.set_read_timeout(Some(TCP_READ_TIMEOUT)).is_err() {
        return;
    }
    // Read at most one byte past the cap so an oversized body is detectable.
    let mut buf = Vec::new();
    if (&mut *stream)
        .take(MAX_TCP_MESSAGE + 1)
        .read_to_end(&mut buf)
        .is_err()
        || buf.len() as u64 > MAX_TCP_MESSAGE
    {
        return;
    }
    if let Ok(env) = serde_json::from_slice::<Envelope>(&buf) {
        handle_incoming(ctx, &env, ip, "TCP");
    }
}

pub fn udp_recv_loop<P: Platform>(socket: UdpSocket, ctx: NetCtx<P>) {
    let mut buf = [0u8; 65535];
    loop {
        let (len, src) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Ok(env) = serde_json::from_slice::<Envelope>(&buf[..len]) {
            handle_incoming(&ctx, &env, src.ip().to_string(), "UDP");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_path_prefers_a_live_link_then_the_directed_choice() {
        let conns: WsConns = Arc::default();
        assert_eq!(best_path(&conns, Directed::Udp, "peer"), SendPath::Udp);
        assert_eq!(best_path(&conns, Directed::Tcp, "peer"), SendPath::Tcp);

        let (tx, _rx) = std::sync::mpsc::channel();
        conns.lock().insert("peer".into(), (1, tx));
        assert_eq!(
            best_path(&conns, Directed::Tcp, "peer"),
            SendPath::Link,
            "live link wins"
        );
        assert_eq!(best_path(&conns, Directed::Tcp, "other"), SendPath::Tcp);
    }

    #[test]
    fn directed_parses_leniently_and_defaults_to_udp() {
        assert_eq!(Directed::parse("tcp"), Directed::Tcp);
        assert_eq!(Directed::parse(" TCP "), Directed::Tcp);
        assert_eq!(Directed::parse("UDP"), Directed::Udp);
        assert_eq!(Directed::parse("carrier pigeon"), Directed::Udp);
        assert_eq!(Directed::default(), Directed::Udp);
    }
}
