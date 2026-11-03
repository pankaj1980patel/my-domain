//! Characterization tests: what two engines do today when they talk to each
//! other over loopback. These pin down current behaviour so the Phase 1 refactor
//! and the Phase 2 tokio/QUIC rewrite can prove they didn't break anything.
//!
//! Every test starts real engines (real sockets on 127.0.0.1, real threads,
//! real crypto) through `md_testkit`. Ports are ephemeral, so tests can run in
//! parallel.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::{CoreError, CoreEvent};

/// Pair two fresh nodes. Ids are unique per test so parallel tests never clash.
fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

// ---------------------------------------------------------------------------
// Chat: every message rides a live link (QUIC unless the ladder found another)
// ---------------------------------------------------------------------------

#[test]
fn chat_opens_and_reuses_a_quic_link_by_default() {
    let (a, b) = pair("chat-quic");
    // No explicit connect(): the first send dials QUIC.
    assert_eq!(a.engine.send(&b.node_id(), "first").unwrap(), "QUIC");
    let msg = b.sink.wait_for_message("first");
    assert!(msg.ok, "decrypted with the shared key");
    assert_eq!(msg.protocol, "QUIC");
    // `from` is the sender's username (offline login); `node_id` says which
    // of the user's devices sent it.
    assert_eq!(msg.from, md_testkit::TEST_USER);
    assert_eq!(msg.node_id.as_deref(), Some(a.node_id().as_str()));

    // Reused, and the receiver can answer on the same link.
    assert_eq!(a.engine.send(&b.node_id(), "second").unwrap(), "QUIC");
    b.sink.wait_for_message("second");
    assert_eq!(b.engine.send(&a.node_id(), "reply").unwrap(), "QUIC");
    a.sink.wait_for_message("reply");
}

#[test]
fn an_unreachable_peer_fails_fast_and_is_not_redialed_every_send() {
    let (a, b) = pair("chat-quic-dead");
    let mut peer = b.as_peer();
    peer.quic_port = 9; // nothing speaks QUIC there
    a.engine.upsert_peer(peer);

    // Held for later (the ladder carries on in the background).
    assert_eq!(
        a.engine.send(&b.node_id(), "nobody home").unwrap(),
        mdcore::engine::QUEUED
    );

    // The failure is remembered, so the next send doesn't wait for a dial.
    let t = std::time::Instant::now();
    assert_eq!(
        a.engine.send(&b.node_id(), "still nobody").unwrap(),
        mdcore::engine::QUEUED
    );
    assert!(
        t.elapsed() < Duration::from_millis(500),
        "took {:?}",
        t.elapsed()
    );
    // Nothing was fired off as a lone datagram either.
    b.sink
        .assert_no_message("nobody home", Duration::from_millis(300));
}

#[test]
fn with_auto_quic_off_a_send_without_a_link_is_held_and_delivered_once_connected() {
    let cfg = || mdcore::CoreConfig {
        auto_quic: false,
        ..md_testkit::test_config()
    };
    let a = TestNode::start_full(MockPlatform::new("no-auto-a"), cfg());
    let b = TestNode::start_full(MockPlatform::new("no-auto-b"), cfg());
    TestNode::pair(&a, &b);

    // No link yet: the message is held, and the ladder starts.
    assert_eq!(
        a.engine.send(&b.node_id(), "too early").unwrap(),
        mdcore::engine::QUEUED
    );
    // The link may already be up for the second one (a fast machine): either
    // way both arrive, in order (checked below).
    let second = a.engine.send(&b.node_id(), "also early").unwrap();
    assert!(
        second == mdcore::engine::QUEUED || second == "QUIC",
        "{second}"
    );
    // The ladder (rung 0, QUIC) connects in the background, and the held
    // messages go out by themselves, in order.
    assert_eq!(
        a.wait_connected(&b, Duration::from_secs(10)).as_deref(),
        Some("quic")
    );
    b.sink.wait_for_message("too early");
    b.sink.wait_for_message("also early");
    let texts: Vec<String> = b
        .sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::MessageReceived(m) => Some(m.text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["too early", "also early"]);
    assert_eq!(a.engine.send(&b.node_id(), "now").unwrap(), "QUIC");
    b.sink.wait_for_message("now");
}

#[test]
fn send_to_unknown_peer_fails_with_peer_not_found() {
    let a = TestNode::start("unknown-peer-a");
    let err = a.engine.send("nobody", "hi").unwrap_err();
    assert!(matches!(err, CoreError::PeerNotFound), "got {err:?}");
}

