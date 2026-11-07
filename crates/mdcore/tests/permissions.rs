//! Per-device permissions: controlling a computer, seeing its screen and
//! taking over its mouse and keyboard are off until allowed for that one
//! device; a refusal says why on the requester and asks the owner on the
//! target; turning one off ends what it allowed at once; the upgrade from
//! the global switches doesn't keep them on.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::model::InputEvent;
use mdcore::CoreEvent;

fn link(a: &TestNode, b: &TestNode) {
    a.engine.connect(&b.node_id()).unwrap();
    assert!(
        a.wait_connected(b, Duration::from_secs(10)).is_some(),
        "no link"
    );
}

/// Wait until `node` has said it reads refusals (its status came).
fn wait_status(a: &TestNode, b: &TestNode) {
    let deadline = Instant::now() + WAIT;
    loop {
        a.engine.request_status(&b.node_id()).ok();
        let who = b.node_id();
        if a.sink
            .wait_until(Duration::from_millis(500), |e| match e {
                CoreEvent::DeviceStatus { from, .. } if *from == who => Some(()),
                _ => None,
            })
            .is_some()
        {
            return;
        }
        assert!(Instant::now() < deadline, "no status");
    }
}

fn played(pc: &TestNode, from: &str) -> usize {
    pc.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::Input { from: f, .. } if f == from))
        .count()
}

fn wait_refused(node: &TestNode, by: &str, perm: &str) {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::PermissionRefused { from, perm: p, why }
                if from == by && p == perm && why == "not_allowed" =>
            {
                Some(())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no refusal for {perm}"));
}

fn wait_ask(node: &TestNode, from_node: &str, perm: &str) {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::PermissionAsk { from, perm: p } if from == from_node && p == perm => {
                Some(())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the owner wasn't asked for {perm}"));
}

fn asks(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::PermissionAsk { .. }))
        .count()
}

#[test]
fn nothing_is_allowed_by_default_and_a_refusal_says_why() {
    let phone = TestNode::start("perm-default-phone");
    let pc = TestNode::start("perm-default-pc");
    TestNode::pair(&phone, &pc);
    link(&phone, &pc);
    wait_status(&phone, &pc);
    wait_status(&pc, &phone);
    let p = pc.engine.peer_permissions(&phone.node_id());
    assert!(!p.control && !p.screen && !p.kvm, "{p:?}");
    assert!(pc.engine.all_permissions().is_empty());

    // Remote input: not played; the phone hears why, the pc's owner is
    // asked (once, however many clicks).
    for _ in 0..5 {
        phone.engine.input_click(&pc.node_id(), "left").unwrap();
    }
    wait_refused(&phone, &pc.node_id(), "control");
    wait_ask(&pc, &phone.node_id(), "control");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(played(&pc, &phone.node_id()), 0);
    assert_eq!(asks(&pc), 1, "asked more than once");

    // Its screen: no watch.
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    wait_refused(&phone, &pc.node_id(), "screen");
    wait_ask(&pc, &phone.node_id(), "screen");
    assert!(pc.engine.screen_watchers().is_empty());
    // Never shown as chat.
    assert!(!phone
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(m) if m.text.contains("perm_refused"))));
}

#[test]
fn allowing_one_device_allows_only_that_one() {
    let phone = TestNode::start("perm-one-phone");
    let tablet = TestNode::start("perm-one-tablet");
    let pc = TestNode::start("perm-one-pc");
    TestNode::pair(&phone, &pc);
    TestNode::pair(&tablet, &pc);
    TestNode::pair(&phone, &tablet);
    link(&phone, &pc);
    link(&tablet, &pc);

    pc.engine
        .set_peer_permission(&phone.node_id(), "control", true)
        .unwrap();
    pc.engine
        .set_peer_permission(&phone.node_id(), "screen", true)
        .unwrap();
    let all = pc.engine.all_permissions();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].node_id, phone.node_id());
    assert!(all[0].control && all[0].screen && !all[0].kvm);

    phone.engine.input_key(&pc.node_id(), "enter").unwrap();
    tablet.engine.input_key(&pc.node_id(), "enter").unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Input { from, .. } if *from == phone.node_id() => Some(()),
            _ => None,
        })
        .expect("the allowed phone's key never came");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        played(&pc, &tablet.node_id()),
        0,
        "the tablet controlled it"
    );

    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    tablet.engine.screen_watch(&pc.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while pc.engine.screen_watchers().is_empty() {
        assert!(Instant::now() < deadline, "the phone's watch never came");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pc.engine.screen_watchers(), [phone.node_id()]);
    // Not a permission: refused.
    assert!(pc
        .engine
        .set_peer_permission(&phone.node_id(), "everything", true)
        .is_err());
}

#[test]
fn turning_it_off_ends_the_watch_at_once() {
    let phone = TestNode::start("perm-revoke-phone");
    let pc = TestNode::start("perm-revoke-pc");
    TestNode::pair(&phone, &pc);
    link(&phone, &pc);
    let who = phone.node_id();
    pc.engine.set_peer_permission(&who, "screen", true).unwrap();
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenWatch { from, on: true } if *from == who => Some(()),
            _ => None,
        })
        .expect("no watch");
    pc.engine
        .set_peer_permission(&who, "screen", false)
        .unwrap();
    // Immediately: no watcher, no frames, and the host told.
    assert!(pc.engine.screen_watchers().is_empty());
    assert_eq!(pc.engine.screen_frame(b"\xff\xd8").unwrap(), 0);
    assert!(pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::ScreenWatch { from, on: false } if *from == who)));
    assert!(pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::PermissionsChanged { node_id } if *node_id == who)));
    // A renewal is refused now.
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(pc.engine.screen_watchers().is_empty());

    // Remote input: the next event after turning it off isn't played.
    pc.engine
        .set_peer_permission(&who, "control", true)
        .unwrap();
    phone.engine.input_key(&pc.node_id(), "esc").unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Input { .. } => Some(()),
            _ => None,
        })
        .expect("no key");
    pc.engine
        .set_peer_permission(&who, "control", false)
        .unwrap();
    let before = played(&pc, &who);
    phone.engine.input_key(&pc.node_id(), "esc").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(played(&pc, &who), before);
}

