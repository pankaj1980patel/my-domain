//! Adding a device by QR code or 6-digit code, end to end through the
//! engine's real HTTP client and `md_testkit::FakeRegistry`'s `/pair/*`.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use md_testkit::{test_config, FakeRegistry, MockPlatform, TestNode};
use mdcore::join::JoinLink;

/// A node signed in to `reg` as alice, with the shared key set.
fn alice_on(reg: &FakeRegistry, id: &str) -> TestNode {
    let n = TestNode::start_logged_out(id);
    n.engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

type Secrets = Arc<Mutex<HashMap<String, String>>>;

/// A signed-out node, and its "OS secure store" (where the key is kept).
fn fresh(id: &str) -> (TestNode, Secrets) {
    let p = MockPlatform::new(id);
    let secrets = p.secrets.clone();
    (TestNode::start_logged_out_with(p, test_config()), secrets)
}

fn kept_key(s: &Secrets) -> Option<String> {
    s.lock().unwrap().get("device_key").cloned()
}

/// The key "family key" gives alice, as kept (hex).
fn family_key() -> String {
    mdcore::crypto::derive_key("family key", "alice")
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn scanning_the_qr_signs_the_new_device_in_with_the_same_key() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let laptop = alice_on(&reg, "join-laptop");

    let invite = laptop.engine.join_invite().unwrap();
    assert!(invite.link.starts_with("mydomain://join?v=1&s="));
    assert_eq!(invite.code.len(), 6);
    assert_eq!(invite.expires_in, 300);
    assert!(invite.qr_size >= 21 && !invite.qr_path.is_empty());
    assert!(!invite.link.contains("hunter22"), "never the password");
    // The server got a hash, not the link's secret.
    let link = JoinLink::parse(&invite.link).unwrap();
    assert_eq!(reg.pairing_claim("alice"), Some(link.claim()));
    assert_eq!(laptop.engine.join_status().unwrap().state, "waiting");

    let (pixel, pixel_secrets) = fresh("Pixel");
    pixel.engine.join_with_link(&invite.link).unwrap();
    assert!(pixel.engine.is_ready());
    assert_eq!(
        pixel.engine.session_info().username.as_deref(),
        Some("alice")
    );
    // The laptop's device key, kept for the next start.
    assert_eq!(kept_key(&pixel_secrets), Some(family_key()));
    assert_eq!(pixel.engine.saved_session().server_url, reg.url());

    // The laptop hears who joined, once.
    let s = laptop.engine.join_status().unwrap();
    assert_eq!(
        (s.state.as_str(), s.name.as_deref()),
        ("joined", Some("Pixel"))
    );
    assert_eq!(laptop.engine.join_status().unwrap().state, "none");

    // Single use.
    let again = TestNode::start_logged_out("join-again");
    let err = again
        .engine
        .join_with_link(&invite.link)
        .unwrap_err()
        .to_string();
    assert!(err.contains("expired or was already used"), "{err}");
    assert!(!again.engine.is_ready());
}

#[test]
fn typing_the_code_brings_the_key_too() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let phone = alice_on(&reg, "join-phone");
    let invite = phone.engine.join_invite().unwrap();
    // The server holds only a box it can't open without the code.
    let held = reg.pairing_box("alice").unwrap();
    assert!(!held.contains(&invite.code));

    let (desk, desk_secrets) = fresh("Desk");
    let typed = format!("{} {}", &invite.code[..3], &invite.code[3..]);
    desk.engine.join_with_code(&reg.url(), &typed).unwrap();
    assert!(desk.engine.is_ready());
    assert_eq!(
        desk.engine.session_info().username.as_deref(),
        Some("alice")
    );
    assert_eq!(kept_key(&desk_secrets), Some(family_key()));
    let s = phone.engine.join_status().unwrap();
    assert_eq!(s.name.as_deref(), Some("Desk"));
}

#[test]
fn a_wrong_or_short_code_and_a_damaged_link_sign_nobody_in() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let phone = alice_on(&reg, "join-wrong");
    let invite = phone.engine.join_invite().unwrap();

    let other = TestNode::start_logged_out("join-other");
    let wrong = if invite.code == "000000" {
        "000001"
    } else {
        "000000"
    };
    assert!(other.engine.join_with_code(&reg.url(), wrong).is_err());
    assert!(other.engine.join_with_code(&reg.url(), "12345").is_err());
    assert!(other.engine.join_with_link("https://example.com").is_err());
    let mut link = JoinLink::parse(&invite.link).unwrap();
    link.sealed_key[30] ^= 1;
    let err = other
        .engine
        .join_with_link(&link.to_uri())
        .unwrap_err()
        .to_string();
    assert!(err.contains("damaged"), "{err}");
    assert!(!other.engine.is_ready());
    // The damaged link was refused before the server was asked: the
    // invite still works.
    assert_eq!(phone.engine.join_status().unwrap().state, "waiting");
}

#[test]
fn closing_the_qr_withdraws_the_invite() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let phone = alice_on(&reg, "join-cancel");
    let invite = phone.engine.join_invite().unwrap();
    phone.engine.join_cancel().unwrap();
    assert_eq!(phone.engine.join_status().unwrap().state, "none");
    let late = TestNode::start_logged_out("join-late");
    assert!(late.engine.join_with_link(&invite.link).is_err());
}

#[test]
fn an_older_server_says_to_update_it() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let phone = alice_on(&reg, "join-old");
    reg.without_pairing();
    let err = phone.engine.join_invite().unwrap_err().to_string();
    assert_eq!(err, mdcore::registry::PAIR_OLD_SERVER);
    let other = TestNode::start_logged_out("join-old-2");
    let err = other
        .engine
        .join_with_code(&reg.url(), "123456")
        .unwrap_err()
        .to_string();
    assert_eq!(err, mdcore::registry::PAIR_OLD_SERVER);
}

#[test]
fn inviting_needs_a_session_and_a_key() {
    let n = TestNode::start_logged_out("join-signed-out");
    assert!(n.engine.join_invite().is_err());
    assert!(n.engine.join_status().is_err());
}
