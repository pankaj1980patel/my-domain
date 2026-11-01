//! UDP hole punching, then QUIC over the punched path.
//!
//! 1. `gather_candidates`: bind a dedicated UDP socket, learn our public
//!    address on it via STUN (the "reflexive" candidate), add our LAN address.
//! 2. The candidates are swapped through the server (`PunchOffer` /
//!    `PunchAnswer` signals).
//! 3. `punch_and_run`: both sides fire packets at each other's candidates at
//!    the same time (`nat::punch::hole_punch`), so each NAT router sees
//!    outgoing traffic and lets the peer's packets in. Then a ping/pong proves
//!    the path works in BOTH directions.
//! 4. The same socket is handed to QUIC (`QuicLink::run_over_socket`), so the
//!    punched path carries reliable, ordered, encrypted streams.
//!
//! Before Phase 2, step 4 was a hand-written loop that chunked messages into
//! 1 KiB datagrams with no retransmission (one lost datagram lost the whole
//! message, max 1 MiB). QUIC replaced it.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::engine::NetCtx;
use crate::nat::{punch::hole_punch, stun::stun_binding};
use crate::platform::Platform;
use crate::signal::Candidate;

use super::TransportKind;

const PUNCH_MAGIC: &[u8] = b"mdpunch1";
/// Round-trip probes after the punch. PING is the same bytes as the punch
/// magic so a peer still finishing `hole_punch` recognizes it; PONG proves the
/// *round trip*. QUIC only starts once a PONG came back.
const PUNCH_PING: &[u8] = PUNCH_MAGIC;
const PUNCH_PONG: &[u8] = b"mdpong01";

/// Bind a UDP socket and collect the addresses a peer could reach it on: our
/// LAN address (host candidate) and, via STUN, our public address (reflexive
/// candidate). The SAME socket must later punch, so the NAT mapping matches.
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

/// Punch toward `peer_cands`, confirm a two-way path, then run QUIC over the
/// socket. Runs on its own thread and returns at once; success shows up as a
/// `PeerConnected { transport: punch }` event.
///
/// Which side dials QUIC: the one with the lower node id. Both sides compute
/// the same answer without talking, so exactly one dials and one accepts.
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
        let Some(addr) = hole_punch(&socket, &addrs, PUNCH_MAGIC, Duration::from_secs(8)) else {
            return;
        };
        let Some(addr) = confirm_round_trip(&socket, addr) else {
            return;
        };
        let dial = ctx.identity.lock().node_id < node_id;
        let quic = ctx.quic.clone();
        let _ = quic.run_over_socket(ctx, socket, addr, dial, TransportKind::Punch);
    });
}

/// Ping/pong over the punched path until a PONG proves packets flow both ways.
/// Returns the address the peer's packets actually arrive from (a NAT may
/// rewrite the port, so it can differ from the candidate we punched toward).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_confirmed_when_both_sides_probe() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (a_addr, b_addr) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let t = std::thread::spawn(move || confirm_round_trip(&b, a_addr));
        assert_eq!(confirm_round_trip(&a, b_addr), Some(b_addr));
        assert_eq!(t.join().unwrap(), Some(a_addr));
    }

    #[test]
    fn round_trip_fails_when_the_peer_is_silent() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        assert_eq!(confirm_round_trip(&a, silent.local_addr().unwrap()), None);
    }
}
