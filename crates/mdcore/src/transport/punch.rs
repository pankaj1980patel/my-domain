//! UDP hole-punch data path. Builds on the `nat::stun` + `nat::punch`
//! primitives: gather host + reflexive candidates on a dedicated socket,
//! exchange them via FCM signaling (`PunchOffer`/`PunchAnswer`), punch, then run
//! a driver thread that ferries the very same encrypted `WsFrame::Msg` frames
//! the WS path uses. We register the punched peer in `ws_conns`, so the rest of
//! the engine (send_payload's "WS" branch, send_control_to, the green dot)
//! treats a punched link exactly like a WebSocket — no special-casing.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::engine::NetCtx;
use crate::model::{Envelope, WsFrame};
use crate::nat::{punch::hole_punch, stun::stun_binding};
use crate::platform::Platform;
use crate::signal::Candidate;

use super::{drop_live, handle_incoming, mark_live, touch_live, TransportKind};

const PUNCH_MAGIC: &[u8] = b"mdpunch1";
/// Driver-level keepalive/handshake probes (distinct from the WsFrame data that
/// also rides this socket). PING is the same bytes as the punch magic so a peer
/// still finishing `hole_punch` recognizes it; PONG is the reply that proves the
/// *round trip*. We only declare the link live once a PONG comes back.
const PUNCH_PING: &[u8] = PUNCH_MAGIC;
const PUNCH_PONG: &[u8] = b"mdpong01";

// --- application-level chunking ---------------------------------------------
// A raw UDP datagram can't reliably carry a large message: >~65 KB fails to send
// outright, and anything past the path MTU is IP-fragmented and routinely
// dropped by NAT/routers. So we split each WsFrame into MTU-safe chunks with a
// tiny header and reassemble on the far side. Header: magic(4) id(4) total(2)
// index(2) = 12 bytes; payload ≤ CHUNK_PAYLOAD.
const CHUNK_MAGIC: &[u8] = b"mdck";
const CHUNK_HDR: usize = 4 + 4 + 2 + 2;
const CHUNK_PAYLOAD: usize = 1024;
/// Cap on simultaneously-reassembling messages, so a peer dribbling partial
/// chunks can't grow memory unbounded.
const REASM_MAX: usize = 256;
/// Largest message the punch path will reassemble: 1024 chunks x 1 KiB = 1 MiB.
/// A datagram claiming more is dropped, so a 12-byte packet can no longer make
/// us preallocate up to 65535 slots.
const MAX_CHUNKS: u16 = 1024;

/// Split `data` into chunk datagrams and send each to `peer`. Always emits at
/// least one datagram (so empty payloads still arrive).
fn send_chunked(
    socket: &UdpSocket,
    peer: SocketAddr,
    msg_id: u32,
    data: &[u8],
) -> std::io::Result<()> {
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[][..]]
    } else {
        data.chunks(CHUNK_PAYLOAD).collect()
    };
    // The receiver drops anything over MAX_CHUNKS; fail loudly here instead.
    if chunks.len() > MAX_CHUNKS as usize {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("message too large for the punch path: {} bytes", data.len()),
        ));
    }
    let total = chunks.len() as u16;
    let mut pkt = Vec::with_capacity(CHUNK_HDR + CHUNK_PAYLOAD);
    for (i, chunk) in chunks.iter().enumerate() {
        pkt.clear();
        pkt.extend_from_slice(CHUNK_MAGIC);
        pkt.extend_from_slice(&msg_id.to_be_bytes());
        pkt.extend_from_slice(&total.to_be_bytes());
        pkt.extend_from_slice(&(i as u16).to_be_bytes());
        pkt.extend_from_slice(chunk);
        socket.send_to(&pkt, peer)?;
    }
    Ok(())
}

/// In-progress reassembly of one chunked message.
struct Reasm {
    total: u16,
    received: u16,
    parts: Vec<Option<Vec<u8>>>,
}

