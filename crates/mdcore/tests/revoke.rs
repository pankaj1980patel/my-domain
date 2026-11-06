//! Removed devices (`features::revoke`): a device the user removes, which
//! may still have the passphrase, can't link again on the LAN, through the
//! relay or over Bluetooth; its mDNS announcements and its entry in the
//! server's list are ignored; the signed revocation reaches the user's
//! other devices; "Allow again" takes it back.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use md_testkit::{test_config, FakeRegistry, FakeRelay, MockPlatform, SignalHub, TestNode, WAIT};
use mdcore::engine::{BleSender, Engine};
use mdcore::{ConnectRungs, CoreConfig, CoreEvent};

const LONG: Duration = Duration::from_secs(15);

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + LONG;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn lists(n: &TestNode, other: &TestNode) -> bool {
    n.engine
        .get_peers()
        .iter()
        .any(|p| p.node_id == other.node_id())
}

fn live(n: &TestNode, other: &TestNode) -> bool {
    n.engine.connected_peers().contains(&other.node_id())
}

/// Send until it goes over QUIC (the first may race the link).
fn linked(from: &TestNode, to: &TestNode, text: &str) {
    let deadline = Instant::now() + LONG;
    let mut n = 0;
    loop {
        n += 1;
        let t = format!("{text} {n}");
        if from.engine.send(&to.node_id(), &t).ok().as_deref() == Some("QUIC") {
            to.sink.wait_for_message(&t);
            return;
        }
        assert!(Instant::now() < deadline, "never linked: {text}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_removed_device_cannot_link_again_on_the_lan_and_the_others_hear_of_it() {
    let a = TestNode::start("rv-lan-a");
    let b = TestNode::start("rv-lan-b");
    let c = TestNode::start("rv-lan-c");
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c)] {
        TestNode::pair(x, y);
    }
    linked(&a, &b, "a to b");
    linked(&a, &c, "a to c");
    assert!(a.engine.removed_devices().is_empty());

    a.engine.remove_device(&c.node_id()).unwrap();
    assert!(a.engine.is_device_removed(&c.node_id()));
    assert!(!lists(&a, &c), "dropped from the list");
    wait_for("its link closed", || !live(&a, &c));
    let removed = a.engine.removed_devices();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].node_id, c.node_id());
    assert!(removed[0].by_me);
    assert!(!removed[0].fingerprint.is_empty(), "its key is named");
    assert!(a.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::RevocationsChanged { node_ids } if node_ids.contains(&c.node_id())
    )));

    // The third device hears of it (signed by a, checked against a's key).
    wait_for("b took the revocation", || {
        b.engine.is_device_removed(&c.node_id())
    });
    assert!(!lists(&b, &c));
    let on_b = b.engine.removed_devices();
    assert_eq!(on_b.len(), 1);
    assert!(!on_b[0].by_me);

    // It still has the passphrase and both devices listed: refused.
    let _ = c.engine.send(&a.node_id(), "let me back in");
    let _ = c.engine.send(&b.node_id(), "me too");
    a.sink
        .assert_no_message("let me back in", Duration::from_secs(3));
    b.sink.assert_no_message("me too", Duration::from_secs(1));
    assert!(!live(&a, &c) && !live(&b, &c));
    // Added back by hand (as a pairing would): not listed.
    TestNode::pair(&a, &c);
    assert!(!lists(&a, &c));

    // "Allow again" on b: signed, it reaches a too, and c links again.
    b.engine.allow_device_again(&c.node_id()).unwrap();
    assert!(!b.engine.is_device_removed(&c.node_id()));
    assert!(b.engine.removed_devices().is_empty());
    wait_for("a took it back", || {
        !a.engine.is_device_removed(&c.node_id())
    });
    TestNode::pair(&a, &c);
    TestNode::pair(&b, &c);
    assert!(lists(&a, &c));
    linked(&c, &a, "allowed again");
}

#[test]
fn a_removed_device_cannot_remove_the_others() {
    let a = TestNode::start("rv-own-a");
    let b = TestNode::start("rv-own-b");
    let c = TestNode::start("rv-own-c");
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c)] {
        TestNode::pair(x, y);
    }
    linked(&a, &b, "a to b");
    linked(&c, &b, "c to b");
    // a removes c; b hears.
    linked(&a, &c, "a to c");
    a.engine.remove_device(&c.node_id()).unwrap();
    wait_for("b took it", || b.engine.is_device_removed(&c.node_id()));
    // c (removed, but it still holds the account key) tries to remove a.
    // It's refused on every link now, so nothing it sends is used: a stays.
    c.engine.remove_device(&a.node_id()).unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(!b.engine.is_device_removed(&a.node_id()));
    assert!(!a.engine.is_device_removed(&a.node_id()));
    // A device can't remove itself, nor allow a device that isn't removed.
    assert!(a.engine.remove_device(&a.node_id()).is_err());
    assert!(a.engine.allow_device_again(&b.node_id()).is_err());
}

#[test]
fn a_removed_device_is_kept_out_across_a_restart() {
    let (kv, secrets) = (Arc::default(), Arc::default());
    let device = || MockPlatform {
        kv: Arc::clone(&kv),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("rv-rs-a")
    };
    let a = TestNode::start_with(device());
    let c = TestNode::start("rv-rs-c");
    TestNode::pair(&a, &c);
    linked(&c, &a, "before");
    a.engine.remove_device(&c.node_id()).unwrap();
    drop(a);
    std::thread::sleep(Duration::from_millis(300));
    let a = TestNode::start_with(device());
    assert!(a.engine.is_device_removed(&c.node_id()));
    TestNode::pair(&a, &c);
    assert!(!lists(&a, &c));
    let _ = c.engine.send(&a.node_id(), "after the restart");
    a.sink
        .assert_no_message("after the restart", Duration::from_secs(3));
}

