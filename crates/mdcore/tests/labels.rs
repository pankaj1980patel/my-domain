//! Device labels shared between your devices (features::labels): a label
//! set on one shows up on the others, never reaches an app from before, and
//! the switch stops both sending and taking.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::SharedLabel;
use mdcore::{CoreConfig, CoreEvent};

/// Wait for a `LabelsChanged` on `node` naming `target` with `nickname`
/// (`None`: cleared).
fn wait_label(node: &TestNode, target: &str, nickname: Option<&str>) -> SharedLabel {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::LabelsChanged { labels } => labels
                .iter()
                .find(|l| l.node == target && l.nickname.as_deref() == nickname)
                .cloned(),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no label {nickname:?} for {target}"))
}

fn label_events(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::LabelsChanged { .. }))
        .count()
}

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

fn no_labels_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("labels")
    )));
}

#[test]
fn two_devices_end_with_the_same_labels() {
    let phone = TestNode::start("labels-phone");
    let mac = TestNode::start("labels-mac");
    TestNode::pair(&phone, &mac);
    assert!(phone.engine.labels_share() && mac.engine.labels_share());

    // Set before there's a link: it goes when the link comes up.
    phone
        .engine
        .set_shared_label(
            &mac.node_id(),
            Some("Work Mac"),
            Some("laptop"),
            Some("blue"),
        )
        .unwrap();
    link(&phone, &mac, "hi");
    let got = wait_label(&mac, &mac.node_id(), Some("Work Mac"));
    assert_eq!(
        (got.icon.as_deref(), got.color.as_deref(), got.by.as_str()),
        (Some("laptop"), Some("blue"), phone.node_id().as_str())
    );
    assert!(phone.engine.labels_supported(&mac.node_id()));
    assert!(mac.engine.labels_supported(&phone.node_id()));

    // The other way, over the live link.
    mac.engine
        .set_shared_label(&phone.node_id(), Some("Pixel"), Some("phone"), None)
        .unwrap();
    wait_label(&phone, &phone.node_id(), Some("Pixel"));

    // A clear goes too.
    mac.engine
        .set_shared_label(&mac.node_id(), None, None, None)
        .unwrap();
    let cleared = wait_label(&phone, &mac.node_id(), None);
    assert!(cleared.is_cleared());

    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(phone.engine.shared_labels(), mac.engine.shared_labels());
    // Setting a label makes no event where it was set (nothing echoes).
    assert!(!mac.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::LabelsChanged { labels } if labels.iter().any(|l| l.node == phone.node_id())
    )));
    no_labels_as_chat(&phone);
    no_labels_as_chat(&mac);
}

#[test]
fn an_app_from_before_never_gets_labels() {
    let new = TestNode::start("labels-new");
    let old = TestNode::start_full(
        MockPlatform::new("labels-old"),
        CoreConfig {
            shared_labels: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    new.engine
        .set_shared_label(&old.node_id(), Some("Old box"), None, None)
        .unwrap();
    link(&new, &old, "hello");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("labels").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without labels");
    // A change over the live link doesn't go either.
    new.engine
        .set_shared_label(&old.node_id(), Some("Older box"), None, None)
        .unwrap();
    std::thread::sleep(Duration::from_millis(1_000));

    assert!(!new.engine.labels_supported(&old.node_id()));
    no_labels_as_chat(&old);
    assert_eq!(label_events(&old), 0);
    // And the old one sends nothing that looks like labels back.
    assert_eq!(label_events(&new), 0);
}

#[test]
fn the_switch_off_neither_sends_nor_takes() {
    let a = TestNode::start("labels-off-a");
    let b = TestNode::start("labels-off-b");
    TestNode::pair(&a, &b);
    b.engine.set_labels_share(false);
    assert!(!b.engine.labels_share());

    a.engine
        .set_shared_label(&b.node_id(), Some("From A"), None, None)
        .unwrap();
    b.engine
        .set_shared_label(&a.node_id(), Some("From B"), None, Some("rose"))
        .unwrap();
    link(&a, &b, "hey");
    std::thread::sleep(Duration::from_millis(1_500));
    assert_eq!(label_events(&a), 0, "b sent while off");
    assert_eq!(label_events(&b), 0, "b took while off");
    assert!(b
        .engine
        .shared_labels()
        .iter()
        .all(|l| l.node != b.node_id()));

    // Back on: b sends what it has, and a answers with what's newer.
    b.engine.set_labels_share(true);
    wait_label(&a, &a.node_id(), Some("From B"));
    wait_label(&b, &b.node_id(), Some("From A"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.engine.shared_labels(), b.engine.shared_labels());
    no_labels_as_chat(&a);
    no_labels_as_chat(&b);
}

#[test]
fn labels_from_before_are_seeded_but_lose_to_new_ones() {
    let a = TestNode::start("labels-seed-a");
    let b = TestNode::start("labels-seed-b");
    TestNode::pair(&a, &b);
    let tv = "some-tv";
    assert!(a.engine.seed_label(tv, Some("Den TV"), Some("tv"), None));
    // Already there: not seeded again.
    assert!(!a.engine.seed_label(tv, Some("Other"), None, None));
    assert!(b.engine.seed_label(tv, Some("Lounge"), None, None));
    b.engine
        .set_shared_label(tv, Some("Living room"), None, None)
        .unwrap();
    link(&a, &b, "yo");
    wait_label(&a, tv, Some("Living room"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.engine.shared_labels(), b.engine.shared_labels());
}
