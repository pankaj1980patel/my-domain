//! Waking a sleeping computer (Wake-on-LAN): what a computer's status says
//! is remembered, and the `wake_relay` message only goes to devices that
//! said they understand it and are on the computer's network.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::{CoreConfig, CoreEvent};

/// The test devices all talk over loopback, so they are "on" 127.0.0.0/24.
const ON_OUR_NETWORK: &str = "127.0.0.255";

fn wol_status(broadcast: &str) -> String {
    format!(
        r#"{{"wol":[{{"mac":"AA:BB:CC:00:11:22","broadcast":"{broadcast}","iface":"en0","wired":true}}]}}"#
    )
}

/// `asker` asks `node` for its status and waits for the `n`th answer from
/// it; returns it.
fn status_of(asker: &TestNode, node: &TestNode, n: usize) -> serde_json::Value {
    let id = node.node_id();
    asker.engine.request_status(&id).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let got: Vec<serde_json::Value> = asker
            .sink
            .all()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::DeviceStatus { from, status } if from == id => Some(status),
                _ => None,
            })
            .collect();
        if let Some(s) = got.get(n - 1) {
            return s.clone();
        }
        assert!(std::time::Instant::now() < deadline, "no status");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Whether `node` showed a `wake_relay` as chat (an app from before would).
fn shown_as_chat(node: &TestNode, within: Duration) -> bool {
    node.sink
        .wait_until(within, |e| match e {
            CoreEvent::MessageReceived(m) if m.text.contains("wake_relay") => Some(()),
            _ => None,
        })
        .is_some()
}

fn old_app(id: &str) -> TestNode {
    TestNode::start_full(
        MockPlatform::new(id),
        CoreConfig {
            wake_relay: false,
            ..test_config()
        },
    )
}

#[test]
fn only_devices_that_said_so_and_share_its_network_are_asked_to_relay() {
    let phone = TestNode::start("wol-phone");
    let pc = TestNode::start("wol-pc");
    let relay = TestNode::start("wol-relay");
    let old = old_app("wol-old");
    for other in [&pc, &relay, &old] {
        TestNode::pair(&phone, other);
    }
    pc.engine.set_device_status(&wol_status(ON_OUR_NETWORK));
    status_of(&phone, &pc, 1);
    assert!(phone.engine.can_wake(&pc.node_id()));
    // The new app says it relays; the old one says nothing.
    assert_eq!(status_of(&phone, &relay, 1)["wol"], 1);
    assert!(status_of(&phone, &old, 1).get("wol").is_none());

    let report = phone.engine.wake_device(&pc.node_id()).unwrap();
    assert_eq!(report.relays, vec![relay.node_id()]);
    // Understood (never chat) where it went; never sent to the old app.
    assert!(!shown_as_chat(&relay, Duration::from_millis(500)));
    assert!(!shown_as_chat(&old, Duration::from_millis(100)));
    assert!(!shown_as_chat(&pc, Duration::from_millis(100)));
}

#[test]
fn a_device_on_another_network_is_not_asked() {
    let phone = TestNode::start("wol-far-phone");
    let pc = TestNode::start("wol-far-pc");
    let relay = TestNode::start("wol-far-relay");
    TestNode::pair(&phone, &pc);
    TestNode::pair(&phone, &relay);
    pc.engine.set_device_status(&wol_status("192.168.77.255"));
    status_of(&phone, &pc, 1);
    assert_eq!(status_of(&phone, &relay, 1)["wol"], 1);
    // Sent from here only (a network without broadcasts can't even do that).
    match phone.engine.wake_device(&pc.node_id()) {
        Ok(r) => assert!(r.relays.is_empty(), "{r:?}"),
        Err(mdcore::CoreError::NoLink(_)) => {}
        Err(e) => panic!("wake failed: {e}"),
    }
    assert!(!shown_as_chat(&relay, Duration::from_millis(300)));
}

#[test]
fn switching_it_off_forgets_and_an_app_from_before_is_still_understood() {
    let phone = TestNode::start("wol-off-phone");
    let pc = TestNode::start("wol-off-pc");
    TestNode::pair(&phone, &pc);
    pc.engine.set_device_status(&wol_status(ON_OUR_NETWORK));
    status_of(&phone, &pc, 1);
    assert!(phone.engine.can_wake(&pc.node_id()));
    // On, but its cards couldn't be read just now: what's known stays.
    pc.engine.set_device_status(r#"{"wol":[]}"#);
    status_of(&phone, &pc, 2);
    assert!(phone.engine.can_wake(&pc.node_id()));
    // Switched off: forgotten.
    pc.engine.set_device_status("{}");
    assert_eq!(status_of(&phone, &pc, 3)["wol"], 1);
    assert!(!phone.engine.can_wake(&pc.node_id()));

    // An app from before said one address, and nothing about relaying.
    let old = old_app("wol-off-old");
    TestNode::pair(&phone, &old);
    old.engine
        .set_device_status(r#"{"wake":{"mac":"AA:BB:CC:00:11:33"}}"#);
    status_of(&phone, &old, 1);
    assert!(phone.engine.can_wake(&old.node_id()));
}
