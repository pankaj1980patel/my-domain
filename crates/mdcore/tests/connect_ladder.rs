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

use md_testkit::{test_config, SignalHub, TestNode};
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
fn rung1_lan_udp_ping_connects_without_any_signal() {
    let (a, b, hub) = pair_on_hub(
        "rung-lan",
        ConnectRungs {
            lan_udp: true,
            ..ConnectRungs::none()
        },
    );
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("udp"));
    assert!(
        hub.log().is_empty(),
        "same-LAN needs no server: {:?}",
        hub.log()
    );

    // The link carries chat.
    a.engine.send(&b.node_id(), "over the lan link").unwrap();
    b.sink.wait_for_message("over the lan link");
}

#[test]
fn rung3_signaled_websocket_connects_via_start_ws_and_ws_ready() {
    let (a, b, hub) = pair_on_hub(
        "rung-sigws",
        ConnectRungs {
            signaled_ws: true,
            ..ConnectRungs::none()
        },
    );
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("ws"));
    assert_eq!(hub.kinds(), vec!["start_ws", "ws_ready"]);

    assert_eq!(
        a.engine.send(&b.node_id(), "over signaled ws").unwrap(),
        "WS"
    );
    b.sink.wait_for_message("over signaled ws");
}

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
    // Large messages are chunked over the punched UDP path and reassembled.
    let big = "x".repeat(50_000);
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

#[test]
fn a_peer_without_quic_falls_through_to_the_next_rung() {
    let (a, b, _hub) = pair_on_hub(
        "quic-skip",
        ConnectRungs {
            quic: true,
            lan_udp: true,
            ..ConnectRungs::none()
        },
    );
    // As an older build would advertise it: no QUIC port.
    let mut old_b = b.as_peer();
    old_b.quic_port = 0;
    a.engine.upsert_peer(old_b);

    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("udp"));
    assert!(!a.ladder_stages(&b).contains(&"quic".to_string()));
}
