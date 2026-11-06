//! Insert from phone (features::insert_capture): the computer asks a phone
//! for a photo or a scan, the phone says what it's doing and sends the file
//! under the request's id, either end cancels, a request times out, and an
//! app from before never gets any of it.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::error::CoreError;
use mdcore::{CoreConfig, CoreEvent};

fn start(id: &str, config: CoreConfig) -> TestNode {
    TestNode::start_full(MockPlatform::new(id), config)
}

/// `a` links to `b` (a chat message), and each hears what the other knows.
fn link(a: &TestNode, b: &TestNode) {
    let hello = format!("hello from {}", a.node_id());
    a.engine.send(&b.node_id(), &hello).unwrap();
    b.sink.wait_for_message(&hello);
    for (x, y) in [(a, b), (b, a)] {
        x.engine.request_status(&y.node_id()).unwrap();
        let who = y.node_id();
        x.sink
            .wait_until(WAIT, |e| match e {
                CoreEvent::DeviceStatus { from, .. } if *from == who => Some(()),
                _ => None,
            })
            .expect("no status");
    }
}

fn pair(test: &str, config: CoreConfig) -> (TestNode, TestNode) {
    let phone = start(&format!("{test}-phone"), config.clone());
    let pc = start(&format!("{test}-pc"), config);
    TestNode::pair(&phone, &pc);
    link(&pc, &phone);
    let deadline = Instant::now() + WAIT;
    while !pc.engine.insert_supported(&phone.node_id()) {
        assert!(Instant::now() < deadline, "never heard it knows insert");
        std::thread::sleep(Duration::from_millis(50));
    }
    (phone, pc)
}

/// The phone hears request `id`; returns its mode.
fn wait_request(phone: &TestNode, id: &str) -> String {
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::InsertRequest { id: i, mode, .. } if i == id => Some(mode.clone()),
            _ => None,
        })
        .expect("the request never arrived")
}

fn wait_ended(node: &TestNode, id: &str, within: Duration) -> String {
    node.sink
        .wait_until(within, |e| match e {
            CoreEvent::InsertEnded { id: i, reason, .. } if i == id => Some(reason.clone()),
            _ => None,
        })
        .expect("the request never ended")
}

fn nothing_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("insert_")
    )));
}

