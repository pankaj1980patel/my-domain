//! Talking to the registry server: accounts, device registration, peer
//! refresh, reachability checks. Runs the engine's real HTTP client against
//! `md_testkit::FakeRegistry` (an in-memory server with the same routes and
//! JSON as apps/server).

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use md_testkit::{FakeRegistry, MockPlatform, TestNode};
use mdcore::model::Source;
use mdcore::CoreError;

/// A node logged in to `reg` as alice, with the shared encryption key set.
fn alice_on(reg: &FakeRegistry, id: &str) -> TestNode {
    let n = TestNode::start_logged_out(id);
    n.engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

#[test]
fn register_then_login_creates_a_session_and_remembers_it() {
    let reg = FakeRegistry::start();
    let n = TestNode::start_logged_out("reg-register");
    assert!(!n.engine.is_ready());

    n.engine
        .auth_register(&reg.url(), "Alice", "hunter22")
        .unwrap();
    // The server lowercases usernames; the engine adopts the server's answer.
    assert_eq!(n.engine.session_info().username.as_deref(), Some("alice"));
    assert!(!n.engine.is_ready(), "no encryption key yet");

    n.engine.set_encryption_key("family key").unwrap();
    assert!(n.engine.is_ready());

    // Persisted through the platform's key-value store for the next start.
    let saved = n.engine.saved_session();
    assert_eq!(
        (saved.server_url, saved.username),
        (reg.url(), "alice".to_string())
    );
}

#[test]
fn a_restarted_device_is_still_signed_in_until_it_signs_out() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let first = MockPlatform::new("reg-restart");
    let (kv, secrets) = (first.kv.clone(), first.secrets.clone());
    let n = TestNode::start_logged_out_with(first, md_testkit::test_config());
    n.engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    drop(n);

    // Same device, new process: the token and key come back from the
    // secret store, and it re-registers with the server by itself.
    let again = || {
        let mut p = MockPlatform::new("reg-restart");
        (p.kv, p.secrets) = (kv.clone(), secrets.clone());
        TestNode::start_logged_out_with(p, md_testkit::test_config())
    };
    let n = again();
    assert!(n.engine.is_ready());
    assert_eq!(n.engine.session_info().username.as_deref(), Some("alice"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while reg.device("alice", "reg-restart").is_none() {
        assert!(std::time::Instant::now() < deadline, "never re-registered");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Signing out forgets the secrets, so the next start is signed out.
    n.engine.logout();
    drop(n);
    assert!(!again().engine.is_ready());
}

#[test]
fn a_pasted_full_endpoint_is_normalized_to_the_server_base() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = TestNode::start_logged_out("reg-normalize");
    n.engine
        .auth_login(&format!("{}/auth/login", reg.url()), "alice", "hunter22")
        .unwrap();
    assert_eq!(n.engine.session_info().server_url, Some(reg.url()));
}

#[test]
fn wrong_password_shows_the_servers_error_message() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = TestNode::start_logged_out("reg-wrongpw");
    let err = n
        .engine
        .auth_login(&reg.url(), "alice", "nope")
        .unwrap_err();
    assert_eq!(err.to_string(), "invalid username or password");
    assert!(
        n.engine.session_info().server_url.is_none(),
        "no half session"
    );
}

#[test]
fn unreachable_server_is_an_error_not_a_hang_or_panic() {
    let n = TestNode::start_logged_out("reg-down");
    // Port 9 (discard) on loopback: nothing listens, connection is refused.
    assert!(n
        .engine
        .auth_login("http://127.0.0.1:9", "alice", "pw")
        .is_err());
}

#[test]
fn refresh_before_login_fails_with_not_logged_in() {
    let n = TestNode::start_logged_out("reg-nologin");
    assert!(matches!(
        n.engine.refresh_from_server(),
        Err(CoreError::NotLoggedIn)
    ));
}

#[test]
fn devices_of_the_same_user_find_each_other_through_the_registry() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let laptop = alice_on(&reg, "reg-laptop");
    let phone = alice_on(&reg, "reg-phone");

    // Each registers itself and fetches the others.
    laptop.engine.refresh_from_server().unwrap();
    phone.engine.refresh_from_server().unwrap();
    laptop.engine.refresh_from_server().unwrap();

    let peers = laptop.engine.get_peers();
    assert_eq!(peers.len(), 1, "only the phone, never itself: {peers:?}");
    let p = &peers[0];
    assert_eq!(p.node_id, phone.node_id());
    assert_eq!(p.source, Source::Registry);
    assert_eq!(p.tcp_port, phone.engine.identity().tcp_port);

    // The record the server stored carries the platform tag.
    let stored = reg.device("alice", &phone.node_id()).unwrap();
    assert_eq!(stored["platform"], "test");

    // ...and registry-discovered peers can talk, end to end.
    laptop
        .engine
        .send(&phone.node_id(), "found you via the registry")
        .unwrap();
    phone.sink.wait_for_message("found you via the registry");
}

