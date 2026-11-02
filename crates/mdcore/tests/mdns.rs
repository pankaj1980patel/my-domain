//! LAN discovery over mDNS / DNS-SD: two engines that know nothing about each
//! other find each other automatically (no "Scan"), then talk.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::model::Source;
use mdcore::CoreConfig;

fn with_mdns(id: &str) -> TestNode {
    let config = CoreConfig {
        mdns: true,
        mdns_loopback: true,
        ..test_config()
    };
    let n = TestNode::start_logged_out_with(MockPlatform::new(id), config);
    n.engine.dev_login("alice", "family key").unwrap();
    n
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
