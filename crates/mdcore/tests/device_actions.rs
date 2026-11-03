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
