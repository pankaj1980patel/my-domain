//! Link keep-alive (CoreConfig::keepalive): pings keep a quiet link up, and
//! silence beyond the idle timeout takes it down. Short timings here; the
//! defaults (20 s / 60 s) are tuned for phones.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::{CoreConfig, CoreEvent, KeepAlive};

fn node(id: &str, keepalive: KeepAlive) -> TestNode {
    let config = CoreConfig {
        keepalive,
        ..test_config()
    };
    let n = TestNode::start_logged_out_with(MockPlatform::new(id), config);
    n.engine.dev_login("alice", "family key").unwrap();
    n
}

fn disconnected(n: &TestNode, peer: &TestNode) -> bool {
    let id = peer.node_id();
    n.sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::PeerDisconnected { node_id } if *node_id == id))
}

#[test]
fn keepalives_hold_a_quiet_link_open() {
    let ka = KeepAlive {
        interval: Some(Duration::from_millis(200)),
        idle_timeout: Duration::from_secs(1),
    };
    let (a, b) = (node("ka-on-a", ka), node("ka-on-b", ka));
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hello").unwrap();
    b.sink.wait_for_message("hello");

    std::thread::sleep(Duration::from_secs(3)); // 3x the idle timeout, no traffic
    assert!(
        !disconnected(&a, &b) && !disconnected(&b, &a),
        "keep-alives held it open"
    );
    a.engine.send(&b.node_id(), "still here").unwrap();
    b.sink.wait_for_message("still here");
}

#[test]
fn without_keepalives_a_silent_link_times_out() {
    let ka = KeepAlive {
        interval: None,
        idle_timeout: Duration::from_secs(1),
    };
    let (a, b) = (node("ka-off-a", ka), node("ka-off-b", ka));
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hello").unwrap();
    b.sink.wait_for_message("hello");

    let deadline = Instant::now() + Duration::from_secs(5);
    while !(disconnected(&a, &b) && disconnected(&b, &a)) {
        assert!(Instant::now() < deadline, "the idle timeout never fired");
        std::thread::sleep(Duration::from_millis(50));
    }
}
