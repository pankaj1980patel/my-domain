//! Rich phone notifications (features::notif_rich): conversations and
//! pictures reach a computer that understands them, reply, buttons and
//! dismiss go back and forth, the subscriber's filters (only while the
//! phone isn't in use, private mode) are kept on the phone, app icons are
//! asked for, subscriptions survive a restart, and an app from before
//! gets exactly the old messages.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use base64::Engine as _;
use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::{CoreConfig, CoreEvent};

const PKG: &str = "com.chat";
const KEY: &str = "0|com.chat|7";

fn start(id: &str, config: CoreConfig) -> TestNode {
    TestNode::start_full(MockPlatform::new(id), config)
}

/// Each hears what the other knows (status both ways).
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

fn wait_for(ok: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn pair(test: &str, phone: CoreConfig, pc: CoreConfig) -> (TestNode, TestNode) {
    let phone = start(&format!("{test}-phone"), phone);
    let pc = start(&format!("{test}-pc"), pc);
    TestNode::pair(&phone, &pc);
    link(&pc, &phone);
    (phone, pc)
}

fn jpeg(n: usize) -> String {
    let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0];
    b.resize(n, 9);
    base64::engine::general_purpose::STANDARD.encode(b)
}

fn rich(title: &str, body: &str, extra: serde_json::Value) -> String {
    let mut n = serde_json::json!({
        "app": "Chat", "title": title, "body": body, "key": KEY,
        "actions": [{"title": "Reply", "reply": true}, {"title": "Mark as read", "reply": false}],
    });
    for (k, v) in extra.as_object().unwrap() {
        n[k] = v.clone();
    }
    n.to_string()
}

/// The next notification `node` hears, as the UI gets it.
fn next_notification(node: &TestNode, after: usize) -> serde_json::Value {
    poll(|| {
        node.sink
            .all()
            .into_iter()
            .filter(|e| matches!(e, CoreEvent::Notification { .. }))
            .nth(after)
            .map(|e| e.ui_payload())
    })
    .expect("no notification")
}

/// `f()` once it says something (up to `WAIT`). Not inside `wait_until`:
/// the sink is locked while its picker runs.
fn poll<T>(f: impl Fn() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(x) = f() {
            return Some(x);
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn notifications(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::Notification { .. }))
        .count()
}

/// The phone has the computer's subscription (it's kept as it lands).
fn subscribe(phone: &TestNode, pc: &TestNode, only_locked: bool, private: bool) {
    pc.engine
        .subscribe_apps_with(
            &phone.node_id(),
            &format!(r#"["{PKG}"]"#),
            only_locked,
            private,
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
}

#[test]
fn a_conversation_with_pictures_arrives_and_reply_buttons_and_dismiss_go_back() {
    let (phone, pc) = pair("rich-conv", test_config(), test_config());
    wait_for(|| pc.engine.notif_rich_supported(&phone.node_id()));
    assert!(phone.engine.notif_rich_supported(&pc.node_id()));
    subscribe(&phone, &pc, false, false);
    phone.engine.share_app_notification_rich(
        PKG,
        &rich(
            "Family",
            "Asha: Dinner at 8?",
            serde_json::json!({
                "conv": "family-chat", "conv_title": "Family", "group_chat": true,
                "sender": "Asha",
                "messages": [{"sender": "Ravi", "text": "Who's cooking?", "ts": 1},
                             {"sender": "Asha", "text": "Dinner at 8?", "ts": 2}],
                "avatar": jpeg(800), "picture": jpeg(4000), "ts": 2, "in_use": true,
            }),
        ),
    );
    let n = next_notification(&pc, 0);
    assert_eq!(n["from"], phone.node_id());
    assert_eq!(n["title"], "Family");
    assert_eq!(n["key"], KEY);
    assert_eq!(n["pkg"], PKG);
    assert_eq!(n["conv"], "family-chat");
    assert_eq!(n["conv_title"], "Family");
    assert_eq!(n["group_chat"], true);
    assert_eq!(n["messages"][1]["text"], "Dinner at 8?");
    assert_eq!(n["avatar"], jpeg(800));
    assert_eq!(n["picture"], jpeg(4000));
    assert!(n.get("in_use").is_none());

    // Reply from the computer.
    pc.engine
        .notification_action(&phone.node_id(), KEY, 0, Some("Me!"))
        .unwrap();
    // A button.
    pc.engine
        .notification_action(&phone.node_id(), KEY, 1, None)
        .unwrap();
    // Dismissed on the computer.
    pc.engine
        .notification_action(&phone.node_id(), KEY, -1, None)
        .unwrap();
    let got = poll(|| {
        let got = phone
            .sink
            .all()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::NotificationAction {
                    key, index, text, ..
                } => Some((key, index, text)),
                _ => None,
            })
            .collect::<Vec<_>>();
        (got.len() == 3).then_some(got)
    })
    .expect("not every action arrived");
    assert_eq!(
        got,
        vec![
            (KEY.to_string(), 0, Some("Me!".to_string())),
            (KEY.to_string(), 1, None),
            (KEY.to_string(), -1, None),
        ]
    );

    // Dismissed on the phone: gone on the computer.
    phone.engine.share_app_notification_removed(PKG, KEY);
    let removed = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationRemoved { key, .. } => Some(key.clone()),
            _ => None,
        })
        .expect("no removal");
    assert_eq!(removed, KEY);
}

