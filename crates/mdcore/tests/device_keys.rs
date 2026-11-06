//! Device keys (Phase 4): a QUIC connection proves which key each side
//! holds, and a device id that turns up with another key is refused.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md_testkit::{MockPlatform, TestNode};
use mdcore::CoreEvent;

#[test]
fn a_device_id_with_another_key_is_refused() {
    let real = MockPlatform::new("keys-a");
    let (real_kv, real_secrets) = (real.kv.clone(), real.secrets.clone());
    let a = TestNode::start_with(real);
    let b = TestNode::start("keys-b");
    TestNode::pair(&a, &b);
    assert_eq!(a.engine.send(&b.node_id(), "first").unwrap(), "QUIC");
    b.sink.wait_for_message("first");
    drop(a);

    // Same node id, but a fresh install's key: b remembers the first one.
    let impostor = TestNode::start("keys-a");
    TestNode::pair(&impostor, &b);
    let _ = impostor.engine.send(&b.node_id(), "from the impostor");
    b.sink
        .assert_no_message("from the impostor", Duration::from_secs(5));
    drop(impostor);

    // The real device, restarted with its stored key, is let in at once
    // (its old connection on b is stale, but b sees the restart in its
    // `Hello` and drops it).
    let again = TestNode::start_with(MockPlatform {
        kv: real_kv,
        secrets: real_secrets,
        ..MockPlatform::new("keys-a")
    });
    TestNode::pair(&again, &b);
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while again.engine.send(&b.node_id(), "back again").is_err() {
        assert!(std::time::Instant::now() < deadline, "never reconnected");
        std::thread::sleep(Duration::from_millis(200));
    }
    b.sink.wait_for_message("back again");
}

/// The `KeyChanged` events `node` got so far: (node id, old, new).
fn key_events(node: &TestNode) -> Vec<(String, String, String)> {
    node.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::KeyChanged {
                node_id,
                old_fingerprint,
                new_fingerprint,
                ..
            } => Some((node_id, old_fingerprint, new_fingerprint)),
            _ => None,
        })
        .collect()
}

/// An install of a device: restarted on the same stores, it has the same key.
struct Install {
    kv: Arc<Mutex<HashMap<String, String>>>,
    secrets: Arc<Mutex<HashMap<String, String>>>,
}

impl Install {
    fn new() -> Install {
        Install {
            kv: Arc::default(),
            secrets: Arc::default(),
        }
    }

    fn start(&self, id: &str) -> TestNode {
        TestNode::start_with(MockPlatform {
            kv: self.kv.clone(),
            secrets: self.secrets.clone(),
            ..MockPlatform::new(id)
        })
    }
}

fn problem_of(node: &TestNode, id: &str) -> Option<String> {
    node.engine
        .links_map()
        .devices
        .into_iter()
        .find(|d| d.node_id == id)
        .and_then(|d| d.problem)
}

#[test]
fn a_reinstalled_device_is_refused_until_the_user_trusts_its_new_key() {
    let id = "kc-a";
    let b_store = Install::new();
    let b = b_store.start("kc-b");
    let first = Install::new();
    let a = first.start(id);
    TestNode::pair(&a, &b);
    assert_eq!(a.engine.send(&b.node_id(), "first").unwrap(), "QUIC");
    b.sink.wait_for_message("first");
    drop(a);

    // Reinstalled: same id, new key. Refused, and b is told once.
    let second = Install::new();
    let a = second.start(id);
    TestNode::pair(&a, &b);
    let _ = a.engine.send(&b.node_id(), "new install");
    let ev = b
        .sink
        .wait_until(Duration::from_secs(10), |e| match e {
            CoreEvent::KeyChanged { node_id, name, .. } => Some((node_id.clone(), name.clone())),
            _ => None,
        })
        .expect("key change event");
    assert_eq!(ev, (id.to_string(), id.to_string()));
    b.sink
        .assert_no_message("new install", Duration::from_secs(2));
    let change = b.engine.key_changes().pop().unwrap();
    assert_eq!(change.node_id, id);
    assert!(!change.dismissed);
    assert_eq!(problem_of(&b, id).as_deref(), Some("key_changed"));
    drop(a);

    // The same new key again: still refused, no second event.
    let a = second.start(id);
    TestNode::pair(&a, &b);
    let _ = a.engine.send(&b.node_id(), "again");
    b.sink.assert_no_message("again", Duration::from_secs(3));
    assert_eq!(key_events(&b).len(), 1, "once per key, not per attempt");
    assert!(b.engine.key_changes()[0].attempts >= 2);

    // "Not now": still refused, not asked again.
    assert!(b.engine.dismiss_key_change(id));
    assert!(b.engine.key_changes()[0].dismissed);
    drop(a);
    let a = second.start(id);
    TestNode::pair(&a, &b);
    let _ = a.engine.send(&b.node_id(), "after not now");
    b.sink
        .assert_no_message("after not now", Duration::from_secs(3));
    assert_eq!(key_events(&b).len(), 1);
    assert_eq!(problem_of(&b, id).as_deref(), Some("key_changed"));

    // Trusting needs the fingerprint shown; then a direct link works.
    assert!(b.engine.trust_new_key(id, "0000 0000 0000 0000").is_err());
    b.engine.trust_new_key(id, &change.new_fingerprint).unwrap();
    assert!(b.engine.key_changes().is_empty());
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut n = 0;
    loop {
        n += 1;
        let text = format!("trusted {n}");
        if a.engine.send(&b.node_id(), &text).ok().as_deref() == Some("QUIC") {
            b.sink.wait_for_message(&text);
            break;
        }
        assert!(Instant::now() < deadline, "never linked directly");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(problem_of(&b, id), None);
    drop(a);

    // A third key is refused again, and asked about again.
    let third = Install::new();
    let a = third.start(id);
    TestNode::pair(&a, &b);
    let _ = a.engine.send(&b.node_id(), "third key");
    let again = b.sink.wait_until(Duration::from_secs(10), |e| match e {
        CoreEvent::KeyChanged {
            old_fingerprint, ..
        } if *old_fingerprint == change.new_fingerprint => Some(()),
        _ => None,
    });
    assert!(again.is_some(), "a third key is asked about");
    b.sink
        .assert_no_message("third key", Duration::from_secs(2));
    assert_eq!(key_events(&b).len(), 2);
    drop(a);

    // Kept across a restart of b.
    drop(b);
    let b = b_store.start("kc-b");
    let kept = b.engine.key_changes();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].old_fingerprint, change.new_fingerprint);
}
