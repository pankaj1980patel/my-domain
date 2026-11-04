//! Interrupted file transfers carry on where they stopped: after a dropped
//! link, after the sender restarts, and (whole again) with a receiver from
//! before resuming. A file that changed meanwhile isn't resumed, and a
//! cancel removes the partial file.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use md_testkit::{MockPlatform, TestNode};
use mdcore::CoreEvent;

const BIG_WAIT: Duration = Duration::from_secs(90);
/// Large enough that a link dropped after the first megabyte catches it
/// half-way, even over loopback.
const SIZE: usize = 48 * 1024 * 1024;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-resume-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Bytes that differ chunk to chunk, so a misplaced chunk shows.
fn pattern(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 997 + seed) as u8).collect()
}

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

/// Wait until `node` has confirmed more than a megabyte of `id`, but not
/// all of it.
fn wait_partway(node: &TestNode, id: &str) {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileProgress {
                id: got,
                done,
                total,
                ..
            } if got == id && *done > 1_000_000 && done < total => Some(()),
            _ => None,
        })
        .expect("no partial progress");
}

fn received(node: &TestNode, id: &str) -> PathBuf {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileReceived { id: got, path, .. } if got == id => Some(path.into()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("not received; got {:#?}", node.sink.all()))
}

fn sent(node: &TestNode, id: &str) -> bool {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileSent { id: got, .. } if got == id => Some(()),
            _ => None,
        })
        .is_some()
}

fn failed(node: &TestNode, id: &str) -> String {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileFailed {
                id: got, reason, ..
            } if got == id => Some(reason.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no failure; got {:#?}", node.sink.all()))
}

/// (done, incoming) of every progress event of `id` that says it resumed.
fn resumed_progress(node: &TestNode, id: &str) -> Vec<(u64, bool)> {
    node.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::FileProgress {
                id: got,
                done,
                incoming,
                resumed: true,
                ..
            } if got == id => Some((done, incoming)),
            _ => None,
        })
        .collect()
}

fn partial(dst: &Path, id: &str, ext: &str) -> PathBuf {
    dst.join(".mydomain-partial").join(format!("{id}.{ext}"))
}

#[test]
fn a_dropped_link_resumes_from_the_offset() {
    let (a, b) = pair("resume-link");
    let (src, dst) = (temp_dir("link-src"), temp_dir("link-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(SIZE, 0);
    let path = src.join("movie.bin");
    std::fs::write(&path, &data).unwrap();

    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    wait_partway(&b, &id);
    // While it's going, the partial file is hidden, with its note.
    assert!(partial(&dst, &id, "part").exists());
    assert!(partial(&dst, &id, "json").exists());
    a.engine.disconnect(&b.node_id()).unwrap();

    let got = received(&b, &id);
    assert!(std::fs::read(&got).unwrap() == data, "content differs");
    assert!(sent(&a, &id));
    // Both sides said it carried on, from where it was, not from 0.
    let on_b = resumed_progress(&b, &id);
    let on_a = resumed_progress(&a, &id);
    assert!(
        on_b.first().is_some_and(|(done, _)| *done > 1_000_000),
        "receiver: {on_b:?}"
    );
    assert!(
        on_a.first().is_some_and(|(done, _)| *done > 1_000_000),
        "sender: {on_a:?}"
    );
    assert!(!partial(&dst, &id, "part").exists());
    assert!(!partial(&dst, &id, "json").exists(), "the note goes too");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_restarted_sender_offers_it_again_and_resumes() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("resume-restart-a")
    };
    let a = TestNode::start_with(device());
    let b = TestNode::start("resume-restart-b");
    TestNode::pair(&a, &b);
    let (src, dst) = (temp_dir("restart-src"), temp_dir("restart-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(SIZE, 1);
    let path = src.join("backup.tar");
    std::fs::write(&path, &data).unwrap();

    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    wait_partway(&b, &id);
    drop(a); // the app is killed

    let a = TestNode::start_with(device());
    TestNode::pair(&a, &b);
    let got = received(&b, &id);
    assert!(std::fs::read(&got).unwrap() == data, "content differs");
    assert!(sent(&a, &id), "the new run hears it was saved");
    assert!(
        resumed_progress(&b, &id)
            .first()
            .is_some_and(|(done, _)| *done > 1_000_000),
        "restarted from 0: {:#?}",
        b.sink.all()
    );
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_receiver_without_resume_still_gets_the_whole_file() {
    let (a, b) = pair("resume-legacy");
    b.engine.test_legacy_file_receiver(true);
    let (src, dst) = (temp_dir("legacy-src"), temp_dir("legacy-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(SIZE, 2);
    let path = src.join("old.bin");
    std::fs::write(&path, &data).unwrap();

    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    wait_partway(&b, &id);
    a.engine.disconnect(&b.node_id()).unwrap();

    let got = received(&b, &id);
    assert!(std::fs::read(&got).unwrap() == data, "content differs");
    assert!(sent(&a, &id));
    // It started over (an accept with no offset is from 0).
    assert_eq!(resumed_progress(&a, &id), vec![]);
    assert_eq!(resumed_progress(&b, &id), vec![]);
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_file_changed_while_paused_is_not_resumed() {
    let (a, b) = pair("resume-changed");
    let (src, dst) = (temp_dir("changed-src"), temp_dir("changed-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let path = src.join("draft.doc");
    std::fs::write(&path, pattern(SIZE, 3)).unwrap();

    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    wait_partway(&b, &id);
    a.engine.disconnect(&b.node_id()).unwrap();
    // Edited meanwhile: same size, other bytes.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, pattern(SIZE, 4)).unwrap();

    assert!(failed(&a, &id).contains("changed"));
    assert!(failed(&b, &id).contains("cancelled"));
    assert!(
        !b.sink
            .all()
            .iter()
            .any(|e| matches!(e, CoreEvent::FileReceived { id: got, .. } if *got == id)),
        "nothing was saved"
    );
    assert!(
        !partial(&dst, &id, "part").exists(),
        "the partial file goes"
    );
    assert!(!partial(&dst, &id, "json").exists());
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn cancelling_a_paused_transfer_removes_the_partial_file() {
    let (a, b) = pair("resume-cancel");
    let (src, dst) = (temp_dir("cancel-src"), temp_dir("cancel-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let path = src.join("big.iso");
    std::fs::write(&path, pattern(SIZE, 5)).unwrap();

    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    wait_partway(&b, &id);
    a.engine.disconnect(&b.node_id()).unwrap();
    // Cancelled as the link closes: the receiver hears once there's one
    // again.
    a.engine.cancel_file(&id);
    assert_eq!(failed(&a, &id), "cancelled");
    assert!(failed(&b, &id).contains("cancelled"));
    assert!(
        !partial(&dst, &id, "part").exists(),
        "the partial file goes"
    );
    assert!(!partial(&dst, &id, "json").exists());
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}
