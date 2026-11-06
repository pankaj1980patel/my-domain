//! Verification codes on the computer (features::otp_relay): the phone
//! sends just {code, app, issuer, expires_at} to your computers that say
//! `otp_relay`; never to a phone or an older app, never kept past five
//! minutes, never in the chat, clipboard history or a status.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::features::otp_relay::{now_ms, TTL};
use mdcore::model::DeviceKind;
use mdcore::{CoreConfig, CoreEvent};

fn kind(os: &str, form: &str) -> DeviceKind {
    DeviceKind {
        os: os.into(),
        form: form.into(),
        ..DeviceKind::default()
    }
}

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

fn wait_supported(phone: &TestNode, pc: &TestNode, on: bool) {
    let deadline = Instant::now() + WAIT;
    while phone.engine.verification_codes_supported(&pc.node_id()) != on {
        assert!(Instant::now() < deadline, "never heard otp_relay = {on}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A phone and a Mac, paired and linked, each knowing what the other is.
fn phone_and_mac(tag: &str) -> (TestNode, TestNode) {
    let phone = TestNode::start(&format!("otp-{tag}-phone"));
    let mac = TestNode::start(&format!("otp-{tag}-mac"));
    TestNode::pair(&phone, &mac);
    phone.engine.set_device_kind(kind("android", "phone"));
    mac.engine.set_device_kind(kind("macos", "laptop"));
    link(&phone, &mac, "hi");
    wait_supported(&phone, &mac, true);
    (phone, mac)
}

fn codes(node: &TestNode) -> Vec<(String, String, String, u64)> {
    node.sink
        .all()
        .iter()
        .filter_map(|e| match e {
            CoreEvent::VerificationCode(c) => Some((
                c.code.clone(),
                c.app.clone(),
                c.issuer.clone(),
                c.expires_at,
            )),
            _ => None,
        })
        .collect()
}

fn wait_code(node: &TestNode, want: &str) -> (String, String, String, u64) {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(c) = codes(node).into_iter().find(|c| c.0 == want) {
            return c;
        }
        assert!(
            Instant::now() < deadline,
            "no code {want}; got {:#?}",
            node.sink.all()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Nothing about the code reached the chat, the clipboard history or a
/// status.
fn nowhere_else(node: &TestNode, code: &str) {
    for e in node.sink.all() {
        match &e {
            CoreEvent::VerificationCode(_) => {}
            other => {
                let json = other.to_json().to_string();
                assert!(!json.contains(code), "the code leaked into {json}");
            }
        }
    }
    assert!(node
        .engine
        .clipboard_history()
        .iter()
        .all(|c| !c.text.contains(code)));
}

#[test]
fn a_code_reaches_the_computer_with_just_its_four_fields() {
    let (phone, mac) = phone_and_mac("send");
    let before = now_ms();
    let to = phone
        .engine
        .send_verification_code("482-913", "HDFC Bank", "HDFC", &[mac.node_id()])
        .unwrap();
    assert_eq!(to, [mac.node_id()]);
    let (code, app, issuer, expires_at) = wait_code(&mac, "482913");
    assert_eq!(
        (code.as_str(), app.as_str(), issuer.as_str()),
        ("482913", "HDFC Bank", "HDFC")
    );
    let ttl = TTL.as_millis() as u64;
    assert!(expires_at > before && expires_at <= now_ms() + ttl);

    let shown = mac.engine.verification_codes();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].from, phone.node_id());
    nowhere_else(&mac, "482913");
    nowhere_else(&phone, "482913");

    // The same code twice is shown once.
    phone
        .engine
        .send_verification_code("482913", "HDFC Bank", "HDFC", &[mac.node_id()])
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(codes(&mac).len(), 1);

    // Dismissed: gone (and said so).
    let id = shown[0].id.clone();
    assert!(mac.engine.dismiss_verification_code(&id));
    assert!(mac.engine.verification_codes().is_empty());
    assert!(mac
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::VerificationCodeGone { id: gone } if *gone == id)));
    assert!(!mac.engine.dismiss_verification_code(&id));
}

#[test]
fn never_to_a_phone_or_a_device_you_did_not_pair() {
    let phone = TestNode::start("otp-phone2phone-a");
    let tablet = TestNode::start("otp-phone2phone-b");
    TestNode::pair(&phone, &tablet);
    phone.engine.set_device_kind(kind("android", "phone"));
    tablet.engine.set_device_kind(kind("android", "tablet"));
    link(&phone, &tablet, "hi");
    // It says otp_relay (same core), but it's not a computer.
    let deadline = Instant::now() + WAIT;
    while phone.engine.device_kind(&tablet.node_id()).os != "android" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!phone.engine.verification_codes_supported(&tablet.node_id()));
    assert!(phone.engine.verification_code_computers().is_empty());
    let sent = phone
        .engine
        .send_verification_code("482913", "Bank", "", &[tablet.node_id(), "stranger".into()])
        .unwrap();
    assert!(sent.is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert!(codes(&tablet).is_empty());
}

#[test]
fn never_to_an_older_app() {
    let phone = TestNode::start("otp-old-phone");
    let mut cfg: CoreConfig = test_config();
    cfg.otp_relay = false;
    let old = TestNode::start_full(MockPlatform::new("otp-old-pc"), cfg);
    TestNode::pair(&phone, &old);
    phone.engine.set_device_kind(kind("android", "phone"));
    old.engine.set_device_kind(kind("windows", "desktop"));
    link(&phone, &old, "hi");
    let deadline = Instant::now() + WAIT;
    while phone.engine.device_kind(&old.node_id()).os != "windows" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        phone.engine.verification_code_computers(),
        [(old.node_id(), false)]
    );
    assert!(phone
        .engine
        .send_verification_code("482913", "Bank", "", &[old.node_id()])
        .is_err());
    // Even a hand-made one is ignored by an app with it off.
    phone
        .engine
        .send(
            &old.node_id(),
            &format!(
                r#"{{"type":"otp_code","code":"111222","app":"x","expires_at":{}}}"#,
                now_ms() + 60_000
            ),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(codes(&old).is_empty());
    assert!(old.engine.verification_codes().is_empty());
}

#[test]
fn a_code_waits_for_the_link_and_then_goes() {
    let (phone, mac) = phone_and_mac("queue");
    phone.engine.disconnect(&mac.node_id()).unwrap();
    let deadline = Instant::now() + WAIT;
    while phone.engine.connected_peers().contains(&mac.node_id()) {
        assert!(Instant::now() < deadline, "the link never went");
        std::thread::sleep(Duration::from_millis(20));
    }
    let to = phone
        .engine
        .send_verification_code("774411", "Google", "Google", &[mac.node_id()])
        .unwrap();
    assert_eq!(to, [mac.node_id()]);
    // It connects for it, and the code goes once the link is up.
    wait_code(&mac, "774411");
    assert_eq!(phone.engine.verification_codes_waiting(), 0);
}

#[test]
fn an_expired_or_bad_code_is_dropped_and_a_long_one_is_cut_to_five_minutes() {
    let (phone, mac) = phone_and_mac("expiry");
    let raw = |code: &str, at: u64| {
        format!(r#"{{"type":"otp_code","code":"{code}","app":"Bank","expires_at":{at}}}"#)
    };
    let now = now_ms();
    phone
        .engine
        .send(&mac.node_id(), &raw("123456", now.saturating_sub(1_000)))
        .unwrap();
    phone
        .engine
        .send(&mac.node_id(), &raw("12", now + 60_000))
        .unwrap();
    phone
        .engine
        .send(&mac.node_id(), &raw("<script>", now + 60_000))
        .unwrap();
    phone
        .engine
        .send(&mac.node_id(), &raw("909090", now + 3_600_000))
        .unwrap();
    let (_, _, _, expires_at) = wait_code(&mac, "909090");
    assert!(
        expires_at <= now_ms() + TTL.as_millis() as u64,
        "cut to five minutes"
    );
    assert_eq!(codes(&mac).len(), 1, "{:#?}", codes(&mac));
    // None of them shows as a chat message.
    assert!(!mac
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(m) if m.text.contains("otp_code"))));
}

