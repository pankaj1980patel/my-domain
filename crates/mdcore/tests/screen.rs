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
    // Seeing a screen is off until allowed for a device.
    pc.engine
        .set_peer_permission(&phone.node_id(), "screen", true)
        .unwrap();
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
    let who = phone.node_id();
    pc.engine
        .set_peer_permission(&who, "screen", false)
        .unwrap();
    assert!(!pc.engine.peer_permissions(&who).screen);
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(pc.engine.screen_watchers().is_empty());

    // Allowed again, then forbidden while watched: the watch ends.
    pc.engine.set_peer_permission(&who, "screen", true).unwrap();
    phone.engine.screen_watch(&pc.node_id(), true).unwrap();
    wait_watch(&pc, &phone, true);
    pc.engine
        .set_peer_permission(&who, "screen", false)
        .unwrap();
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

#[test]
fn a_phone_shares_its_screen_with_the_computer_it_chose() {
    let (phone, pc) = linked("screen-offer");
    let other = TestNode::start("screen-offer-other");
    TestNode::pair(&phone, &other);
    other.engine.connect(&phone.node_id()).unwrap();
    assert!(other
        .wait_connected(&phone, Duration::from_secs(10))
        .is_some());

    // The phone offers; the computer hears it (and not as chat).
    phone.engine.screen_offer(&pc.node_id(), true).unwrap();
    let who = phone.node_id();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenOffer { from, on: true } if *from == who => Some(()),
            _ => None,
        })
        .expect("no offer");
    assert!(
        !pc.sink
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::MessageReceived(_))),
        "the offer showed as chat"
    );

    // Both computers watch (the phone allows both to see its screen);
    // frames go only to the one the phone chose.
    for c in [&pc, &other] {
        phone
            .engine
            .set_peer_permission(&c.node_id(), "screen", true)
            .unwrap();
    }
    pc.engine.screen_watch(&phone.node_id(), true).unwrap();
    other.engine.screen_watch(&phone.node_id(), true).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while phone.engine.screen_watchers().len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(phone.engine.screen_watchers().len(), 2, "both watch");
    let chosen = [pc.node_id()];
    assert_eq!(
        phone.engine.screen_frame_to(b"\xff\xd8a", &chosen).unwrap(),
        1
    );
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenFrame { from, .. } if *from == who => Some(()),
            _ => None,
        })
        .expect("no frame for the chosen computer");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !other
            .sink
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::ScreenFrame { .. })),
        "a computer the phone didn't choose saw its screen"
    );
    assert_eq!(
        phone
            .engine
            .screen_frame_to(b"\xff\xd8a", &["nobody".to_string()])
            .unwrap(),
        0
    );

    // Stopping is news too.
    phone.engine.screen_offer(&pc.node_id(), false).unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ScreenOffer { from, on: false } if *from == who => Some(()),
            _ => None,
        })
        .expect("no stop");
}

#[test]
fn an_offer_needs_a_known_linked_computer() {
    let phone = TestNode::start("screen-offer-nolink-phone");
    let pc = TestNode::start("screen-offer-nolink-pc");
    TestNode::pair(&phone, &pc);
    assert!(matches!(
        phone.engine.screen_offer("nobody", true),
        Err(CoreError::PeerNotFound)
    ));
    assert!(matches!(
        phone.engine.screen_offer(&pc.node_id(), true),
        Err(CoreError::NoLink(_))
    ));
}

#[test]
fn touches_for_the_phone_arrive_and_unknown_kinds_are_dropped_one_by_one() {
    use mdcore::model::InputEvent;
    let (phone, pc) = linked("screen-touch");
    // From a newer computer: a tap, a swipe, and a kind nobody knows yet.
    // The phone plays what it knows; nothing shows as chat.
    let raw = serde_json::json!({
        "type": "input", "from": pc.node_id(),
        "events": [
            {"kind": "tap", "x": 5000, "y": 12000},
            {"kind": "hologram", "z": 1},
            {"kind": "swipe", "x": 1, "y": 2, "x2": 3, "y2": 4, "ms": 250},
        ],
    })
    .to_string();
    pc.engine.send(&phone.node_id(), &raw).unwrap();
    let got = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Input { events, .. } => Some(events.clone()),
            _ => None,
        })
        .expect("no touches");
    assert_eq!(
        got,
        [
            InputEvent::Tap { x: 5000, y: 10_000 },
            InputEvent::Swipe {
                x: 1,
                y: 2,
                x2: 3,
                y2: 4,
                ms: 250
            },
        ]
    );
    phone
        .sink
        .assert_no_message(&raw, Duration::from_millis(300));
}
