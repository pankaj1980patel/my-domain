//! LAN discovery: UDP multicast + broadcast + /24 unicast sweep beacons. The
//! interface strategy (`IfaceMode`) is supplied by the platform so the desktop
//! "all interfaces" and android "single Wi-Fi interface" behaviors share code.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};

use socket2::{Domain, Protocol, Socket, Type};

use crate::events::{CoreEvent, EventSink};
use crate::model::{Beacon, Identity, Peer, Source};

pub const MCAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 42, 98);
pub const DISCOVERY_PORT: u16 = 45678;

pub type PeerMap = Arc<Mutex<HashMap<String, Peer>>>;

/// Bind the always-listening discovery socket and join the multicast group per
/// the platform's interface strategy.
pub fn bind_multicast(mode: &crate::platform::IfaceMode) -> std::io::Result<UdpSocket> {
    use crate::platform::IfaceMode;
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), DISCOVERY_PORT).into())?;
    match mode {
        IfaceMode::All { v4_addrs } => {
            for v4 in v4_addrs {
                let _ = socket.join_multicast_v4(&MCAST_GROUP, v4);
            }
            let _ = socket.join_multicast_v4(&MCAST_GROUP, &Ipv4Addr::UNSPECIFIED);
        }
        IfaceMode::Single(iface) => {
            socket.join_multicast_v4(&MCAST_GROUP, iface)?;
            socket.set_multicast_if_v4(iface)?;
        }
    }
    socket.set_multicast_loop_v4(true)?;
    Ok(socket.into())
}

/// Targets for a LAN announce: multicast + global broadcast + per-interface /24
/// (subnet broadcast + unicast sweep of every host).
pub fn lan_targets(mode: &crate::platform::IfaceMode) -> Vec<SocketAddr> {
    use crate::platform::IfaceMode;
    let mut v = vec![
        SocketAddr::new(MCAST_GROUP.into(), DISCOVERY_PORT),
        SocketAddr::new(Ipv4Addr::BROADCAST.into(), DISCOVERY_PORT),
    ];
    let addrs: Vec<Ipv4Addr> = match mode {
        IfaceMode::All { v4_addrs } => v4_addrs.clone(),
        IfaceMode::Single(ip) => vec![*ip],
    };
    for ip in addrs {
        if ip.is_loopback() || ip.is_unspecified() {
            continue;
        }
        let o = ip.octets();
        v.push(SocketAddr::new(
            Ipv4Addr::new(o[0], o[1], o[2], 255).into(),
            DISCOVERY_PORT,
        ));
        for host in 1..=254u8 {
            if host == o[3] {
                continue;
            }
            v.push(SocketAddr::new(
                Ipv4Addr::new(o[0], o[1], o[2], host).into(),
                DISCOVERY_PORT,
            ));
        }
    }
    v
}

pub fn send_beacon(socket: &UdpSocket, id: &Identity, reply: bool, to: &[SocketAddr]) {
    let beacon = Beacon {
        node_id: id.node_id.clone(),
        name: id.name.clone(),
        tcp_port: id.tcp_port,
        udp_port: id.udp_port,
        ws_port: id.ws_port,
        reply,
    };
    if let Ok(payload) = serde_json::to_vec(&beacon) {
        let _ = socket.set_broadcast(true);
        for dst in to {
            // Errors for empty hosts in the sweep (EHOSTDOWN/UNREACH/ENETUNREACH)
            // are expected and ignored.
            let _ = socket.send_to(&payload, dst);
        }
    }
}

/// Always-listening receiver. On an announce, record the peer (emitting a
/// `PeersUpdated`) and reply once (unicast) so a single scan discovers both
/// directions.
/// Fold one LAN beacon (received from `src`) into the peer map. Returns `true`
/// if the map changed, so callers only notify the UI on real changes (beacons
/// repeat every couple of seconds).
///
/// Trust rule: a beacon is unauthenticated, so it may create or refresh a
/// LAN-learned peer, but it may NOT re-point a peer we learned from the
/// (authenticated) registry or that the user added by hand to a different IP.
/// Otherwise any host on the network could claim a known device's node_id and
/// redirect its traffic. (Per-device keys in Phase 4 replace this heuristic.)
pub fn merge_beacon(peers: &mut HashMap<String, Peer>, beacon: &Beacon, src: IpAddr) -> bool {
    let ip = src.to_string();
    if let Some(existing) = peers.get_mut(&beacon.node_id) {
        if existing.source != Source::Lan && existing.ip != ip {
            return false;
        }
        let before = existing.clone();
        existing.name = beacon.name.clone();
        existing.ip = ip;
        existing.tcp_port = beacon.tcp_port;
        existing.udp_port = beacon.udp_port;
        existing.ws_port = beacon.ws_port;
        return *existing != before;
    }
    peers.insert(
        beacon.node_id.clone(),
        Peer {
            node_id: beacon.node_id.clone(),
            name: beacon.name.clone(),
            ip,
            tcp_port: beacon.tcp_port,
            udp_port: beacon.udp_port,
            ws_port: beacon.ws_port,
            source: Source::Lan,
            ws_open: false,
            inbound_blocked: false,
            reflexive_ip: None,
            reflexive_udp_port: None,
        },
    );
    true
}

