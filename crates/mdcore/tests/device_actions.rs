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
    b.engine.set_device_status(r#"{"battery":42,"charging":true}"#);
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
    assert_eq!(all.iter().filter(|e| matches!(e, CoreEvent::Ring { .. })).count(), 1);
    assert!(!all.iter().any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn only_web_links_are_sent_and_opened() {
    let (a, b) = pair("open-url");
    assert!(a.engine.open_url(&b.node_id(), "file:///etc/passwd").is_err());
    a.engine.open_url(&b.node_id(), " https://example.com/x ").unwrap();
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
fn an_action_reaches_the_device_with_its_arguments() {
    let (a, b) = pair("action");
    assert!(a.engine.run_action(&b.node_id(), "Bad Name", "").is_err());
    assert!(a.engine.run_action(&b.node_id(), "flashlight", "{nope").is_err());
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
