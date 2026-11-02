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

    // B upgrades too (a moment later under load); until then its receipts
    // may still go by relay.
    let a_id = a.node_id();
    b.sink
        .wait_until(WAIT, |e| match e {
            mdcore::CoreEvent::PeerConnected { node_id, transport }
                if *node_id == a_id && *transport != mdcore::transport::TransportKind::Relay =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("b never upgraded");

    // New messages go direct: the relay forwards none to b. (A late hello
    // may still be greeted back by relay; that's not a message.)
    let to_b = || {
        relay
            .forwarded()
            .iter()
            .filter(|v| v["to"].as_str() == Some(id.as_str()))
            .filter(|v| v["data"].as_str().is_some_and(|d| d.contains("\"msg\"")))
            .count()
    };
    let before = to_b();
    a.engine.send(&b.node_id(), "direct now").unwrap();
    b.sink.wait_for_message("direct now");
    assert_eq!(to_b(), before, "the relay is no longer used");
}

#[test]
fn signals_arrive_over_the_servers_websocket_without_any_push_service() {
    let relay = FakeRelay::start();
    let reg = FakeRegistry::start_with_relay(&relay);
    reg.add_user("alice", "alice-pw");
    // No SignalHub and no FCM: signals go POST /signal -> the server -> the
    // target's WebSocket. Only signaled QUIC is enabled.
    let start = |id: &str| {
        let cfg = CoreConfig {
            relay_url: Some(relay.url()),
            signal_channel: true,
            rungs: ConnectRungs {
                signaled_quic: true,
                ..ConnectRungs::none()
            },
            ..test_config()
        };
        let n = TestNode::start_logged_out_with(md_testkit::MockPlatform::new(id), cfg);
        n.engine
            .auth_login(&reg.url(), "alice", "alice-pw")
            .unwrap();
        n.engine.set_encryption_key("family key").unwrap();
        n
    };
    let (a, b) = (start("sigws-a"), start("sigws-b"));
    TestNode::pair(&a, &b);
    let deadline = std::time::Instant::now() + WAIT;
    while !(relay.connected(&a.node_id()) && relay.connected(&b.node_id())) {
        assert!(
            std::time::Instant::now() < deadline,
            "signal channels never opened"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("quic"));
    assert!(
        reg.signals_via_ws() >= 2,
        "offer and answer went over the WebSocket"
    );
    a.engine.send(&b.node_id(), "signaled over ws").unwrap();
    b.sink.wait_for_message("signaled over ws");
}

/// A relayed peer whose direct link replaced the relay and then died must be
/// routed through the relay again, not shown as live with nowhere to send.
/// (Seen on the phone: relay, then a punched link that died with the screen
/// off, then the relay again, and every send failed with "not connected".)
#[test]
fn the_relay_route_comes_back_after_a_direct_link_dies() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    let a = node("reroute-a", "alice", &reg, &hub, &relay);
    // b can upgrade to direct QUIC, and closes idle links within a second
    // when told.
    let b = {
        let cfg = CoreConfig {
            rungs: ConnectRungs {
                quic: true,
                relay: true,
                ..ConnectRungs::none()
            },
            idle_close_after: Some(Duration::from_secs(1)),
            ..relay_only(&relay)
        };
        let n = TestNode::start_on_hub("reroute-b", &hub, cfg);
        n.engine
            .auth_login(&reg.url(), "alice", "alice-pw")
            .unwrap();
        n.engine.set_encryption_key("family key").unwrap();
        n
    };
    TestNode::pair(&a, &b);

    // 1. Relay.
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("relay"));
    // 2. b upgrades to direct QUIC: it replaces the relay on a's side.
    b.engine.connect(&a.node_id()).unwrap();
    let upgraded = a.sink.wait_until(WAIT, |e| match e {
        mdcore::CoreEvent::PeerConnected { node_id, transport }
            if *node_id == b.node_id() && *transport == mdcore::transport::TransportKind::Quic =>
        {
            Some(())
        }
        _ => None,
    });
    assert!(upgraded.is_some(), "a direct link replaced the relay");
    // 3. The direct link dies (b closes it as idle).
    b.engine.set_background(true);
    let gone = a.sink.wait_until(WAIT, |e| match e {
        mdcore::CoreEvent::PeerDisconnected { node_id } if *node_id == b.node_id() => Some(()),
        _ => None,
    });
    assert!(gone.is_some(), "the direct link closed");
    b.engine.set_background(false);
    // 4. Reconnected (the relay, or b may upgrade to QUIC again first), and
    // a's sends go through. Before the fix a stayed "live" on a relay route
    // with no queue, and every send failed.
    a.engine.connect(&b.node_id()).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        match a.engine.send(&b.node_id(), "via the relay again") {
            Ok(_) => break,
            Err(e) => {
                assert!(std::time::Instant::now() < deadline, "never sendable: {e}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    b.sink.wait_for_message("via the relay again");
}
