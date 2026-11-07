//! The phone as a touchpad and keyboard for a computer: pointer moves,
//! clicks, scrolls, text and keys, between two engines over a real link.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::model::{InputEvent, InputKey, MouseButton, SwipeDir};
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
    // Remote input is off until allowed for a device.
    pc.engine
        .set_peer_permission(&phone.node_id(), "control", true)
        .unwrap();
    (phone, pc)
}

/// Every input event the computer was handed so far, in order, and how
/// many messages they came in.
fn played(pc: &TestNode) -> (Vec<InputEvent>, usize) {
    let mut events = Vec::new();
    let mut messages = 0;
    for e in pc.sink.all() {
        if let CoreEvent::Input { events: got, .. } = e {
            events.extend(got);
            messages += 1;
        }
    }
    (events, messages)
}

/// Wait until the computer has been handed an event matching `ok`.
fn wait_for(pc: &TestNode, ok: impl Fn(&InputEvent) -> bool) {
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Input { events, .. } => events.iter().any(&ok).then_some(()),
            _ => None,
        })
        .expect("the event never arrived");
}

#[test]
fn touchpad_moves_are_batched_and_keep_their_order() {
    let (phone, pc) = linked("input-order");
    let node = pc.node_id();
    // 200 small moves, as a finger drag makes them.
    for _ in 0..200 {
        phone.engine.input_move(&node, 2, -1).unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
    phone.engine.input_scroll(&node, 0, 90).unwrap();
    phone.engine.input_click(&node, "left").unwrap();
    phone.engine.input_text(&node, "héllo\n").unwrap();
    phone.engine.input_key(&node, "enter").unwrap();
    wait_for(&pc, |e| {
        *e == InputEvent::Key {
            key: InputKey::Enter,
        }
    });

    let (events, messages) = played(&pc);
    let (mut dx, mut dy) = (0, 0);
    for e in &events {
        if let InputEvent::Move { dx: x, dy: y } = e {
            dx += x;
            dy += y;
        }
    }
    assert_eq!((dx, dy), (400, -200), "every move arrived, added up");
    assert!(messages < 100, "moves went in {messages} messages");
    // The moves came first, then the scroll, click, text and key, in order.
    let tail: Vec<&InputEvent> = events
        .iter()
        .filter(|e| !matches!(e, InputEvent::Move { .. }))
        .collect();
    assert_eq!(
        tail,
        [
            &InputEvent::Scroll { dx: 0, dy: 90 },
            &InputEvent::Click {
                button: MouseButton::Left
            },
            &InputEvent::Text {
                text: "héllo".into()
            },
            &InputEvent::Key {
                key: InputKey::Enter
            },
        ]
    );
    let last_move = events
        .iter()
        .rposition(|e| matches!(e, InputEvent::Move { .. }))
        .unwrap();
    assert!(last_move < events.len() - 4, "a move came after the click");
    // Never shown as chat.
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn a_computer_with_remote_input_off_ignores_it() {
    let (phone, pc) = linked("input-off");
    assert!(pc.engine.peer_permissions(&phone.node_id()).control);
    pc.engine
        .set_peer_permission(&phone.node_id(), "control", false)
        .unwrap();
    assert!(!pc.engine.peer_permissions(&phone.node_id()).control);
    phone.engine.input_click(&pc.node_id(), "right").unwrap();
    phone.engine.input_key(&pc.node_id(), "esc").unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(played(&pc).1, 0, "played back while turned off");

    pc.engine
        .set_peer_permission(&phone.node_id(), "control", true)
        .unwrap();
    phone.engine.input_key(&pc.node_id(), "backspace").unwrap();
    wait_for(&pc, |e| {
        *e == InputEvent::Key {
            key: InputKey::Backspace,
        }
    });
    // Only what was sent after it was turned back on.
    assert_eq!(played(&pc).0.len(), 1);

    // A four-finger swipe and a middle click arrive as they were made.
    phone
        .engine
        .input_gesture(&pc.node_id(), 4, "left")
        .unwrap();
    phone.engine.input_click(&pc.node_id(), "middle").unwrap();
    wait_for(&pc, |e| {
        *e == InputEvent::Click {
            button: MouseButton::Middle,
        }
    });
    assert!(played(&pc).0.contains(&InputEvent::Gesture {
        fingers: 4,
        dir: SwipeDir::Left
    }));
}

#[test]
fn input_is_never_queued_for_later() {
    let phone = TestNode::start("input-nolink-phone");
    let pc = TestNode::start("input-nolink-pc");
    TestNode::pair(&phone, &pc);
    pc.engine
        .set_peer_permission(&phone.node_id(), "control", true)
        .unwrap();
    // No link yet: refused, and the phone starts connecting.
    let err = phone.engine.input_click(&pc.node_id(), "left").unwrap_err();
    assert!(matches!(err, CoreError::NoLink(_)), "{err:?}");
    assert!(
        phone.wait_connected(&pc, Duration::from_secs(10)).is_some(),
        "it didn't start connecting"
    );
    // The link is up: the refused click doesn't turn up late.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(played(&pc).1, 0);
    phone.engine.input_click(&pc.node_id(), "left").unwrap();
    wait_for(&pc, |e| matches!(e, InputEvent::Click { .. }));
}

#[test]
fn unknown_buttons_keys_and_devices_are_refused() {
    let (phone, pc) = linked("input-bad");
    let node = pc.node_id();
    assert!(phone.engine.input_click(&node, "back").is_err());
    assert!(phone.engine.input_gesture(&node, 2, "up").is_err());
    assert!(phone.engine.input_gesture(&node, 3, "sideways").is_err());
    assert!(phone.engine.input_key(&node, "f13").is_err());
    assert!(phone.engine.input_text(&node, "\n\u{7}").is_err());
    assert!(matches!(
        phone.engine.input_click("nobody", "left"),
        Err(CoreError::PeerNotFound)
    ));
}
