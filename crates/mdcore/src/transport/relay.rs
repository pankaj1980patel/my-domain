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
//! Closing a relayed link on purpose sends a `Bye`, so the other side
//! doesn't stay "live" on a route its peer no longer listens on.
//!
//! Binary media: a server that forwards binary frames says `"bin": 1` in its
//! welcome (an older one closes the socket on one, so we never send one to
//! it). Then our relayed `Hello` says `bin_media`, and to a peer whose
//! relayed `Hello` said it too, screen and audio records
//! (`transport::media`) go as binary frames `[1][to_len][to][record]`
//! (arriving as `[1][from_len][from][record]`, `from` set by the server)
//! instead of base64 JSON messages. Everything else stays JSON.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tungstenite::Message as WsMessage;

use crate::engine::NetCtx;
use crate::model::{Envelope, WsFrame};
use crate::platform::Platform;

use super::{drop_live, handle_incoming, mark_live, TransportKind};

static RELAY_CONN_SEQ: AtomicU64 = AtomicU64::new(4_000_000);
/// Queued on a relay connection's own channel: close it.
const CLOSE: &str = "\0close";
/// The binary relay frame version we write and read.
const BIN_VERSION: u8 = 1;
/// The server's cap on a binary frame (apps/server `MAX_BIN_FRAME`): a
/// record that won't fit goes as JSON.
const BIN_MAX: usize = 1024 * 1024;
/// Binary frames waiting for the socket. Full (the relay is slower than the
/// frames): a frame is dropped, as a stale frame is worth nothing.
const BIN_QUEUE: usize = 64;

/// This engine's relay connection, if open.
pub struct RelayConn {
    id: u64,
    /// Raw text frames to send to the server.
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    /// Peers reached through the relay: node_id -> their `ws_conns` conn id.
    peers: HashMap<String, u64>,
    /// Peers whose `Hello` we've received (and answered once).
    confirmed: HashSet<String>,
    /// Opened, or last given a peer to route: an idle socket is closed
    /// only a while after this (`close_if_unused`).
    used_at: Instant,
    /// The server forwards binary frames (its welcome said `"bin": 1`).
    bin: bool,
    /// Binary media records to send, already framed.
    bin_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    /// Peers whose relayed `Hello` said they read binary media.
    bin_peers: HashSet<String>,
}

/// The relay URL for a registry base URL: `http://h` -> `ws://h/relay`,
/// `https://h` -> `wss://h/relay` (TLS, verified against the webpki roots).
pub fn relay_url_for(server_base: &str) -> Result<String, String> {
    let base = server_base.trim_end_matches('/');
    if let Some(rest) = base.strip_prefix("http://") {
        Ok(format!("ws://{rest}/relay"))
    } else if let Some(rest) = base.strip_prefix("https://") {
        Ok(format!("wss://{rest}/relay"))
    } else {
        Err(format!("unsupported server URL for relay: {server_base}"))
    }
}

/// `host:port` to dial for a `ws://` / `wss://` URL (default ports 80 / 443).
fn relay_hostport(url: &str) -> Option<String> {
    let (rest, default_port) = if let Some(r) = url.strip_prefix("wss://") {
        (r, 443)
    } else {
        (url.strip_prefix("ws://")?, 80)
    };
    let authority = rest.split('/').next().filter(|a| !a.is_empty())?;
    let has_port = authority
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    Some(if has_port {
        authority.to_string()
    } else {
        format!("{authority}:{default_port}")
    })
}

/// Whether the relay at `url` takes connections: its name resolves and a
/// TCP connection opens within `timeout` (nothing is sent; for the
/// connection check, `features::diag`).
pub fn probe(url: &str, timeout: Duration) -> Result<(), String> {
    let hostport = relay_hostport(url).ok_or_else(|| format!("bad relay url: {url}"))?;
    let addr: SocketAddr = hostport
        .to_socket_addrs()
        .map_err(|e| format!("relay host did not resolve: {e}"))?
        .next()
        .ok_or("relay host did not resolve")?;
    TcpStream::connect_timeout(&addr, timeout)
        .map(drop)
        .map_err(|e| format!("relay connect failed: {e}"))
}