#[test]
fn other_users_devices_are_not_visible() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    reg.add_user("bob", "letmein1");
    let alice = alice_on(&reg, "reg-iso-alice");
    let bob = TestNode::start_logged_out("reg-iso-bob");
    bob.engine
        .auth_login(&reg.url(), "bob", "letmein1")
        .unwrap();

    bob.engine.refresh_from_server().unwrap();
    alice.engine.refresh_from_server().unwrap();
    assert!(alice.engine.get_peers().is_empty());
}

#[test]
fn rotating_the_key_requires_the_account_password() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "reg-rotate");

    let err = n
        .engine
        .update_encryption_key("new key", "wrong")
        .unwrap_err();
    assert_eq!(err.to_string(), "incorrect password");
    n.engine
        .update_encryption_key("new key", "hunter22")
        .unwrap();
    assert!(n.engine.is_ready());
}

#[test]
fn logout_drops_token_and_key() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "reg-logout");
    n.engine.logout();
    assert!(!n.engine.is_ready());
    assert!(!n.engine.session_info().has_key);
    assert!(matches!(
        n.engine.refresh_from_server(),
        Err(CoreError::NotLoggedIn)
    ));
}

#[test]
fn firewall_check_reports_reachability_and_stores_it_on_the_server() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "reg-firewall");
    n.engine.refresh_from_server().unwrap(); // device must exist server-side

    let status = n.engine.firewall_check().unwrap();
    assert!(status.outbound_ok);
    assert!(!status.inbound_blocked, "fake probe says tcp/ws reachable");
    let stored = reg.device("alice", &n.node_id()).unwrap();
    assert_eq!(stored["inbound_blocked"], false);
}

#[test]
fn without_a_custom_signaler_signals_go_to_the_servers_relay() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "reg-signal");
    n.engine
        .send_signal(
            "some-other-device",
            &mdcore::signal::Signal::Bye { sid: "s1".into() },
        )
        .unwrap();
    let sig = &reg.signals()[0];
    assert_eq!(sig["from"], n.node_id());
    assert_eq!(sig["to"], "some-other-device");
    assert_eq!(sig["data"]["type"], "bye");
}

#[test]
fn registry_peers_carry_their_quic_port_and_connect_over_quic() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let laptop = alice_on(&reg, "reg-quic-laptop");
    let phone = alice_on(&reg, "reg-quic-phone");
    phone.engine.refresh_from_server().unwrap();
    laptop.engine.refresh_from_server().unwrap();

    let peer = &laptop.engine.get_peers()[0];
    assert_eq!(peer.quic_port, phone.engine.identity().quic_port);
    assert_ne!(peer.quic_port, 0);

    laptop.engine.connect(&phone.node_id()).unwrap();
    assert_eq!(
        laptop
            .wait_connected(&phone, std::time::Duration::from_secs(10))
            .as_deref(),
        Some("quic")
    );
}

#[test]
fn the_measured_nat_type_reaches_the_other_devices() {
    use md_testkit::{test_config, FakeStun, MockPlatform};
    use mdcore::nat::NatKind;
    use std::time::Duration;

    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    // Two STUN servers that see different public ports: a symmetric NAT.
    let stuns = [
        FakeStun::start("203.0.113.9:40000"),
        FakeStun::start("203.0.113.9:40001"),
    ];
    let config = mdcore::CoreConfig {
        stun_servers: stuns.iter().map(FakeStun::server).collect(),
        ..test_config()
    };
    let phone = TestNode::start_logged_out_with(MockPlatform::new("reg-nat-phone"), config);
    phone
        .engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    phone.wait_nat(NatKind::Symmetric, Duration::from_secs(10));
    phone.engine.refresh_from_server().unwrap();
    assert_eq!(
        reg.device("alice", &phone.node_id()).unwrap()["nat"],
        "symmetric"
    );

    let laptop = alice_on(&reg, "reg-nat-laptop");
    laptop.engine.refresh_from_server().unwrap();
    let seen = laptop
        .engine
        .get_peers()
        .into_iter()
        .find(|p| p.node_id == phone.node_id())
        .unwrap();
    assert_eq!(seen.nat, NatKind::Symmetric);
}
