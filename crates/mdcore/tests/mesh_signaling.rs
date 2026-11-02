//! Signals without the server: through a device both peers are linked to.
//! No registry, no signal hub, no FCM: the phone and the desktop have never
//! been linked; both are linked to the laptop.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::{ConnectRungs, CoreConfig};

#[test]
fn a_mutual_neighbour_carries_the_signals_and_the_two_connect_directly() {
    let only_signaled_quic = || CoreConfig {
        rungs: ConnectRungs {
            signaled_quic: true,
            ..ConnectRungs::none()
        },
        auto_quic: true,
        ..test_config()
    };
    let node = |id: &str| {
        let n = TestNode::start_logged_out_with(MockPlatform::new(id), only_signaled_quic());
        n.engine.dev_login("alice", "family key").unwrap();
        n
    };
    let (phone, laptop, desktop) = (
        node("mesh-phone"),
        node("mesh-laptop"),
        node("mesh-desktop"),
    );

    // phone <-> laptop and laptop <-> desktop are linked (QUIC).
    TestNode::pair(&phone, &laptop);
    TestNode::pair(&laptop, &desktop);
    phone.engine.send(&laptop.node_id(), "p-l").unwrap();
    laptop.sink.wait_for_message("p-l");
    laptop.engine.send(&desktop.node_id(), "l-d").unwrap();
    desktop.sink.wait_for_message("l-d");

    // The phone knows the desktop exists (e.g. from its peer cache) but has
    // no link, and its known address for it is dead, so rung 0 can't work.
    let mut desk = desktop.as_peer();
    desk.ip = "192.0.2.1".into();
    phone.engine.upsert_peer(desk);
    desktop.engine.upsert_peer(phone.as_peer());

    phone.engine.connect(&desktop.node_id()).unwrap();
    assert_eq!(
        phone
            .wait_connected(&desktop, Duration::from_secs(15))
            .as_deref(),
        Some("quic"),
        "{:?}",
        phone.ladder_stages(&desktop)
    );
    phone
        .engine
        .send(&desktop.node_id(), "direct, no server")
        .unwrap();
    desktop.sink.wait_for_message("direct, no server");
}

/// phone - a - b - desktop, plus phone - c - a (a loop). The only route to
/// the desktop crosses two relays (a, then b), and the loop must not make
/// anyone forward the signal twice.
#[test]
fn signals_cross_several_hops_and_loops_dont_echo() {
    let only_signaled_quic = || CoreConfig {
        rungs: ConnectRungs {
            signaled_quic: true,
            ..ConnectRungs::none()
        },
        auto_quic: true,
        ..test_config()
    };
    let node = |id: &str| {
        let n = TestNode::start_logged_out_with(MockPlatform::new(id), only_signaled_quic());
        n.engine.dev_login("alice", "family key").unwrap();
        n
    };
    let (phone, a, b, c, desktop) = (
        node("hop-phone"),
        node("hop-a"),
        node("hop-b"),
        node("hop-c"),
        node("hop-desktop"),
    );
    let link = |x: &TestNode, y: &TestNode, tag: &str| {
        TestNode::pair(x, y);
        x.engine.send(&y.node_id(), tag).unwrap();
        y.sink.wait_for_message(tag);
    };
    link(&phone, &a, "phone-a");
    link(&a, &b, "a-b");
    link(&b, &desktop, "b-desktop");
    link(&phone, &c, "phone-c");
    link(&c, &a, "c-a");
    // The phone knows the desktop only at a dead address.
    let mut desk = desktop.as_peer();
    desk.ip = "192.0.2.1".into();
    phone.engine.upsert_peer(desk);
    desktop.engine.upsert_peer(phone.as_peer());

    phone.engine.connect(&desktop.node_id()).unwrap();
    assert_eq!(
        phone
            .wait_connected(&desktop, Duration::from_secs(15))
            .as_deref(),
        Some("quic"),
        "{:?}",
        phone.ladder_stages(&desktop)
    );
    phone
        .engine
        .send(&desktop.node_id(), "four hops away")
        .unwrap();
    desktop.sink.wait_for_message("four hops away");
}
