//! Changing the passphrase on all devices (`features::rekey`): the new key
//! reaches every trusted device sealed to its own device key, they switch
//! on Finish and messages keep flowing; a removed device never gets it and
//! can't talk after; a device that was offline gets it when it comes back
//! before the change expires, and after that must type the new
//! passphrase; Cancel before Finish leaves the old key; forged or tampered
//! offers and notices are refused.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::CoreConfig;

const LONG: Duration = Duration::from_secs(20);
const NEW: &str = "a brand new passphrase";

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + LONG;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// Send until it arrives (the first may race the link).
fn delivered(from: &TestNode, to: &TestNode, text: &str) {
    let deadline = Instant::now() + LONG;
    let mut n = 0;
    loop {
        n += 1;
        let t = format!("{text} {n}");
        if from.engine.send(&to.node_id(), &t).is_ok()
            && to
                .sink
                .wait_until(Duration::from_millis(800), |e| match e {
                    mdcore::CoreEvent::MessageReceived(m) if m.text == t && m.ok => Some(()),
                    _ => None,
                })
                .is_some()
        {
            return;
        }
        assert!(Instant::now() < deadline, "never delivered: {text}");
    }
}

fn status(n: &TestNode, of: &TestNode) -> String {
    status_of(n, &of.node_id())
}

fn status_of(n: &TestNode, node_id: &str) -> String {
    n.engine
        .key_rotation()
        .devices
        .iter()
        .find(|d| d.node_id == node_id)
        .map(|d| d.status.clone())
        .unwrap_or_default()
}

fn state(n: &TestNode) -> String {
    n.engine.key_rotation().state
}

/// Three devices that have all linked (so each pinned the others' keys,
/// and they agreed on signed envelopes).
fn three(prefix: &str, config: &CoreConfig) -> (TestNode, TestNode, TestNode) {
    let start = |id: &str| TestNode::start_full(MockPlatform::new(id), config.clone());
    let a = start(&format!("{prefix}-a"));
    let b = start(&format!("{prefix}-b"));
    let c = start(&format!("{prefix}-c"));
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c)] {
        TestNode::pair(x, y);
    }
    for (x, y) in [(&a, &b), (&a, &c), (&b, &c), (&b, &a), (&c, &a), (&c, &b)] {
        delivered(x, y, "hello");
    }
    (a, b, c)
}

#[test]
fn all_devices_switch_to_the_new_key_and_messages_flow() {
    let (a, b, c) = three("kr-all", &test_config());
    let r = a.engine.start_key_rotation(NEW).unwrap();
    assert_eq!(r.state, "preparing");
    assert!(r.by_me);
    assert_eq!(r.devices.len(), 2);
    wait_for("both ready", || {
        status(&a, &b) == "ready" && status(&a, &c) == "ready"
    });
    assert_eq!(state(&b), "preparing");
    assert!(!b.engine.key_rotation().by_me);
    // Still on the old key until Finish.
    delivered(&b, &c, "before finish");

    a.engine.finish_key_rotation().unwrap();
    wait_for("both switched and done", || state(&a) == "done");
    assert_eq!(status(&a, &b), "updated");
    assert_eq!(status(&a, &c), "updated");
    wait_for("b and c done", || {
        state(&b) == "done" && state(&c) == "done"
    });
    for (x, y) in [(&a, &b), (&b, &c), (&c, &a), (&b, &a)] {
        delivered(x, y, "after");
    }

    // The new passphrase typed on a fresh install gives the same key.
    let d = TestNode::start("kr-all-d");
    d.engine.set_encryption_key(NEW).unwrap();
    TestNode::pair(&a, &d);
    delivered(&d, &a, "new install");
    // One with the old passphrase is out.
    let old = TestNode::start("kr-all-old");
    TestNode::pair(&b, &old);
    let _ = old.engine.send(&b.node_id(), "old key");
    b.sink.assert_no_message("old key", Duration::from_secs(2));
}