#[test]
fn sending_before_a_key_is_set_fails_with_no_key() {
    // Started without dev_login, so no username and no key.
    let a = mdcore::Engine::start(
        MockPlatform::new("no-key-a"),
        std::sync::Arc::new(md_testkit::RecordingSink::default()),
    )
    .unwrap();
    let b = TestNode::start("no-key-b");
    a.upsert_peer(b.as_peer());
    let err = a.send(&b.node_id(), "hi").unwrap_err();
    assert!(matches!(err, CoreError::NoKey), "got {err:?}");
    assert_eq!(err.to_string(), "set your encryption key first");
}

#[test]
fn setting_a_key_requires_login() {
    let a = mdcore::Engine::start(
        MockPlatform::new("no-login-a"),
        std::sync::Arc::new(md_testkit::RecordingSink::default()),
    )
    .unwrap();
    let err = a.set_encryption_key("pass").unwrap_err();
    assert!(matches!(err, CoreError::NotLoggedIn), "got {err:?}");
}

// ---------------------------------------------------------------------------
// Encryption
// ---------------------------------------------------------------------------

#[test]
fn different_passphrase_cannot_read_messages() {
    let (a, b) = pair("wrong-key");
    // b switches to another key; a still uses the shared test passphrase.
    b.engine
        .set_encryption_key("a different passphrase")
        .unwrap();

    a.engine.send(&b.node_id(), "secret").unwrap();

    // The user gets ONE visible hint that keys don't match (ok: false,
    // placeholder text, never the plaintext).
    let undecryptable = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::MessageReceived(m) if !m.ok => Some(m.clone()),
            _ => None,
        })
        .expect("an ok:false notice is emitted");
    assert_ne!(undecryptable.text, "secret", "plaintext must never leak");
    b.sink
        .assert_no_message("secret", Duration::from_millis(300));
}

