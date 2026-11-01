//! IPv6 and dialing several addresses at once. A peer is dialed at its IPv6
//! and IPv4 addresses in parallel; whichever answers first wins. On many
//! mobile networks the IPv6 address is public (no NAT), so this alone
//! connects phone <-> home without hole punching.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use md_testkit::TestNode;

#[test]
fn a_dead_ipv4_address_is_raced_by_a_working_ipv6_one() {
    let (a, b) = (TestNode::start("v6-a"), TestNode::start("v6-b"));
    // b knows a by an IPv4 address nothing answers on (TEST-NET-1) and by
    // a's IPv6 address (loopback here). Only IPv6 can work.
    let mut a_as_seen_by_b = a.as_peer();
    a_as_seen_by_b.ip = "192.0.2.1".into();
    a_as_seen_by_b.ipv6 = Some("::1".into());
    b.engine.upsert_peer(a_as_seen_by_b);
    a.engine.upsert_peer(b.as_peer());

    let started = std::time::Instant::now();
    assert_eq!(b.engine.send(&a.node_id(), "over ipv6").unwrap(), "QUIC");
    assert_eq!(a.sink.wait_for_message("over ipv6").protocol, "QUIC");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the IPv6 path wins without waiting out the dead IPv4 one"
    );
}

#[test]
fn ipv4_peers_still_work_on_the_dual_stack_endpoint() {
    let (a, b) = (TestNode::start("v4-a"), TestNode::start("v4-b"));
    TestNode::pair(&a, &b); // 127.0.0.1 only
    assert_eq!(a.engine.send(&b.node_id(), "plain v4").unwrap(), "QUIC");
    let m = b.sink.wait_for_message("plain v4");
    // The sender shows up as a plain IPv4 address, not ::ffff:127.0.0.1.
    assert_eq!(m.ip, "127.0.0.1");
}