/// Whether this engine's relay socket is open now.
pub fn is_open<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.relay.lock().is_some()
}

/// Open the relay connection if it isn't already. Blocks for the WebSocket
/// handshake and the server's auth reply (a few seconds at most).
pub fn ensure_relay<P: Platform>(ctx: &NetCtx<P>, url: &str, token: &str) -> Result<(), String> {
    // One attempt at a time per device. Two at once (two signals arriving
    // together) would both register with the server; the loser's socket
    // then closes and takes the winner's registration with it, leaving us
    // "connected" to a relay that no longer knows us.
    static CONNECTING: std::sync::OnceLock<parking_lot::Mutex<HashSet<String>>> =
        std::sync::OnceLock::new();
    struct Connecting(String);
    impl Drop for Connecting {
        fn drop(&mut self) {
            CONNECTING
                .get_or_init(Default::default)
                .lock()
                .remove(&self.0);
        }
    }
    let me = ctx.identity.lock().node_id.clone();
    let _connecting = loop {
        if let Some(r) = ctx.relay.lock().as_mut() {
            // About to be used: not "idle" for `close_if_unused`.
            r.used_at = Instant::now();
            return Ok(());
        }
        if CONNECTING
            .get_or_init(Default::default)
            .lock()
            .insert(me.clone())
        {
            break Connecting(me);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    connect_relay(ctx, url, token)
}

fn connect_relay<P: Platform>(ctx: &NetCtx<P>, url: &str, token: &str) -> Result<(), String> {
    if ctx.relay.lock().is_some() {
        return Ok(());
    }
    let hostport = relay_hostport(url).ok_or_else(|| format!("bad relay url: {url}"))?;
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
    // Keep a handle on the raw socket: under TLS the WebSocket wraps it, and
    // the read timeout is set on the socket itself.
    let raw = stream.try_clone().map_err(|e| e.to_string())?;
    // Plain for ws://, rustls for wss:// (chosen from the URL scheme), trusting
    // the OS certificate store too (see crate::tls).
    let tls = tungstenite::Connector::Rustls(crate::tls::client_config());
    let (mut ws, _) = tungstenite::client_tls_with_config(url, stream, None, Some(tls))
        .map_err(|e| format!("relay handshake failed: {e}"))?;

    let me = ctx.identity.lock().node_id.clone();
    ws.send(WsMessage::Text(
        json!({"auth": token, "node_id": me}).to_string(),
    ))
    .map_err(|e| e.to_string())?;
    let bin = match ws.read() {
        Ok(WsMessage::Text(t)) if parse(&t)["ok"] == true => {
            parse(&t)["bin"].as_u64() >= Some(u64::from(BIN_VERSION))
        }
        Ok(WsMessage::Text(t)) => return Err(format!("relay refused: {t}")),
        other => return Err(format!("relay auth failed: {other:?}")),
    };
    let _ = raw.set_read_timeout(Some(Duration::from_millis(100)));

    let id = RELAY_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (bin_tx, bin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(BIN_QUEUE);
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
            used_at: Instant::now(),
            bin,
            bin_tx,
            bin_peers: HashSet::new(),
        });
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || relay_loop(ws, rx, bin_rx, ctx, id));
    Ok(())
}

