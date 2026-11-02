//! Sending files between two engines over a real link.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::CoreEvent;

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Bytes that differ chunk to chunk, so a misplaced chunk shows.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 997) as u8).collect()
}

const BIG_WAIT: Duration = Duration::from_secs(60);

fn received(node: &TestNode, id: &str) -> (String, u64) {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileReceived { id: got, path, size, .. } if got == id => {
                Some((path.clone(), *size))
            }
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

#[test]
fn a_file_arrives_intact_under_its_name() {
    let (a, b) = pair("file-small");
    let (src, dst) = (temp_dir("small-src"), temp_dir("small-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let path = src.join("notes.txt");
    std::fs::write(&path, b"hello file").unwrap();

    let id = a.engine.send_file(&b.node_id(), path.to_str().unwrap()).unwrap();
    let (got, size) = received(&b, &id);
    assert_eq!(PathBuf::from(&got), dst.join("notes.txt"));
    assert_eq!(size, 10);
    assert_eq!(std::fs::read(&got).unwrap(), b"hello file");
    assert!(sent(&a, &id), "the sender hears it was saved");

    // The same name again gets a number instead of overwriting.
    let id2 = a.engine.send_file(&b.node_id(), path.to_str().unwrap()).unwrap();
    let (got2, _) = received(&b, &id2);
    assert_eq!(PathBuf::from(got2), dst.join("notes (1).txt"));
}

#[test]
fn a_multi_megabyte_file_goes_through_the_window() {
    let (a, b) = pair("file-big");
    let (src, dst) = (temp_dir("big-src"), temp_dir("big-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(3 * 1024 * 1024 + 12345);
    let path = src.join("video.bin");
    std::fs::write(&path, &data).unwrap();

    let id = a.engine.send_file(&b.node_id(), path.to_str().unwrap()).unwrap();
    let (got, _) = received(&b, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    assert!(sent(&a, &id));
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
    let progress = b
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::FileProgress { id: got, incoming: true, .. } if *got == id))
        .count();
    assert!(progress >= 2, "progress is reported along the way");
}

#[test]
fn a_device_without_a_downloads_folder_declines() {
    let (a, b) = pair("file-declined");
    let src = temp_dir("declined-src");
    let path = src.join("x.txt");
    std::fs::write(&path, b"x").unwrap();
    let id = a.engine.send_file(&b.node_id(), path.to_str().unwrap()).unwrap();
    let reason = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileFailed { id: got, reason, .. } if *got == id => Some(reason.clone()),
            _ => None,
        })
        .expect("no failure");
    assert!(reason.contains("can't receive"), "{reason}");
}

/// The receiver goes away half-way (app killed) and comes back: the
/// transfer carries on from the partial file instead of starting over.
#[test]
fn an_interrupted_transfer_resumes_where_it_stopped() {
    use md_testkit::MockPlatform;
    use std::sync::Arc;

    let store = Arc::default();
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        ..MockPlatform::new("file-resume-b")
    };
    let a = TestNode::start("file-resume-a");
    let b = TestNode::start_with(device());
    TestNode::pair(&a, &b);
    let (src, dst) = (temp_dir("resume-src"), temp_dir("resume-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(40 * 1024 * 1024);
    let path = src.join("movie.bin");
    std::fs::write(&path, &data).unwrap();

    let id = a.engine.send_file(&b.node_id(), path.to_str().unwrap()).unwrap();
    // Wait until some, but not all, of it is in.
    b.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileProgress { id: got, done, total, .. }
                if *got == id && *done > 1_000_000 && done < total =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("no partial progress");
    drop(b);

    let b = TestNode::start_with(device());
    b.engine.set_download_dir(dst.to_str().unwrap());
    b.engine.upsert_peer(a.as_peer());
    a.engine.upsert_peer(b.as_peer());
    // The first word from the new receiver is the offset it resumes at.
    let resumed_at = b
        .sink
        .wait_until(Duration::from_secs(60), |e| match e {
            CoreEvent::FileProgress { id: got, done, .. } if *got == id => Some(*done),
            _ => None,
        })
        .expect("the transfer never came back");
    assert!(resumed_at > 1_000_000, "restarted from {resumed_at}");
    let (got, _) = received(&b, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    assert!(sent(&a, &id));
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_missing_file_fails_at_once() {
    let (a, b) = pair("file-missing");
    assert!(a.engine.send_file(&b.node_id(), "/no/such/file").is_err());
    assert!(a.engine.send_file("nobody", "/etc/hosts").is_err());
}
