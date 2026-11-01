//! Characterization tests: what two engines do today when they talk to each
//! other over loopback. These pin down current behaviour so the Phase 1 refactor
//! and the Phase 2 tokio/QUIC rewrite can prove they didn't break anything.
//!
//! Every test starts real engines (real sockets on 127.0.0.1, real threads,
//! real crypto) through `md_testkit`. Ports are ephemeral, so tests can run in
//! parallel.

use std::time::Duration;

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::CoreEvent;

/// Pair two fresh nodes. Ids are unique per test so parallel tests never clash.
fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

// ---------------------------------------------------------------------------
// Chat over each directed transport
// ---------------------------------------------------------------------------

#[test]
fn chat_over_udp_by_default() {
    let (a, b) = pair("chat-udp");
    let used = a.engine.send(&b.node_id(), "hello over udp").unwrap();
    assert_eq!(used, "UDP");

    let msg = b.sink.wait_for_message("hello over udp");
    assert!(msg.ok, "decrypted with the shared key");
    assert_eq!(msg.protocol, "UDP");
    // `from` is the sender's username (offline login), not its node id.
    assert_eq!(msg.from, md_testkit::TEST_USER);
}

#[test]
fn chat_over_tcp_when_directed() {
    let (a, b) = pair("chat-tcp");
    a.engine.set_directed_transport("TCP");
    let used = a.engine.send(&b.node_id(), "hello over tcp").unwrap();
    assert_eq!(used, "TCP");
    assert_eq!(b.sink.wait_for_message("hello over tcp").protocol, "TCP");
}

#[test]
fn chat_over_websocket_once_connected() {
    let (a, b) = pair("chat-ws");
    a.engine.connect_ws(&b.node_id()).unwrap();

    // Both ends report the link; the receiver learns the sender's node id from
    // the WS Hello frame.
    let a_id = a.node_id();
    let b_id = b.node_id();
    a.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::WsConnected { node_id } if *node_id == b_id => Some(()),
            _ => None,
        })
        .expect("a sees the WS link");
    b.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::WsConnected { node_id } if *node_id == a_id => Some(()),
            _ => None,
        })
        .expect("b sees the WS link");

    // A live persistent link beats the directed transport.
    let used = a.engine.send(&b_id, "hello over ws").unwrap();
    assert_eq!(used, "WS");
    assert_eq!(b.sink.wait_for_message("hello over ws").protocol, "WS");

    // The link is bidirectional: b can answer on the same socket.
    assert_eq!(b.engine.send(&a_id, "and back").unwrap(), "WS");
    a.sink.wait_for_message("and back");
}

#[test]
fn send_to_unknown_peer_fails() {
    let a = TestNode::start("unknown-peer-a");
    assert!(a.engine.send("nobody", "hi").is_err());
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

    // CURRENT BEHAVIOUR (a known weakness, fixed in Phase 1): an undecryptable
    // message is still surfaced to the UI, flagged `ok: false`, with a
    // placeholder text. Any LAN host can therefore spam the chat.
    let undecryptable = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::MessageReceived(m) if !m.ok => Some(m.clone()),
            _ => None,
        })
        .expect("an ok:false message is emitted");
    assert_ne!(undecryptable.text, "secret", "plaintext must never leak");
    b.sink
        .assert_no_message("secret", Duration::from_millis(300));
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