#[test]
fn a_removed_device_never_gets_the_new_key_and_cannot_talk_after() {
    let (a, b, c) = three("kr-rev", &test_config());
    a.engine.remove_device(&c.node_id()).unwrap();
    wait_for("b took the removal", || {
        b.engine.is_device_removed(&c.node_id())
    });
    let r = a.engine.start_key_rotation(NEW).unwrap();
    assert!(
        r.devices.iter().all(|d| d.node_id != c.node_id()),
        "not a target"
    );
    wait_for("b ready", || status(&a, &b) == "ready");
    a.engine.finish_key_rotation().unwrap();
    wait_for("done", || state(&a) == "done" && state(&b) == "done");
    assert_eq!(state(&c), "none", "c never heard of it");
    delivered(&a, &b, "after");

    // c, and c installed again (a new device id and key) with the old
    // passphrase, as a removed device that kept it would: nothing gets in.
    let _ = c.engine.send(&b.node_id(), "removed one");
    b.sink
        .assert_no_message("removed one", Duration::from_secs(2));
    let c2 = TestNode::start("kr-rev-c2");
    TestNode::pair(&a, &c2);
    TestNode::pair(&b, &c2);
    let _ = c2.engine.send(&a.node_id(), "rejoined");
    let _ = c2.engine.send(&b.node_id(), "rejoined too");
    a.sink.assert_no_message("rejoined", Duration::from_secs(3));
    b.sink
        .assert_no_message("rejoined too", Duration::from_secs(1));
}

#[test]
fn cancel_before_finish_keeps_the_old_key() {
    let (a, b, c) = three("kr-cancel", &test_config());
    a.engine.start_key_rotation(NEW).unwrap();
    wait_for("ready", || {
        status(&a, &b) == "ready" && status(&a, &c) == "ready"
    });
    let r = a.engine.cancel_key_rotation().unwrap();
    assert_eq!(r.state, "cancelled");
    wait_for("b and c dropped it", || {
        state(&b) == "cancelled" && state(&c) == "cancelled"
    });
    for (x, y) in [(&a, &b), (&b, &c), (&c, &a)] {
        delivered(x, y, "still old");
    }
    // Not after Finish; and a new change can start.
    assert!(a.engine.finish_key_rotation().is_err());
    a.engine.start_key_rotation(NEW).unwrap();
    wait_for("ready again", || status(&a, &b) == "ready");
    a.engine.finish_key_rotation().unwrap();
    assert!(
        a.engine.cancel_key_rotation().is_err(),
        "switched: too late"
    );
}

/// A device that can be stopped and started again (same id and storage).
struct Restartable {
    kv: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    secrets: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    id: String,
    config: CoreConfig,
}

impl Restartable {
    fn new(id: &str, config: &CoreConfig) -> Self {
        Restartable {
            kv: Arc::default(),
            secrets: Arc::default(),
            id: id.into(),
            config: config.clone(),
        }
    }
    fn start(&self) -> TestNode {
        TestNode::start_full(
            MockPlatform {
                kv: Arc::clone(&self.kv),
                secrets: Arc::clone(&self.secrets),
                ..MockPlatform::new(&self.id)
            },
            self.config.clone(),
        )
    }
}

fn with_offline(prefix: &str, config: &CoreConfig) -> (TestNode, TestNode, Restartable) {
    let start = |id: &str| TestNode::start_full(MockPlatform::new(id), config.clone());
    let a = start(&format!("{prefix}-a"));
    let b = start(&format!("{prefix}-b"));
    let dev = Restartable::new(&format!("{prefix}-l"), config);
    let l = dev.start();
    for (x, y) in [(&a, &b), (&a, &l), (&b, &l)] {
        TestNode::pair(x, y);
    }
    for (x, y) in [(&a, &b), (&a, &l), (&b, &l), (&b, &a), (&l, &a), (&l, &b)] {
        delivered(x, y, "hello");
    }
    drop(l);
    std::thread::sleep(Duration::from_millis(500));
    (a, b, dev)
}

#[test]
fn an_offline_device_gets_the_new_key_when_it_comes_back() {
    let (a, b, dev) = with_offline("kr-late", &test_config());
    a.engine.start_key_rotation(NEW).unwrap();
    wait_for("b ready", || status(&a, &b) == "ready");
    assert_eq!(
        a.engine
            .key_rotation()
            .devices
            .iter()
            .filter(|d| d.status == "waiting")
            .count(),
        1
    );
    a.engine.finish_key_rotation().unwrap();
    wait_for("b updated", || status(&a, &b) == "updated");
    assert_eq!(state(&a), "switching", "waiting for the offline one");
    delivered(&a, &b, "new key");

    // It comes back with the old key: still understood, and it's handed
    // the new one.
    let l = dev.start();
    TestNode::pair(&a, &l);
    TestNode::pair(&b, &l);
    delivered(&l, &a, "back");
    wait_for("it switched", || status(&a, &l) == "updated");
    wait_for("done everywhere", || {
        state(&a) == "done" && state(&b) == "done" && state(&l) == "done"
    });
    for (x, y) in [(&l, &b), (&b, &l), (&a, &l)] {
        delivered(x, y, "all new");
    }
}

