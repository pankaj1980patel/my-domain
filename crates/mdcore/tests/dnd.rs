//! Do Not Disturb follows the phone: the computers hear when it goes into
//! or out of Do Not Disturb.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::CoreEvent;

/// The `Dnd` events `node` got so far, as (from, on).
fn heard(node: &TestNode) -> Vec<(String, bool)> {
    node.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::Dnd { from, on } => Some((from, on)),
            _ => None,
        })
        .collect()
}

fn wait_dnd(node: &TestNode, want: bool) {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Dnd { on, .. } if *on == want => Some(()),
            _ => None,
        })
        .expect("no dnd event");
}

#[test]
fn the_computer_hears_when_the_phone_goes_into_and_out_of_dnd() {
    let phone = TestNode::start("dnd-phone");
    let pc = TestNode::start("dnd-pc");
    TestNode::pair(&phone, &pc);
    assert!(pc.engine.dnd_active().is_empty());

    // No link yet: it connects and delivers.
    phone.engine.set_dnd(true);
    wait_dnd(&pc, true);
    assert_eq!(pc.engine.dnd_active(), [phone.node_id()]);

    // The same state again isn't sent again.
    phone.engine.set_dnd(true);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(heard(&pc), [(phone.node_id(), true)]);

    phone.engine.set_dnd(false);
    wait_dnd(&pc, false);
    assert!(pc.engine.dnd_active().is_empty());
    // Not shown as chat.
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn a_restarted_phone_does_not_repeat_its_state() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let phone_platform = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("dnd-restart-phone")
    };
    let pc = TestNode::start("dnd-restart-pc");
    let phone = TestNode::start_with(phone_platform());
    TestNode::pair(&phone, &pc);
    phone.engine.set_dnd(true);
    wait_dnd(&pc, true);

    drop(phone);
    let phone = TestNode::start_with(phone_platform());
    TestNode::pair(&phone, &pc);
    phone.engine.set_dnd(true);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(heard(&pc).len(), 1);
    phone.engine.set_dnd(false);
    wait_dnd(&pc, false);
}

#[test]
fn a_restarted_computer_learns_the_state_when_the_link_comes_up() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let pc_platform = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("dnd-pc-restart-pc")
    };
    let phone = TestNode::start("dnd-pc-restart-phone");
    let pc = TestNode::start_with(pc_platform());
    TestNode::pair(&phone, &pc);
    phone.engine.set_dnd(true);
    wait_dnd(&pc, true);

    // The computer restarts: it forgot, and the phone's state hasn't
    // changed, so only the new link tells it.
    drop(pc);
    let pc = TestNode::start_with(pc_platform());
    assert!(pc.engine.dnd_active().is_empty());
    TestNode::pair(&phone, &pc);
    pc.engine.connect(&phone.node_id()).unwrap();
    wait_dnd(&pc, true);
    assert_eq!(pc.engine.dnd_active(), [phone.node_id()]);

    // Heard again on a later link, it isn't a new event.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(heard(&pc).len(), 1);
}
