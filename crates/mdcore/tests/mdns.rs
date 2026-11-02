//! LAN discovery over mDNS / DNS-SD: two engines that know nothing about each
//! other find each other automatically (no "Scan"), then talk.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::model::Source;
use mdcore::CoreConfig;

fn with_mdns(id: &str) -> TestNode {
    with_mdns_key(MockPlatform::new(id), "family key")
}

fn with_mdns_key(platform: MockPlatform, key: &str) -> TestNode {
    let config = CoreConfig {
        mdns: true,
        mdns_loopback: true,
        ..test_config()
    };
    let n = TestNode::start_logged_out_with(platform, config);
    n.engine.dev_login("alice", key).unwrap();
    n
}

fn wait_for_peer(n: &TestNode, other: &TestNode) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !n
        .engine
        .get_peers()
        .iter()
        .any(|p| p.node_id == other.node_id())
    {
        assert!(
            Instant::now() < deadline,
            "never discovered {}",
            other.node_id()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn engines_find_each_other_over_mdns_and_talk() {
    let run = std::process::id();
    let (a, b) = (
        with_mdns(&format!("mdns-a-{run}")),
        with_mdns(&format!("mdns-b-{run}")),
    );

    let deadline = Instant::now() + Duration::from_secs(15);
    let found = loop {
        if let Some(p) = a
            .engine
            .get_peers()
            .into_iter()
            .find(|p| p.node_id == b.node_id())
        {
            break p;
        }
        assert!(Instant::now() < deadline, "a never discovered b over mDNS");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(found.source, Source::Lan);
    assert_eq!(found.quic_port, b.engine.identity().quic_port);
    assert_eq!(found.name, b.engine.identity().name);

    a.engine.send(&b.node_id(), "found you on the LAN").unwrap();
    b.sink.wait_for_message("found you on the LAN");
}

#[test]
fn only_devices_with_the_same_key_are_listed() {
    let run = std::process::id();
    let a = with_mdns(&format!("tag-a-{run}"));
    let b = with_mdns(&format!("tag-b-{run}"));
    // Someone else's device (or a test run) on the same network.
    let stranger = with_mdns_key(MockPlatform::new(&format!("tag-x-{run}")), "another key");

    wait_for_peer(&a, &b);
    std::thread::sleep(Duration::from_secs(2)); // the stranger had time too
    let ids: Vec<String> = a
        .engine
        .get_peers()
        .into_iter()
        .map(|p| p.node_id)
        .collect();
    assert!(
        !ids.contains(&stranger.node_id()),
        "a listed a stranger: {ids:?}"
    );
    assert!(
        stranger.engine.get_peers().is_empty(),
        "the stranger listed us"
    );
}

#[test]
fn devices_found_on_the_lan_are_not_restored_from_the_cache() {
    let run = std::process::id();
    let first = MockPlatform::new(&format!("cache-a-{run}"));
    let kv = first.kv.clone();
    let a = with_mdns_key(first, "family key");
    let b = with_mdns(&format!("cache-b-{run}"));
    wait_for_peer(&a, &b);
    std::thread::sleep(Duration::from_secs(3)); // the peer cache saves every 2 s
    assert!(kv.lock().unwrap().contains_key("peers"), "cache saved");
    drop(a);

    // Restart without LAN discovery: nothing LAN-found comes back.
    let mut again = MockPlatform::new(&format!("cache-a-{run}"));
    again.kv = kv;
    let a = TestNode::start_logged_out_with(again, test_config());
    assert!(
        a.engine.get_peers().iter().all(|p| p.source != Source::Lan),
        "{:?}",
        a.engine.get_peers()
    );
}
