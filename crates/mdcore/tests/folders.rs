//! Sending a whole folder between two engines over a real link: the tree
//! arrives under the receiver's downloads, without hidden files or what
//! symlinks point at.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use md_testkit::TestNode;
use mdcore::CoreEvent;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-folders-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Bytes that differ chunk to chunk, so a misplaced chunk shows.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 997) as u8).collect()
}

const BIG_WAIT: Duration = Duration::from_secs(60);

/// Poll `f` until it gives `Some`.
fn poll<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + BIG_WAIT;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn received(node: &TestNode) -> Vec<PathBuf> {
    node.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::FileReceived { path, .. } => Some(PathBuf::from(path)),
            _ => None,
        })
        .collect()
}

fn sent(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::FileSent { .. }))
        .count()
}

#[test]
fn a_nested_folder_arrives_as_a_tree() {
    let a = TestNode::start("folder-tree-a");
    let b = TestNode::start("folder-tree-b");
    TestNode::pair(&a, &b);
    let (src, dst) = (temp_dir("tree-src"), temp_dir("tree-dst"));
    b.engine.set_download_dir(dst.to_str().unwrap());

    let trip = src.join("Trip");
    std::fs::create_dir_all(trip.join("day 1").join("raw")).unwrap();
    std::fs::create_dir_all(trip.join("empty")).unwrap();
    std::fs::create_dir_all(trip.join(".git")).unwrap();
    std::fs::write(trip.join("notes.txt"), b"top").unwrap();
    // Several chunks, so it really goes the chunked way.
    let big = pattern(300 * 1024);
    std::fs::write(trip.join("day 1").join("photo.bin"), &big).unwrap();
    std::fs::write(trip.join("day 1").join("raw").join("deep.txt"), b"deep").unwrap();
    std::fs::write(trip.join(".DS_Store"), b"hidden").unwrap();
    std::fs::write(trip.join(".git").join("HEAD"), b"hidden").unwrap();
    // Outside the folder, reached only through symlinks: never sent.
    let outside = src.join("secret.txt");
    std::fs::write(&outside, b"secret").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, trip.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&src, trip.join("up")).unwrap();
    }

    let n = a
        .engine
        .send_folder(&b.node_id(), trip.to_str().unwrap())
        .unwrap();
    assert_eq!(n, 3, "the three plain files");
    // Each file needs the other device's answer, so it's still going.
    assert_eq!(a.engine.folders_sending(), 1);

    let got = poll(|| {
        let r = received(&b);
        (r.len() >= 3).then_some(r)
    })
    .unwrap_or_else(|| panic!("not all received; got {:#?}", b.sink.all()));
    let mut got = got;
    got.sort();
    let mut want = vec![
        dst.join("Trip").join("notes.txt"),
        dst.join("Trip").join("day 1").join("photo.bin"),
        dst.join("Trip").join("day 1").join("raw").join("deep.txt"),
    ];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        std::fs::read(dst.join("Trip").join("notes.txt")).unwrap(),
        b"top"
    );
    assert_eq!(
        std::fs::read(dst.join("Trip").join("day 1").join("photo.bin")).unwrap(),
        big
    );
    assert_eq!(
        std::fs::read(dst.join("Trip").join("day 1").join("raw").join("deep.txt")).unwrap(),
        b"deep"
    );
    assert!(
        poll(|| (sent(&a) >= 3).then_some(())).is_some(),
        "the sender hears each was saved"
    );
    assert!(
        poll(|| (a.engine.folders_sending() == 0).then_some(())).is_some(),
        "the folder is done once every file is"
    );

    // Nothing else came: no hidden files, nothing through the links, no
    // empty folder.
    let mut all: Vec<PathBuf> = Vec::new();
    let mut dirs = vec![dst.join("Trip")];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p.clone());
            }
            all.push(p);
        }
    }
    all.sort();
    let mut tree = want.clone();
    tree.push(dst.join("Trip").join("day 1"));
    tree.push(dst.join("Trip").join("day 1").join("raw"));
    tree.sort();
    assert_eq!(all, tree);

    let _ = std::fs::remove_dir_all(&src);
    let _ = std::fs::remove_dir_all(&dst);
}

#[test]
fn only_folders_with_files_can_be_sent_as_folders() {
    let a = TestNode::start("folder-bad-a");
    let b = TestNode::start("folder-bad-b");
    TestNode::pair(&a, &b);
    let src = temp_dir("bad-src");
    let file = src.join("a.txt");
    std::fs::write(&file, b"x").unwrap();
    let empty = src.join("empty");
    std::fs::create_dir_all(empty.join("sub")).unwrap();

    let to = b.node_id();
    assert!(a.engine.send_folder(&to, file.to_str().unwrap()).is_err());
    assert!(a.engine.send_folder(&to, empty.to_str().unwrap()).is_err());
    assert!(a.engine.send_folder(&to, "/no/such/folder").is_err());
    assert!(a
        .engine
        .send_folder("nobody", src.to_str().unwrap())
        .is_err());
    // A folder is still not a file.
    let err = a
        .engine
        .send_file(&to, src.to_str().unwrap())
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a file"), "{err}");

    let _ = std::fs::remove_dir_all(&src);
}
