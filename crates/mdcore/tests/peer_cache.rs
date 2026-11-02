//! Less dependence on the server: the peer list (with every known address)
//! is cached on the device, so after a restart it reconnects directly. No
//! registry, no signals.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{MockPlatform, TestNode};

#[test]
fn after_a_restart_known_peers_are_reached_without_any_server() {
    let store = Arc::default(); // the device's persistent key-value store
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        ..MockPlatform::new("cache-laptop")
    };
    let phone = TestNode::start("cache-phone");

    let laptop = TestNode::start_with(device());
    TestNode::pair(&laptop, &phone); // learned once (registry / LAN in real life)
    let deadline = Instant::now() + Duration::from_secs(10);
    while !store
        .lock()
        .unwrap()
        .get("peers")
        .is_some_and(|v: &String| v.contains(&phone.node_id()))
    {
        assert!(Instant::now() < deadline, "the peer list was never cached");
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(laptop);

    // Restart: same device, same store, nobody tells it about the phone.
    let laptop = TestNode::start_with(device());
    let known = laptop.engine.get_peers();
    let cached = known
        .iter()
        .find(|p| p.node_id == phone.node_id())
        .expect("phone remembered");
    assert_eq!(cached.quic_port, phone.engine.identity().quic_port);

    laptop
        .engine
        .send(&phone.node_id(), "back after a restart")
        .unwrap();
    phone.sink.wait_for_message("back after a restart");
}
