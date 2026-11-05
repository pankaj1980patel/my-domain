//! Quick device actions between your own devices: status, ring, open a link.
//! They ride the reliable message path, so they also reach a device that
//! wasn't connected yet.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::CoreEvent;

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

#[test]
fn a_device_reports_its_status() {
    let (a, b) = pair("status");
    b.engine.set_device_status(
        r#"{"battery":42,"charging":true,"storage":{"free":1000,"total":4000},
            "battery_temp":31,"battery_health":"good",
            "wifi":{"on":true,"connected":true,"signal":3},"bluetooth":false,
            "ram":{"free":2000,"total":8000}}"#,
    );
    a.engine.request_status(&b.node_id()).unwrap();
    let (from, status) = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status } => Some((from.clone(), status.clone())),
            _ => None,
        })
        .expect("no status");
    assert_eq!(from, b.node_id());
    assert_eq!(status["battery"], 42);
    assert_eq!(status["charging"], true);
    assert_eq!(status["storage"]["free"], 1000);
    assert_eq!(status["battery_temp"], 31);
    assert_eq!(status["wifi"]["signal"], 3);
    assert_eq!(status["bluetooth"], false);
    assert_eq!(status["ram"]["total"], 8000);
    assert!(status["platform"].is_string());
    // Nothing shared yet, but it says it can be browsed.
    assert_eq!(status["shared_folders"], 0);
}

#[test]
fn ring_reaches_the_device_once_and_is_not_shown_as_chat() {
    let (a, b) = pair("ring");
    a.engine.ring(&b.node_id()).unwrap();
    let from = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Ring { from } => Some(from.clone()),
            _ => None,
        })
        .expect("no ring");
    assert_eq!(from, a.node_id());
    std::thread::sleep(Duration::from_millis(300));
    let all = b.sink.all();
    assert_eq!(
        all.iter()
            .filter(|e| matches!(e, CoreEvent::Ring { .. }))
            .count(),
        1
    );
    assert!(!all
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn only_web_links_are_sent_and_opened() {
    let (a, b) = pair("open-url");
    assert!(a
        .engine
        .open_url(&b.node_id(), "file:///etc/passwd")
        .is_err());
    a.engine
        .open_url(&b.node_id(), " https://example.com/x ")
        .unwrap();
    let url = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::OpenUrl { url, .. } => Some(url.clone()),
            _ => None,
        })
        .expect("no open-url");
    assert_eq!(url, "https://example.com/x");
}

#[test]
fn a_link_can_name_the_app_to_open_it_in() {
    let (a, b) = pair("open-in");
    assert!(a
        .engine
        .open_url_in(&b.node_id(), "https://youtu.be/x", "not a package")
        .is_err());
    a.engine
        .open_url_in(
            &b.node_id(),
            "https://youtu.be/x",
            "com.google.android.youtube",
        )
        .unwrap();
    let got = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::OpenUrl { url, app, .. } => Some((url.clone(), app.clone())),
            _ => None,
        })
        .expect("no open-url");
    assert_eq!(
        got,
        (
            "https://youtu.be/x".to_string(),
            Some("com.google.android.youtube".to_string())
        )
    );
}

#[test]
fn a_search_reaches_the_device_as_an_action() {
    let (a, b) = pair("search");
    assert!(a.engine.search_in_app(&b.node_id(), "tiktok", "x").is_err());
    assert!(a
        .engine
        .search_in_app(&b.node_id(), "youtube", "  ")
        .is_err());
    a.engine
        .search_in_app(&b.node_id(), "spotify", " Kind of Blue ")
        .unwrap();
    let args = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Action { name, args, .. } if name == "search" => Some(args.clone()),
            _ => None,
        })
        .expect("no search");
    assert_eq!(args["app"], "spotify");
    assert_eq!(args["query"], "Kind of Blue");
}

#[test]
fn an_action_reaches_the_device_with_its_arguments() {
    let (a, b) = pair("action");
    assert!(a.engine.run_action(&b.node_id(), "Bad Name", "").is_err());
    assert!(a
        .engine
        .run_action(&b.node_id(), "flashlight", "{nope")
        .is_err());
    a.engine
        .run_action(&b.node_id(), "flashlight", r#"{"on":true}"#)
        .unwrap();
    let (name, args) = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Action { name, args, .. } => Some((name.clone(), args.clone())),
            _ => None,
        })
        .expect("no action");
    assert_eq!(name, "flashlight");
    assert_eq!(args["on"], true);
}

