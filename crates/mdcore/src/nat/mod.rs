//! NAT traversal: STUN reflexive-address discovery + a UDP hole-punch primitive.
//!
//! The full ladder (gather candidates → `PunchOffer`/`PunchAnswer` via signaling
//! → simultaneous punch → promote to the data channel; TURN relay fallback with
//! coturn ephemeral creds) is wired during the streaming/NAT phase. These
//! primitives are the building blocks and require live multi-network testing.

pub mod portmap;
pub mod punch;
pub mod stun;

use std::net::{SocketAddr, UdpSocket};

use serde::{Deserialize, Serialize};

/// How our NAT maps a socket's outgoing traffic, as far as hole punching cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum NatKind {
    /// Not measured (no STUN configured, offline, or only one server answered).
    #[default]
    Unknown,
    /// Same public `ip:port` towards every destination ("endpoint-independent
    /// mapping"). Hole punching works.
    Easy,
    /// A different public port per destination. The address STUN reports is
    /// useless to a peer, so punching (and WebRTC's direct path) fails when
    /// BOTH sides are like this.
    Symmetric,
}

impl NatKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NatKind::Unknown => "unknown",
            NatKind::Easy => "easy",
            NatKind::Symmetric => "symmetric",
        }
    }

    /// Lenient parse of the registry's string; anything unexpected is `Unknown`.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "easy" => NatKind::Easy,
            "symmetric" => NatKind::Symmetric,
            _ => NatKind::Unknown,
        }
    }

    /// Both sides symmetric: hole punching and WebRTC's direct path can't
    /// work, so the connect ladder skips them.
    pub fn hopeless_with(self, other: NatKind) -> bool {
        self == NatKind::Symmetric && other == NatKind::Symmetric
    }
}

/// Classify our NAT: from ONE socket, ask two different STUN servers what our
/// public address is. The same answer from both means the mapping doesn't
/// depend on the destination (`Easy`); different answers mean `Symmetric`.
/// Fewer than two answers: `Unknown`. Takes up to ~2 s per silent server.
pub fn classify(servers: &[SocketAddr]) -> NatKind {
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return NatKind::Unknown;
    };
    let mapped: Vec<SocketAddr> = servers
        .iter()
        .filter_map(|s| stun::stun_binding(&socket, *s))
        .take(2)
        .collect();
    match mapped.as_slice() {
        [a, b] if a == b => NatKind::Easy,
        [_, _] => NatKind::Symmetric,
        _ => NatKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A one-socket STUN server on loopback that answers every Binding
    /// Request with `mapped` as our "public" address.
    fn fake_stun(mapped: SocketAddr) -> SocketAddr {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                if n >= 20 {
                    let _ = sock.send_to(&stun::binding_response(&buf[8..20], mapped), from);
                }
            }
        });
        addr
    }

    fn silent() -> SocketAddr {
        // Bound but never answered: the request times out.
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        std::thread::spawn(move || {
            let _keep = sock;
            std::thread::sleep(std::time::Duration::from_secs(5));
        });
        addr
    }

    #[test]
    fn same_mapping_from_two_servers_is_easy() {
        let m = "203.0.113.7:40000".parse().unwrap();
        assert_eq!(classify(&[fake_stun(m), fake_stun(m)]), NatKind::Easy);
    }

    #[test]
    fn a_different_port_per_destination_is_symmetric() {
        let a = fake_stun("203.0.113.7:40000".parse().unwrap());
        let b = fake_stun("203.0.113.7:40001".parse().unwrap());
        assert_eq!(classify(&[a, b]), NatKind::Symmetric);
    }

    #[test]
    fn one_answer_is_not_enough_to_tell() {
        let a = fake_stun("203.0.113.7:40000".parse().unwrap());
        assert_eq!(classify(&[a, silent()]), NatKind::Unknown);
        assert_eq!(classify(&[]), NatKind::Unknown);
    }

    #[test]
    fn only_two_symmetric_sides_are_hopeless() {
        use NatKind::*;
        assert!(Symmetric.hopeless_with(Symmetric));
        assert!(!Symmetric.hopeless_with(Easy));
        assert!(!Symmetric.hopeless_with(Unknown));
        assert!(!Easy.hopeless_with(Easy));
    }

    #[test]
    fn parses_leniently_and_round_trips() {
        for k in [NatKind::Unknown, NatKind::Easy, NatKind::Symmetric] {
            assert_eq!(NatKind::parse(k.as_str()), k);
        }
        assert_eq!(NatKind::parse("weird"), NatKind::Unknown);
    }
}
