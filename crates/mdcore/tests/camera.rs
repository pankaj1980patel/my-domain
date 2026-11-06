//! A phone's camera on a computer (features::camera): the computer asks,
//! the phone answers and streams to the computers its owner allowed, the
//! computer turns it about; and an app from before never gets any of it.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use base64::Engine as _;
use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::features::camera::CameraState;
use mdcore::{CoreConfig, CoreEvent};

fn start(id: &str, config: CoreConfig) -> TestNode {
    TestNode::start_full(MockPlatform::new(id), config)
}

/// `a` links to `b` (a chat message), and each hears what the other knows
/// (the `addr_update` every link starts with).
fn link(a: &TestNode, b: &TestNode) {
    let hello = format!("hello from {}", a.node_id());
    a.engine.send(&b.node_id(), &hello).unwrap();
    b.sink.wait_for_message(&hello);
    for (x, y) in [(a, b), (b, a)] {
        x.engine.request_status(&y.node_id()).unwrap();
        let who = y.node_id();
        x.sink
            .wait_until(WAIT, |e| match e {
                CoreEvent::DeviceStatus { from, .. } if *from == who => Some(()),
                _ => None,
            })
            .expect("no status");
    }
}

fn wait_supported(a: &TestNode, b: &TestNode) {
    let deadline = Instant::now() + WAIT;
    while !a.engine.camera_supported(&b.node_id()) {
        assert!(Instant::now() < deadline, "never heard it knows the camera");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_watch(phone: &TestNode, pc: &TestNode, on: bool) {
    let who = pc.node_id();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraWatch { from, on: o } if *from == who && *o == on => Some(()),
            _ => None,
        })
        .expect("the camera watch never arrived");
}

/// Nothing about the camera showed on `node` as chat or as the screen.
fn nothing_misread(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| match e {
        CoreEvent::MessageReceived(m) =>
            m.text.contains("camera_ctl") || m.text.contains("screen_"),
        CoreEvent::ScreenWatch { .. }
        | CoreEvent::ScreenOffer { .. }
        | CoreEvent::ScreenFrame { .. } => true,
        _ => false,
    }));
}

fn round_trip(test: &str, config: CoreConfig) {
    let phone = start(&format!("{test}-phone"), config.clone());
    let pc = start(&format!("{test}-pc"), config);
    TestNode::pair(&phone, &pc);
    link(&pc, &phone);
    wait_supported(&pc, &phone);
    wait_supported(&phone, &pc);

    // The computer asks; the phone hears a camera watch, not a screen one.
    pc.engine.camera_watch(&phone.node_id(), true).unwrap();
    wait_watch(&phone, &pc, true);
    assert_eq!(phone.engine.camera_watchers(), [pc.node_id()]);
    assert!(phone.engine.screen_watchers().is_empty());

    // Its owner said yes: the computer hears the camera's state.
    let state = CameraState {
        on: true,
        front: false,
        torch: Some(false),
        reason: None,
    };
    phone.engine.camera_offer(&pc.node_id(), &state).unwrap();
    let who = phone.node_id();
    let got = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraOffer {
                from,
                on,
                front,
                torch,
                reason,
            } if *from == who => Some((*on, *front, *torch, reason.clone())),
            _ => None,
        })
        .expect("no offer");
    assert_eq!(got, (true, false, Some(false), None));

    // A picture arrives as a camera frame, bytes intact.
    let jpeg = b"\xff\xd8 a camera's picture".to_vec();
    assert_eq!(
        phone
            .engine
            .camera_frame_to(&jpeg, &[pc.node_id()])
            .unwrap(),
        1
    );
    let got = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraFrame { from, jpeg, .. } if *from == who => Some(jpeg.clone()),
            _ => None,
        })
        .expect("no camera frame");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(got)
            .unwrap(),
        jpeg
    );

    // The computer turns the torch on and takes a photo.
    pc.engine
        .camera_control(&phone.node_id(), "torch", true)
        .unwrap();
    pc.engine
        .camera_control(&phone.node_id(), "photo", false)
        .unwrap();
    let pcid = pc.node_id();
    for want in ["torch", "photo"] {
        let got = phone
            .sink
            .wait_until(WAIT, |e| match e {
                CoreEvent::CameraControl { from, op, on } if *from == pcid && op == want => {
                    Some(*on)
                }
                _ => None,
            })
            .expect("no camera control");
        assert_eq!(got, want == "torch");
    }
    assert!(matches!(
        pc.engine
            .camera_control(&phone.node_id(), "selfdestruct", true),
        Err(CoreError::InvalidInput(_))
    ));

    // Renewing isn't news; stopping is, and then nothing more goes.
    pc.engine.camera_watch(&phone.node_id(), true).unwrap();
    pc.engine.camera_watch(&phone.node_id(), false).unwrap();
    wait_watch(&phone, &pc, false);
    let starts = phone
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::CameraWatch { on: true, .. }))
        .count();
    assert_eq!(starts, 1, "a renewal was reported as a new watch");
    assert_eq!(
        phone
            .engine
            .camera_frame_to(&jpeg, &[pc.node_id()])
            .unwrap(),
        0
    );

    // Off, and why.
    let off = CameraState {
        reason: Some("stopped".into()),
        ..CameraState::default()
    };
    phone.engine.camera_offer(&pc.node_id(), &off).unwrap();
    let got = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraOffer {
                from,
                on: false,
                reason,
                ..
            } if *from == who => Some(reason.clone()),
            _ => None,
        })
        .expect("no stop");
    assert_eq!(got.as_deref(), Some("stopped"));

    nothing_misread(&phone);
    nothing_misread(&pc);
}

