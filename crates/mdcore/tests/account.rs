//! Account security against `md_testkit::FakeRegistry`: a session the
//! server no longer takes (401) becomes a `SignedOut` event and a "sign in
//! again" state instead of failing silently; changing the account password
//! keeps this device signed in and signs the others out; "Sign out all
//! devices" with the password signs this one back in.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{FakeRegistry, TestNode};
use mdcore::events::CoreEvent;

fn alice_on(reg: &FakeRegistry, id: &str) -> TestNode {
    let n = TestNode::start_logged_out(id);
    n.engine
        .auth_login(&reg.url(), "alice", "hunter22")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

fn signed_out(n: &TestNode) -> Option<String> {
    n.sink.wait_until(Duration::from_secs(5), |e| match e {
        CoreEvent::SignedOut { username } => Some(username.clone()),
        _ => None,
    })
}

#[test]
fn a_refused_token_signs_out_with_an_event_and_signing_in_again_needs_only_the_password() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "acct-401");
    assert!(n.engine.is_ready());
    assert!(!n.engine.signed_out_elsewhere());

    // The password was changed on another device: this token is dead.
    reg.sign_out_everywhere("alice");
    assert!(n.engine.refresh_from_server().is_err());
    assert_eq!(signed_out(&n).as_deref(), Some("alice"));
    assert!(!n.engine.is_ready(), "the token is forgotten");
    assert!(n.engine.signed_out_elsewhere());
    assert!(
        n.engine.session_info().has_key,
        "the encryption key is kept"
    );

    // The password alone signs back in (server and username are saved).
    assert!(n.engine.sign_in_again("wrong").is_err());
    n.engine.sign_in_again("hunter22").unwrap();
    assert!(n.engine.is_ready());
    assert!(!n.engine.signed_out_elsewhere());
    n.engine.refresh_from_server().unwrap();
}

#[test]
fn a_401_on_a_call_with_another_token_is_not_this_devices_sign_out() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "acct-other");
    // Some other session's token is refused: nothing happens here.
    assert!(mdcore::registry::registry_fetch(&reg.url(), "not-our-token", "").is_err());
    assert!(signed_out_within(&n, Duration::from_millis(400)).is_none());
    assert!(n.engine.is_ready());
}

fn signed_out_within(n: &TestNode, d: Duration) -> Option<String> {
    n.sink.wait_until(d, |e| match e {
        CoreEvent::SignedOut { username } => Some(username.clone()),
        _ => None,
    })
}

#[test]
fn changing_the_password_keeps_this_device_and_signs_out_the_others() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let laptop = alice_on(&reg, "acct-laptop");
    let phone = alice_on(&reg, "acct-phone");

    let err = laptop
        .engine
        .change_account_password("nope", "new-secret")
        .unwrap_err();
    assert_eq!(err.to_string(), mdcore::registry::WRONG_PASSWORD);
    assert!(laptop
        .engine
        .change_account_password("hunter22", "short")
        .is_err());

    laptop
        .engine
        .change_account_password("hunter22", "new-secret")
        .unwrap();
    assert!(laptop.engine.is_ready());
    laptop.engine.refresh_from_server().unwrap();
    assert!(signed_out_within(&laptop, Duration::from_millis(400)).is_none());

    // The phone's next call is refused: it asks to sign in again.
    assert!(phone.engine.refresh_from_server().is_err());
    assert_eq!(signed_out(&phone).as_deref(), Some("alice"));
    assert!(phone.engine.sign_in_again("hunter22").is_err());
    phone.engine.sign_in_again("new-secret").unwrap();
    assert!(phone.engine.is_ready());
}

#[test]
fn sign_out_all_devices_with_the_password_signs_this_one_back_in() {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "hunter22");
    let n = alice_on(&reg, "acct-all");
    assert!(n.engine.sign_out_all_devices("wrong").is_err());
    assert!(n.engine.is_ready(), "a wrong password signs nobody out");

    assert!(n.engine.sign_out_all_devices("hunter22").unwrap());
    assert!(n.engine.is_ready());
    n.engine.refresh_from_server().unwrap();
    let list = n.engine.account_sessions().unwrap();
    assert!(list.iter().any(|s| s.current));

    // Without the password: signed out here too, and asked to sign in.
    assert!(!n.engine.sign_out_all_devices("").unwrap());
    assert!(!n.engine.is_ready());
    assert!(n.engine.signed_out_elsewhere());
    assert!(signed_out(&n).is_some());
}
