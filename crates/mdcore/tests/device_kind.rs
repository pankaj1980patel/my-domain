//! What kind of device each one is (features::device_kind): a device's
//! kind reaches its peers (status and addr_update) with the right icon,
//! changes follow over a live link, and an older app that says nothing
//! gets a guess or the "unknown" icon.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::DeviceKind;
use mdcore::{CoreConfig, CoreEvent};

fn kind(os: &str, form: &str, model: &str, maker: &str) -> DeviceKind {
    DeviceKind {
        os: os.into(),
        form: form.into(),
        model: model.into(),
        maker: maker.into(),
        icon: String::new(),
    }
}

/// Wait for a `DeviceKind` event on `node` about `target` with `icon`.
fn wait_kind(node: &TestNode, target: &str, icon: &str) -> DeviceKind {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceKind { node, device } if node == target && device.icon == icon => {
                Some(device.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no kind {icon} for {target}"))
}

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

#[test]
fn each_device_learns_what_the_other_is() {
    let phone = TestNode::start("kind-phone");
    let mac = TestNode::start("kind-mac");
    TestNode::pair(&phone, &mac);
    phone
        .engine
        .set_device_kind(kind("android", "foldable", "Galaxy Z Fold5", "Samsung"));
    mac.engine
        .set_device_kind(kind("macos", "laptop", "MacBook Pro", "Apple"));
    assert_eq!(mac.engine.own_device_kind().unwrap().icon, "mac_laptop");

    link(&phone, &mac, "hi");
    let got = wait_kind(&phone, &mac.node_id(), "mac_laptop");
    assert_eq!(
        (got.model.as_str(), got.maker.as_str()),
        ("MacBook Pro", "Apple")
    );
    let got = wait_kind(&mac, &phone.node_id(), "android_foldable");
    assert_eq!(got.os, "android");
    assert_eq!(
        mac.engine.device_kind(&phone.node_id()).icon,
        "android_foldable"
    );
    assert!(mac.engine.device_kinds().contains_key(&phone.node_id()));

    // A change goes over the live link at once.
    mac.engine
        .set_device_kind(kind("macos", "all_in_one", "iMac", "Apple"));
    wait_kind(&phone, &mac.node_id(), "imac");

    // A status reply says it too.
    phone.engine.request_status(&mac.node_id()).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status } if *from == mac.node_id() => {
                (status["device"]["form"] == "all_in_one").then_some(())
            }
            _ => None,
        })
        .expect("status says the kind");
}

#[test]
fn an_older_app_gets_unknown_or_a_guess() {
    // An app from before: says no kind, and (kvm off) no OS either.
    let old = TestNode::start_full(
        MockPlatform::new("kind-old"),
        CoreConfig {
            kvm: false,
            ..test_config()
        },
    );
    let new = TestNode::start("kind-new");
    TestNode::pair(&old, &new);
    new.engine
        .set_device_kind(kind("windows", "desktop", "OptiPlex 7090", "Dell"));
    link(&new, &old, "hello");
    // The older one hears it (the field is just ignored by a real old app).
    wait_kind(&old, &new.node_id(), "win_desktop");
    new.engine.request_status(&old.node_id()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(new.engine.device_kind(&old.node_id()).icon, "unknown");
    assert!(!new.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::DeviceKind { node, .. } if *node == old.node_id()
    )));

    // One that says its OS for mouse and keyboard sharing: a guess.
    let kvm_old = TestNode::start("kind-old-kvm");
    TestNode::pair(&kvm_old, &new);
    link(&new, &kvm_old, "hey");
    let guessed = new
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceKind { node, device } if *node == kvm_old.node_id() => {
                Some(device.clone())
            }
            _ => None,
        })
        .expect("a guess from kvm_os");
    assert_eq!(guessed.os, std::env::consts::OS);
    assert!(guessed.form.is_empty());
}