/// Close the relay connection if no peer is routed through it any more
/// (their links closed as idle). Its 20 s pings would otherwise wake the
/// radio all day; it opens again when a link needs it. Returns whether it
/// was closed.
pub fn close_if_unused<P: Platform>(ctx: &NetCtx<P>, idle: Duration) -> bool {
    // One lock at a time (other paths take them in the other order).
    let Some((id, peers)) = ctx
        .relay
        .lock()
        .as_ref()
        .filter(|r| r.used_at.elapsed() >= idle) // not one just opened
        .map(|r| (r.id, r.peers.clone()))
    else {
        return false;
    };
    // In use = a live link goes through it. A route that never went live
    // (the peer never answered) doesn't count, or a socket would stay open
    // all day for a device that's switched off.
    let routed: Vec<String> = {
        let conns = ctx.ws_conns.lock();
        peers
            .iter()
            .filter(|(peer, conn)| conns.get(*peer).is_some_and(|(c, _)| c == *conn))
            .map(|(peer, _)| peer.clone())
            .collect()
    };
    if routed.iter().any(|p| super::is_live(ctx, p)) {
        return false;
    }
    // A stream moved off it to a local link (`features::nearby`) falls
    // back to it if that link goes: kept open while the stream runs.
    if peers
        .keys()
        .any(|p| crate::features::nearby::media_running(ctx, p))
    {
        return false;
    }
    // Take it out now, not when its loop gets round to closing: a link
    // that needs the relay meanwhile must open a fresh connection, not
    // route through this one as it goes.
    let closing = {
        let mut relay = ctx.relay.lock();
        match relay.as_ref() {
            Some(r) if r.id == id && r.peers.len() == peers.len() => relay.take(),
            _ => None,
        }
    };
    let Some(r) = closing else {
        return false;
    };
    // Its routes go with it (its loop's teardown no longer finds it).
    {
        let mut conns = ctx.ws_conns.lock();
        for (peer, conn) in &r.peers {
            if conns.get(peer).is_some_and(|(c, _)| c == conn) {
                conns.remove(peer);
            }
        }
    }
    r.tx.send(CLOSE.to_string()).is_ok()
}

/// Route `peer` through the relay: register its outgoing queue in
/// `ws_conns` (unless it already has a link). Does NOT mark it live; that
/// happens when its `Hello` arrives.
pub fn add_relay_link<P: Platform>(ctx: &NetCtx<P>, peer: &str) -> bool {
    let (relay_tx, known) = {
        let relay = ctx.relay.lock();
        let Some(r) = relay.as_ref() else {
            return false;
        };
        (r.tx.clone(), r.peers.get(peer).copied())
    };
    // Already routed, and that route is still the peer's link. (A direct
    // link may have replaced it and since died: then route it again, or
    // the peer shows as live with no way to send to it.)
    if let Some(id) = known {
        if ctx.ws_conns.lock().get(peer).is_some_and(|(c, _)| *c == id) {
            return true;
        }
    }
    let conn_id = RELAY_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    {
        let mut conns = ctx.ws_conns.lock();
        if conns.contains_key(peer) {
            return false; // a direct link exists; prefer it
        }
        let route = crate::engine::Route::relayed(relay_tx, peer);
        conns.insert(peer.to_string(), (conn_id, route));
    }
    if let Some(r) = ctx.relay.lock().as_mut() {
        r.peers.insert(peer.to_string(), conn_id);
        r.used_at = Instant::now();
    }
    true
}

/// Stop routing `peer` through the relay (its link was closed on purpose),
/// and tell it so with a `Bye`. Otherwise its side stays "live" on a route
/// that no longer leads anywhere (our socket may close next, as idle): its
/// connect requests are skipped as already connected, and what it sends is
/// lost until its resend timer gives up on the link.
pub fn forget_peer<P: Platform>(ctx: &NetCtx<P>, peer: &str) {
    // One lock at a time (other paths take them in the other order).
    let routed = {
        let mut relay = ctx.relay.lock();
        let Some(r) = relay.as_mut() else {
            return;
        };
        // A new link must greet afresh (see `on_hello`).
        r.confirmed.remove(peer);
        r.bin_peers.remove(peer);
        r.peers.remove(peer).map(|id| (id, r.tx.clone()))
    };
    let Some((conn_id, tx)) = routed else {
        return;
    };
    // Only while the relay is still its link (not a direct one since).
    if ctx
        .ws_conns
        .lock()
        .get(peer)
        .is_some_and(|(c, _)| *c == conn_id)
    {
        let bye = serde_json::to_string(&WsFrame::Bye).unwrap_or_default();
        let _ = tx.send(json!({"to": peer, "data": bye}).to_string());
    }
}

