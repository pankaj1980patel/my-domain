//! Share mouse and keyboard between computers (features::kvm): the layout
//! agrees on both ends, control goes over and comes back with the
//! clipboard, input arrives in order, a computer that doesn't allow it
//! refuses, a lost link always lets go, and an app from before never gets a
//! kvm message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::{KvmEvent, KvmNeighbor, KvmState};
use mdcore::{CoreConfig, CoreEvent};

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Wait until `a` knows `b` shares mouse and keyboard.
fn wait_supported(a: &TestNode, b: &TestNode) {
    let deadline = Instant::now() + WAIT;
    while !a.engine.kvm_supported(&b.node_id()) {
        assert!(Instant::now() < deadline, "never heard it shares input");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_state(node: &TestNode, what: &str, ok: impl Fn(&KvmState) -> bool) -> KvmState {
    wait_state_for(node, what, WAIT, ok)
}

fn wait_state_for(
    node: &TestNode,
    what: &str,
    within: Duration,
    ok: impl Fn(&KvmState) -> bool,
) -> KvmState {
    node.sink
        .wait_until(within, |e| match e {
            CoreEvent::KvmChanged { state } if ok(state) => Some(state.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no kvm state: {what}"))
}

fn inputs(node: &TestNode) -> Vec<KvmEvent> {
    node.sink
        .all()
        .iter()
        .filter_map(|e| match e {
            CoreEvent::KvmInput { events, .. } => Some(events.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn no_kvm_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("kvm")
    )));
}

/// Two computers, linked, each knowing the other shares input, `pc` to the
/// right of `mac`.
fn pair_side_by_side(a: &str, b: &str) -> (TestNode, TestNode) {
    let mac = TestNode::start(a);
    let pc = TestNode::start(b);
    TestNode::pair(&mac, &pc);
    link(&mac, &pc, "hi");
    wait_supported(&mac, &pc);
    wait_supported(&pc, &mac);
    mac.engine
        .kvm_set_neighbor(&pc.node_id(), Some("right"))
        .unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::KvmLayout { layout } if !layout.is_empty() => Some(()),
            _ => None,
        })
        .expect("the layout reached the pc");
    (mac, pc)
}

#[test]
fn control_goes_over_and_comes_back_with_the_clipboard() {
    let (mac, pc) = pair_side_by_side("kvm-mac", "kvm-pc");
    // Both ends agree: the pc sits right of the mac, the mac left of the pc.
    assert_eq!(
        mac.engine.kvm_layout(),
        [KvmNeighbor {
            node_id: pc.node_id(),
            side: "right".into()
        }]
    );
    assert_eq!(
        pc.engine.kvm_layout(),
        [KvmNeighbor {
            node_id: mac.node_id(),
            side: "left".into()
        }]
    );
    assert_eq!(mac.engine.kvm_neighbor_at("right"), Some(pc.node_id()));
    // Only the computer at that side can be controlled from that edge.
    assert!(mac.engine.kvm_start(&pc.node_id(), "left", 10).is_err());

    mac.clip().user_copies("copied on the mac");
    mac.engine.kvm_start(&pc.node_id(), "right", 2_500).unwrap();
    let st = wait_state(&mac, "controlling", |s| s.role == "controlling");
    assert_eq!(st.node_id, Some(pc.node_id()));
    let st = wait_state(&pc, "controlled", |s| s.role == "controlled");
    assert_eq!(st.node_id, Some(mac.node_id()));
    assert_eq!(st.edge.as_deref(), Some("left"));
    assert_eq!(st.pos, Some(2_500));
    // The clipboard followed the pointer.
    assert_eq!(pc.clip().current().as_deref(), Some("copied on the mac"));
    // Starting again while it's going is refused.
    assert!(mac.engine.kvm_start(&pc.node_id(), "right", 1).is_err());

    // Motion adds up and goes; keys and buttons follow it, in order.
    for _ in 0..5 {
        mac.engine.kvm_motion(3, -1);
    }
    mac.engine
        .kvm_send(vec![
            KvmEvent::Key {
                code: 0xE3,
                down: true,
            },
            KvmEvent::Key {
                code: 0x06,
                down: true,
            },
            KvmEvent::Key {
                code: 0x06,
                down: false,
            },
            KvmEvent::Key {
                code: 0xE3,
                down: false,
            },
            KvmEvent::Button {
                button: 1,
                down: true,
            },
            KvmEvent::Button {
                button: 1,
                down: false,
            },
            KvmEvent::Wheel { dx: 0, dy: 2 },
        ])
        .unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let got = inputs(&pc);
        let moved: (i32, i32) = got.iter().fold((0, 0), |(x, y), e| match e {
            KvmEvent::Move { dx, dy } => (x + dx, y + dy),
            _ => (x, y),
        });
        let rest: Vec<KvmEvent> = got
            .iter()
            .filter(|e| !matches!(e, KvmEvent::Move { .. }))
            .copied()
            .collect();
        if moved == (15, -5) && rest.len() == 7 {
            assert_eq!(
                rest[0],
                KvmEvent::Key {
                    code: 0xE3,
                    down: true
                }
            );
            assert_eq!(rest[6], KvmEvent::Wheel { dx: 0, dy: 2 });
            break;
        }
        assert!(Instant::now() < deadline, "input never arrived: {got:?}");
        std::thread::sleep(Duration::from_millis(20));
    }

    // It stays up past the timeouts while both ends talk.
    std::thread::sleep(Duration::from_millis(4_000));
    assert_eq!(mac.engine.kvm_state().role, "controlling");
    assert_eq!(pc.engine.kvm_state().role, "controlled");

    // The pointer is pushed out of the pc's left edge: back to the mac,
    // with what was copied on the pc. Another edge doesn't give it back.
    assert!(!pc.engine.kvm_leave("right", 100));
    pc.clip().user_copies("copied on the pc");
    assert!(pc.engine.kvm_leave("left", 7_000));
    let st = wait_state(&mac, "returned", |s| s.role == "idle");
    assert_eq!(st.reason.as_deref(), Some("returned"));
    assert_eq!(st.edge.as_deref(), Some("right"));
    assert_eq!(st.pos, Some(7_000));
    assert_eq!(mac.clip().current().as_deref(), Some("copied on the pc"));
    assert_eq!(pc.engine.kvm_state().role, "idle");

    // No input reaches the pc once it isn't controlled.
    let before = inputs(&pc).len();
    mac.engine.kvm_motion(5, 5);
    assert!(mac
        .engine
        .kvm_send(vec![KvmEvent::Key {
            code: 4,
            down: true
        }])
        .is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(inputs(&pc).len(), before);

    // The hotkey on the mac: the pc lets go too.
    mac.engine.kvm_start(&pc.node_id(), "right", 5_000).unwrap();
    wait_state(&pc, "controlled again", |s| s.role == "controlled");
    mac.engine.kvm_stop("stopped");
    wait_state(&pc, "stopped", |s| {
        s.role == "idle" && s.reason.as_deref() == Some("stopped")
    });

    // "Take back" on the pc: the mac lets go.
    mac.engine.kvm_start(&pc.node_id(), "right", 5_000).unwrap();
    wait_state(&pc, "controlled a third time", |s| s.role == "controlled");
    pc.engine.kvm_stop("stopped");
    let st = wait_state(&mac, "taken back", |s| {
        s.role == "idle" && s.reason.as_deref() == Some("stopped")
    });
    assert_eq!(st.edge, None);
    no_kvm_as_chat(&mac);
    no_kvm_as_chat(&pc);
}

#[test]
fn a_password_never_follows_the_pointer() {
    let (mac, pc) = pair_side_by_side("kvm-pw-mac", "kvm-pw-pc");
    pc.clip().user_copies("what the pc had");
    mac.clip().user_copies_password("hunter2");
    mac.engine.kvm_start(&pc.node_id(), "right", 1).unwrap();
    wait_state(&pc, "controlled", |s| s.role == "controlled");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pc.clip().current().as_deref(), Some("what the pc had"));
    mac.engine.kvm_stop("stopped");
}

#[test]
fn a_computer_that_doesnt_allow_it_refuses() {
    let (mac, pc) = pair_side_by_side("kvm-off-mac", "kvm-off-pc");
    pc.engine.set_kvm_allowed(false);
    assert!(!pc.engine.kvm_allowed());
    mac.engine.kvm_start(&pc.node_id(), "right", 1).unwrap();
    let st = wait_state(&mac, "refused", |s| s.role == "idle");
    assert_eq!(st.reason.as_deref(), Some("off"));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::KvmChanged { state } if state.role == "controlled")));

    // Turned off while controlled: it lets go at once, on both ends.
    pc.engine.set_kvm_allowed(true);
    mac.engine.kvm_start(&pc.node_id(), "right", 1).unwrap();
    wait_state(&pc, "controlled", |s| s.role == "controlled");
    pc.engine.set_kvm_allowed(false);
    wait_state(&pc, "let go", |s| s.role == "idle");
    wait_state(&mac, "told", |s| {
        s.role == "idle" && s.reason.as_deref() == Some("stopped")
    });
}

