//! Dropping an Engine stops it: its QUIC port is released, so the same
//! device can start again in the same process on the same port.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::TestNode;

#[test]
fn a_dropped_engine_releases_its_quic_port() {
    let node = TestNode::start("shutdown-a");
    let port = node.engine.identity().quic_port;
    drop(node);

    let deadline = Instant::now() + Duration::from_secs(3);
    while std::net::UdpSocket::bind(("::", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "port {port} still bound 3 s after the engine was dropped"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Restart in the same process: same store, same QUIC port, and a peer that
/// only knows the old address reaches it with no refresh.
#[test]
fn a_restarted_engine_comes_back_on_its_port_and_is_reachable() {
    use md_testkit::MockPlatform;
    use std::sync::Arc;

    let store = Arc::default();
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        ..MockPlatform::new("restart-mac")
    };
    let phone = TestNode::start("restart-phone");

    let mac = TestNode::start_with(device());
    let port = mac.engine.identity().quic_port;
    TestNode::pair(&phone, &mac);
    drop(mac);

    // The old socket closes within a moment of the drop.
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::net::UdpSocket::bind(("::", port)).is_err() {
        assert!(Instant::now() < deadline, "port {port} never released");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mac = TestNode::start_with(device());
    assert_eq!(mac.engine.identity().quic_port, port);
    mac.engine.upsert_peer(phone.as_peer());
    phone
        .engine
        .send(&mac.node_id(), "found you after your restart")
        .unwrap();
    mac.sink.wait_for_message("found you after your restart");
}