/// The peer closed its relayed link to us (`forget_peer`): drop our side
/// too, so the next connect or send starts a fresh link (and wakes it)
/// instead of sending into the void. A direct link is left alone.
fn on_bye<P: Platform>(ctx: &NetCtx<P>, from: &str) {
    let Some(conn_id) = ctx
        .relay
        .lock()
        .as_ref()
        .and_then(|r| r.peers.get(from).copied())
    else {
        return;
    };
    let ours = {
        let mut conns = ctx.ws_conns.lock();
        let ours = conns.get(from).is_some_and(|(c, _)| *c == conn_id);
        if ours {
            conns.remove(from);
        }
        ours
    };
    if !ours {
        return;
    }
    if let Some(r) = ctx.relay.lock().as_mut() {
        if r.peers.get(from) == Some(&conn_id) {
            r.peers.remove(from);
            r.confirmed.remove(from);
            r.bin_peers.remove(from);
        }
    }
    drop_live(ctx, from);
}

/// Send our `Hello` to `peer` through the relay. It says `bin_media` when
/// we read binary media and our server forwards it (older peers ignore it).
pub fn send_hello<P: Platform>(ctx: &NetCtx<P>, peer: &str) {
    let Some((tx, bin)) = ctx.relay.lock().as_ref().map(|r| (r.tx.clone(), r.bin)) else {
        return;
    };
    let hello = {
        let id = ctx.identity.lock();
        serde_json::to_string(&WsFrame::Hello {
            node_id: id.node_id.clone(),
            name: id.name.clone(),
            boot: String::new(),
            bin_media: if bin && ctx.config.bin_media {
                super::media::VERSION
            } else {
                0
            },
        })
        .unwrap_or_default()
    };
    let _ = tx.send(json!({"to": peer, "data": hello}).to_string());
}

/// The binary queue for `peer`, if `route` (its link's id) is its relayed
/// route, the server forwards binary frames and the peer reads them.
fn media_lane<P: Platform>(
    ctx: &NetCtx<P>,
    peer: &str,
    route: u64,
) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
    ctx.relay.lock().as_ref().and_then(|r| {
        (r.bin && r.peers.get(peer) == Some(&route) && r.bin_peers.contains(peer))
            .then(|| r.bin_tx.clone())
    })
}

/// Whether `peer`'s relayed route `route` takes binary media records.
pub(crate) fn reads_media<P: Platform>(ctx: &NetCtx<P>, peer: &str, route: u64) -> bool {
    media_lane(ctx, peer, route).is_some()
}

/// Send a media record to `peer` as a binary relay frame, if `route` (its
/// link's id) is its relayed route, the server forwards binary frames and
/// the peer's `Hello` said it reads them.
pub(crate) fn send_media<P: Platform>(
    ctx: &NetCtx<P>,
    peer: &str,
    route: u64,
    record: &[u8],
) -> super::media::Sent {
    use super::media::Sent;
    let Some(tx) = media_lane(ctx, peer, route) else {
        return Sent::NotBinary;
    };
    let Some(frame) = bin_frame(peer, record) else {
        return Sent::NotBinary;
    };
    match tx.try_send(frame) {
        Ok(()) => Sent::Yes,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => Sent::Dropped,
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => Sent::NotBinary,
    }
}

/// `[version][to_len][to][record]`; `None` if it can't be one (an id
/// longer than 255 bytes, or bigger than the server takes).
fn bin_frame(to: &str, record: &[u8]) -> Option<Vec<u8>> {
    let n = u8::try_from(to.len()).ok().filter(|n| *n > 0)?;
    let len = 2 + to.len() + record.len();
    if len > BIN_MAX {
        return None;
    }
    let mut f = Vec::with_capacity(len);
    f.extend_from_slice(&[BIN_VERSION, n]);
    f.extend_from_slice(to.as_bytes());
    f.extend_from_slice(record);
    Some(f)
}