#[test]
fn a_lost_link_always_lets_go() {
    let (mac, pc) = pair_side_by_side("kvm-lost-mac", "kvm-lost-pc");
    mac.engine.kvm_start(&pc.node_id(), "right", 1).unwrap();
    wait_state(&pc, "controlled", |s| s.role == "controlled");
    wait_state(&mac, "controlling", |s| s.role == "controlling");
    // The pc goes away without a word.
    drop(pc);
    let st = wait_state_for(&mac, "let go", Duration::from_secs(8), |s| s.role == "idle");
    assert!(
        matches!(st.reason.as_deref(), Some("link_lost" | "timeout")),
        "{st:?}"
    );
    assert_eq!(mac.engine.kvm_state().role, "idle");
}

#[test]
fn the_layout_is_one_computer_per_side_on_both_ends() {
    let mac = TestNode::start("kvm-lay-mac");
    let pc = TestNode::start("kvm-lay-pc");
    let linux = TestNode::start("kvm-lay-linux");
    TestNode::pair(&mac, &pc);
    TestNode::pair(&mac, &linux);
    TestNode::pair(&pc, &linux);
    link(&mac, &pc, "hi pc");
    link(&mac, &linux, "hi linux");
    wait_supported(&mac, &pc);
    wait_supported(&mac, &linux);
    mac.engine
        .kvm_set_neighbor(&pc.node_id(), Some("top"))
        .unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::KvmLayout { layout } if layout.iter().any(|n| n.side == "bottom") => {
                Some(())
            }
            _ => None,
        })
        .expect("the pc has the mac below it");
    // The linux box takes the top: the pc comes off, on both ends.
    let now = mac
        .engine
        .kvm_set_neighbor(&linux.node_id(), Some("top"))
        .unwrap();
    assert_eq!(
        now,
        [KvmNeighbor {
            node_id: linux.node_id(),
            side: "top".into()
        }]
    );
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::KvmLayout { layout } if layout.is_empty() => Some(()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "the pc took the mac off: {:?} / {:?}",
                pc.engine.kvm_layout(),
                mac.engine.kvm_layout()
            )
        });
    linux
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::KvmLayout { layout } if !layout.is_empty() => Some(()),
            _ => None,
        })
        .expect("the linux box has the mac below it");
    assert_eq!(
        linux.engine.kvm_layout(),
        [KvmNeighbor {
            node_id: mac.node_id(),
            side: "bottom".into()
        }]
    );
    // Taken off here: off there too.
    mac.engine.kvm_set_neighbor(&linux.node_id(), None).unwrap();
    linux
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::KvmLayout { layout } if layout.is_empty() => Some(()),
            _ => None,
        })
        .expect("taken off on the linux box");
    assert!(mac.engine.kvm_layout().is_empty());
    assert!(mac.engine.kvm_set_neighbor("nobody", Some("left")).is_err());
    assert!(mac
        .engine
        .kvm_set_neighbor(&pc.node_id(), Some("sideways"))
        .is_err());
}

