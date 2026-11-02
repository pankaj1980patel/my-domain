//! Connect-ladder rung 6: relay through the server. Engines log in to a
//! `FakeRegistry` (for session tokens), signal through a `SignalHub`, and
//! relay through a `FakeRelay`, all real network code on loopback.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use md_testkit::{test_config, FakeRegistry, FakeRelay, SignalHub, TestNode};
use mdcore::{ConnectRungs, CoreConfig};

const WAIT: Duration = Duration::from_secs(15);

fn relay_only(relay: &FakeRelay) -> CoreConfig {
    CoreConfig {
        relay_url: Some(relay.url()),
        rungs: ConnectRungs {
            relay: true,
            ..ConnectRungs::none()
        },
        auto_quic: false, // keep sends on the relay link
        ..test_config()
    }
}

/// A node on `hub`, logged in to `reg` as `user` (password = user + "-pw").
fn node(
    id: &str,
    user: &str,
    reg: &FakeRegistry,
    hub: &Arc<SignalHub>,
    relay: &FakeRelay,
) -> TestNode {
    let n = TestNode::start_on_hub(id, hub, relay_only(relay));
    n.engine
        .auth_login(&reg.url(), user, &format!("{user}-pw"))
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

#[test]
fn relay_connects_both_sides_and_never_sees_plaintext() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    let a = node("relay-a", "alice", &reg, &hub, &relay);
    let b = node("relay-b", "alice", &reg, &hub, &relay);
    TestNode::pair(&a, &b);

    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("relay"));
    assert_eq!(b.wait_connected(&a, WAIT).as_deref(), Some("relay"));
    assert!(hub.kinds().contains(&"relay_offer".to_string()));

    assert_eq!(
        a.engine
            .send(&b.node_id(), "top secret over relay")
            .unwrap(),
        "relay"
    );
    assert_eq!(
        b.sink.wait_for_message("top secret over relay").protocol,
        "RELAY"
    );
    assert_eq!(
        b.engine.send(&a.node_id(), "relayed reply").unwrap(),
        "relay"
    );
    a.sink.wait_for_message("relayed reply");

    let big = "r".repeat(2_000_000);
    a.engine.send(&b.node_id(), &big).unwrap();
    b.sink.wait_for_message(&big);

    // Everything the relay forwarded is opaque to it: no plaintext anywhere.
    for frame in relay.forwarded() {
        let s = frame.to_string();
        assert!(!s.contains("top secret"), "plaintext leaked to relay: {s}");
        assert!(!s.contains("relayed reply"));
    }
}

#[test]
fn relay_rung_is_skipped_without_a_server_session() {
    let (relay, hub) = (FakeRelay::start(), SignalHub::new());
    // Offline (dev) login only: no session token for the relay.
    let a = TestNode::start_on_hub("relay-nosession-a", &hub, relay_only(&relay));
    let b = TestNode::start_on_hub("relay-nosession-b", &hub, relay_only(&relay));
    TestNode::pair(&a, &b);

    a.engine.connect(&b.node_id()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while a.ladder_stages(&b).last().map(String::as_str) != Some("failed") {
        assert!(
            std::time::Instant::now() < deadline,
            "{:?}",
            a.ladder_stages(&b)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!a.ladder_stages(&b).contains(&"relay".to_string()));
}

#[test]
fn different_accounts_cannot_reach_each_other_through_the_relay() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    reg.add_user("bob", "bob-pw");
    let alice = node("relay-iso-alice", "alice", &reg, &hub, &relay);
    let bob = node("relay-iso-bob", "bob", &reg, &hub, &relay);
    TestNode::pair(&alice, &bob); // even if bob somehow knows alice's node id

    bob.engine.connect(&alice.node_id()).unwrap();
    assert_eq!(bob.wait_connected(&alice, Duration::from_secs(10)), None);
}

#[test]
fn relay_comes_up_first_then_a_direct_path_takes_over() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    // Relay + hole punch: the quick direct rungs are off, so the relay comes
    // up first and the punch upgrades it.
    let cfg = || CoreConfig {
        relay_url: Some(relay.url()),
        rungs: ConnectRungs {
            relay: true,
            punch: true,
            ..ConnectRungs::none()
        },
        auto_quic: false,
        ..test_config()
    };
    let start = |id: &str| {
        let n = TestNode::start_on_hub(id, &hub, cfg());
        n.engine
            .auth_login(&reg.url(), "alice", "alice-pw")
            .unwrap();
        n.engine.set_encryption_key("family key").unwrap();
        n
    };
    let (a, b) = (start("upgrade-a"), start("upgrade-b"));
    TestNode::pair(&a, &b);

    a.engine.connect(&b.node_id()).unwrap();
    // First the relay...
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("relay"));
    // ...then the direct path replaces it (a second PeerConnected).
    let id = b.node_id();
    let upgraded = a.sink.wait_until(WAIT, |e| match e {
        mdcore::CoreEvent::PeerConnected { node_id, transport }
            if *node_id == id && *transport != mdcore::transport::TransportKind::Relay =>
        {
            Some(*transport)
        }
        _ => None,
    });
    assert_eq!(upgraded, Some(mdcore::transport::TransportKind::Punch));
    let stages = a.ladder_stages(&b);
    let pos = |s: &str| stages.iter().position(|x| x == s).unwrap();
    assert!(pos("relay") < pos("punching"), "{stages:?}");

    // New traffic goes direct: the relay forwards nothing more.
    let before = relay.forwarded().len();
    a.engine.send(&b.node_id(), "direct now").unwrap();
    b.sink.wait_for_message("direct now");
    assert_eq!(
        relay.forwarded().len(),
        before,
        "the relay is no longer used"
    );
}
