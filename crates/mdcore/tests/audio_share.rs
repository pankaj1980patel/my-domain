//! A phone's audio on a computer: the computer listens, the phone sends
//! PCM frames, between two engines over a real link; and nothing of it
//! reaches a device whose app is too old to know it.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use base64::Engine as _;
use md_testkit::{TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::features::audio_share::{MAX_FRAME_BYTES, RATE_DIRECT};
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

/// `asker` asks `of` for its status and waits for the answer.
fn learn_status(asker: &TestNode, of: &TestNode) -> serde_json::Value {
    asker.engine.request_status(&of.node_id()).unwrap();
    let who = of.node_id();
    asker
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status } if *from == who => Some(status.clone()),
            _ => None,
        })
        .expect("no status")
}

/// Both new: the computer plays, the phone sends; each knows the other's.
fn linked_new(test: &str) -> (TestNode, TestNode) {
    let (phone, pc) = linked(test);
    pc.engine.set_audio_roles(true, false);
    phone.engine.set_audio_roles(false, true);
    assert_eq!(learn_status(&pc, &phone)["audio_source"], true);
    assert_eq!(learn_status(&phone, &pc)["audio_sink"], true);
    (phone, pc)
}

fn wait_listen(phone: &TestNode, pc: &TestNode, on: bool) {
    let who = pc.node_id();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioListen { from, on: o } if *from == who && *o == on => Some(()),
            _ => None,
        })
        .expect("the listen never arrived");
}

fn no_chat(n: &TestNode) {
    assert!(
        !n.sink
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::MessageReceived(_))),
        "an audio message showed as chat"
    );
}

#[test]
fn audio_round_trip_from_the_phone_to_the_computer() {
    let (phone, pc) = linked_new("audio-trip");
    let to = [pc.node_id()];
    // Nobody listening: a frame goes nowhere.
    assert_eq!(
        phone
            .engine
            .audio_frame_to(&[0, 0, 0, 0], RATE_DIRECT, 2, &to)
            .unwrap(),
        0
    );

    pc.engine.audio_listen(&phone.node_id(), true).unwrap();
    wait_listen(&phone, &pc, true);
    assert_eq!(phone.engine.audio_listeners(), [pc.node_id()]);

    let pcm: Vec<u8> = (0..3840u32).map(|i| (i % 251) as u8).collect();
    for _ in 0..3 {
        assert_eq!(
            phone
                .engine
                .audio_frame_to(&pcm, RATE_DIRECT, 2, &to)
                .unwrap(),
            1
        );
    }
    let who = phone.node_id();
    let frames_from = |pc: &TestNode| -> Vec<(u64, u32, u8, String)> {
        pc.sink
            .all()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::AudioFrame {
                    from,
                    seq,
                    rate,
                    channels,
                    pcm,
                } if from == who => Some((seq, rate, channels, pcm)),
                _ => None,
            })
            .collect()
    };
    let deadline = std::time::Instant::now() + WAIT;
    while frames_from(&pc).len() < 3 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let frames = frames_from(&pc);
    assert_eq!(frames.len(), 3);
    for (i, (seq, rate, ch, data)) in frames.iter().enumerate() {
        assert_eq!((*rate, *ch), (RATE_DIRECT, 2));
        if i > 0 {
            assert!(*seq > frames[i - 1].0, "seq counts up");
        }
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap(),
            pcm
        );
    }

    // Renewing isn't news; stopping is.
    pc.engine.audio_listen(&phone.node_id(), true).unwrap();
    pc.engine.audio_listen(&phone.node_id(), false).unwrap();
    wait_listen(&phone, &pc, false);
    let starts = phone
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::AudioListen { on: true, .. }))
        .count();
    assert_eq!(starts, 1, "a renewal was reported as a new listen");
    assert!(phone.engine.audio_listeners().is_empty());
    no_chat(&pc);
    no_chat(&phone);
}