#[test]
fn the_status_says_otp_relay() {
    let (phone, mac) = phone_and_mac("status");
    phone.engine.request_status(&mac.node_id()).unwrap();
    let st = phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status } if *from == mac.node_id() => {
                Some(status.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(st["otp_relay"], 1);
}

#[test]
fn a_flood_of_codes_still_expires_each_one() {
    let (phone, mac) = phone_and_mac("flood");
    let now = now_ms();
    // Many distinct codes (one expiry thread for all, not one each); a
    // few soon-expiring ones among them.
    for i in 0..200u32 {
        let at = if i % 50 == 0 {
            now + 1_500
        } else {
            now + 60_000
        };
        phone
            .engine
            .send(
                &mac.node_id(),
                &format!(
                    r#"{{"type":"otp_code","code":"{}","app":"Bank","expires_at":{at}}}"#,
                    100_000 + i
                ),
            )
            .unwrap();
    }
    link(&phone, &mac, "after the flood");
    assert!(
        mac.engine.verification_codes().len() <= 4,
        "at most four kept"
    );
    // The last code (a long-lived one) stays; a short-lived one sent last
    // goes on time.
    phone
        .engine
        .send(
            &mac.node_id(),
            &format!(
                r#"{{"type":"otp_code","code":"424242","app":"Bank","expires_at":{}}}"#,
                now_ms() + 1_000
            ),
        )
        .unwrap();
    wait_code(&mac, "424242");
    mac.sink
        .wait_until(Duration::from_secs(5), |e| match e {
            CoreEvent::VerificationCodeGone { .. }
                if !mac
                    .engine
                    .verification_codes()
                    .iter()
                    .any(|c| c.code == "424242") =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("the short-lived code expired");
}
