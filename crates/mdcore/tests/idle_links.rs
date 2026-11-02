//! In the background, links that carry no messages are closed so their
//! keep-alives stop waking the radio; a send re-opens one on demand.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::config::CoreConfig;
use mdcore::CoreEvent;

fn node(id: &str) -> TestNode {
    let cfg = CoreConfig {
        idle_close_after: Some(Duration::from_secs(1)),
        ..test_config()
    };
    TestNode::start_full(MockPlatform::new(id), cfg)
}

fn disconnected(n: &TestNode, peer: &str, within: Duration) -> bool {
    n.sink
        .wait_until(within, |e| match e {
            CoreEvent::PeerDisconnected { node_id } if node_id == peer => Some(()),
            _ => None,
        })
        .is_some()
}

#[test]
fn idle_links_close_only_in_the_background_and_reopen_on_send() {
    let (a, b) = (node("idle-a"), node("idle-b"));
    TestNode::pair(&a, &b);
    assert_eq!(a.engine.send(&b.node_id(), "hi").unwrap(), "QUIC");
    b.sink.wait_for_message("hi");

    // In the foreground an idle link stays up.
    assert!(!disconnected(&a, &b.node_id(), Duration::from_millis(2500)));
    assert!(a.engine.connected_peers().contains(&b.node_id()));

    // In the background it is closed, on both ends.
    a.engine.set_background(true);
    assert!(disconnected(&a, &b.node_id(), WAIT));
    assert!(disconnected(&b, &a.node_id(), WAIT));
    assert!(a.engine.connected_peers().is_empty());

    // A message still gets through: the send re-opens the link.
    assert_eq!(a.engine.send(&b.node_id(), "back").unwrap(), "QUIC");
    b.sink.wait_for_message("back");
}