#[test]
fn the_phone_offers_its_audio_and_says_when_it_stops() {
    let (phone, pc) = linked_new("audio-offer");
    let rate = phone.engine.audio_offer(&pc.node_id(), true).unwrap();
    assert_eq!(rate, RATE_DIRECT, "a direct link carries 48 kHz");
    let who = phone.node_id();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioOffer {
                from,
                on: true,
                rate,
            } if *from == who => Some(*rate),
            _ => None,
        })
        .expect("no offer");

    // Frames from a phone the computer doesn't listen to are dropped.
    phone.engine.audio_listen(&pc.node_id(), true).ok();
    let raw = serde_json::json!({
        "type": "audio_frame", "from": phone.node_id(), "seq": 1, "rate": 48000, "ch": 2,
        "data": "AAAAAA==",
    })
    .to_string();
    phone.engine.send(&pc.node_id(), &raw).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::AudioFrame { .. })));

    assert_eq!(phone.engine.audio_offer(&pc.node_id(), false).unwrap(), 0);
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioOffer {
                from, on: false, ..
            } if *from == who => Some(()),
            _ => None,
        })
        .expect("no stop");
    no_chat(&pc);
}

#[test]
fn nothing_goes_to_an_app_too_old_to_know_it() {
    // Neither says it plays or sends audio (an older app).
    let (phone, pc) = linked("audio-old");
    assert!(learn_status(&pc, &phone).get("audio_source").is_none());
    assert!(learn_status(&phone, &pc).get("audio_sink").is_none());
    assert_eq!(phone.engine.audio_caps(&pc.node_id()), (false, false));

    assert!(matches!(
        phone.engine.audio_offer(&pc.node_id(), true),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        pc.engine.audio_listen(&phone.node_id(), true),
        Err(CoreError::Unsupported(_))
    ));
    // An old phone (it never said it sends audio) ignores a listen, and
    // shows nothing as chat.
    let raw =
        serde_json::json!({ "type": "audio_play", "from": pc.node_id(), "on": true }).to_string();
    pc.engine.send(&phone.node_id(), &raw).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(phone.engine.audio_listeners().is_empty());
    assert!(!phone
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::AudioListen { .. })));
    no_chat(&pc);
    no_chat(&phone);

    // Once it updates (says audio_sink), the offer goes.
    pc.engine.set_audio_roles(true, false);
    phone.engine.request_status(&pc.node_id()).unwrap();
    let who = pc.node_id();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status }
                if *from == who && status["audio_sink"] == true =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("no new status");
    assert_eq!(phone.engine.audio_caps(&pc.node_id()), (true, false));
    assert!(phone.engine.audio_offer(&pc.node_id(), true).is_ok());
}

#[test]
fn bad_frames_and_missing_links_are_refused() {
    let (phone, pc) = linked_new("audio-limits");
    let to = [pc.node_id()];
    for (pcm, rate, ch) in [
        (vec![0u8; MAX_FRAME_BYTES + 4], RATE_DIRECT, 2),
        (vec![], RATE_DIRECT, 2),
        (vec![0u8; 4], 12_345, 2),
        (vec![0u8; 4], RATE_DIRECT, 6),
        (vec![0u8; 3], RATE_DIRECT, 2),
    ] {
        assert!(matches!(
            phone.engine.audio_frame_to(&pcm, rate, ch, &to),
            Err(CoreError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        phone.engine.audio_offer("nobody", true),
        Err(CoreError::PeerNotFound)
    ));
    assert!(matches!(
        pc.engine.audio_listen("nobody", true),
        Err(CoreError::PeerNotFound)
    ));
}

#[test]
fn a_computer_that_asks_is_known_to_play_audio() {
    // The phone never asked for the computer's status: the computer's
    // listen says it understands audio.
    let (phone, pc) = linked("audio-learned");
    pc.engine.set_audio_roles(true, false);
    phone.engine.set_audio_roles(false, true);
    assert_eq!(learn_status(&pc, &phone)["audio_source"], true);
    assert_eq!(phone.engine.audio_caps(&pc.node_id()), (false, false));
    pc.engine.audio_listen(&phone.node_id(), true).unwrap();
    wait_listen(&phone, &pc, true);
    assert_eq!(phone.engine.audio_caps(&pc.node_id()), (true, false));
    assert_eq!(
        phone.engine.audio_offer(&pc.node_id(), true).unwrap(),
        RATE_DIRECT
    );
    assert_eq!(
        phone
            .engine
            .audio_frame_to(&[1, 0, 2, 0], RATE_DIRECT, 2, &[pc.node_id()])
            .unwrap(),
        1
    );
    let who = phone.node_id();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioFrame { from, .. } if *from == who => Some(()),
            _ => None,
        })
        .expect("no frame");
}
