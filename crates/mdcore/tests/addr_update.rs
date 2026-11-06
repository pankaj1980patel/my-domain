//! Peers keep each other's addresses fresh peer-to-peer (features/addr_update):
//! no server is involved in either test.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{test_config, FakeRouter, MockPlatform, TestNode};
use mdcore::CoreConfig;

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn seen_by(n: &TestNode, peer: &TestNode) -> mdcore::model::Peer {
    n.engine
        .get_peers()
        .into_iter()
        .find(|p| p.node_id == peer.node_id())
        .unwrap()
}

#[test]
fn a_device_that_moved_tells_its_peers_its_new_address() {
    let (laptop, phone) = (
        TestNode::start("addr-move-laptop"),
        TestNode::start("addr-move-phone"),
    );
    TestNode::pair(&laptop, &phone);
    laptop.engine.send(&phone.node_id(), "hi").unwrap(); // laptop dials: link survives a move
    phone.sink.wait_for_message("hi");

    laptop.engine.network_changed(Some("127.0.0.3"));
    wait_for("the phone to learn the new address", || {
        seen_by(&phone, &laptop).ip == "127.0.0.3"
    });
}

#[test]
fn a_new_link_carries_addresses_the_server_never_had() {
    let router = Arc::new(FakeRouter::default());
    let config = CoreConfig {
        port_mapper: Some(router),
        ..test_config()
    };
    let laptop = TestNode::start_full(MockPlatform::new("addr-new-laptop"), config);
    let phone = TestNode::start("addr-new-phone");
    wait_for("the laptop's router mapping", || {
        laptop.engine.identity().mapped_quic.is_some()
    });
    TestNode::pair(&laptop, &phone); // the phone's entry has no mapping
    assert_eq!(seen_by(&phone, &laptop).mapped_quic, None);

    phone.engine.send(&laptop.node_id(), "hello").unwrap(); // link comes up
    let expected = laptop.engine.identity().mapped_quic;
    wait_for("the phone to learn the mapping", || {
        seen_by(&phone, &laptop).mapped_quic == expected
    });
}

#[test]
fn a_device_cannot_rewrite_another_devices_address() {
    let (laptop, phone, tablet) = (
        TestNode::start("addr-spoof-laptop"),
        TestNode::start("addr-spoof-phone"),
        TestNode::start("addr-spoof-tablet"),
    );
    TestNode::pair(&laptop, &phone);
    TestNode::pair(&laptop, &tablet);
    TestNode::pair(&phone, &tablet);
    let before = seen_by(&laptop, &tablet).ip;
    // The phone claims to be the tablet and says it moved.
    let spoof = format!(
        r#"{{"type":"addr_update","from":"{}","ip":"127.0.0.77","quic_port":1}}"#,
        tablet.node_id()
    );
    phone.engine.send(&laptop.node_id(), &spoof).unwrap();
    // A real message after it, so the spoof has surely been handled.
    phone.engine.send(&laptop.node_id(), "after").unwrap();
    laptop.sink.wait_for_message("after");
    std::thread::sleep(Duration::from_millis(200));
    let now = seen_by(&laptop, &tablet);
    assert_eq!(
        now.ip, before,
        "the tablet's address was rewritten by the phone"
    );
    assert_ne!(now.quic_port, 1);
}