#[test]
fn private_mode_keeps_a_sensitive_ones_content_on_the_phone() {
    let (phone, pc) = pair("rich-private", test_config(), test_config());
    wait_for(|| phone.engine.notif_rich_supported(&pc.node_id()));
    subscribe(&phone, &pc, false, true);
    phone.engine.share_app_notification_rich(
        PKG,
        &rich(
            "Bank",
            "Your code is 482913",
            serde_json::json!({"sensitive": true, "sender": "Bank", "picture": jpeg(500)}),
        ),
    );
    let n = next_notification(&pc, 0);
    assert_eq!(n["private"], true);
    assert_eq!(n["title"], "");
    assert_eq!(n["body"], "");
    assert_eq!(n["actions"], serde_json::json!([]));
    assert_eq!(n["key"], KEY);
    assert_eq!(n["app"], "Chat");
    assert!(!n.to_string().contains("482913"));
    assert!(n.get("picture").is_none() && n.get("sender").is_none());

    // One that isn't sensitive comes whole.
    phone
        .engine
        .share_app_notification_rich(PKG, &rich("Asha", "hi", serde_json::json!({})));
    let n = next_notification(&pc, 1);
    assert_eq!(n["title"], "Asha");
    assert!(n.get("private").is_none());
}

#[test]
fn only_while_the_phone_is_not_in_use() {
    let (phone, pc) = pair("rich-locked", test_config(), test_config());
    wait_for(|| phone.engine.notif_rich_supported(&pc.node_id()));
    subscribe(&phone, &pc, true, false);
    phone.engine.share_app_notification_rich(
        PKG,
        &rich("In use", "x", serde_json::json!({"in_use": true})),
    );
    phone.engine.share_app_notification_rich(
        PKG,
        &rich("Locked", "y", serde_json::json!({"in_use": false})),
    );
    let n = next_notification(&pc, 0);
    assert_eq!(n["title"], "Locked");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(notifications(&pc), 1);
}

#[test]
fn an_app_icon_is_asked_for_once_named() {
    let (phone, pc) = pair("rich-icon", test_config(), test_config());
    wait_for(|| pc.engine.notif_rich_supported(&phone.node_id()));
    subscribe(&phone, &pc, false, false);
    let icon = jpeg(1200);
    assert!(phone.engine.set_app_icon(PKG, &icon));
    assert!(!phone.engine.set_app_icon(PKG, "bm90IGEganBlZw=="));
    phone
        .engine
        .share_app_notification_rich(PKG, &rich("Asha", "hi", serde_json::json!({})));
    let n = next_notification(&pc, 0);
    let hash = n["icon"].as_str().unwrap().to_string();
    pc.engine
        .request_notification_icon(&phone.node_id(), PKG)
        .unwrap();
    let got = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationIcon {
                from,
                pkg,
                hash,
                data,
            } => Some((from.clone(), pkg.clone(), hash.clone(), data.clone())),
            _ => None,
        })
        .expect("no icon");
    assert_eq!(got, (phone.node_id(), PKG.to_string(), hash, icon));
    // Not for an app it doesn't follow.
    phone.engine.set_app_icon("com.other", &jpeg(300));
    pc.engine
        .request_notification_icon(&phone.node_id(), "com.other")
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let icons = pc
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::NotificationIcon { .. }))
        .count();
    assert_eq!(icons, 1);
}