#[test]
fn a_device_that_missed_the_change_must_type_the_new_passphrase() {
    let config = CoreConfig {
        key_rotation_ttl: Duration::from_secs(3),
        ..test_config()
    };
    let (a, b, dev) = with_offline("kr-miss", &config);
    a.engine.start_key_rotation(NEW).unwrap();
    wait_for("b ready", || status(&a, &b) == "ready");
    a.engine.finish_key_rotation().unwrap();
    wait_for("b updated", || status(&a, &b) == "updated");
    wait_for("it expired", || state(&a) == "expired");
    assert_eq!(status_of(&a, &dev.id), "expired");
    // a told b it ended (or b saw it expire): the old key is dropped.
    wait_for("b dropped the old key", || {
        matches!(state(&b).as_str(), "done" | "expired")
    });
    // Cleared from Settings on b: still not mistaken for b missing it.
    b.engine.dismiss_key_rotation();
    assert_eq!(state(&b), "none");

    let l = dev.start();
    assert!(!l.engine.key_rotation().needs_passphrase);
    TestNode::pair(&a, &l);
    TestNode::pair(&b, &l);
    let _ = l.engine.send(&a.node_id(), "old key");
    let _ = l.engine.send(&b.node_id(), "old key to b");
    a.sink.assert_no_message("old key", Duration::from_secs(2));
    b.sink
        .assert_no_message("old key to b", Duration::from_millis(200));
    let _ = a.engine.send(&l.node_id(), "new key");
    wait_for("it knows", || l.engine.key_rotation().needs_passphrase);
    let r = l.engine.key_rotation();
    assert!(!r.mismatch_name.is_empty());
    // a and b know it's the one behind: no prompt there.
    assert!(!a.engine.key_rotation().needs_passphrase);
    assert!(!b.engine.key_rotation().needs_passphrase);

    l.engine.set_encryption_key(NEW).unwrap();
    assert!(!l.engine.key_rotation().needs_passphrase);
    delivered(&l, &a, "typed it");
    delivered(&b, &l, "welcome back");
}

#[test]
fn forged_and_tampered_rotations_are_refused() {
    let (a, b, c) = three("kr-forge", &test_config());
    a.engine.start_key_rotation(NEW).unwrap();
    wait_for("ready", || {
        status(&a, &b) == "ready" && status(&a, &c) == "ready"
    });
    let offer = a.engine.test_key_rotation_offer(&c.node_id()).unwrap();

    // b replays a's offer to c as its own sender: refused (not a's link).
    b.engine.test_send_control(&c.node_id(), &offer);
    // a's offer with its expiry pushed out, from a's own link: the
    // signature no longer checks out.
    let mut v: serde_json::Value = serde_json::from_str(&offer).unwrap();
    v["o"]["expires"] = (v["o"]["expires"].as_u64().unwrap() + 1).into();
    v["o"]["id"] = "ffeeddccbbaa99887766554433221100".into();
    a.engine.test_send_control(&c.node_id(), &v.to_string());
    // b forges a commit and a cancel "from a".
    let id = offer_id(&offer);
    for kind in ["commit", "cancel"] {
        let forged = serde_json::json!({
            "type": format!("rekey_{kind}"), "from": a.node_id(),
            "n": {"kind": kind, "id": id, "by": a.node_id(), "at": 1,
                  "switched": [b.node_id()], "sig": "AAAA"},
        });
        b.engine
            .test_send_control(&c.node_id(), &forged.to_string());
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(state(&c), "preparing", "nothing forged was taken");
    delivered(&c, &b, "still old");

    // A removed device starting its own change: nobody takes it.
    a.engine.cancel_key_rotation().unwrap();
    wait_for("cancelled", || state(&c) == "cancelled");
    a.engine.remove_device(&c.node_id()).unwrap();
    wait_for("b took the removal", || {
        b.engine.is_device_removed(&c.node_id())
    });
    c.engine
        .start_key_rotation("the removed device's key")
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(state(&a), "cancelled");
    assert_eq!(state(&b), "cancelled");
    assert!(c
        .engine
        .key_rotation()
        .devices
        .iter()
        .all(|d| d.status == "waiting"));
}

fn offer_id(offer: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(offer).unwrap();
    v["o"]["id"].as_str().unwrap().to_string()
}