pub fn discovery_recv_loop(
    recv: UdpSocket,
    send: Arc<UdpSocket>,
    peers: PeerMap,
    identity: Arc<Mutex<Identity>>,
    sink: Arc<dyn EventSink>,
) {
    let mut buf = [0u8; 2048];
    loop {
        let (len, src) = match recv.recv_from(&mut buf) {
            Ok(v) => v,
            // Back off on a persistent socket error instead of spinning a core.
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
        };
        let beacon: Beacon = match serde_json::from_slice(&buf[..len]) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if beacon.node_id == identity.lock().unwrap().node_id {
            continue;
        }
        let snapshot = {
            let mut map = peers.lock().unwrap();
            merge_beacon(&mut map, &beacon, src.ip()).then(|| map.values().cloned().collect())
        };
        if let Some(list) = snapshot {
            sink.emit(CoreEvent::PeersUpdated { peers: list });
        }
        // Reply to announces (not to replies) so the scanner is discovered too.
        if !beacon.reply {
            let id = identity.lock().unwrap().clone();
            send_beacon(
                &send,
                &id,
                true,
                &[SocketAddr::new(src.ip(), DISCOVERY_PORT)],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::IfaceMode;

    #[test]
    fn lan_targets_cover_multicast_broadcast_and_the_whole_slash_24() {
        let me = Ipv4Addr::new(192, 168, 1, 20);
        let t = lan_targets(&IfaceMode::Single(me));
        let has =
            |ip: [u8; 4]| t.contains(&SocketAddr::new(Ipv4Addr::from(ip).into(), DISCOVERY_PORT));
        assert!(has([239, 255, 42, 98]), "multicast group");
        assert!(has([255, 255, 255, 255]), "global broadcast");
        assert!(has([192, 168, 1, 255]), "subnet broadcast");
        assert!(
            has([192, 168, 1, 1]) && has([192, 168, 1, 254]),
            "unicast sweep"
        );
        assert!(!has([192, 168, 1, 20]), "never ourselves");
        // 2 global + subnet broadcast + 253 other hosts
        assert_eq!(t.len(), 2 + 1 + 253);
    }

    fn beacon(id: &str, tcp: u16) -> Beacon {
        Beacon {
            node_id: id.into(),
            name: format!("{id}-name"),
            tcp_port: tcp,
            udp_port: tcp + 1,
            ws_port: tcp + 2,
            reply: false,
        }
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn new_beacon_adds_a_lan_peer() {
        let mut peers = HashMap::new();
        assert!(merge_beacon(
            &mut peers,
            &beacon("phone", 5000),
            ip("192.168.1.7")
        ));
        let p = &peers["phone"];
        assert_eq!(
            (p.ip.as_str(), p.tcp_port, p.source),
            ("192.168.1.7", 5000, Source::Lan)
        );
    }

    #[test]
    fn repeated_identical_beacon_reports_no_change() {
        let mut peers = HashMap::new();
        merge_beacon(&mut peers, &beacon("phone", 5000), ip("192.168.1.7"));
        assert!(!merge_beacon(
            &mut peers,
            &beacon("phone", 5000),
            ip("192.168.1.7")
        ));
    }

    #[test]
    fn lan_peer_follows_its_new_address_and_ports() {
        let mut peers = HashMap::new();
        merge_beacon(&mut peers, &beacon("phone", 5000), ip("192.168.1.7"));
        assert!(merge_beacon(
            &mut peers,
            &beacon("phone", 6000),
            ip("192.168.1.9")
        ));
        assert_eq!(
            (peers["phone"].ip.as_str(), peers["phone"].tcp_port),
            ("192.168.1.9", 6000)
        );
    }

    #[test]
    fn beacon_cannot_repoint_a_registry_peer_to_another_ip() {
        let mut peers = HashMap::new();
        merge_beacon(&mut peers, &beacon("phone", 5000), ip("192.168.1.7"));
        peers.get_mut("phone").unwrap().source = Source::Registry;

        // An attacker on 192.168.1.66 claims to be "phone".
        assert!(!merge_beacon(
            &mut peers,
            &beacon("phone", 1234),
            ip("192.168.1.66")
        ));
        assert_eq!(
            (peers["phone"].ip.as_str(), peers["phone"].tcp_port),
            ("192.168.1.7", 5000)
        );
        assert_eq!(peers["phone"].source, Source::Registry);
    }

    #[test]
    fn registry_peer_may_refresh_ports_from_its_own_ip() {
        let mut peers = HashMap::new();
        merge_beacon(&mut peers, &beacon("phone", 5000), ip("192.168.1.7"));
        peers.get_mut("phone").unwrap().source = Source::Registry;
        assert!(merge_beacon(
            &mut peers,
            &beacon("phone", 7000),
            ip("192.168.1.7")
        ));
        assert_eq!(peers["phone"].tcp_port, 7000);
        assert_eq!(peers["phone"].source, Source::Registry, "source is kept");
    }

    #[test]
    fn loopback_interfaces_get_no_sweep() {
        let t = lan_targets(&IfaceMode::Single(Ipv4Addr::LOCALHOST));
        assert_eq!(t.len(), 2, "only multicast + broadcast");
    }
}