/// Feed one received datagram into the reassembler. Returns `Some(bytes)` when a
/// message is complete, `None` while still collecting (or if it isn't a chunk).
fn reasm_feed(pending: &mut HashMap<u32, Reasm>, pkt: &[u8]) -> Option<Vec<u8>> {
    if pkt.len() < CHUNK_HDR || &pkt[..4] != CHUNK_MAGIC {
        return None;
    }
    let id = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
    let total = u16::from_be_bytes([pkt[8], pkt[9]]);
    let index = u16::from_be_bytes([pkt[10], pkt[11]]);
    if total == 0 || total > MAX_CHUNKS || index >= total {
        return None;
    }
    let payload = pkt[CHUNK_HDR..].to_vec();

    if pending.len() >= REASM_MAX && !pending.contains_key(&id) {
        pending.clear(); // shed stalled partials rather than grow forever
    }
    let entry = pending.entry(id).or_insert_with(|| Reasm {
        total,
        received: 0,
        parts: vec![None; total as usize],
    });
    if entry.total != total {
        return None; // inconsistent — ignore
    }
    if entry.parts[index as usize].is_none() {
        entry.received += 1;
        entry.parts[index as usize] = Some(payload);
    }
    if entry.received == entry.total {
        let done = pending.remove(&id)?;
        let mut out = Vec::new();
        for p in done.parts {
            out.extend_from_slice(&p?);
        }
        return Some(out);
    }
    None
}
/// Public STUN server for reflexive-candidate discovery.
/// Punch-connection ids live in a high range so they never collide with WS ids.
static PUNCH_CONN_SEQ: AtomicU64 = AtomicU64::new(1_000_000);

/// Bind a fresh UDP socket and gather this device's candidates on it: a host
/// candidate (our LAN ip + that socket's port) plus a STUN reflexive candidate.
/// The SAME socket must be used to punch so the NAT mapping matches.
pub fn gather_candidates<P: Platform>(ctx: &NetCtx<P>) -> Option<(UdpSocket, Vec<Candidate>)> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    let port = socket.local_addr().ok()?.port();
    let my_ip = ctx.identity.lock().ip.clone();
    let mut cands = Vec::new();
    if !my_ip.is_empty() && my_ip != "0.0.0.0" {
        cands.push(Candidate {
            ip: my_ip,
            port,
            kind: "host".into(),
        });
    }
    if let Some(server) = ctx.config.stun_server() {
        if let Some(refl) = stun_binding(&socket, server) {
            cands.push(Candidate {
                ip: refl.ip().to_string(),
                port: refl.port(),
                kind: "srflx".into(),
            });
        }
    }
    if cands.is_empty() {
        None
    } else {
        Some((socket, cands))
    }
}

fn to_addrs(cands: &[Candidate]) -> Vec<SocketAddr> {
    cands
        .iter()
        .filter_map(|c| format!("{}:{}", c.ip, c.port).parse().ok())
        .collect()
}

/// Spawn a thread that punches toward `peer_cands` on `socket` and, on success,
/// runs the driver loop. Returns immediately; the link appears as a
/// `PeerConnected` event when it comes up.
pub fn punch_and_run<P: Platform>(
    ctx: NetCtx<P>,
    socket: UdpSocket,
    peer_cands: Vec<Candidate>,
    node_id: String,
) {
    std::thread::spawn(move || {
        let addrs = to_addrs(&peer_cands);
        if addrs.is_empty() {
            return;
        }
        if let Some(addr) = hole_punch(&socket, &addrs, PUNCH_MAGIC, Duration::from_secs(8)) {
            run_punch_conn(ctx, socket, addr, node_id);
        }
    });
}

/// Confirm the path is usable in BOTH directions before we trust it. Receiving
/// the peer's `hole_punch` probe only proves peer→us; we must also learn that
/// our packets reach the peer. So ping and wait for a pong: a returned pong means
/// our ping arrived (us→peer) and its reply came back (peer→us). Returns the
/// proven peer address, or `None` if the round trip never completes (asymmetric
/// NAT) — the caller then lets the ladder fall through to WebRTC.
fn confirm_round_trip(socket: &UdpSocket, mut peer_addr: SocketAddr) -> Option<SocketAddr> {
    let _ = socket.set_read_timeout(Some(Duration::from_millis(150)));
    let mut buf = [0u8; 1500];
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut last_ping = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if last_ping.elapsed() >= Duration::from_millis(150) {
            let _ = socket.send_to(PUNCH_PING, peer_addr);
            last_ping = Instant::now();
        }
        if let Ok((n, src)) = socket.recv_from(&mut buf) {
            let p = &buf[..n];
            if p == PUNCH_PONG {
                return Some(src); // round trip proven
            }
            if p == PUNCH_PING {
                // Peer's probe — reply so its side can confirm too, and lock onto
                // the address its packets actually come from.
                peer_addr = src;
                let _ = socket.send_to(PUNCH_PONG, src);
            }
        }
    }
    None
}