#[test]
fn a_slide_key_reaches_the_computer() {
    let (a, b) = pair("slides");
    for key in ["next", "previous", "black"] {
        a.engine
            .run_action(&b.node_id(), "slides", &format!(r#"{{"key":"{key}"}}"#))
            .unwrap();
    }
    let mut keys = Vec::new();
    while keys.len() < 3 {
        let k = b
            .sink
            .wait_until(WAIT, |e| match e {
                CoreEvent::Action { name, args, .. } if name == "slides" => {
                    let k = args["key"].as_str().unwrap_or_default().to_string();
                    (!keys.contains(&k)).then_some(k)
                }
                _ => None,
            })
            .expect("not every key arrived");
        keys.push(k);
    }
    keys.sort();
    assert_eq!(keys, ["black", "next", "previous"]);
}

#[test]
fn a_shared_code_lands_on_the_other_clipboard() {
    let (a, b) = pair("code");
    assert!(a.engine.share_code("hello", None).is_err());
    // No link yet: it connects and delivers.
    a.engine.share_code("482913", Some("Messages")).unwrap();
    let code = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::CodeCopied { code, app, .. } => Some((code.clone(), app.clone())),
            _ => None,
        })
        .expect("no code");
    assert_eq!(code, ("482913".to_string(), Some("Messages".to_string())));
    assert_eq!(b.clip().current().as_deref(), Some("482913"));
}

#[test]
fn a_notification_can_be_answered_from_another_device() {
    let (a, b) = pair("notif-action");
    // b subscribes to a's Messages app, a forwards one with a reply button.
    b.engine.send(&a.node_id(), "hi").unwrap();
    a.sink.wait_for_message("hi");
    b.engine
        .subscribe_apps(&a.node_id(), r#"["com.messages"]"#)
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    a.engine.share_app_notification_with(
        "com.messages",
        "Messages",
        "Asha",
        "Coming tonight?",
        Some("0|com.messages|7"),
        r#"[{"title":"Reply","reply":true},{"title":"Mark as read","reply":false}]"#,
    );
    let (key, actions) = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Notification {
                key: Some(k),
                actions,
                ..
            } => Some((k.clone(), actions.clone())),
            _ => None,
        })
        .expect("no notification");
    assert_eq!(actions[0]["title"], "Reply");
    b.engine
        .notification_action(&a.node_id(), &key, 0, Some("Yes, 8pm"))
        .unwrap();
    let got = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationAction {
                key, index, text, ..
            } => Some((key.clone(), *index, text.clone())),
            _ => None,
        })
        .expect("no action");
    assert_eq!(
        got,
        (
            "0|com.messages|7".to_string(),
            0,
            Some("Yes, 8pm".to_string())
        )
    );
}

#[test]
fn a_notification_gone_on_the_phone_goes_away_on_the_computer() {
    let (phone, pc) = pair("notif-removed");
    pc.engine.send(&phone.node_id(), "hi").unwrap();
    phone.sink.wait_for_message("hi");
    pc.engine
        .subscribe_apps(&phone.node_id(), r#"["com.messages"]"#)
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    phone.engine.share_app_notification_with(
        "com.messages",
        "Messages",
        "Asha",
        "Coming tonight?",
        Some("0|com.messages|7"),
        "[]",
    );
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Notification { key: Some(_), .. } => Some(()),
            _ => None,
        })
        .expect("no notification");
    // An app the computer doesn't follow: nothing to say about it.
    phone
        .engine
        .share_app_notification_removed("com.other", "0|com.other|1");
    phone
        .engine
        .share_app_notification_removed("com.messages", "0|com.messages|7");
    let got = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationRemoved { from, key } => Some((from.clone(), key.clone())),
            _ => None,
        })
        .expect("no removal");
    assert_eq!(got, (phone.node_id(), "0|com.messages|7".to_string()));
    std::thread::sleep(Duration::from_millis(300));
    let removed: Vec<String> = pc
        .sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::NotificationRemoved { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(removed, vec!["0|com.messages|7".to_string()]);
}

#[test]
fn an_older_subscriber_is_never_told_of_removals() {
    let (phone, pc) = pair("notif-removed-old");
    // What an older build sends: no `removals`. To it `notif_removed` would
    // be an unknown message (shown as chat), so it must not get one.
    let old = format!(
        r#"{{"type":"subscribe_apps","from":"{}","apps":["com.messages"]}}"#,
        pc.node_id()
    );
    pc.engine.send(&phone.node_id(), &old).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    phone.engine.share_app_notification_with(
        "com.messages",
        "Messages",
        "Asha",
        "Coming tonight?",
        Some("0|com.messages|7"),
        "[]",
    );
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Notification { key: Some(_), .. } => Some(()),
            _ => None,
        })
        .expect("no notification");
    phone
        .engine
        .share_app_notification_removed("com.messages", "0|com.messages|7");
    std::thread::sleep(Duration::from_millis(800));
    assert!(!pc.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::NotificationRemoved { .. } | CoreEvent::MessageReceived(_)
    )));
}

