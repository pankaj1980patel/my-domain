//! A fake STUN server on loopback: answers every Binding Request with a fixed
//! "public" address. Two of them with different answers make an engine
//! classify its NAT as symmetric; the same answer makes it easy.

use std::net::{SocketAddr, UdpSocket};

const MAGIC: [u8; 4] = 0x2112_A442u32.to_be_bytes();

pub struct FakeStun {
    addr: SocketAddr,
}

impl FakeStun {
    /// Start a server that reports `mapped` (IPv4) as the caller's address.
    pub fn start(mapped: &str) -> Self {
        let mapped: SocketAddr = mapped.parse().unwrap();
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                // Binding Request: type 0x0001, then length, magic, txid.
                if n >= 20 && buf[0..2] == [0, 1] {
                    let _ = sock.send_to(&response(&buf[8..20], mapped), from);
                }
            }
        });
        FakeStun { addr }
    }

    /// `host:port` for `CoreConfig::stun_servers`.
    pub fn server(&self) -> String {
        self.addr.to_string()
    }
}

fn response(txid: &[u8], mapped: SocketAddr) -> Vec<u8> {
    let SocketAddr::V4(v4) = mapped else {
        panic!("FakeStun only maps IPv4");
    };
    let mut m = Vec::with_capacity(32);
    m.extend_from_slice(&0x0101u16.to_be_bytes()); // Binding Success
    m.extend_from_slice(&12u16.to_be_bytes());
    m.extend_from_slice(&MAGIC);
    m.extend_from_slice(txid);
    m.extend_from_slice(&0x0020u16.to_be_bytes()); // XOR-MAPPED-ADDRESS
    m.extend_from_slice(&8u16.to_be_bytes());
    m.extend_from_slice(&[0, 0x01]);
    m.extend_from_slice(&(v4.port() ^ 0x2112).to_be_bytes());
    for (i, b) in v4.ip().octets().iter().enumerate() {
        m.push(b ^ MAGIC[i]);
    }
    m
}