#[test]
fn a_photo_goes_from_the_phone_to_the_computer_that_asked() {
    let (phone, pc) = pair("insert-photo", test_config());
    let dir = std::env::temp_dir().join(format!("insert-photo-{}", std::process::id()));
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    pc.engine.set_download_dir(downloads.to_str().unwrap());
    let id = pc.engine.insert_request(&phone.node_id(), "photo").unwrap();
    assert_eq!(wait_request(&phone, &id), "photo");
    let asks = phone.engine.insert_requests();
    assert_eq!(asks.len(), 1);
    assert_eq!(asks[0].id, id);
    assert_eq!(asks[0].from, pc.node_id());

    // The screen is up: the computer hears it.
    phone.engine.insert_opened(&id).unwrap();
    pc.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::InsertState { id: i, state, .. } if *i == id && state == "open" => Some(()),
            _ => None,
        })
        .expect("no open state");

    // The photo goes back under the request's id.
    let file = dir.join("Photo 1.jpg");
    let bytes = b"\xff\xd8 a photo for the computer".to_vec();
    std::fs::write(&file, &bytes).unwrap();
    phone
        .engine
        .insert_send(&id, file.to_str().unwrap())
        .unwrap();
    assert!(phone.engine.insert_requests().is_empty());
    let (mode, path) = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::InsertReceived {
                id: i,
                mode,
                path,
                from,
                ..
            } if *i == id && *from == phone.node_id() => Some((mode.clone(), path.clone())),
            _ => None,
        })
        .expect("the photo never arrived");
    assert_eq!(mode, "photo");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    // Said "sending" first, and not reported as an ordinary file.
    assert!(pc.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::InsertState { id: i, state, .. } if *i == id && state == "sending"
    )));
    assert!(!pc
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::FileReceived { id: i, .. } if *i == id)));
    assert!(pc.engine.insert_pending().is_empty());
    // The request is over: nothing more can be sent for it.
    assert!(phone
        .engine
        .insert_send(&id, file.to_str().unwrap())
        .is_err());
    nothing_as_chat(&phone);
    nothing_as_chat(&pc);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_transfer_that_fails_ends_the_request() {
    // The computer has no downloads folder: it refuses the file.
    let (phone, pc) = pair("insert-failed", test_config());
    let id = pc.engine.insert_request(&phone.node_id(), "scan").unwrap();
    wait_request(&phone, &id);
    let dir = std::env::temp_dir().join(format!("insert-failed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Scan.pdf");
    std::fs::write(&file, b"%PDF-1.4 a scan").unwrap();
    phone
        .engine
        .insert_send(&id, file.to_str().unwrap())
        .unwrap();
    assert_eq!(wait_ended(&pc, &id, WAIT), "failed");
    assert!(pc.engine.insert_pending().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn either_end_can_cancel() {
    let (phone, pc) = pair("insert-cancel", test_config());

    // The computer cancels: the phone closes its screen.
    let id = pc.engine.insert_request(&phone.node_id(), "scan").unwrap();
    assert_eq!(wait_request(&phone, &id), "scan");
    pc.engine.insert_cancel(&id, "cancelled");
    assert_eq!(wait_ended(&phone, &id, WAIT), "cancelled");
    assert!(phone.engine.insert_requests().is_empty());
    assert!(phone.engine.insert_opened(&id).is_err());

    // The phone cancels (no camera permission): the computer hears why.
    let id = pc.engine.insert_request(&phone.node_id(), "photo").unwrap();
    wait_request(&phone, &id);
    phone.engine.insert_cancel(&id, "no_permission");
    assert_eq!(wait_ended(&pc, &id, WAIT), "no_permission");
    assert!(pc.engine.insert_pending().is_empty());

    // A new request to the same phone replaces the one waiting.
    let first = pc.engine.insert_request(&phone.node_id(), "photo").unwrap();
    wait_request(&phone, &first);
    let second = pc.engine.insert_request(&phone.node_id(), "scan").unwrap();
    assert_eq!(wait_ended(&phone, &first, WAIT), "cancelled");
    wait_request(&phone, &second);
    assert_eq!(
        pc.engine
            .insert_pending()
            .into_iter()
            .map(|(id, ..)| id)
            .collect::<Vec<_>>(),
        [second]
    );

    // Only a mode the phone knows.
    assert!(matches!(
        pc.engine.insert_request(&phone.node_id(), "video"),
        Err(CoreError::InvalidInput(_))
    ));
    nothing_as_chat(&phone);
}

#[test]
fn a_request_nobody_answers_times_out_on_both_ends() {
    let config = CoreConfig {
        insert_timeout: Duration::from_secs(1),
        ..test_config()
    };
    let (phone, pc) = pair("insert-timeout", config);
    let id = pc.engine.insert_request(&phone.node_id(), "photo").unwrap();
    wait_request(&phone, &id);
    let within = Duration::from_secs(10);
    assert_eq!(wait_ended(&pc, &id, within), "timeout");
    assert_eq!(wait_ended(&phone, &id, within), "timeout");
    assert!(pc.engine.insert_pending().is_empty());
    assert!(phone.engine.insert_requests().is_empty());
}

#[test]
fn an_app_from_before_never_gets_an_insert_message() {
    let old = CoreConfig {
        insert_capture: false,
        ..test_config()
    };
    let old_phone = start("insert-old-phone", old);
    let pc = start("insert-old-pc", test_config());
    TestNode::pair(&old_phone, &pc);
    link(&pc, &old_phone);
    let says = pc
        .sink
        .all()
        .into_iter()
        .find_map(|e| match e {
            CoreEvent::DeviceStatus { from, status } if from == old_phone.node_id() => {
                Some(status.get("insert_capture").is_some())
            }
            _ => None,
        })
        .unwrap();
    assert!(!says);
    assert!(!pc.engine.insert_supported(&old_phone.node_id()));
    assert!(matches!(
        pc.engine.insert_request(&old_phone.node_id(), "photo"),
        Err(CoreError::Unsupported(_))
    ));
    assert!(pc.engine.insert_pending().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    nothing_as_chat(&old_phone);
}