#[test]
fn the_camera_goes_both_ways_as_binary_frames() {
    round_trip("camera-bin", test_config());
}

#[test]
fn the_camera_goes_both_ways_as_json_messages() {
    round_trip(
        "camera-json",
        CoreConfig {
            bin_media: false,
            ..test_config()
        },
    );
}

#[test]
fn an_app_from_before_never_gets_a_camera_message() {
    let old_config = CoreConfig {
        camera: false,
        ..test_config()
    };
    let old_phone = start("camera-old-phone", old_config.clone());
    let pc = start("camera-old-pc", test_config());
    TestNode::pair(&old_phone, &pc);
    link(&pc, &old_phone);
    // Its status doesn't say it.
    let says = pc
        .sink
        .all()
        .into_iter()
        .find_map(|e| match e {
            CoreEvent::DeviceStatus { from, status } if from == old_phone.node_id() => {
                Some(status.get("camera_view").is_some())
            }
            _ => None,
        })
        .unwrap();
    assert!(!says);
    assert!(!pc.engine.camera_supported(&old_phone.node_id()));

    // The computer won't ask it, or turn it about.
    assert!(matches!(
        pc.engine.camera_watch(&old_phone.node_id(), true),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        pc.engine
            .camera_control(&old_phone.node_id(), "photo", false),
        Err(CoreError::Unsupported(_))
    ));

    // A new phone never offers its camera to an old computer, nor sends
    // it a picture, even one that watches its screen.
    let phone = start("camera-old-newphone", test_config());
    let old_pc = start("camera-old-oldpc", old_config);
    TestNode::pair(&phone, &old_pc);
    link(&old_pc, &phone);
    assert!(!phone.engine.camera_supported(&old_pc.node_id()));
    assert!(matches!(
        phone.engine.camera_offer(
            &old_pc.node_id(),
            &CameraState {
                on: true,
                ..CameraState::default()
            }
        ),
        Err(CoreError::Unsupported(_))
    ));
    old_pc.engine.screen_watch(&phone.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while phone.engine.screen_watchers().is_empty() {
        assert!(Instant::now() < deadline, "the screen watch never came");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(phone.engine.camera_watchers().is_empty());
    assert_eq!(
        phone
            .engine
            .camera_frame_to(b"\xff\xd8", &[old_pc.node_id()])
            .unwrap(),
        0
    );

    std::thread::sleep(Duration::from_millis(500));
    nothing_misread(&old_phone);
    assert!(!old_pc.sink.all().iter().any(|e| match e {
        CoreEvent::MessageReceived(m) => m.text.contains("screen_offer"),
        CoreEvent::ScreenOffer { .. } => true,
        _ => false,
    }));
}

#[test]
fn pictures_go_only_to_the_computer_the_owner_allowed() {
    let phone = start("camera-only-phone", test_config());
    let pc = start("camera-only-pc", test_config());
    let other = start("camera-only-other", test_config());
    TestNode::pair(&phone, &pc);
    TestNode::pair(&phone, &other);
    link(&pc, &phone);
    link(&other, &phone);
    for c in [&pc, &other] {
        wait_supported(c, &phone);
        wait_supported(&phone, c);
    }

    // Both ask; the owner allowed only `pc`.
    pc.engine.camera_watch(&phone.node_id(), true).unwrap();
    other.engine.camera_watch(&phone.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while phone.engine.camera_watchers().len() < 2 {
        assert!(Instant::now() < deadline, "both should watch");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        phone
            .engine
            .camera_frame_to(b"\xff\xd8a", &[pc.node_id()])
            .unwrap(),
        1
    );
    let who = phone.node_id();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraFrame { from, .. } if *from == who => Some(()),
            _ => None,
        })
        .expect("no frame for the allowed computer");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !other.sink.all().iter().any(|e| matches!(
            e,
            CoreEvent::CameraFrame { .. } | CoreEvent::ScreenFrame { .. }
        )),
        "a computer the owner didn't allow saw the camera"
    );

    // A computer that stopped watching can't turn the camera about.
    other.engine.camera_watch(&phone.node_id(), false).unwrap();
    wait_watch(&phone, &other, false);
    other
        .engine
        .camera_control(&phone.node_id(), "photo", false)
        .unwrap();
    // (The one watching still can: its message comes after.)
    pc.engine
        .camera_control(&phone.node_id(), "switch", false)
        .unwrap();
    let pcid = pc.node_id();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CameraControl { from, op, .. } if *from == pcid && op == "switch" => {
                Some(())
            }
            _ => None,
        })
        .expect("the watching computer's switch");
    std::thread::sleep(Duration::from_millis(300));
    let oid = other.node_id();
    assert!(!phone
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::CameraControl { from, .. } if *from == oid)));

    // Its screen watchers never get the camera's pictures either.
    other.engine.screen_watch(&phone.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while phone.engine.screen_watchers().is_empty() {
        assert!(Instant::now() < deadline, "the screen watch never came");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        phone
            .engine
            .camera_frame_to(b"\xff\xd8b", &[other.node_id()])
            .unwrap(),
        0
    );
}
