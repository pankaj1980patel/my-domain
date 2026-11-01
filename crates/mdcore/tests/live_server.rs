//! Against the REAL registry server (not the fakes): HTTPS login, then a chat
//! that can only go through the server's relay over wss://. Ignored by
//! default (needs the network and a test account); run with:
//!
//!   set -a; . scripts/lab/test-creds.env; set +a
//!   cargo test -p mdcore --test live_server -- --ignored --nocapture

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::{ConnectRungs, CoreConfig};

fn creds() -> Option<(String, String, String)> {
    let v = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
    Some((v("MD_DEV_SERVER")?, v("MD_DEV_USER")?, v("MD_DEV_PASS")?))
}

#[test]
#[ignore = "talks to the real server; needs MD_DEV_* (scripts/lab/test-creds.env)"]
fn real_server_https_login_and_relay_over_wss() {
    let Some((server, user, pass)) = creds() else {
        panic!("set MD_DEV_SERVER / MD_DEV_USER / MD_DEV_PASS");
    };
    // Relay only: the two engines must talk through the server.
    let config = || CoreConfig {
        rungs: ConnectRungs {
            relay: true,
            ..ConnectRungs::NONE
        },
        auto_quic: false,
        ..test_config()
    };
    let run = std::process::id();
    let a = TestNode::start_logged_out_with(MockPlatform::new(&format!("live-a-{run}")), config());
    let b = TestNode::start_logged_out_with(MockPlatform::new(&format!("live-b-{run}")), config());
    for n in [&a, &b] {
        n.engine.auth_login(&server, &user, &pass).unwrap();
        n.engine.set_encryption_key("live test key").unwrap();
        n.engine.refresh_from_server().unwrap(); // registers, learns the other
    }
    a.engine.refresh_from_server().unwrap();
    assert!(
        a.engine
            .get_peers()
            .iter()
            .any(|p| p.node_id == b.node_id()),
        "a sees b through the registry"
    );

    // The server would normally invite b with an FCM push (RelayOffer); test
    // engines have no FCM token, so both sides dial the relay themselves.
    b.engine.refresh_from_server().unwrap();
    a.engine.connect(&b.node_id()).unwrap();
    b.engine.connect(&a.node_id()).unwrap();
    if a.wait_connected(&b, Duration::from_secs(20)).as_deref() != Some("relay") {
        let progress: Vec<String> = a
            .sink
            .all()
            .iter()
            .filter_map(|e| match e {
                mdcore::CoreEvent::ConnectProgress { stage, detail, .. } => {
                    Some(format!("{stage} {detail:?}"))
                }
                _ => None,
            })
            .collect();
        panic!("no relay link; ladder said: {progress:?}");
    }
    assert_eq!(
        b.wait_connected(&a, Duration::from_secs(20)).as_deref(),
        Some("relay")
    );
    assert_eq!(
        a.engine.send(&b.node_id(), "hello over wss").unwrap(),
        "relay"
    );
    assert_eq!(b.sink.wait_for_message("hello over wss").protocol, "RELAY");
    b.engine.send(&a.node_id(), "and back").unwrap();
    a.sink.wait_for_message("and back");
    println!("OK: https login + relay over wss via {server}");
}