#[test]
fn a_computer_from_before_gets_the_old_shape_and_no_new_messages() {
    let old = CoreConfig {
        notif_rich: false,
        ..test_config()
    };
    let (phone, pc) = pair("rich-old-pc", test_config(), old);
    // It never says it, so the phone never thinks it does.
    std::thread::sleep(Duration::from_millis(300));
    assert!(!phone.engine.notif_rich_supported(&pc.node_id()));
    // An older computer's subscription: no filters.
    pc.engine
        .subscribe_apps(&phone.node_id(), &format!(r#"["{PKG}"]"#))
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    phone.engine.set_app_icon(PKG, &jpeg(300));
    phone.engine.share_app_notification_rich(
        PKG,
        &rich(
            "Bank",
            "code 1234",
            serde_json::json!({"conv": "c", "sensitive": true, "avatar": jpeg(300), "in_use": true}),
        ),
    );
    let n = next_notification(&pc, 0);
    // Sorted: serde_json keeps insertion order when another crate in the
    // build turns on its preserve_order feature (as a whole-workspace build does).
    let mut keys: Vec<&String> = n.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        ["actions", "app", "body", "from", "key", "title"]
            .iter()
            .collect::<Vec<_>>()
    );
    assert_eq!(n["body"], "code 1234");
    // Neither end sends the icon messages to the other.
    assert!(matches!(
        pc.engine.request_notification_icon(&phone.node_id(), PKG),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        phone.engine.request_notification_icon(&pc.node_id(), PKG),
        Err(CoreError::Unsupported(_))
    ));
    std::thread::sleep(Duration::from_millis(300));
    for node in [&phone, &pc] {
        assert!(!node
            .sink
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::NotificationIcon { .. })
                || matches!(
                    e,
                    CoreEvent::MessageReceived(m) if m.text.contains("notif_")
                )));
    }
}

#[test]
fn a_phone_from_before_still_reaches_a_new_computer() {
    let old = CoreConfig {
        notif_rich: false,
        ..test_config()
    };
    let (phone, pc) = pair("rich-old-phone", old, test_config());
    std::thread::sleep(Duration::from_millis(300));
    assert!(!pc.engine.notif_rich_supported(&phone.node_id()));
    subscribe(&phone, &pc, true, true);
    phone.engine.share_app_notification_with(
        PKG,
        "Chat",
        "Asha",
        "hi",
        Some(KEY),
        r#"[{"title":"Reply","reply":true}]"#,
    );
    let n = next_notification(&pc, 0);
    assert_eq!(n["title"], "Asha");
    assert_eq!(n["actions"][0]["title"], "Reply");
    assert!(n.get("pkg").is_none() && n.get("private").is_none());
}

#[test]
fn subscriptions_are_kept_across_a_restart() {
    let platform = MockPlatform::new("rich-keep-phone");
    let kv = platform.kv.clone();
    let phone = TestNode::start_with(platform);
    let pc = TestNode::start("rich-keep-pc");
    TestNode::pair(&phone, &pc);
    link(&pc, &phone);
    pc.engine
        .subscribe_computer_notifications(&phone.node_id(), true)
        .unwrap();
    wait_for(|| !phone.engine.computer_notification_subscribers().is_empty());
    // The same device starting again finds its subscribers.
    let mut again = MockPlatform::new("rich-keep-phone-2");
    again.kv = kv;
    let restarted = TestNode::start_with(again);
    assert_eq!(
        restarted.engine.computer_notification_subscribers(),
        vec![pc.node_id()]
    );
}
