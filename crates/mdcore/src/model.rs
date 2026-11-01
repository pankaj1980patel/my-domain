//! Wire protocol + shared data model. These structs are serialized to JSON and
//! must stay byte-compatible across desktop, android, and the registry server.

use serde::{Deserialize, Serialize};

/// This device's own endpoints, advertised to peers and the registry.
#[derive(Clone, Debug, Serialize)]
pub struct Identity {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port. 0 = no QUIC (older builds).
    #[serde(default)]
    pub quic_port: u16,
    /// Global IPv6 address, if this device has one. Often directly reachable
    /// (many mobile networks give phones public IPv6 with no NAT).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<String>,
}

/// LAN discovery beacon (UDP datagram body).
#[derive(Serialize, Deserialize)]
pub struct Beacon {
    pub node_id: String,
    pub name: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port; 0 when the peer doesn\'t speak QUIC yet.
    #[serde(default)]
    pub quic_port: u16,
    #[serde(default)]
    pub reply: bool,
}

/// Where a peer entry came from. Serializes to "registry" | "lan" | "manual"
/// to stay compatible with the existing frontend / Kotlin JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Registry,
    Lan,
    Manual,
}

/// A known peer device.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port; 0 when the peer doesn\'t speak QUIC yet.
    #[serde(default)]
    pub quic_port: u16,
    pub source: Source,
    /// Peer-reported reachability (from the registry; `false`/`None` for
    /// LAN/manual peers we discovered ourselves). Drives the connect ladder.
    #[serde(default)]
    pub ws_open: bool,
    #[serde(default)]
    pub inbound_blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_udp_port: Option<u16>,
    /// The peer's NAT type from the registry (`Unknown` for LAN/manual peers).
    #[serde(default)]
    pub nat: crate::nat::NatKind,
    /// The peer's global IPv6 address (from the registry), dialed in parallel
    /// with `ip`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<String>,
}

impl Peer {
    /// Addresses to dial for this peer's QUIC port, IPv6 first (no NAT in
    /// the way, when it exists). Unparseable or duplicate entries are skipped.
    pub fn quic_targets(&self) -> Vec<std::net::IpAddr> {
        let mut out = Vec::new();
        for s in self.ipv6.iter().chain(std::iter::once(&self.ip)) {
            if let Ok(ip) = s.trim().parse::<std::net::IpAddr>() {
                if !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
        out
    }
}

/// Decrypted message body.
#[derive(Serialize, Deserialize)]
pub struct Plaintext {
    pub from: String,
    pub text: String,
}

/// Encrypted on-the-wire envelope (TCP/UDP body, and WS `msg` frames).
#[derive(Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub nonce: String,
    pub ciphertext: String,
}

/// A message surfaced to the UI.
#[derive(Clone, Debug, Serialize)]
pub struct IncomingMessage {
    pub from: String,
    pub ip: String,
    pub protocol: String,
    pub text: String,
    pub ts: u64,
    pub ok: bool,
}

/// WebSocket frames (legacy protocol v1; v2 framing lives in `wire::frame`).
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    #[serde(rename = "hello")]
    Hello { node_id: String, name: String },
    #[serde(rename = "msg")]
    Msg { nonce: String, ciphertext: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn peer(ip: &str, ipv6: Option<&str>) -> Peer {
        Peer {
            node_id: "n".into(),
            name: "n".into(),
            ip: ip.into(),
            tcp_port: 1,
            udp_port: 2,
            ws_port: 3,
            quic_port: 4,
            source: Source::Registry,
            ws_open: false,
            inbound_blocked: false,
            reflexive_ip: None,
            reflexive_udp_port: None,
            nat: Default::default(),
            ipv6: ipv6.map(String::from),
        }
    }

    fn ips(v: &[&str]) -> Vec<IpAddr> {
        v.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn quic_targets_try_ipv6_first_then_ipv4() {
        assert_eq!(
            peer("192.168.1.5", Some("2409:4040::7")).quic_targets(),
            ips(&["2409:4040::7", "192.168.1.5"])
        );
        assert_eq!(
            peer("192.168.1.5", None).quic_targets(),
            ips(&["192.168.1.5"])
        );
    }

    #[test]
    fn quic_targets_skip_garbage_and_duplicates() {
        assert_eq!(
            peer("not-an-ip", Some(" ::1 ")).quic_targets(),
            ips(&["::1"])
        );
        assert_eq!(peer("::1", Some("::1")).quic_targets(), ips(&["::1"]));
        assert!(peer("", None).quic_targets().is_empty());
    }
}