#[test]
fn allow_once_and_dont_allow() {
    let phone = TestNode::start("perm-once-phone");
    let pc = TestNode::start("perm-once-pc");
    TestNode::pair(&phone, &pc);
    link(&phone, &pc);
    let who = phone.node_id();

    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    wait_ask(&pc, &who, "screen");
    // "Allow once": the next watch (the phone renews) goes through, but
    // nothing is kept.
    pc.engine.permission_answer(&who, "screen", "once").unwrap();
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while pc.engine.screen_watchers().is_empty() {
        assert!(Instant::now() < deadline, "allow once didn't let it watch");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!pc.engine.peer_permissions(&who).screen);
    // The watch ends: the once is used up.
    phone.engine.screen_watch(&pc.node_id(), false).unwrap();
    let deadline = Instant::now() + WAIT;
    while !pc.engine.screen_watchers().is_empty() {
        assert!(Instant::now() < deadline, "the watch didn't end");
        std::thread::sleep(Duration::from_millis(50));
    }
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(pc.engine.screen_watchers().is_empty(), "once lasted");

    // "Don't allow": not asked again for a while.
    pc.engine
        .permission_answer(&who, "control", "deny")
        .unwrap();
    phone.engine.input_click(&pc.node_id(), "left").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::PermissionAsk { perm, .. } if perm == "control")));
    assert_eq!(played(&pc, &who), 0);

    // "Always": kept.
    pc.engine
        .permission_answer(&who, "control", "always")
        .unwrap();
    assert!(pc.engine.peer_permissions(&who).control);
    assert!(pc
        .engine
        .permission_answer(&who, "control", "maybe")
        .is_err());
    assert!(pc
        .engine
        .permission_answer("nobody", "control", "once")
        .is_err());
}

#[test]
fn taps_for_a_phone_are_not_remote_input() {
    // The phone plays taps only while it shares its screen (its host's
    // call); the core doesn't need "control" for them.
    let phone = TestNode::start("perm-tap-phone");
    let pc = TestNode::start("perm-tap-pc");
    TestNode::pair(&phone, &pc);
    link(&pc, &phone);
    pc.engine
        .send_input(
            &phone.node_id(),
            vec![
                InputEvent::Tap { x: 1, y: 2 },
                InputEvent::Text { text: "hi".into() },
            ],
        )
        .unwrap();
    let got = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Input { events, .. } => Some(events.clone()),
            _ => None,
        })
        .expect("no tap");
    assert_eq!(got, [InputEvent::Tap { x: 1, y: 2 }]);
}

/// The same device's store across a restart.
fn device(store: &Arc<Mutex<HashMap<String, String>>>, id: &str) -> MockPlatform {
    MockPlatform {
        kv: Arc::clone(store),
        ..MockPlatform::new(id)
    }
}

#[test]
fn the_upgrade_turns_the_global_switches_off_and_says_so_once() {
    // A fresh install: nothing to say.
    let fresh = TestNode::start("perm-fresh");
    assert_eq!(fresh.engine.permission_notice(), None);

    // An install from before, with a paired phone, the switches never
    // touched (they were on), screen sharing turned off by hand.
    let store: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let phone = TestNode::start("perm-upgrade-phone");
    let pc = TestNode::start_with(device(&store, "perm-upgrade-pc"));
    TestNode::pair(&pc, &phone);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !store
        .lock()
        .unwrap()
        .get("peers")
        .is_some_and(|v| v.contains(&phone.node_id()))
    {
        assert!(Instant::now() < deadline, "the peer list was never cached");
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(pc);
    {
        let mut kv = store.lock().unwrap();
        kv.remove("perms_migrated");
        kv.remove("perms_notice");
        kv.remove("remote_input");
        kv.insert("screen_share".into(), "off".into());
        kv.remove("kvm_allowed");
    }

    let pc = TestNode::start_with(device(&store, "perm-upgrade-pc"));
    let notice = pc.engine.permission_notice().expect("a notice");
    assert_eq!(notice.features, ["control", "kvm"]);
    // Nothing kept on: the phone is allowed nothing.
    let p = pc.engine.peer_permissions(&phone.node_id());
    assert!(!p.control && !p.screen && !p.kvm);
    // "Allow for <device>" from the notice.
    pc.engine
        .set_peer_permission(&phone.node_id(), "control", true)
        .unwrap();
    pc.engine.dismiss_permission_notice();
    assert_eq!(pc.engine.permission_notice(), None);
    drop(pc);

    // Started again: no notice, the allow kept.
    let pc = TestNode::start_with(device(&store, "perm-upgrade-pc"));
    assert_eq!(pc.engine.permission_notice(), None);
    assert!(pc.engine.peer_permissions(&phone.node_id()).control);
}

#[test]
fn a_removed_device_loses_what_it_was_allowed() {
    let phone = TestNode::start("perm-removed-phone");
    let pc = TestNode::start("perm-removed-pc");
    TestNode::pair(&phone, &pc);
    let who = phone.node_id();
    pc.engine
        .set_peer_permission(&who, "control", true)
        .unwrap();
    pc.engine.set_peer_permission(&who, "kvm", true).unwrap();
    pc.engine.remove_device(&who).unwrap();
    let p = pc.engine.peer_permissions(&who);
    assert!(!p.control && !p.kvm, "{p:?}");
}
