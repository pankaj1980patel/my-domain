//! Minimal STUN (RFC 5389) Binding client — enough to learn our reflexive
//! (public) IPv4:port mapping for hole punching. Must be run on the SAME socket
//! that will later punch, so the mapping matches.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

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

    socket.send_to(&req, server).ok()?;

    // Other datagrams (e.g. a peer's early punch packets) can arrive on this
    // socket first, so keep reading until a response with OUR transaction id
    // shows up or the 2 s budget runs out.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buf = [0u8; 512];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        socket.set_read_timeout(Some(left)).ok()?;
        let (n, _src) = socket.recv_from(&mut buf).ok()?;
        if let Some(addr) = parse_mapped(&buf[..n], &txid) {
            return Some(addr);
        }
    }
}

/// Parse a Binding Success Response, accepting it only if it echoes `txid`
/// (RFC 5389 sec. 7.3: a response whose transaction id doesn't match an
/// outstanding request must be discarded; otherwise anyone can spoof our
/// "public address").
fn parse_mapped(msg: &[u8], txid: &[u8; 12]) -> Option<SocketAddr> {
    if msg.len() < 20 || msg[8..20] != txid[..] {
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

/// A Binding Success Response for `txid` carrying `mapped` (IPv4) as an
/// XOR-MAPPED-ADDRESS. Test-only: lets tests stand up fake STUN servers.
#[cfg(test)]
pub(crate) fn binding_response(txid: &[u8], mapped: SocketAddr) -> Vec<u8> {
    let magic = MAGIC.to_be_bytes();
    let (ip, port) = match mapped {
        SocketAddr::V4(v4) => (v4.ip().octets(), v4.port()),
        SocketAddr::V6(_) => ([0; 4], 0),
    };
    let mut m = Vec::with_capacity(32);
    m.extend_from_slice(&0x0101u16.to_be_bytes());
    m.extend_from_slice(&12u16.to_be_bytes());
    m.extend_from_slice(&magic);
    m.extend_from_slice(txid);
    m.extend_from_slice(&0x0020u16.to_be_bytes());
    m.extend_from_slice(&8u16.to_be_bytes());
    m.extend_from_slice(&[0, 0x01]);
    m.extend_from_slice(&(port ^ 0x2112).to_be_bytes());
    for (i, b) in ip.iter().enumerate() {
        m.push(b ^ magic[i]);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    const TXID: [u8; 12] = [0x5A; 12];

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
        m.extend_from_slice(&TXID);
        m.extend_from_slice(&attr_type.to_be_bytes());
        m.extend_from_slice(&8u16.to_be_bytes());
        m.extend_from_slice(&[0, 0x01]); // reserved, family IPv4
        m.extend_from_slice(&port.to_be_bytes());
        m.extend_from_slice(&ip);
        m
    }

    #[test]
    fn parses_xor_mapped_address() {
        let got = parse_mapped(&response(0x0020, 54321, [203, 0, 113, 7]), &TXID).unwrap();
        assert_eq!(got, "203.0.113.7:54321".parse().unwrap());
    }

    #[test]
    fn parses_legacy_mapped_address() {
        let got = parse_mapped(&response(0x0001, 4000, [198, 51, 100, 2]), &TXID).unwrap();
        assert_eq!(got, "198.51.100.2:4000".parse().unwrap());
    }

    #[test]
    fn rejects_non_success_and_truncated_messages() {
        let mut not_success = response(0x0020, 1, [1, 2, 3, 4]);
        not_success[1] = 0x11; // 0x0111 = Binding Error Response
        assert!(parse_mapped(&not_success, &TXID).is_none());
        assert!(parse_mapped(&[0u8; 10], &TXID).is_none());
        let full = response(0x0020, 1, [1, 2, 3, 4]);
        assert!(parse_mapped(&full[..full.len() - 3], &TXID).is_none());
    }

    #[test]
    fn response_with_a_different_transaction_id_is_rejected() {
        // Regression: the txid used to be ignored, so any datagram shaped like
        // a STUN success response was believed as our public address.
        let mut spoofed = response(0x0020, 9, [6, 6, 6, 6]);
        spoofed[8..20].copy_from_slice(&[0xAB; 12]);
        assert!(parse_mapped(&spoofed, &TXID).is_none());
    }

    #[test]
    fn binding_ignores_stray_datagrams_until_the_real_answer() {
        // A fake STUN server on loopback: first sends junk, then a spoofed
        // response, then the genuine one echoing the request's txid.
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            assert_eq!(n, 20, "binding request is a bare 20-byte header");
            let mut txid = [0u8; 12];
            txid.copy_from_slice(&buf[8..20]);
            server.send_to(b"mdpunch!", from).unwrap();
            let mut spoof = response(0x0020, 1, [6, 6, 6, 6]);
            spoof[8..20].copy_from_slice(&[0xEE; 12]);
            server.send_to(&spoof, from).unwrap();
            let mut real = response(0x0020, 40000, [203, 0, 113, 9]);
            real[8..20].copy_from_slice(&txid);
            server.send_to(&real, from).unwrap();
        });
        let got = stun_binding(&client, addr).unwrap();
        t.join().unwrap();
        assert_eq!(got, "203.0.113.9:40000".parse().unwrap());
    }
}
