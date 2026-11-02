//! The connect ladder, rung by rung (see learnings/01-architecture.md).
//!
//! Nodes are wired to an in-memory `SignalHub` (standing in for the registry's
//! /signal relay + FCM), and each test enables exactly one rung via
//! `CoreConfig::rungs`, so we prove *that* rung works on its own. Everything
//! else is real: sockets, threads, crypto, the hole-punch handshake, and the
//! WebRTC ICE/DTLS/SCTP stack.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, FakeStun, SignalHub, TestNode};
use mdcore::nat::NatKind;
use mdcore::{ConnectRungs, CoreConfig};

fn only(rungs: ConnectRungs) -> CoreConfig {
    CoreConfig {
        rungs,
        ..test_config()
    }
}

/// Two nodes on one hub, paired, both limited to the given rungs.
fn pair_on_hub(test: &str, rungs: ConnectRungs) -> (TestNode, TestNode, std::sync::Arc<SignalHub>) {
    let hub = SignalHub::new();
    let a = TestNode::start_on_hub(&format!("{test}-a"), &hub, only(rungs.clone()));
    let b = TestNode::start_on_hub(&format!("{test}-b"), &hub, only(rungs));
    TestNode::pair(&a, &b);
    (a, b, hub)
}

const LADDER_WAIT: Duration = Duration::from_secs(20);

#[test]
fn rung4_hole_punch_connects_and_carries_chat() {
    let (a, b, hub) = pair_on_hub(
        "rung-punch",
        ConnectRungs {
            punch: true,
            ..ConnectRungs::none()
        },
    );
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("punch"));
    assert_eq!(hub.kinds(), vec!["punch_offer", "punch_answer"]);

    assert_eq!(
        a.engine.send(&b.node_id(), "through the hole").unwrap(),
        "punch"
    );
    b.sink.wait_for_message("through the hole");
    // The punched path runs QUIC, so large messages arrive intact (the old
    // datagram path capped them at 1 MiB and dropped them on any loss).
    let big = "x".repeat(2_000_000);
    a.engine.send(&b.node_id(), &big).unwrap();
    b.sink.wait_for_message(&big);
}

#[cfg(feature = "webrtc")]
#[test]
fn rung5_webrtc_data_channel_connects_and_carries_chat() {
    let (a, b, hub) = pair_on_hub(
        "rung-rtc",
        ConnectRungs {
            webrtc: true,
            ..ConnectRungs::none()
        },
    );
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("webrtc"));
    assert_eq!(hub.kinds(), vec!["sdp_offer", "sdp_answer"]);

    assert_eq!(
        a.engine.send(&b.node_id(), "over webrtc").unwrap(),
        "webrtc"
    );
    b.sink.wait_for_message("over webrtc");
}

