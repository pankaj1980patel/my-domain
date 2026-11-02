//! Connection quality: QUIC's measured RTT and loss, rated in plain words.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use md_testkit::TestNode;
use mdcore::transport::quality::Quality;

#[test]
fn a_loopback_quic_link_is_rated_excellent() {
    let (a, b) = (TestNode::start("q-a"), TestNode::start("q-b"));
    TestNode::pair(&a, &b);
    for i in 0..20 {
        a.engine.send(&b.node_id(), &format!("ping {i}")).unwrap();
    }
    b.sink.wait_for_message("ping 19");

    let stats = a.engine.link_stats();
    let s = stats.iter().find(|s| s.node_id == b.node_id()).unwrap();
    assert!(s.direct);
    assert!(s.rtt_ms.unwrap() < 60, "loopback RTT: {:?}", s.rtt_ms);
    assert_eq!(s.loss_pct, Some(0.0));
    assert_eq!(s.quality, Quality::Excellent);
    assert_eq!(s.label, "Excellent");
}

#[test]
fn peers_without_a_link_are_not_listed() {
    let (a, b) = (TestNode::start("q-none-a"), TestNode::start("q-none-b"));
    TestNode::pair(&a, &b);
    assert!(a.engine.link_stats().is_empty());
}
