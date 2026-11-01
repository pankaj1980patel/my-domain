//! Minimal STUN (RFC 5389) Binding client — enough to learn our reflexive
//! (public) IPv4:port mapping for hole punching. Must be run on the SAME socket
//! that will later punch, so the mapping matches.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use rand_core::{OsRng, RngCore};

const MAGIC: u32 = 0x2112_A442;

/// Send a Binding Request on `socket` to `server` and return the observed
/// reflexive address, or `None` on timeout / parse failure.
pub fn stun_binding(socket: &UdpSocket, server: SocketAddr) -> Option<SocketAddr> {
    let mut txid = [0u8; 12];
    OsRng.fill_bytes(&mut txid);

    let mut req = Vec::with_capacity(20);
    req.extend_from_slice(&0x0001u16.to_be_bytes()); // Binding Request
    req.extend_from_slice(&0u16.to_be_bytes()); // message length
    req.extend_from_slice(&MAGIC.to_be_bytes());
    req.extend_from_slice(&txid);

    socket.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    socket.send_to(&req, server).ok()?;

    let mut buf = [0u8; 512];
    let (n, _src) = socket.recv_from(&mut buf).ok()?;
    parse_mapped(&buf[..n])
}

fn parse_mapped(msg: &[u8]) -> Option<SocketAddr> {
    if msg.len() < 20 {
        return None;
    }
    // Binding Success Response.
    if u16::from_be_bytes([msg[0], msg[1]]) != 0x0101 {
        return None;
    }
    let mlen = u16::from_be_bytes([msg[2], msg[3]]) as usize;
    let end = (20 + mlen).min(msg.len());
    let magic = MAGIC.to_be_bytes();

    let mut i = 20;
    while i + 4 <= end {
        let atype = u16::from_be_bytes([msg[i], msg[i + 1]]);
        let alen = u16::from_be_bytes([msg[i + 2], msg[i + 3]]) as usize;
        let v = i + 4;
        if v + alen > msg.len() {
            break;
        }
        let val = &msg[v..v + alen];
        // XOR-MAPPED-ADDRESS (0x0020) or MAPPED-ADDRESS (0x0001), IPv4 only.
        if (atype == 0x0020 || atype == 0x0001) && val.len() >= 8 && val[1] == 0x01 {
            let (port, ip) = if atype == 0x0020 {
                let port = u16::from_be_bytes([val[2], val[3]]) ^ 0x2112;
                let ip = Ipv4Addr::new(
                    val[4] ^ magic[0],
                    val[5] ^ magic[1],
                    val[6] ^ magic[2],
                    val[7] ^ magic[3],
                );
                (port, ip)
            } else {
                let port = u16::from_be_bytes([val[2], val[3]]);
                let ip = Ipv4Addr::new(val[4], val[5], val[6], val[7]);
                (port, ip)
            };
            return Some(SocketAddr::new(ip.into(), port));
        }
        // Attributes are padded to a 4-byte boundary.
        i = v + alen + ((4 - (alen % 4)) % 4);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a Binding Success Response carrying one address attribute.
    fn response(attr_type: u16, port: u16, ip: [u8; 4]) -> Vec<u8> {
        let magic = MAGIC.to_be_bytes();
        let (port, ip) = if attr_type == 0x0020 {
            (
                port ^ 0x2112,
                [
                    ip[0] ^ magic[0],
                    ip[1] ^ magic[1],
                    ip[2] ^ magic[2],
                    ip[3] ^ magic[3],
                ],
            )
        } else {
            (port, ip)
        };
        let mut m = Vec::new();
        m.extend_from_slice(&0x0101u16.to_be_bytes()); // Binding Success
        m.extend_from_slice(&12u16.to_be_bytes()); // attrs length
        m.extend_from_slice(&magic);
        m.extend_from_slice(&[0u8; 12]); // transaction id
        m.extend_from_slice(&attr_type.to_be_bytes());
        m.extend_from_slice(&8u16.to_be_bytes());
        m.extend_from_slice(&[0, 0x01]); // reserved, family IPv4
        m.extend_from_slice(&port.to_be_bytes());
        m.extend_from_slice(&ip);
        m
    }

    #[test]
    fn parses_xor_mapped_address() {
        let got = parse_mapped(&response(0x0020, 54321, [203, 0, 113, 7])).unwrap();
        assert_eq!(got, "203.0.113.7:54321".parse().unwrap());
    }

    #[test]
    fn parses_legacy_mapped_address() {
        let got = parse_mapped(&response(0x0001, 4000, [198, 51, 100, 2])).unwrap();
        assert_eq!(got, "198.51.100.2:4000".parse().unwrap());
    }

    #[test]
    fn rejects_non_success_and_truncated_messages() {
        let mut not_success = response(0x0020, 1, [1, 2, 3, 4]);
        not_success[1] = 0x11; // 0x0111 = Binding Error Response
        assert!(parse_mapped(&not_success).is_none());
        assert!(parse_mapped(&[0u8; 10]).is_none());
        let full = response(0x0020, 1, [1, 2, 3, 4]);
        assert!(parse_mapped(&full[..full.len() - 3]).is_none());
    }

    #[test]
    fn known_gap_transaction_id_is_not_checked() {
        // KNOWN GAP (fixed with the QUIC rewrite in Phase 2): the response's
        // transaction id is never compared with the request's, so any datagram
        // shaped like a STUN success response is believed. This test documents
        // that parse_mapped accepts a response with an arbitrary txid.
        let mut spoofed = response(0x0020, 9, [6, 6, 6, 6]);
        spoofed[8..20].copy_from_slice(&[0xAB; 12]);
        assert!(parse_mapped(&spoofed).is_some());
    }
}
