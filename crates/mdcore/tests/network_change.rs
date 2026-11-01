//! What the engine does when the network changes (new Wi-Fi, Wi-Fi -> mobile
//! data, wake from sleep): close links that were on the old network, then
//! re-register and re-dial. See engine/netchange.rs.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{FakeRegistry, TestNode};
use mdcore::CoreEvent;

const WAIT: Duration = Duration::from_secs(15);

/// How many times `node` has reported `peer` connected / disconnected.
fn link_events(node: &TestNode, peer: &TestNode) -> (usize, usize) {
    let id = peer.node_id();
    node.sink
        .all()
        .iter()
        .fold((0, 0), |(up, down), e| match e {
            CoreEvent::PeerConnected { node_id, .. } if *node_id == id => (up + 1, down),
            CoreEvent::PeerDisconnected { node_id } if *node_id == id => (up, down + 1),
            _ => (up, down),
        })
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn moving_networks_closes_links_and_redials_connected_peers() {
    let (a, b) = (TestNode::start("move-a"), TestNode::start("move-b"));
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "before the move").unwrap(); // dials QUIC
    b.sink.wait_for_message("before the move");
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("quic"));

    // Our address changed: the old link must go, and come back by itself.
    a.engine.network_changed(Some("127.0.0.2"));
    assert_eq!(a.engine.identity().ip, "127.0.0.2");
    wait_for("the link to drop and come back", || {
        link_events(&a, &b) == (2, 1)
    });
    // The peer saw the old connection close too, then accepted the new one.
    wait_for("the peer to see it", || {
        let (up, down) = link_events(&b, &a);
        down >= 1 && up >= 2
    });

    a.engine.send(&b.node_id(), "after the move").unwrap();
    b.sink.wait_for_message("after the move");
    b.engine.send(&a.node_id(), "and back").unwrap();
    a.sink.wait_for_message("and back");
}

#[test]
fn same_address_keeps_links_and_only_refreshes() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let (a, b) = (
        TestNode::start_logged_out("same-a"),
        TestNode::start("same-b"),
    );
    a.engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    a.engine.set_encryption_key("family key").unwrap();
    b.engine.dev_login("alice", "family key").unwrap();
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hello").unwrap();
    b.sink.wait_for_message("hello");

    let registers = || {
        reg.requests()
            .iter()
            .filter(|r| r.as_str() == "POST /devices/register")
            .count()
    };
    let before = registers();
    let ip = a.engine.identity().ip;
    a.engine.network_changed(Some(&ip)); // e.g. Android's callback with the same Wi-Fi
    wait_for("a re-register", || registers() > before);
    assert!(reg.requests().iter().any(|r| r.starts_with("GET /devices")));
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(link_events(&a, &b).1, 0, "the live link was left alone");
}