/// Driver loop for an established punched link. First confirms a bidirectional
/// round trip (so we never declare a one-way path "connected"), then registers an
/// outgoing sender in `ws_conns` and ferries `WsFrame::Msg` frames over the bound
/// UDP path, replying to keepalive probes and tracking the peer's live address.
/// Exits (and cleans up) when the heartbeat evicts the peer or a newer link wins.
fn run_punch_conn<P: Platform>(
    ctx: NetCtx<P>,
    socket: UdpSocket,
    peer_addr: SocketAddr,
    node_id: String,
) {
    let Some(mut peer_addr) = confirm_round_trip(&socket, peer_addr) else {
        return; // one-way path only — let the ladder advance to WebRTC
    };

    let (tx, rx) = mpsc::channel::<String>();
    let my_conn_id = PUNCH_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    {
        let mut g = ctx.ws_conns.lock();
        if g.contains_key(&node_id) {
            return; // a link already exists (e.g. WS) — don't clobber it
        }
        g.insert(node_id.clone(), (my_conn_id, tx));
    }
    mark_live(&ctx, &node_id, TransportKind::Punch, Some(peer_addr));
    let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
    let mut buf = [0u8; 65535];
    let mut last_ka = Instant::now();
    let mut next_msg_id: u32 = 0;
    let mut pending: HashMap<u32, Reasm> = HashMap::new();
    loop {
        // Flush queued outgoing frames to the proven peer address, fragmenting
        // each into MTU-safe chunks so large payloads (e.g. clipboard) survive.
        let mut dead = false;
        while let Ok(frame) = rx.try_recv() {
            let id = next_msg_id;
            next_msg_id = next_msg_id.wrapping_add(1);
            if send_chunked(&socket, peer_addr, id, frame.as_bytes()).is_err() {
                dead = true;
                break;
            }
        }
        if dead {
            break;
        }
        // Socket-level keepalive keeps the NAT binding open between data.
        if last_ka.elapsed() >= Duration::from_secs(3) {
            let _ = socket.send_to(PUNCH_PING, peer_addr);
            last_ka = Instant::now();
        }
        if let Ok((n, src)) = socket.recv_from(&mut buf) {
            let p = &buf[..n];
            if p == PUNCH_PING {
                peer_addr = src;
                touch_live(&ctx, &node_id);
                let _ = socket.send_to(PUNCH_PONG, src);
            } else if p == PUNCH_PONG {
                peer_addr = src;
                touch_live(&ctx, &node_id);
            } else if let Some(frame) = reasm_feed(&mut pending, p) {
                // A full message reassembled — track the peer's current mapping
                // and dispatch it as the WsFrame it is.
                peer_addr = src;
                touch_live(&ctx, &node_id);
                if let Ok(WsFrame::Msg { nonce, ciphertext }) =
                    serde_json::from_slice::<WsFrame>(&frame)
                {
                    let env = Envelope { nonce, ciphertext };
                    handle_incoming(&ctx, &env, src.ip().to_string(), "UDP");
                }
            }
        }
        // Stop if a newer link replaced our ws_conns slot, or the heartbeat
        // evicted us for staleness.
        let still_ours = ctx
            .ws_conns
            .lock()
            .get(&node_id)
            .map(|(id, _)| *id == my_conn_id)
            .unwrap_or(false);
        if !still_ours || !ctx.p2p_conns.lock().contains_key(&node_id) {
            break;
        }
    }
    {
        let mut g = ctx.ws_conns.lock();
        if g.get(&node_id)
            .map(|(id, _)| *id == my_conn_id)
            .unwrap_or(false)
        {
            g.remove(&node_id);
        }
    }
    drop_live(&ctx, &node_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send `data` through `send_chunked` over real loopback sockets and
    /// collect the datagrams the receiver sees.
    fn chunk_on_the_wire(data: &[u8]) -> Vec<Vec<u8>> {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        send_chunked(&tx, rx.local_addr().unwrap(), 42, data).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        while let Ok(n) = rx.recv(&mut buf) {
            out.push(buf[..n].to_vec());
        }
        out
    }

    fn reassemble(pkts: &[Vec<u8>]) -> Option<Vec<u8>> {
        let mut pending = HashMap::new();
        pkts.iter().find_map(|p| reasm_feed(&mut pending, p))
    }

    #[test]
    fn small_message_is_one_datagram() {
        let pkts = chunk_on_the_wire(b"hi");
        assert_eq!(pkts.len(), 1);
        assert_eq!(reassemble(&pkts).unwrap(), b"hi");
    }

    #[test]
    fn empty_message_still_arrives() {
        let pkts = chunk_on_the_wire(b"");
        assert_eq!(pkts.len(), 1);
        assert_eq!(reassemble(&pkts).unwrap(), b"");
    }

    #[test]
    fn large_message_splits_into_mtu_safe_chunks_and_reassembles() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let pkts = chunk_on_the_wire(&data);
        assert_eq!(pkts.len(), 5); // ceil(5000 / 1024)
        assert!(pkts.iter().all(|p| p.len() <= CHUNK_HDR + CHUNK_PAYLOAD));
        assert_eq!(reassemble(&pkts).unwrap(), data);
    }

    #[test]
    fn out_of_order_and_duplicate_chunks_still_reassemble() {
        let data = vec![9u8; 3000];
        let mut pkts = chunk_on_the_wire(&data);
        pkts.reverse();
        pkts.insert(1, pkts[0].clone()); // duplicate
        assert_eq!(reassemble(&pkts).unwrap(), data);
    }

    #[test]
    fn sender_refuses_messages_over_the_cap() {
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        let too_big = vec![0u8; MAX_CHUNKS as usize * CHUNK_PAYLOAD + 1];
        let err = send_chunked(&tx, "127.0.0.1:9".parse().unwrap(), 1, &too_big).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_lost_chunk_loses_the_whole_message() {
        // CURRENT BEHAVIOUR: no retransmission. QUIC (Phase 2) replaces this.
        let mut pkts = chunk_on_the_wire(&[1u8; 3000]);
        pkts.remove(1);
        assert!(reassemble(&pkts).is_none());
    }

    #[test]
    fn non_chunk_and_malformed_datagrams_are_ignored() {
        let mut pending = HashMap::new();
        assert!(reasm_feed(&mut pending, b"mdpong01").is_none());
        assert!(reasm_feed(&mut pending, b"mdck").is_none()); // too short
                                                              // total = 0 and index >= total are rejected
        let mut bad = b"mdck".to_vec();
        bad.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
        assert!(reasm_feed(&mut pending, &bad).is_none());
        let mut bad = b"mdck".to_vec();
        bad.extend_from_slice(&[0, 0, 0, 1, 0, 2, 0, 2]);
        assert!(reasm_feed(&mut pending, &bad).is_none());
        assert!(pending.is_empty());
    }

    #[test]
    fn pending_reassemblies_are_capped() {
        let mut pending = HashMap::new();
        for id in 0..(REASM_MAX as u32 + 10) {
            let mut pkt = b"mdck".to_vec();
            pkt.extend_from_slice(&id.to_be_bytes());
            pkt.extend_from_slice(&2u16.to_be_bytes()); // total 2, send only #0
            pkt.extend_from_slice(&0u16.to_be_bytes());
            reasm_feed(&mut pending, &pkt);
        }
        assert!(pending.len() <= REASM_MAX);
    }

    #[test]
    fn oversized_chunk_counts_are_dropped() {
        // Regression: one 12-byte datagram claiming total = 65535 used to make
        // the reassembler preallocate 65535 slots (x256 ids ~ 16.7M slots).
        let mut pending = HashMap::new();
        let mut pkt = b"mdck".to_vec();
        pkt.extend_from_slice(&7u32.to_be_bytes());
        pkt.extend_from_slice(&u16::MAX.to_be_bytes());
        pkt.extend_from_slice(&0u16.to_be_bytes());
        assert!(reasm_feed(&mut pending, &pkt).is_none());
        assert!(pending.is_empty(), "nothing allocated");
    }

    #[test]
    fn a_one_mib_message_is_still_accepted() {
        let data = vec![3u8; MAX_CHUNKS as usize * CHUNK_PAYLOAD];
        let mut pending = HashMap::new();
        let mut done = None;
        // Feed chunk by chunk through the real encoder without a socket
        // (1024 loopback datagrams could overflow the OS receive buffer).
        for (i, chunk) in data.chunks(CHUNK_PAYLOAD).enumerate() {
            let mut pkt = b"mdck".to_vec();
            pkt.extend_from_slice(&1u32.to_be_bytes());
            pkt.extend_from_slice(&MAX_CHUNKS.to_be_bytes());
            pkt.extend_from_slice(&(i as u16).to_be_bytes());
            pkt.extend_from_slice(chunk);
            done = reasm_feed(&mut pending, &pkt).or(done);
        }
        assert_eq!(done.unwrap(), data);
    }
}
