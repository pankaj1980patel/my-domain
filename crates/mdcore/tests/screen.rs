//! Seeing a computer's screen on the phone: the phone watches, the computer
//! sends snapshots while it's watched, between two engines over a real link.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use base64::Engine as _;
use md_testkit::{TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::features::screen::MAX_FRAME_BYTES;
use mdcore::CoreEvent;

/// A phone and a computer that know each other, with a live link.
fn linked(test: &str) -> (TestNode, TestNode) {
    let phone = TestNode::start(&format!("{test}-phone"));
    let pc = TestNode::start(&format!("{test}-pc"));
    TestNode::pair(&phone, &pc);
    phone.engine.connect(&pc.node_id()).unwrap();
    assert!(
        phone.wait_connected(&pc, Duration::from_secs(10)).is_some(),
        "no link"
    );
    (phone, pc)
}

/// Wait until the computer hears that the phone started (`on`) or stopped
/// watching.
fn wait_watch(pc: &TestNode, phone: &TestNode, on: bool) {
    let who = phone.node_id();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenWatch { from, on: o } if *from == who && *o == on => Some(()),
            _ => None,
        })
        .expect("the watch never arrived");
}

#[test]
fn a_watched_screen_sends_its_snapshots_to_the_phone() {
    let (phone, pc) = linked("screen-watch");
    // Nobody watching: a snapshot goes nowhere.
    assert_eq!(pc.engine.screen_frame(b"\xff\xd8jpeg").unwrap(), 0);

    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    wait_watch(&pc, &phone, true);
    assert_eq!(pc.engine.screen_watchers(), [phone.node_id()]);

    let jpeg = b"\xff\xd8 not really a picture".to_vec();
    assert_eq!(pc.engine.screen_frame(&jpeg).unwrap(), 1);
    let got = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenFrame { from, jpeg, .. } if *from == pc.node_id() => {
                Some(jpeg.clone())
            }
            _ => None,
        })
        .expect("no snapshot");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(got)
            .unwrap(),
        jpeg
    );

    // Renewing isn't news; stopping is.
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    phone.engine.screen_watch(&pc.node_id(), false).unwrap();
    wait_watch(&pc, &phone, false);
    let starts = pc
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::ScreenWatch { on: true, .. }))
        .count();
    assert_eq!(starts, 1, "a renewal was reported as a new watch");
    assert!(pc.engine.screen_watchers().is_empty());
}

#[test]
fn a_computer_that_doesnt_share_its_screen_ignores_watchers() {
    let (phone, pc) = linked("screen-off");
    pc.engine.set_screen_allowed(false);
    assert!(!pc.engine.screen_allowed());
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(pc.engine.screen_watchers().is_empty());

    // Allowed again, then forbidden while watched: the watch ends.
    pc.engine.set_screen_allowed(true);
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    wait_watch(&pc, &phone, true);
    pc.engine.set_screen_allowed(false);
    assert!(pc.engine.screen_watchers().is_empty());
    assert_eq!(pc.engine.screen_frame(b"\xff\xd8").unwrap(), 0);
}

#[test]
fn snapshots_too_big_or_without_a_link_are_refused() {
    let (phone, pc) = linked("screen-limits");
    let big = vec![0u8; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        pc.engine.screen_frame(&big),
        Err(CoreError::InvalidInput(_))
    ));
    assert!(matches!(
        phone.engine.screen_watch("nobody", true),
        Err(CoreError::PeerNotFound)
    ));
}
