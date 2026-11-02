//! Less dependence on the server: the peer list (with every known address)
//! is cached on the device, so after a restart it reconnects directly. No
//! registry, no signals.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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

/// A restarted device comes back on the QUIC port it saved, so peers that
/// only know its old address (no registry refresh) still reach it. A real
/// restart is a new process; here the "previous run" is a saved port.
#[test]
fn a_restarted_device_reuses_its_saved_quic_port() {
    // A port that is free right now.
    let port = std::net::UdpSocket::bind(("::", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let store: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    store
        .lock()
        .unwrap()
        .insert("quic_port".into(), port.to_string());
    let mac = TestNode::start_with(MockPlatform {
        kv: Arc::clone(&store),
        ..MockPlatform::new("port-mac")
    });
    assert_eq!(mac.engine.identity().quic_port, port);

    // A peer that knows only that address reaches it.
    let phone = TestNode::start("port-phone");
    TestNode::pair(&phone, &mac);
    phone
        .engine
        .send(&mac.node_id(), "same port as before")
        .unwrap();
    mac.sink.wait_for_message("same port as before");
}

/// If the saved port is taken, the engine still starts, on another port, and
/// saves that one.
#[test]
fn a_busy_saved_port_falls_back_to_a_free_one() {
    let squatter = std::net::UdpSocket::bind(("::", 0)).unwrap();
    let taken = squatter.local_addr().unwrap().port();
    let store: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    store
        .lock()
        .unwrap()
        .insert("quic_port".into(), taken.to_string());
    let node = TestNode::start_with(MockPlatform {
        kv: Arc::clone(&store),
        ..MockPlatform::new("port-busy")
    });
    let port = node.engine.identity().quic_port;
    assert_ne!(port, taken);
    assert_eq!(
        store.lock().unwrap().get("quic_port"),
        Some(&port.to_string())
    );
}
