//! Device keys (Phase 4): a QUIC connection proves which key each side
//! holds, and a device id that turns up with another key is refused.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{MockPlatform, TestNode};

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