#[test]
fn an_app_from_before_never_gets_kvm() {
    let new = TestNode::start("kvm-new");
    let old = TestNode::start_full(
        MockPlatform::new("kvm-old"),
        CoreConfig {
            kvm: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    // Placed before there's a link: nothing goes when it comes up.
    new.engine
        .kvm_set_neighbor(&old.node_id(), Some("left"))
        .unwrap();
    link(&new, &old, "hello");
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("kvm").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without kvm");
    assert!(!new.engine.kvm_supported(&old.node_id()));
    assert!(new.engine.kvm_start(&old.node_id(), "left", 1).is_err());
    assert_eq!(new.engine.kvm_state().role, "idle");
    std::thread::sleep(Duration::from_millis(500));
    no_kvm_as_chat(&old);
    assert!(!old.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::KvmLayout { .. } | CoreEvent::KvmChanged { .. }
    )));
    assert!(old.engine.kvm_layout().is_empty());
}

#[test]
fn only_the_computer_placed_on_that_side_can_take_control() {
    let (mac, pc) = pair_side_by_side("kvm-side-mac", "kvm-side-pc");
    // A third device, paired and linked, but placed nowhere.
    let other = TestNode::start("kvm-side-other");
    TestNode::pair(&other, &pc);
    link(&other, &pc, "hello");
    let enter = |edge: &str| {
        format!(
            r#"{{"type":"kvm_enter","from":"{}","edge":"{edge}","pos":5000}}"#,
            other.node_id()
        )
    };
    for edge in ["left", "right", "top", "bottom"] {
        other.engine.send(&pc.node_id(), &enter(edge)).unwrap();
    }
    // A layout from far in the future can't place it either.
    other
        .engine
        .send(
            &pc.node_id(),
            r#"{"type":"kvm_layout","from":"x","side":"right","at":18446744073709551615}"#,
        )
        .unwrap();
    other.engine.send(&pc.node_id(), &enter("left")).unwrap();
    link(&other, &pc, "after");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::KvmChanged { state } if state.role == "controlled")));
    assert!(pc
        .engine
        .kvm_layout()
        .iter()
        .all(|n| n.node_id != other.node_id()));
    // The mac, placed on the left of the pc, still can.
    mac.engine.kvm_start(&pc.node_id(), "right", 1).unwrap();
    wait_state(&pc, "controlled", |s| s.role == "controlled");
}
