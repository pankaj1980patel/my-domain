//! Photo backup from a phone to a computer, between two engines over a real
//! link. The test plays the phone's host app: it reports the conditions
//! (Wi-Fi, charging) and hands over new photos, like the Android app does.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::model::{BackupItem, BackupStatus};
use mdcore::CoreEvent;

const BIG_WAIT: Duration = Duration::from_secs(60);

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-backup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn item(key: &str, path: &Path) -> BackupItem {
    BackupItem {
        key: key.into(),
        path: path.to_string_lossy().into_owned(),
        taken: 1_793_700_000, // 2026-11-03 (UTC)
        month: None,
    }
}

/// Wait until the phone's backup state satisfies `ok`.
fn state_until(phone: &TestNode, what: &str, ok: impl Fn(&BackupStatus) -> bool) -> BackupStatus {
    let deadline = Instant::now() + BIG_WAIT;
    loop {
        let s = phone.engine.backup_state();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: state is {s:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn backup_folder(dst: &Path, phone: &TestNode) -> PathBuf {
    dst.join("Phone backup")
        .join(phone.engine.identity().name)
        .join("2026-11")
}

#[test]
fn new_photos_go_to_the_computer_only_on_wifi_and_never_twice() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let phone_platform = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("backup-wifi-phone")
    };
    let pc = TestNode::start("backup-wifi-pc");
    let phone = TestNode::start_with(phone_platform());
    TestNode::pair(&pc, &phone);
    let (src, dst) = (temp_dir("wifi-src"), temp_dir("wifi-dst"));
    pc.engine.set_download_dir(dst.to_str().unwrap());

    // Errors first: no computer, an unknown one.
    assert!(phone.engine.backup_configure(true, "", false).is_err());
    assert!(phone
        .engine
        .backup_configure(true, "nobody", false)
        .is_err());

    let s = phone.engine.backup_state();
    assert!(!s.enabled && s.waiting_for == "off");
    phone
        .engine
        .backup_configure(true, &pc.node_id(), false)
        .unwrap();
    let s = phone.engine.backup_state();
    assert!(s.enabled && s.since > 0, "{s:#?}");
    assert_eq!(s.waiting_for, "wifi");

    let (a, b) = (src.join("IMG_1.jpg"), src.join("VID_2.mp4"));
    std::fs::write(&a, vec![1u8; 200_000]).unwrap();
    std::fs::write(&b, vec![2u8; 300_000]).unwrap();
    assert_eq!(
        phone
            .engine
            .backup_enqueue(vec![item("1:100", &a), item("2:100", &b)]),
        2
    );
    // On mobile data: nothing goes.
    phone.engine.backup_conditions(false, true);
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(phone.engine.backup_state().queued, 2);
    assert!(!dst.join("Phone backup").exists());

    // Wi-Fi: both go, into Phone backup/<phone>/2026-11.
    phone.engine.backup_conditions(true, false);
    let s = state_until(&phone, "both sent", |s| s.sent == 2 && s.queued == 0);
    assert!(s.last_sent_at > 0 && s.last_error.is_none(), "{s:#?}");
    let folder = backup_folder(&dst, &phone);
    assert_eq!(
        std::fs::read(folder.join("IMG_1.jpg")).unwrap().len(),
        200_000
    );
    assert_eq!(
        std::fs::read(folder.join("VID_2.mp4")).unwrap().len(),
        300_000
    );
    // The phone heard about it.
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::BackupState(s) if s.sent == 2 => Some(()),
            _ => None,
        })
        .expect("no backup-state event");

    // The same photos again: skipped.
    assert_eq!(
        phone
            .engine
            .backup_enqueue(vec![item("1:100", &a), item("2:100", &b)]),
        0
    );

    // After a restart the phone still knows what it sent.
    drop(phone);
    let phone = TestNode::start_with(phone_platform());
    TestNode::pair(&pc, &phone);
    let s = phone.engine.backup_state();
    assert!(
        s.enabled && s.sent == 2 && s.target == pc.node_id(),
        "{s:#?}"
    );
    assert_eq!(phone.engine.backup_enqueue(vec![item("1:100", &a)]), 0);
    let c = src.join("IMG_3.jpg");
    std::fs::write(&c, b"third").unwrap();
    assert_eq!(phone.engine.backup_enqueue(vec![item("3:100", &c)]), 1);
    phone.engine.backup_conditions(true, false);
    state_until(&phone, "third sent", |s| s.sent == 3 && s.queued == 0);
    assert_eq!(std::fs::read(folder.join("IMG_3.jpg")).unwrap(), b"third");
    let files = std::fs::read_dir(&folder).unwrap().count();
    assert_eq!(files, 3, "nothing sent twice");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn charging_only_waits_for_the_charger() {
    let pc = TestNode::start("backup-charge-pc");
    let phone = TestNode::start("backup-charge-phone");
    TestNode::pair(&pc, &phone);
    let (src, dst) = (temp_dir("charge-src"), temp_dir("charge-dst"));
    pc.engine.set_download_dir(dst.to_str().unwrap());
    phone
        .engine
        .backup_configure(true, &pc.node_id(), true)
        .unwrap();
    let a = src.join("IMG_1.jpg");
    std::fs::write(&a, b"photo").unwrap();
    let mut it = item("k1", &a);
    it.month = Some("2025-07".into());
    phone.engine.backup_enqueue(vec![it]);
    phone.engine.backup_conditions(true, false);
    assert_eq!(phone.engine.backup_state().waiting_for, "charging");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(phone.engine.backup_state().sent, 0);
    phone.engine.backup_conditions(true, true);
    state_until(&phone, "sent", |s| s.sent == 1);
    // The host's month (its local time) picks the folder.
    let name = phone.engine.identity().name;
    let got = dst
        .join("Phone backup")
        .join(name)
        .join("2025-07")
        .join("IMG_1.jpg");
    assert_eq!(std::fs::read(got).unwrap(), b"photo");

    // Turned off: new photos wait.
    phone
        .engine
        .backup_configure(false, &pc.node_id(), true)
        .unwrap();
    let b = src.join("IMG_2.jpg");
    std::fs::write(&b, b"two").unwrap();
    phone.engine.backup_enqueue(vec![item("k2", &b)]);
    std::thread::sleep(Duration::from_millis(500));
    let s = phone.engine.backup_state();
    assert_eq!((s.sent, s.queued, s.waiting_for.as_str()), (1, 1, "off"));
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Wi-Fi goes away in the middle of a big video: the transfer stops, and
/// when Wi-Fi is back it carries on from where the computer got to.
#[test]
fn a_backup_cut_off_wifi_resumes_where_it_stopped() {
    let pc = TestNode::start("backup-resume-pc");
    let phone = TestNode::start("backup-resume-phone");
    TestNode::pair(&pc, &phone);
    let (src, dst) = (temp_dir("resume-src"), temp_dir("resume-dst"));
    pc.engine.set_download_dir(dst.to_str().unwrap());
    let data: Vec<u8> = (0..40 * 1024 * 1024u32)
        .map(|i| (i * 13 + i / 1001) as u8)
        .collect();
    let video = src.join("VID_big.mp4");
    std::fs::write(&video, &data).unwrap();
    phone
        .engine
        .backup_configure(true, &pc.node_id(), false)
        .unwrap();
    phone.engine.backup_enqueue(vec![item("big", &video)]);
    phone.engine.backup_conditions(true, false);

    state_until(&phone, "partly sent", |s| s.done > 1_000_000);
    phone.engine.backup_conditions(false, false);
    let s = state_until(&phone, "paused", |s| s.current.is_none());
    assert_eq!((s.waiting_for.as_str(), s.sent), ("wifi", 0));
    std::thread::sleep(Duration::from_millis(500));
    let seen = pc.sink.all().len();

    phone.engine.backup_conditions(true, false);
    state_until(&phone, "sent", |s| s.sent == 1);
    // The computer took it up again past the first megabyte.
    let resumed_at = pc.sink.all()[seen..]
        .iter()
        .find_map(|e| match e {
            CoreEvent::FileProgress {
                done,
                incoming: true,
                ..
            } => Some(*done),
            _ => None,
        })
        .expect("no progress after resuming");
    assert!(resumed_at > 1_000_000, "started again from {resumed_at}");
    let got = backup_folder(&dst, &phone).join("VID_big.mp4");
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A phone that lost its memory (reinstalled) offers the same photo again:
/// the computer already has it and says "done" without it being sent.
#[test]
fn a_photo_the_computer_has_is_not_sent_again() {
    let pc = TestNode::start("backup-dedupe-pc");
    let phone = TestNode::start("backup-dedupe-phone");
    TestNode::pair(&pc, &phone);
    let (src, dst) = (temp_dir("dedupe-src"), temp_dir("dedupe-dst"));
    pc.engine.set_download_dir(dst.to_str().unwrap());
    let photo = src.join("IMG_9.jpg");
    std::fs::write(&photo, vec![9u8; 500_000]).unwrap();
    let folder = backup_folder(&dst, &phone);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy(&photo, folder.join("IMG_9.jpg")).unwrap();

    phone
        .engine
        .backup_configure(true, &pc.node_id(), false)
        .unwrap();
    phone.engine.backup_enqueue(vec![item("9:1", &photo)]);
    phone.engine.backup_conditions(true, false);
    state_until(&phone, "counted as sent", |s| s.sent == 1);
    let chunks = pc
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::FileReceived { .. }))
        .count();
    assert_eq!(chunks, 0, "nothing was received");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "no copy");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}