/// A binary frame from the server: `(from, record)`.
fn parse_bin(f: &[u8]) -> Option<(&str, &[u8])> {
    let (&ver, rest) = f.split_first()?;
    let (&n, rest) = rest.split_first()?;
    if ver != BIN_VERSION || n == 0 || rest.len() < usize::from(n) {
        return None;
    }
    let (from, record) = rest.split_at(usize::from(n));
    Some((std::str::from_utf8(from).ok()?, record))
}

type RelaySocket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

fn relay_loop<P: Platform>(
    mut ws: RelaySocket,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    mut bin_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    ctx: NetCtx<P>,
    my_id: u64,
) {
    let mut last_ping = Instant::now();
    loop {
        match ws.read() {
            Ok(WsMessage::Text(t)) => on_server_frame(&ctx, &parse(&t)),
            // A media record from a device of ours (the server says which).
            // It opens only with our key; one that doesn't is dropped.
            Ok(WsMessage::Binary(b)) => {
                if let Some((from, record)) = parse_bin(&b) {
                    super::media::on_record(&ctx, from, record);
                }
            }
            Ok(WsMessage::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(_) => break,
        }
        let mut dead = false;
        while let Ok(out) = rx.try_recv() {
            if out == CLOSE {
                let _ = ws.close(None);
                let _ = ws.flush();
                dead = true;
                break;
            }
            if ws.send(WsMessage::Text(out)).is_err() {
                dead = true;
                break;
            }
        }
        while let (false, Ok(frame)) = (dead, bin_rx.try_recv()) {
            if ws.send(WsMessage::Binary(frame)).is_err() {
                dead = true;
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
    // A signal the server routed to us over this socket (instead of FCM).
    if let Some(sig) = v.get("signal") {
        if let (Some(from), Some(payload)) = (sig["sender"].as_str(), sig["payload"].as_str()) {
            let (ctx, from, payload) = (ctx.clone(), from.to_string(), payload.to_string());
            std::thread::spawn(move || crate::engine::handle_signal(&ctx, &from, &payload));
        }
        return;
    }
    // {"error":"offline"} isn't acted on: it may answer a frame sent while
    // the peer was away and arrive after it's back. A route that really
    // went dead shows as unacknowledged messages (`outbox::retry_due`).
    let (Some(from), Some(data)) = (v["from"].as_str(), v["data"].as_str()) else {
        return;
    };
    match serde_json::from_str::<WsFrame>(data) {
        Ok(WsFrame::Hello { bin_media, .. }) => {
            if let Some(r) = ctx.relay.lock().as_mut() {
                // Every Hello says it afresh (the peer may have restarted
                // as an older app).
                if bin_media >= super::media::VERSION {
                    r.bin_peers.insert(from.to_string());
                } else {
                    r.bin_peers.remove(from);
                }
            }
            on_hello(ctx, from)
        }
        Ok(WsFrame::Bye) => on_bye(ctx, from),
        Ok(WsFrame::Msg { nonce, ciphertext }) => {
            // A message from a device we have no way back to (its hello, or
            // the push that set this up, got lost): route it now, or our
            // receipts and replies would have nowhere to go.
            if !ctx.ws_conns.lock().contains_key(from) {
                on_hello(ctx, from);
            } else {
                check_direct(ctx, from);
            }
            let env = Envelope { nonce, ciphertext };
            handle_incoming(ctx, &env, Some(from), String::new(), "RELAY");
        }
        Err(_) => {}
    }
}

/// A message came by relay from a peer we hold a QUIC link to. Either the
/// peer is still upgrading, or our link died without closing (the phone
/// moved from Wi-Fi to mobile data): then everything we send it, receipts
/// included, vanishes. Ping over the link; if QUIC hears nothing back in
/// `PROBE_WAIT`, close it and answer by relay. At most once per
/// `PROBE_EVERY` per peer.
fn check_direct<P: Platform>(ctx: &NetCtx<P>, from: &str) {
    const PROBE_WAIT: Duration = Duration::from_secs(3);
    const PROBE_EVERY: Duration = Duration::from_secs(10);
    static LAST: std::sync::OnceLock<parking_lot::Mutex<HashMap<String, Instant>>> =
        std::sync::OnceLock::new();
    let Some(before) = ctx.quic.rx_datagrams(from) else {
        return;
    };
    let key = format!("{}>{from}", ctx.identity.lock().node_id);
    {
        let mut last = LAST.get_or_init(Default::default).lock();
        if last.get(&key).is_some_and(|t| t.elapsed() < PROBE_EVERY) {
            return;
        }
        last.insert(key, Instant::now());
    }
    let (ctx, from) = (ctx.clone(), from.to_string());
    std::thread::spawn(move || {
        let me = ctx.identity.lock().node_id.clone();
        let ping = json!({ "type": "p2p_ping", "from": me }).to_string();
        super::send_control_to(&ctx, &from, &ping);
        std::thread::sleep(PROBE_WAIT);
        if ctx.quic.rx_datagrams(&from) != Some(before) {
            return; // alive (or already gone)
        }
        super::close_link(&ctx, &from);
        for _ in 0..30 {
            if !ctx.ws_conns.lock().contains_key(&from) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        on_hello(&ctx, &from);
    });
}

/// The peer is on the relay: route it, greet it back once, mark it live.
fn on_hello<P: Platform>(ctx: &NetCtx<P>, from: &str) {
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

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_frames_round_trip_and_bad_ones_are_refused() {
        let f = bin_frame("phone", b"record").unwrap();
        assert_eq!(f, b"\x01\x05phonerecord");
        // What the server forwards has the same shape, with `from`.
        assert_eq!(parse_bin(&f), Some(("phone", &b"record"[..])));
        assert_eq!(parse_bin(b"\x02\x05phonerecord"), None, "another version");
        assert_eq!(parse_bin(b"\x01\x09ph"), None, "short");
        assert_eq!(parse_bin(b"\x01\x00x"), None, "no id");
        assert_eq!(parse_bin(b"\x01\x01\xff"), None, "not UTF-8");
        assert_eq!(parse_bin(b""), None);
        assert!(bin_frame(&"x".repeat(256), b"r").is_none(), "id too long");
        assert!(bin_frame("", b"r").is_none());
        assert!(bin_frame("p", &vec![0; BIN_MAX - 3]).is_some());
        assert!(
            bin_frame("p", &vec![0; BIN_MAX - 2]).is_none(),
            "over the cap"
        );
    }

    #[test]
    fn relay_url_follows_the_server_scheme() {
        assert_eq!(
            relay_url_for("http://1.2.3.4:6969/").unwrap(),
            "ws://1.2.3.4:6969/relay"
        );
        assert_eq!(
            relay_url_for("https://md.example").unwrap(),
            "wss://md.example/relay"
        );
        assert!(relay_url_for("ftp://x").is_err());
    }

    #[test]
    fn relay_hostport_fills_in_default_ports() {
        assert_eq!(
            relay_hostport("ws://1.2.3.4:6969/relay").as_deref(),
            Some("1.2.3.4:6969")
        );
        assert_eq!(
            relay_hostport("wss://md.example/relay").as_deref(),
            Some("md.example:443")
        );
        assert_eq!(relay_hostport("ws://h/relay").as_deref(), Some("h:80"));
        assert_eq!(
            relay_hostport("wss://h:8443/relay").as_deref(),
            Some("h:8443")
        );
        assert_eq!(relay_hostport("http://h/relay"), None);
        assert_eq!(relay_hostport("wss:///relay"), None);
    }
}
