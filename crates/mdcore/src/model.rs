//! Wire protocol + shared data model. These structs are serialized to JSON and
//! must stay byte-compatible across desktop, android, and the registry server.

use serde::{Deserialize, Serialize};

/// This device's own endpoints, advertised to peers and the registry.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
    /// Public `ip:port` the home router forwards to our QUIC port (UPnP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_quic: Option<String>,
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
    /// `crypto::lan_tag` of the sender's key: beacons with another tag (or
    /// none) are ignored.
    #[serde(default)]
    pub tag: String,
}

/// Where a peer entry came from. Serializes to "registry" | "lan" | "manual"
/// to stay compatible with the existing frontend / Kotlin JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum Source {
    Registry,
    Lan,
    Manual,
}

/// A known peer device. Deserializable too: the peer list is cached on the
/// device (see engine/background.rs `spawn_peer_cache`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
    /// Public `ip:port` the peer's router forwards to its QUIC port (UPnP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_quic: Option<String>,
}

impl Peer {
    /// Where to dial this peer's QUIC endpoint, all raced in parallel: its
    /// IPv6 address (no NAT in the way, when it exists), the port its router
    /// forwards for it (UPnP), then its LAN/registry IPv4 address.
    /// Unparseable or duplicate entries are skipped.
    pub fn quic_targets(&self) -> Vec<std::net::SocketAddr> {
        use std::net::{IpAddr, SocketAddr};
        let with_port = |s: &str| {
            s.trim()
                .parse::<IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, self.quic_port))
        };
        let candidates = [
            self.ipv6.as_deref().and_then(with_port),
            self.mapped_quic
                .as_deref()
                .and_then(|s| s.trim().parse().ok()),
            with_port(&self.ip),
        ];
        let mut out: Vec<SocketAddr> = Vec::new();
        for a in candidates.into_iter().flatten() {
            if a.port() != 0 && !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }
}

/// Decrypted message body.
#[derive(Default, Serialize, Deserialize)]
pub struct Plaintext {
    pub from: String,
    pub text: String,
    /// A chat message's id. The receiver acknowledges it (`ack`) and drops
    /// repeats, so the sender can resend until it hears back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Set on an acknowledgement: the id of the message that arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack: Option<String>,
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
    /// The sender's display name (its account username).
    pub from: String,
    /// The sending device's node id, when the link knows it (it always does
    /// for QUIC, WebRTC and the relay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub ip: String,
    pub protocol: String,
    pub text: String,
    pub ts: u64,
    pub ok: bool,
    /// The message's id, when the sender gave it one (chat does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// WebSocket frames (legacy protocol v1; v2 framing lives in `wire::frame`).
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    #[serde(rename = "hello")]
    Hello {
        node_id: String,
        name: String,
        /// New every time the engine starts (QUIC only): a peer that sees
        /// a new one knows we restarted and drops our old connection.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        boot: String,
    },
    #[serde(rename = "msg")]
    Msg { nonce: String, ciphertext: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

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
            mapped_quic: None,
        }
    }

    fn addrs(v: &[&str]) -> Vec<SocketAddr> {
        v.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn quic_targets_try_ipv6_then_the_router_mapping_then_ipv4() {
        let mut p = peer("192.168.1.5", Some("2409:4040::7"));
        assert_eq!(
            p.quic_targets(),
            addrs(&["[2409:4040::7]:4", "192.168.1.5:4"])
        );
        p.mapped_quic = Some("152.67.7.50:41000".into());
        assert_eq!(
            p.quic_targets(),
            addrs(&["[2409:4040::7]:4", "152.67.7.50:41000", "192.168.1.5:4"])
        );
        assert_eq!(
            peer("192.168.1.5", None).quic_targets(),
            addrs(&["192.168.1.5:4"])
        );
    }

    #[test]
    fn quic_targets_skip_garbage_and_duplicates() {
        assert_eq!(
            peer("not-an-ip", Some(" ::1 ")).quic_targets(),
            addrs(&["[::1]:4"])
        );
        assert_eq!(peer("::1", Some("::1")).quic_targets(), addrs(&["[::1]:4"]));
        let mut p = peer("", None);
        p.mapped_quic = Some("garbage".into());
        assert!(p.quic_targets().is_empty());
        let mut no_quic = peer("10.0.0.1", None);
        no_quic.quic_port = 0;
        assert!(
            no_quic.quic_targets().is_empty(),
            "no QUIC port, nothing to dial"
        );
    }
}

/// One clipboard value in the history: what it was, which device it came
/// from (`None`: copied on this one) and when (unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub ts: u64,
}