#[test]
fn undecryptable_flood_produces_one_notice_but_is_fully_counted() {
    // Regression: every undecryptable envelope used to become a chat entry, so
    // any LAN host could flood the UI. Now: one notice per sender IP per
    // minute, and every failure is counted.
    let (a, b) = pair("wrong-key-flood");
    b.engine.set_encryption_key("another key").unwrap();

    for i in 0..20 {
        a.engine.send(&b.node_id(), &format!("junk {i}")).unwrap();
    }
    let deadline = std::time::Instant::now() + WAIT;
    while b.engine.decrypt_failures() < 20 {
        assert!(
            std::time::Instant::now() < deadline,
            "not all failures counted"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let notices = b
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::MessageReceived(m) if !m.ok))
        .count();
    assert_eq!(notices, 1);
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

#[test]
fn clipboard_pull_copies_peer_clipboard_locally() {
    let (a, b) = pair("clip-pull");
    b.clip().user_copies("copied on b");

    let got = a.engine.get_clipboard(&b.node_id()).unwrap();
    assert_eq!(got, "copied on b");
    assert_eq!(a.clip().current().as_deref(), Some("copied on b"));
}

#[test]
fn clipboard_pull_from_device_without_clipboard_times_out() {
    let a = TestNode::start("clip-none-a");
    let b = TestNode::start_with(MockPlatform::new("clip-none-b").without_clipboard());
    TestNode::pair(&a, &b);
    // b has no clipboard, so it never answers; a gives up after its timeout.
    assert!(a.engine.get_clipboard(&b.node_id()).is_err());
}

#[test]
fn clipboard_auto_sync_pushes_changes_when_both_enabled() {
    let (a, b) = pair("clip-push");
    a.engine.enable_clipboard_sync();
    b.engine.enable_clipboard_sync();

    a.clip().user_copies("auto synced");

    // The watcher polls every second; give it a few cycles.
    let deadline = std::time::Instant::now() + WAIT;
    while b.clip().current().as_deref() != Some("auto synced") {
        assert!(
            std::time::Instant::now() < deadline,
            "b never received the clipboard"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn the_clipboard_history_keeps_recent_values_newest_first() {
    let (a, b) = pair("clip-history");
    a.engine.enable_clipboard_sync();
    b.engine.enable_clipboard_sync();
    let wait_for = |texts: &[&str]| {
        let deadline = std::time::Instant::now() + WAIT;
        loop {
            let got: Vec<String> = b
                .engine
                .clipboard_history()
                .into_iter()
                .map(|e| e.text)
                .collect();
            if got == texts {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "history {got:?}, want {texts:?}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };

    a.clip().user_copies("first");
    wait_for(&["first"]);
    let entry = b.engine.clipboard_history().remove(0);
    assert_eq!(
        entry.from.as_deref(),
        Some(a.node_id().as_str()),
        "came from a"
    );

    a.clip().user_copies("second");
    wait_for(&["second", "first"]);
    // A value copied again moves to the top instead of repeating.
    a.clip().user_copies("first");
    wait_for(&["first", "second"]);
    // Copied on b itself: from this device.
    b.clip().user_copies("local");
    wait_for(&["local", "first", "second"]);
    assert_eq!(b.engine.clipboard_history()[0].from, None);

    b.engine.clear_clipboard_history();
    assert!(b.engine.clipboard_history().is_empty());
}

#[test]
fn clipboard_push_is_ignored_when_receiver_disabled_sync() {
    let (a, b) = pair("clip-push-off");
    a.engine.enable_clipboard_sync();
    b.clip().user_copies("mine");

    a.clip().user_copies("theirs");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(b.clip().current().as_deref(), Some("mine"));
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

#[test]
fn shared_notification_is_posted_on_peer() {
    let (a, b) = pair("notif");
    a.engine
        .share_notification("Battery low", "15% left", Some("system"));

    let (title, app) = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Notification { title, app, .. } => Some((title.clone(), app.clone())),
            _ => None,
        })
        .expect("notification event");
    assert_eq!(title, "Battery low");
    assert_eq!(app.as_deref(), Some("system"));
    // ...and it went to the OS notifier, not into the chat.
    assert_eq!(
        b.notifier.posted(),
        vec![("Battery low".to_string(), "15% left".to_string())]
    );
    b.sink
        .assert_no_message("Battery low", Duration::from_millis(200));
}

#[test]
fn a_hand_entered_peer_is_reached_over_quic_and_relisted_under_its_real_id() {
    let a = TestNode::start("manual-a");
    let b = TestNode::start("manual-b");
    let port = b.engine.identity().quic_port;
    a.engine
        .add_manual_peer("my mac", "127.0.0.1", port)
        .unwrap();

    assert_eq!(
        a.engine.send("manual:127.0.0.1", "typed by hand").unwrap(),
        "QUIC"
    );
    b.sink.wait_for_message("typed by hand");

    let peers = a.engine.get_peers();
    assert!(!peers.iter().any(|p| p.node_id == "manual:127.0.0.1"));
    let real = peers
        .iter()
        .find(|p| p.node_id == b.node_id())
        .expect("listed under its real node id");
    assert_eq!(real.name, "my mac");

    // The placeholder keeps working, even after the entry changes again
    // (e.g. LAN discovery updates its address).
    let mut moved = real.clone();
    moved.ip = "192.0.2.7".into();
    a.engine.upsert_peer(moved);
    assert_eq!(
        a.engine
            .send("manual:127.0.0.1", "still reachable")
            .unwrap(),
        "QUIC"
    );
    b.sink.wait_for_message("still reachable");
}

// ---------------------------------------------------------------------------
// Delivery receipts: every chat message is acknowledged; resends show once
// ---------------------------------------------------------------------------

fn delivered(node: &TestNode, id: &str) -> bool {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::MessageDelivered { id: got, .. } if got == id => Some(()),
            _ => None,
        })
        .is_some()
}

#[test]
fn the_sender_hears_when_a_message_arrived() {
    let (a, b) = pair("receipt");
    assert_eq!(
        a.engine
            .send_with_id(&b.node_id(), "did you get this", "m1")
            .unwrap(),
        "QUIC"
    );
    let msg = b.sink.wait_for_message("did you get this");
    assert_eq!(msg.id.as_deref(), Some("m1"));
    assert!(delivered(&a, "m1"), "no receipt; got {:#?}", a.sink.all());
}

#[test]
fn a_resent_message_is_shown_once_and_still_acknowledged() {
    let (a, b) = pair("receipt-dup");
    a.engine
        .send_with_id(&b.node_id(), "only once", "dup")
        .unwrap();
    assert!(delivered(&a, "dup"));
    // The same message again, as a resend after a lost receipt would be.
    a.engine
        .send_with_id(&b.node_id(), "only once", "dup")
        .unwrap();
    let receipts = || {
        a.sink
            .all()
            .iter()
            .filter(|e| matches!(e, CoreEvent::MessageDelivered { id, .. } if id == "dup"))
            .count()
    };
    let t = std::time::Instant::now();
    while receipts() < 2 && t.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(receipts(), 2, "the resend is acknowledged too");
    let copies = b
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::MessageReceived(m) if m.text == "only once"))
        .count();
    assert_eq!(copies, 1, "shown once");
}
