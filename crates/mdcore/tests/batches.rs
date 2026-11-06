//! Files sent together as one batch: a folder's tree or several files
//! arrive whole and both sides report one batch; it survives the sender
//! restarting; a damaged file is asked for again; a paused transfer waits;
//! names that would leave the downloads folder are refused; an app from
//! before still gets the files.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::model::BatchItem;
use mdcore::{CoreConfig, CoreEvent};

const WAIT: Duration = Duration::from_secs(90);

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-batch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pattern(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 997 + seed) as u8).collect()
}

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

/// Both know each other's features (status exchanged over a link).
fn introduce(a: &TestNode, b: &TestNode) {
    a.engine.send(&b.node_id(), "hello").unwrap();
    b.sink.wait_for_message("hello");
    let deadline = Instant::now() + WAIT;
    loop {
        if a.engine.transfer_batches_supported(&b.node_id())
            && b.engine.transfer_batches_supported(&a.node_id())
        {
            return;
        }
        let _ = a.engine.request_status(&b.node_id());
        let _ = b.engine.request_status(&a.node_id());
        assert!(
            Instant::now() < deadline,
            "they never said they know batches"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The batch `id` as `node` last reported it, once it reached `state`.
fn batch_state(node: &TestNode, id: Option<&str>, state: &str) -> BatchItem {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TransferBatch { item }
                if id.is_none_or(|i| i == item.id) && item.state == state =>
            {
                Some(item.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no batch {state}; got {:#?}", node.sink.all()))
}

fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

#[test]
fn a_folder_arrives_whole_as_one_batch() {
    let (a, b) = pair("batch-folder");
    introduce(&a, &b);
    let (src, dst) = (temp_dir("folder-src"), temp_dir("folder-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let top = src.join("Trip");
    let files = [
        ("a.txt", pattern(1000, 1)),
        ("day 1/photo.jpg", pattern(300_000, 2)),
        ("day 1/raw/x.bin", pattern(70_000, 3)),
        ("day 2/notes.md", pattern(10, 4)),
    ];
    for (rel, data) in &files {
        write(&top.join(rel), data);
    }
    write(&top.join(".DS_Store"), b"hidden");

    let n = a
        .engine
        .send_folder(&b.node_id(), top.to_str().unwrap())
        .unwrap();
    assert_eq!(n, 4);
    let theirs = batch_state(&b, None, "done");
    let ours = batch_state(&a, Some(&theirs.id), "done");
    assert_eq!(
        (theirs.kind.as_str(), theirs.name.as_str()),
        ("folder", "Trip")
    );
    assert_eq!(
        (theirs.count, theirs.files_done, theirs.files_failed),
        (4, 4, 0)
    );
    assert_eq!((ours.count, ours.files_done), (4, 4));
    let total: u64 = files.iter().map(|(_, d)| d.len() as u64).sum();
    assert_eq!((theirs.total, theirs.done), (total, total));
    assert_eq!(
        theirs.path.as_deref(),
        Some(dst.join("Trip").to_str().unwrap())
    );
    for (rel, data) in &files {
        assert!(
            std::fs::read(dst.join("Trip").join(rel)).unwrap() == *data,
            "{rel}"
        );
    }
    assert!(!dst.join("Trip").join(".DS_Store").exists());
    // Each file's events say which batch it's in.
    let in_batch = b
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::FileProgress { batch: Some(x), .. } if *x == theirs.id))
        .count();
    assert!(in_batch >= 4, "{in_batch}");
    assert_eq!(a.engine.transfer_batches()[0].state, "done");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn several_files_go_as_one_batch() {
    let (a, b) = pair("batch-files");
    introduce(&a, &b);
    let (src, dst) = (temp_dir("files-src"), temp_dir("files-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let mut paths = Vec::new();
    for i in 0..7 {
        let p = src.join(format!("f{i}.bin"));
        write(&p, &pattern(200_000 + i * 1000, i));
        paths.push(p.to_str().unwrap().to_string());
    }
    let started = a.engine.send_batch(&b.node_id(), &paths).unwrap();
    assert_eq!(started.count, 7);
    let theirs = batch_state(&b, Some(&started.id), "done");
    assert_eq!((theirs.kind.as_str(), theirs.files_done), ("files", 7));
    batch_state(&a, Some(&started.id), "done");
    for i in 0..7 {
        let got = std::fs::read(dst.join(format!("f{i}.bin"))).unwrap();
        assert!(got == pattern(200_000 + i * 1000, i), "f{i}");
    }
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_batch_carries_on_after_the_sender_restarts() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("batch-restart-a")
    };
    let a = TestNode::start_with(device());
    let b = TestNode::start("batch-restart-b");
    TestNode::pair(&a, &b);
    introduce(&a, &b);
    let (src, dst) = (temp_dir("restart-src"), temp_dir("restart-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let big = 40 * 1024 * 1024;
    let names = ["big1.bin", "big2.bin", "small.txt"];
    let datas = [pattern(big, 7), pattern(big, 8), pattern(100, 9)];
    let mut paths = Vec::new();
    for (n, d) in names.iter().zip(&datas) {
        let p = src.join(n);
        write(&p, d);
        paths.push(p.to_str().unwrap().to_string());
    }
    let started = a.engine.send_batch(&b.node_id(), &paths).unwrap();
    b.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TransferBatch { item } if item.id == started.id && item.done > 1_000_000 => {
                Some(())
            }
            _ => None,
        })
        .expect("no progress");
    drop(a); // the app is killed

    let a = TestNode::start_with(device());
    TestNode::pair(&a, &b);
    let theirs = batch_state(&b, Some(&started.id), "done");
    assert_eq!(theirs.files_done, 3);
    batch_state(&a, Some(&started.id), "done");
    for (n, d) in names.iter().zip(&datas) {
        assert!(std::fs::read(dst.join(n)).unwrap() == *d, "{n}");
    }
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_damaged_file_is_asked_for_again() {
    let (a, b) = pair("batch-hash");
    let (src, dst) = (temp_dir("hash-src"), temp_dir("hash-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(500_000, 5);
    let path = src.join("doc.pdf");
    write(&path, &data);
    a.engine.test_corrupt_file_chunks(1);
    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    let got: PathBuf = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileReceived { id: x, path, .. } if *x == id => Some(path.into()),
            CoreEvent::FileFailed { id: x, reason, .. } if *x == id => {
                panic!("failed: {reason}")
            }
            _ => None,
        })
        .expect("not received");
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    // It started over once all of it was in.
    let restarts = b
        .sink
        .all()
        .windows(2)
        .filter(|w| {
            matches!(
                (&w[0], &w[1]),
                (CoreEvent::FileProgress { done: d0, .. }, CoreEvent::FileProgress { done: 0, .. }) if *d0 > 0
            )
        })
        .count();
    assert!(restarts >= 1, "{:#?}", b.sink.all());
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_paused_transfer_waits_until_resumed() {
    let (a, b) = pair("batch-pause");
    introduce(&a, &b);
    let (src, dst) = (temp_dir("pause-src"), temp_dir("pause-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(48 * 1024 * 1024, 6);
    let path = src.join("movie.mp4");
    write(&path, &data);
    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    b.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileProgress { id: x, done, .. } if *x == id && *done > 1_000_000 => {
                Some(())
            }
            _ => None,
        })
        .expect("no progress");
    // The receiver pauses it: the sender hears and stops.
    b.engine.pause_file(&id, true).unwrap();
    a.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FilePaused {
                id: x,
                paused: true,
                ..
            } if *x == id => Some(()),
            _ => None,
        })
        .expect("the sender never paused");
    std::thread::sleep(Duration::from_millis(1500));
    let count = |n: &TestNode| {
        n.sink
            .all()
            .iter()
            .filter(|e| matches!(e, CoreEvent::FileProgress { id: x, .. } if *x == id))
            .count()
    };
    let before = count(&b);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(count(&b), before, "it kept going while paused");
    assert!(!b
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::FileReceived { id: x, .. } if *x == id)));
    // The sender resumes it: it carries on and arrives whole.
    a.engine.pause_file(&id, false).unwrap();
    let got: PathBuf = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileReceived { id: x, path, .. } if *x == id => Some(path.into()),
            _ => None,
        })
        .expect("not received after resuming");
    assert!(std::fs::read(got).unwrap() == data);
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn an_offer_that_would_leave_the_downloads_folder_is_refused() {
    let (a, b) = pair("batch-escape");
    let dst = temp_dir("escape-dst").join("downloads");
    std::fs::create_dir_all(&dst).unwrap();
    b.engine.set_download_dir(dst.to_str().unwrap());
    for (folder, name) in [
        ("../../evil", "../../Trip"),
        ("Trip/../..", "Trip"),
        ("/etc", "x"),
    ] {
        let offer = serde_json::json!({
            "type": "file_offer", "from": a.node_id(), "id": format!("evil{}", folder.len()),
            "name": "../x.txt", "size": 3, "sha256": "00",
            "folder": folder,
            "batch": { "id": "bevil", "name": name, "kind": "folder", "count": 1, "bytes": 3 },
        });
        a.engine.send(&b.node_id(), &offer.to_string()).unwrap();
    }
    std::thread::sleep(Duration::from_secs(2));
    let parent = dst.parent().unwrap();
    assert!(!parent.join("evil").exists());
    assert!(!parent.join("x.txt").exists());
    for e in b.sink.all() {
        match e {
            CoreEvent::FileReceived { .. } | CoreEvent::FileProgress { .. } => {
                panic!("it was taken: {e:?}")
            }
            CoreEvent::TransferBatch { item } => {
                panic!("a batch for a refused file: {item:?}")
            }
            _ => {}
        }
    }
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn an_app_from_before_still_gets_the_folder() {
    let a = TestNode::start("batch-old-a");
    let old = TestNode::start_full(
        MockPlatform::new("batch-old-b"),
        CoreConfig {
            transfer_batches: false,
            ..test_config()
        },
    );
    TestNode::pair(&a, &old);
    a.engine.send(&old.node_id(), "hello").unwrap();
    old.sink.wait_for_message("hello");
    let (src, dst) = (temp_dir("old-src"), temp_dir("old-dst"));
    old.engine.set_download_dir(dst.to_str().unwrap());
    let top = src.join("Album");
    write(&top.join("one.jpg"), &pattern(5000, 1));
    write(&top.join("sub/two.jpg"), &pattern(7000, 2));
    a.engine
        .send_folder(&old.node_id(), top.to_str().unwrap())
        .unwrap();
    let ours = batch_state(&a, None, "done");
    assert_eq!(ours.files_done, 2);
    assert!(std::fs::read(dst.join("Album/one.jpg")).unwrap() == pattern(5000, 1));
    assert!(std::fs::read(dst.join("Album/sub/two.jpg")).unwrap() == pattern(7000, 2));
    assert!(!a.engine.transfer_batches_supported(&old.node_id()));
    // It shows no batch, and can't pause what it gets.
    assert!(!old
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::TransferBatch { .. })));
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}