#[test]
fn all_rungs_disabled_reports_failed() {
    let (a, b, _hub) = pair_on_hub("rung-none", ConnectRungs::none());
    a.engine.connect(&b.node_id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while a.ladder_stages(&b).last().map(String::as_str) != Some("failed") {
        assert!(
            std::time::Instant::now() < deadline,
            "got {:?}",
            a.ladder_stages(&b)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn connect_to_unknown_peer_is_rejected_immediately() {
    let a = TestNode::start("rung-unknown-a");
    assert!(matches!(
        a.engine.connect("nobody"),
        Err(mdcore::CoreError::PeerNotFound)
    ));
}

// ---------------------------------------------------------------------------
// QUIC (Phase 2)
// ---------------------------------------------------------------------------

fn only_quic() -> ConnectRungs {
    ConnectRungs {
        quic: true,
        ..ConnectRungs::none()
    }
}

#[test]
fn rung0_quic_connects_without_any_signal() {
    let (a, b, hub) = pair_on_hub("rung-quic", only_quic());
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("quic"));
    // The acceptor registers the link too (it learns a's node id from Hello).
    assert_eq!(b.wait_connected(&a, LADDER_WAIT).as_deref(), Some("quic"));
    assert!(
        hub.log().is_empty(),
        "direct QUIC needs no server: {:?}",
        hub.log()
    );

    assert_eq!(a.engine.send(&b.node_id(), "over quic").unwrap(), "QUIC");
    assert_eq!(b.sink.wait_for_message("over quic").protocol, "QUIC");
    // Same connection, other direction.
    assert_eq!(b.engine.send(&a.node_id(), "quic reply").unwrap(), "QUIC");
    a.sink.wait_for_message("quic reply");
}

#[test]
fn quic_delivers_messages_far_larger_than_the_punch_path_allows() {
    // The punch path caps messages at 1 MiB and loses the whole message if one
    // datagram drops. QUIC streams are reliable and flow-controlled.
    let (a, b, _hub) = pair_on_hub("quic-big", only_quic());
    a.engine.connect(&b.node_id()).unwrap();
    a.wait_connected(&b, LADDER_WAIT).unwrap();

    let big: String = (0..2_000_000u32)
        .map(|i| (b'a' + (i % 26) as u8) as char)
        .collect();
    a.engine.send(&b.node_id(), &big).unwrap();
    let got = b.sink.wait_for_message(&big);
    assert_eq!(got.text.len(), 2_000_000);
}

#[test]
fn features_ride_the_quic_link() {
    let (a, b, _hub) = pair_on_hub("quic-clip", only_quic());
    a.engine.connect(&b.node_id()).unwrap();
    a.wait_connected(&b, LADDER_WAIT).unwrap();
    b.wait_connected(&a, LADDER_WAIT).unwrap();

    b.clip().user_copies("copied on b, pulled over quic");
    assert_eq!(
        a.engine.get_clipboard(&b.node_id()).unwrap(),
        "copied on b, pulled over quic"
    );
}

#[test]
fn with_every_rung_enabled_quic_is_preferred() {
    let (a, b, hub) = pair_on_hub("quic-first", ConnectRungs::default());
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("quic"));
    assert_eq!(
        a.ladder_stages(&b).first().map(String::as_str),
        Some("quic")
    );
    assert!(hub.log().is_empty());
}

/// A node whose NAT looks `kind` to it: two fake STUN servers that report
/// the same (easy) or a different (symmetric) public port.
fn node_behind(
    test: &str,
    hub: &std::sync::Arc<SignalHub>,
    kind: NatKind,
    rungs: ConnectRungs,
) -> (TestNode, Vec<FakeStun>) {
    let second = if kind == NatKind::Symmetric {
        "203.0.113.9:40001"
    } else {
        "203.0.113.9:40000"
    };
    let stuns = vec![
        FakeStun::start("203.0.113.9:40000"),
        FakeStun::start(second),
    ];
    let config = CoreConfig {
        stun_servers: stuns.iter().map(FakeStun::server).collect(),
        rungs,
        ..test_config()
    };
    let node = TestNode::start_on_hub(test, hub, config);
    node.wait_nat(kind, Duration::from_secs(10));
    (node, stuns)
}

fn wait_for_stage(a: &TestNode, b: &TestNode, stage: &str) {
    let deadline = std::time::Instant::now() + LADDER_WAIT;
    while !a.ladder_stages(b).iter().any(|s| s == stage) {
        assert!(
            std::time::Instant::now() < deadline,
            "no {stage:?} in {:?}",
            a.ladder_stages(b)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

const PUNCH_AND_RTC: ConnectRungs = ConnectRungs {
    punch: true,
    webrtc: true,
    ..ConnectRungs::NONE
};

#[test]
fn both_symmetric_nats_skip_punch_and_webrtc() {
    let hub = SignalHub::new();
    let (a, _sa) = node_behind("nat-sym-a", &hub, NatKind::Symmetric, PUNCH_AND_RTC);
    let (b, _sb) = node_behind("nat-sym-b", &hub, NatKind::Symmetric, PUNCH_AND_RTC);
    TestNode::pair(&a, &b);

    let started = std::time::Instant::now();
    a.engine.connect(&b.node_id()).unwrap();
    wait_for_stage(&a, &b, "failed");
    let stages = a.ladder_stages(&b);
    assert!(stages.contains(&"nat_skip".to_string()), "{stages:?}");
    assert!(
        !stages.iter().any(|s| s == "punching" || s == "webrtc"),
        "{stages:?}"
    );
    assert!(hub.log().is_empty(), "no signal is wasted: {:?}", hub.log());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "skipping is instant"
    );
}

#[test]
fn one_easy_nat_still_tries_the_hole_punch() {
    let hub = SignalHub::new();
    let (a, _sa) = node_behind("nat-mix-a", &hub, NatKind::Symmetric, PUNCH_AND_RTC);
    let (b, _sb) = node_behind("nat-mix-b", &hub, NatKind::Easy, PUNCH_AND_RTC);
    TestNode::pair(&a, &b);

    a.engine.connect(&b.node_id()).unwrap();
    wait_for_stage(&a, &b, "punching");
    assert!(!a.ladder_stages(&b).contains(&"nat_skip".to_string()));
}

#[test]
fn rung3_signaled_quic_swaps_addresses_and_both_sides_dial() {
    let (a, b, hub) = pair_on_hub(
        "rung-sigquic",
        ConnectRungs {
            signaled_quic: true,
            ..ConnectRungs::none()
        },
    );
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("quic"));
    assert_eq!(b.wait_connected(&a, LADDER_WAIT).as_deref(), Some("quic"));
    let kinds = hub.kinds();
    assert!(kinds.contains(&"quic_offer".to_string()), "{kinds:?}");
    assert!(kinds.contains(&"quic_answer".to_string()), "{kinds:?}");

    a.engine.send(&b.node_id(), "via signaled quic").unwrap();
    b.sink.wait_for_message("via signaled quic");
}

#[test]
fn both_sides_connecting_at_once_settle_on_one_link() {
    let (a, b, _hub) = pair_on_hub(
        "glare",
        ConnectRungs {
            signaled_quic: true,
            ..ConnectRungs::none()
        },
    );
    // Both dial each other at the same moment (QUIC glare).
    a.engine.connect(&b.node_id()).unwrap();
    b.engine.connect(&a.node_id()).unwrap();
    assert!(a.wait_connected(&b, LADDER_WAIT).is_some());
    assert!(b.wait_connected(&a, LADDER_WAIT).is_some());
    std::thread::sleep(Duration::from_millis(1500)); // let the losers close

    // Both ends kept the same connection: nobody saw the link go down...
    let downs = |n: &TestNode, peer: &TestNode| {
        let id = peer.node_id();
        n.sink
            .all()
            .iter()
            .filter(
                |e| matches!(e, mdcore::CoreEvent::PeerDisconnected { node_id } if *node_id == id),
            )
            .count()
    };
    assert_eq!((downs(&a, &b), downs(&b, &a)), (0, 0));
    // ...and it carries traffic both ways.
    a.engine.send(&b.node_id(), "a to b").unwrap();
    b.sink.wait_for_message("a to b");
    b.engine.send(&a.node_id(), "b to a").unwrap();
    a.sink.wait_for_message("b to a");
}
