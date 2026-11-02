//! UPnP port mapping: the engine asks the home router to forward its QUIC
//! port, publishes the public address, and peers dial it. A `FakeRouter`
//! stands in for the router (tests never touch a real one).

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{test_config, FakeRegistry, FakeRouter, MockPlatform, TestNode};
use mdcore::CoreConfig;

fn behind_router(id: &str, router: &Arc<FakeRouter>) -> TestNode {
    let config = CoreConfig {
        port_mapper: Some(router.clone()),
        ..test_config()
    };
    TestNode::start_logged_out_with(MockPlatform::new(id), config)
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_router_mapping_is_published_and_peers_dial_it() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let router = Arc::new(FakeRouter::default());
    let laptop = behind_router("map-laptop", &router);
    laptop
        .engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    laptop.engine.set_encryption_key("family key").unwrap();

    let expected = format!("127.0.0.1:{}", laptop.engine.identity().quic_port);
    wait_for("the mapping", || {
        laptop.engine.identity().mapped_quic.as_deref() == Some(expected.as_str())
    });
    assert_eq!(router.log(), vec![format!("map {expected}")]);
    // Published with the next registration (the apps refresh right after login).
    laptop.engine.refresh_from_server().unwrap();
    wait_for("the registry to have it", || {
        reg.device("alice", &laptop.node_id())
            .is_some_and(|d| d["mapped_quic"] == expected.as_str())
    });

    // A phone that can only reach the laptop through its router.
    let phone = TestNode::start("map-phone");
    phone.engine.dev_login("alice", "family key").unwrap();
    let mut laptop_seen = laptop.as_peer();
    laptop_seen.ip = "192.0.2.1".into(); // its LAN address: unreachable from outside
    laptop_seen.mapped_quic = laptop.engine.identity().mapped_quic;
    phone.engine.upsert_peer(laptop_seen);
    laptop.engine.upsert_peer(phone.as_peer());

    assert_eq!(
        phone
            .engine
            .send(&laptop.node_id(), "through the router")
            .unwrap(),
        "QUIC"
    );
    laptop.sink.wait_for_message("through the router");
}

#[test]
fn the_mapping_is_removed_when_the_engine_stops() {
    let router = Arc::new(FakeRouter::default());
    let node = behind_router("map-stop", &router);
    let port = node.engine.identity().quic_port;
    wait_for("the mapping", || {
        node.engine.identity().mapped_quic.is_some()
    });
    drop(node);
    wait_for("the unmap", || {
        router.log().contains(&format!("unmap {port}"))
    });
}

/// Asks the REAL router on this network for a PCP / NAT-PMP mapping, prints
/// what it got, and removes it. Ignored by default (touches real hardware):
///   cargo test -p mdcore --test port_mapping -- --ignored --nocapture
#[test]
#[ignore = "talks to the real home router"]
fn real_router_pcp_or_natpmp() {
    use mdcore::nat::portmap::{PcpMapper, PortMapper, UpnpMapper};
    let ip: std::net::Ipv4Addr = mdcore::platform::detect_local_ip().parse().unwrap();
    let sock = std::net::UdpSocket::bind((ip, 0)).unwrap();
    let local = std::net::SocketAddrV4::new(ip, sock.local_addr().unwrap().port());
    for (name, m) in [
        ("UPnP", Box::new(UpnpMapper) as Box<dyn PortMapper>),
        ("PCP/NAT-PMP", Box::new(PcpMapper::default())),
    ] {
        let got = m.map_udp(local, 120);
        println!("{name}: {local} -> {got:?}");
        if let Some(p) = got {
            m.unmap_udp(p.port());
        }
    }
}
