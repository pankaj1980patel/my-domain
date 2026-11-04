//! The device page's Disconnect and Reconnect buttons.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::CoreEvent;

fn disconnects(node: &TestNode, peer: &TestNode) -> usize {
    let id = peer.node_id();
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::PeerDisconnected { node_id } if *node_id == id))
        .count()
}

fn wait_disconnects(node: &TestNode, peer: &TestNode, n: usize) {
    let t = std::time::Instant::now();
    while disconnects(node, peer) < n {
        assert!(t.elapsed() < WAIT, "no disconnect");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn disconnect_closes_the_link_and_a_message_brings_it_back() {
    let (a, b) = (TestNode::start("btn-a"), TestNode::start("btn-b"));
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hello").unwrap();
    b.sink.wait_for_message("hello");

    a.engine.disconnect(&b.node_id()).unwrap();
    wait_disconnects(&a, &b, 1);
    assert!(!a.engine.connected_peers().contains(&b.node_id()));

    // Nothing is blocked: the next message connects again.
    a.engine.send(&b.node_id(), "again").unwrap();
    b.sink.wait_for_message("again");

    assert!(a.engine.disconnect("nobody").is_err());
}

#[test]
fn reconnect_replaces_the_link_and_connects_when_there_is_none() {
    let (a, b) = (TestNode::start("rec-a"), TestNode::start("rec-b"));
    TestNode::pair(&a, &b);

    // No link yet: Reconnect simply connects.
    a.engine.reconnect(&b.node_id()).unwrap();
    a.wait_connected(&b, WAIT).expect("never connected");

    // A live link: closed, then a fresh one.
    let ups = |n: &TestNode| {
        let id = b.node_id();
        n.sink
            .all()
            .iter()
            .filter(|e| matches!(e, CoreEvent::PeerConnected { node_id, .. } if *node_id == id))
            .count()
    };
    let before = ups(&a);
    a.engine.reconnect(&b.node_id()).unwrap();
    wait_disconnects(&a, &b, 1);
    let t = std::time::Instant::now();
    while ups(&a) <= before {
        assert!(t.elapsed() < WAIT * 2, "never came back");
        std::thread::sleep(Duration::from_millis(50));
    }
    a.engine.send(&b.node_id(), "fresh").unwrap();
    b.sink.wait_for_message("fresh");
}
