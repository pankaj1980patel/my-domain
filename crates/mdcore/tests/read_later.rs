//! The "Read later" list shared between your devices
//! (features::read_later): two devices end with the same links, changes
//! made apart merge at the next link, a delete survives, and an app from
//! before never gets a read-later message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::ReadLaterItem;
use mdcore::{CoreConfig, CoreEvent};

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Close the link between `a` and `b`, and wait until both ends know.
fn unlink(a: &TestNode, b: &TestNode) {
    a.engine.disconnect(&b.node_id()).unwrap();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let a_up = a.engine.connected_peers().contains(&b.node_id());
        let b_up = b.engine.connected_peers().contains(&a.node_id());
        if !a_up && !b_up {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the link stayed up");
}

/// Wait until `a` and `b` have the same list; that list.
fn converged(a: &TestNode, b: &TestNode) -> Vec<ReadLaterItem> {
    let deadline = Instant::now() + WAIT * 2;
    loop {
        let (x, y) = (a.engine.read_later(), b.engine.read_later());
        if x == y {
            return x;
        }
        assert!(Instant::now() < deadline, "never the same:\n{x:#?}\n{y:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Wait for a `ReadLaterChanged` on `node` with link `url` matching `f`.
fn wait_item(node: &TestNode, url: &str, f: impl Fn(&ReadLaterItem) -> bool) -> ReadLaterItem {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ReadLaterChanged { items, .. } => {
                items.iter().find(|i| i.url == url && f(i)).cloned()
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no change to {url:?}"))
}

fn events(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::ReadLaterChanged { .. }))
        .count()
}

fn none_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("read_later")
    )));
}

#[test]
fn two_devices_end_with_the_same_list() {
    let phone = TestNode::start("rl-phone");
    let mac = TestNode::start("rl-mac");
    TestNode::pair(&phone, &mac);

    // Saved before there's a link: it goes when the link comes up.
    let article = phone
        .engine
        .save_read_later("https://www.example.com/article", Some("A good article"))
        .unwrap();
    phone
        .engine
        .save_read_later("https://news.site.org/story?id=7", None)
        .unwrap();
    link(&phone, &mac, "hi");
    let got = wait_item(&mac, "https://www.example.com/article", |_| true);
    assert_eq!(got.id, article.id);
    assert_eq!(got.title, "A good article");
    assert_eq!(got.host, "example.com");
    assert_eq!(got.saved_on, "rl-phone");
    assert!(phone.engine.read_later_supported(&mac.node_id()));
    assert!(mac.engine.read_later_supported(&phone.node_id()));
    let list = converged(&phone, &mac);
    assert_eq!(list.len(), 2);
    // No title from the share: its host (nothing is fetched).
    assert!(list.iter().any(|i| i.title == "news.site.org"));

    // Marked read on the computer, over the live link.
    mac.engine.mark_read_later(&article.id, true).unwrap();
    wait_item(&phone, &article.url, |i| i.read);

    // The same link saved on the computer is the same item, unread again.
    let again = mac
        .engine
        .save_read_later("https://www.example.com/article", None)
        .unwrap();
    assert_eq!(again.id, article.id);
    assert_eq!(again.title, "A good article");
    wait_item(&phone, &article.url, |i| !i.read);

    // A delete goes too.
    let story = phone
        .engine
        .read_later()
        .into_iter()
        .find(|i| i.host == "news.site.org")
        .unwrap();
    mac.engine.delete_read_later(&story.id).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::ReadLaterChanged { removed, .. } => {
                removed.contains(&story.id).then_some(())
            }
            _ => None,
        })
        .expect("the delete arrives");
    assert_eq!(converged(&phone, &mac).len(), 1);
    none_as_chat(&phone);
    none_as_chat(&mac);
}

#[test]
fn changes_made_apart_merge_and_a_delete_survives() {
    let a = TestNode::start("rl-apart-a");
    let b = TestNode::start("rl-apart-b");
    TestNode::pair(&a, &b);
    let keep = a.engine.save_read_later("https://x.io/keep", None).unwrap();
    let gone = a.engine.save_read_later("https://x.io/gone", None).unwrap();
    link(&a, &b, "first");
    assert_eq!(converged(&a, &b).len(), 2);
    unlink(&a, &b);

    // Apart: b deletes one and marks the other read; a saves a new one.
    b.engine.delete_read_later(&gone.id).unwrap();
    b.engine.mark_read_later(&keep.id, true).unwrap();
    a.engine
        .save_read_later("https://x.io/new", Some("New"))
        .unwrap();

    link(&a, &b, "again");
    let list = converged(&a, &b);
    let urls: Vec<&str> = list.iter().map(|i| i.url.as_str()).collect();
    assert_eq!(urls, ["https://x.io/new", "https://x.io/keep"]);
    assert!(list[1].read);

    // The delete stays: another round of digests doesn't bring it back.
    unlink(&a, &b);
    link(&b, &a, "third");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(converged(&a, &b).len(), 2);
    assert!(!a.engine.read_later().iter().any(|i| i.id == gone.id));
}

#[test]
fn an_app_from_before_never_gets_the_list() {
    let new = TestNode::start("rl-new");
    let old = TestNode::start_full(
        MockPlatform::new("rl-old"),
        CoreConfig {
            read_later: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    new.engine
        .save_read_later("https://x.io/secret", None)
        .unwrap();
    link(&new, &old, "hello");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("read_later").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without read_later");
    // A link saved over the live link doesn't go either.
    new.engine
        .save_read_later("https://x.io/another", None)
        .unwrap();
    std::thread::sleep(Duration::from_millis(1_000));

    assert!(!new.engine.read_later_supported(&old.node_id()));
    none_as_chat(&old);
    assert_eq!(events(&old), 0);
    assert!(old.engine.read_later().is_empty());
    assert_eq!(events(&new), 0);
}

#[test]
fn only_web_links_are_saved() {
    let node = TestNode::start("rl-only-web");
    assert!(node
        .engine
        .save_read_later("file:///etc/passwd", None)
        .is_err());
    assert!(node
        .engine
        .save_read_later("javascript:alert(1)", None)
        .is_err());
    assert!(node.engine.save_read_later("not a link", None).is_err());
    assert!(node.engine.read_later().is_empty());
}