#[test]
fn a_computers_own_notifications_show_on_the_phone_and_dismiss_back() {
    let (pc, phone) = pair("pc-notifs");
    phone.engine.send(&pc.node_id(), "hi").unwrap();
    pc.sink.wait_for_message("hi");
    phone
        .engine
        .subscribe_computer_notifications(&pc.node_id(), true)
        .unwrap();
    wait_for(|| !pc.engine.computer_notification_subscribers().is_empty());
    assert_eq!(
        pc.engine.computer_notification_subscribers(),
        vec![phone.node_id()]
    );
    pc.engine
        .share_computer_notification("Thunderbird", "Asha", "Lunch?", "fd:42");
    let got = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Notification {
                from,
                title,
                body,
                app,
                key,
                ..
            } => Some((
                from.clone(),
                title.clone(),
                body.clone(),
                app.clone(),
                key.clone(),
            )),
            _ => None,
        })
        .expect("no notification");
    assert_eq!(
        got,
        (
            pc.node_id(),
            "Asha".to_string(),
            "Lunch?".to_string(),
            Some("Thunderbird".to_string()),
            Some("fd:42".to_string())
        )
    );
    // Swiped away on the phone: the computer hears it.
    phone
        .engine
        .notification_action(&pc.node_id(), "fd:42", -1, None)
        .unwrap();
    let action = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationAction { key, index, .. } => Some((key.clone(), *index)),
            _ => None,
        })
        .expect("no dismiss");
    assert_eq!(action, ("fd:42".to_string(), -1));
    // Closed on the computer: gone on the phone.
    pc.engine.share_computer_notification_removed("fd:42");
    let removed = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotificationRemoved { from, key } => Some((from.clone(), key.clone())),
            _ => None,
        })
        .expect("no removal");
    assert_eq!(removed, (pc.node_id(), "fd:42".to_string()));
    // Turned off on the phone: nothing more comes.
    phone
        .engine
        .subscribe_computer_notifications(&pc.node_id(), false)
        .unwrap();
    wait_for(|| pc.engine.computer_notification_subscribers().is_empty());
    pc.engine
        .share_computer_notification("Thunderbird", "Asha", "Again", "fd:43");
    std::thread::sleep(Duration::from_millis(800));
    let shown = phone
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::Notification { .. }))
        .count();
    assert_eq!(shown, 1);
}

#[test]
fn a_computers_notifications_never_reach_a_device_that_didnt_ask() {
    let (pc, old_phone) = pair("pc-notifs-old");
    let other = TestNode::start("pc-notifs-old-c");
    TestNode::pair(&pc, &other);
    // An older phone: it follows an "app" here the old way (a computer
    // never listed any), but never asks for this computer's notifications.
    let old = format!(
        r#"{{"type":"subscribe_apps","from":"{}","apps":["com.messages"]}}"#,
        old_phone.node_id()
    );
    old_phone.engine.send(&pc.node_id(), &old).unwrap();
    other.engine.send(&pc.node_id(), "hi").unwrap();
    pc.sink.wait_for_message("hi");
    std::thread::sleep(Duration::from_millis(500));
    assert!(pc.engine.computer_notification_subscribers().is_empty());
    pc.engine
        .share_computer_notification("Thunderbird", "Asha", "Lunch?", "fd:1");
    pc.engine.share_computer_notification_removed("fd:1");
    std::thread::sleep(Duration::from_millis(800));
    for node in [&old_phone, &other] {
        assert!(!node.sink.all().iter().any(|e| matches!(
            e,
            CoreEvent::Notification { .. }
                | CoreEvent::NotificationRemoved { .. }
                | CoreEvent::MessageReceived(_)
        )));
    }
}

#[test]
fn a_phone_never_shares_its_apps_with_a_computer_notification_subscriber() {
    // The computer-notifications name is no wildcard: a phone producer
    // sends nothing to a device subscribed to it.
    let (phone, pc) = pair("pc-notifs-phone");
    pc.engine.send(&phone.node_id(), "hi").unwrap();
    phone.sink.wait_for_message("hi");
    pc.engine
        .subscribe_computer_notifications(&phone.node_id(), true)
        .unwrap();
    wait_for(|| !phone.engine.computer_notification_subscribers().is_empty());
    phone.engine.share_app_notification_with(
        "com.messages",
        "Messages",
        "Asha",
        "Coming tonight?",
        Some("0|com.messages|7"),
        "[]",
    );
    std::thread::sleep(Duration::from_millis(800));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::Notification { .. })));
}

/// Wait (up to `WAIT`) until `ok()`.
fn wait_for(ok: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !ok() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_computer_that_says_how_to_wake_it_can_be_woken_later() {
    let (phone, pc) = pair("wake");
    assert!(!phone.engine.can_wake(&pc.node_id()));
    assert!(phone.engine.wake_device(&pc.node_id()).is_err());
    pc.engine
        .set_device_status(r#"{"wake":{"mac":"AA:BB:CC:00:11:22"}}"#);
    phone.engine.request_status(&pc.node_id()).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, .. } if *from == pc.node_id() => Some(()),
            _ => None,
        })
        .expect("no status");
    // Remembered, so it works while the computer sleeps and says nothing.
    assert!(phone.engine.can_wake(&pc.node_id()));
    // Sending needs a network that allows broadcasts (CI's macOS machines
    // don't): only "couldn't send" may stop it, never "doesn't know how".
    match phone.engine.wake_device(&pc.node_id()) {
        Ok(()) | Err(mdcore::CoreError::NoLink(_)) => {}
        Err(e) => panic!("wake failed: {e}"),
    }
}