// --- relay and the server's list -----------------------------------------------

fn relay_node(id: &str, reg: &FakeRegistry, hub: &Arc<SignalHub>, relay: &FakeRelay) -> TestNode {
    let config = CoreConfig {
        relay_url: Some(relay.url()),
        rungs: ConnectRungs {
            relay: true,
            ..ConnectRungs::none()
        },
        auto_quic: false,
        ..test_config()
    };
    let n = TestNode::start_on_hub(id, hub, config);
    n.engine
        .auth_login(&reg.url(), "alice", "alice-pw")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

#[test]
fn a_removed_device_gets_no_relayed_link_and_the_servers_list_is_ignored() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    let a = relay_node("rv-rl-a", &reg, &hub, &relay);
    let c = relay_node("rv-rl-c", &reg, &hub, &relay);
    // Each registers, then learns the other.
    a.engine.refresh_from_server().unwrap();
    c.engine.refresh_from_server().unwrap();
    a.engine.refresh_from_server().unwrap();
    assert!(lists(&a, &c), "the server lists it");
    c.engine.connect(&a.node_id()).unwrap();
    assert_eq!(c.wait_connected(&a, LONG).as_deref(), Some("relay"));
    c.engine.send(&a.node_id(), "relayed before").unwrap();
    a.sink.wait_for_message("relayed before");

    a.engine.remove_device(&c.node_id()).unwrap();
    wait_for("its relayed link closed", || !live(&a, &c));
    // The server still lists it: not added back.
    a.engine.refresh_from_server().unwrap();
    assert!(!lists(&a, &c));
    // It still reaches the relay: nothing it sends is used, no link.
    let _ = c.engine.connect(&a.node_id());
    let _ = c.engine.send(&a.node_id(), "relayed after");
    a.sink
        .assert_no_message("relayed after", Duration::from_secs(3));
    assert!(!live(&a, &c));

    // Allowed again: the server's list brings it back.
    a.engine.allow_device_again(&c.node_id()).unwrap();
    a.engine.refresh_from_server().unwrap();
    assert!(lists(&a, &c));
}

// --- Bluetooth -------------------------------------------------------------------

#[derive(Default)]
struct Pipe(
    Mutex<Option<(Weak<Engine<MockPlatform>>, u64)>>,
    Mutex<Vec<Vec<u8>>>,
);

impl Pipe {
    fn connect(&self, engine: &Arc<Engine<MockPlatform>>, id: u64) {
        *self.0.lock().unwrap() = Some((Arc::downgrade(engine), id));
        for f in std::mem::take(&mut *self.1.lock().unwrap()) {
            engine.ble_frame(id, &f);
        }
    }
}

impl BleSender for Pipe {
    fn send(&self, frame: Vec<u8>) {
        let to = self.0.lock().unwrap().clone();
        match to.and_then(|(e, id)| e.upgrade().map(|e| (e, id))) {
            Some((engine, id)) => engine.ble_frame(id, &frame),
            None => self.1.lock().unwrap().push(frame),
        }
    }
}

fn radio(a: &TestNode, b: &TestNode) {
    let (to_b, to_a) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    let id_a = a.engine.ble_opened(to_b.clone());
    let id_b = b.engine.ble_opened(to_a.clone());
    to_b.connect(&b.engine, id_b);
    to_a.connect(&a.engine, id_a);
}

fn offline() -> CoreConfig {
    CoreConfig {
        auto_quic: false,
        rungs: ConnectRungs::none(),
        ..test_config()
    }
}

#[test]
fn a_removed_device_gets_no_bluetooth_link() {
    let a = TestNode::start_full(MockPlatform::new("rv-ble-a"), offline());
    let c = TestNode::start_full(MockPlatform::new("rv-ble-c"), offline());
    TestNode::pair(&a, &c);
    a.engine.remove_device(&c.node_id()).unwrap();
    radio(&a, &c);
    assert_eq!(c.wait_connected(&a, WAIT).as_deref(), Some("ble"));
    let _ = c.engine.send(&a.node_id(), "over bluetooth");
    a.sink
        .assert_no_message("over bluetooth", Duration::from_secs(3));
    assert!(!live(&a, &c), "no link for it");
    assert!(a.engine.envelope_info(&c.node_id()).revoked >= 1);
}

// --- mDNS ------------------------------------------------------------------------

fn with_mdns(id: &str) -> TestNode {
    let config = CoreConfig {
        mdns: true,
        mdns_loopback: true,
        ..test_config()
    };
    let n = TestNode::start_logged_out_with(MockPlatform::new(id), config);
    n.engine.dev_login("alice", "family key").unwrap();
    n
}

#[test]
fn a_removed_devices_mdns_announcement_is_ignored() {
    let run = std::process::id();
    let a = with_mdns(&format!("rv-mdns-a-{run}"));
    let c = with_mdns(&format!("rv-mdns-c-{run}"));
    wait_for("c found over mDNS", || lists(&a, &c));
    a.engine.remove_device(&c.node_id()).unwrap();
    assert!(!lists(&a, &c));
    // c announces itself again (as after a restart or a network change).
    c.engine.network_changed(None);
    a.engine.scan_lan();
    std::thread::sleep(Duration::from_secs(4));
    assert!(!lists(&a, &c), "not added back from its announcement");
}
