//! Synced folders between two engines over real links: the first sync
//! both ways, an edit, a delete (to the trash), a rename, a conflict (both
//! kept), a peer too old to sync (never sent a `sync_*` message), a big
//! file whose link drops half way (resumes), and names that differ only by
//! letter case on a disk that can't tell them apart.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use md_testkit::{test_config, MockPlatform, TestNode};
use mdcore::model::SyncPair;
use mdcore::{CoreConfig, CoreEvent};

const WAIT: Duration = Duration::from_secs(60);
const BIG_WAIT: Duration = Duration::from_secs(120);

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-sync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn config(sync: bool) -> CoreConfig {
    CoreConfig {
        folder_sync: sync,
        sync_every: Duration::from_millis(700),
        ..test_config()
    }
}

fn node(id: &str, sync: bool) -> TestNode {
    TestNode::start_full(MockPlatform::new(id), config(sync))
}

/// `asker` asks `of` for its status and waits for the answer.
fn learn_status(asker: &TestNode, of: &TestNode) -> serde_json::Value {
    let since = asker.sink.all().len();
    asker.engine.request_status(&of.node_id()).unwrap();
    let who = of.node_id();
    let deadline = Instant::now() + WAIT;
    loop {
        let found = asker
            .sink
            .all()
            .into_iter()
            .skip(since)
            .find_map(|e| match e {
                CoreEvent::DeviceStatus { from, status } if from == who => Some(status),
                _ => None,
            });
        if let Some(s) = found {
            return s;
        }
        assert!(Instant::now() < deadline, "no status from {who}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn linked(test: &str) -> (TestNode, TestNode) {
    let a = node(&format!("sync-{test}-a"), true);
    let b = node(&format!("sync-{test}-b"), true);
    TestNode::pair(&a, &b);
    assert_eq!(learn_status(&a, &b)["sync"], 1);
    (a, b)
}

/// Poll `f` until it gives `Some`.
fn poll<T>(within: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + within;
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

fn pair_of(n: &TestNode, id: &str) -> Option<SyncPair> {
    n.engine.sync_pairs().into_iter().find(|p| p.id == id)
}

/// `a` syncs `dir_a` with `b`, which accepts into `dir_b`. Returns the id.
fn start_pair(a: &TestNode, b: &TestNode, dir_a: &Path, dir_b: &Path, mode: &str) -> String {
    let id = a
        .engine
        .sync_create(&b.node_id(), dir_a.to_str().unwrap(), mode)
        .unwrap();
    assert_eq!(pair_of(a, &id).unwrap().state, "pending");
    let invited = poll(WAIT, || pair_of(b, &id).filter(|p| p.state == "invited"))
        .unwrap_or_else(|| panic!("no invitation: {:#?}", b.engine.sync_pairs()));
    assert_eq!(invited.peer, a.node_id());
    assert_eq!(invited.path, "");
    b.engine.sync_accept(&id, dir_b.to_str().unwrap()).unwrap();
    id
}

fn wait_content(path: &Path, want: &[u8]) {
    let ok = poll(WAIT, || {
        std::fs::read(path)
            .ok()
            .filter(|got| got == want)
            .map(|_| ())
    });
    assert!(
        ok.is_some(),
        "{} never held {:?} (has {:?})",
        path.display(),
        String::from_utf8_lossy(want),
        std::fs::read(path)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    );
}

fn wait_gone(path: &Path) {
    assert!(
        poll(WAIT, || (!path.exists()).then_some(())).is_some(),
        "{} still there",
        path.display()
    );
}

fn wait_in_sync(n: &TestNode, id: &str) {
    let ok = poll(WAIT, || pair_of(n, id).filter(|p| p.state == "in_sync"));
    assert!(ok.is_some(), "not in sync: {:#?}", pair_of(n, id));
}

/// Every file under `root` (relative, `/`-separated), hidden folders left
/// out.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(".mydomain-") {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy();
                out.push(rel.replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

fn set_mtime(path: &Path, secs: u64) {
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn trashed(root: &Path, rel: &str) -> bool {
    std::fs::read_dir(root.join(".mydomain-trash"))
        .map(|days| days.flatten().any(|d| d.path().join(rel).is_file()))
        .unwrap_or(false)
}

#[test]
fn a_folder_syncs_both_ways_then_edits_deletes_and_renames_follow() {
    let (a, b) = linked("flow");
    let (da, db) = (temp_dir("flow-a"), temp_dir("flow-b"));
    std::fs::create_dir_all(da.join("sub")).unwrap();
    std::fs::write(da.join("a.txt"), b"from a").unwrap();
    std::fs::write(da.join("sub").join("x.txt"), b"deep").unwrap();
    std::fs::write(da.join(".DS_Store"), b"hidden").unwrap();
    std::fs::write(da.join("~$lock.docx"), b"lock").unwrap();
    std::fs::write(db.join("b.txt"), b"from b").unwrap();

    let id = start_pair(&a, &b, &da, &db, "both");
    // First sync, both ways; hidden and lock files stay put.
    wait_content(&db.join("a.txt"), b"from a");
    wait_content(&db.join("sub").join("x.txt"), b"deep");
    wait_content(&da.join("b.txt"), b"from b");
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);
    assert_eq!(tree(&db), ["a.txt", "b.txt", "sub/x.txt"]);
    assert_eq!(tree(&da).len(), 5, "{:?}", tree(&da));
    assert!(!db.join(".DS_Store").exists() && !db.join("~$lock.docx").exists());
    let p = pair_of(&a, &id).unwrap();
    assert_eq!((p.files, p.conflicts, p.mode.as_str()), (3, 0, "both"));
    assert!(p.last_synced > 0);
    assert_eq!(
        pair_of(&b, &id).unwrap().path,
        std::fs::canonicalize(&db).unwrap().to_string_lossy()
    );

    // An edit on one side reaches the other.
    std::fs::write(db.join("a.txt"), b"edited on b, longer").unwrap();
    wait_content(&da.join("a.txt"), b"edited on b, longer");

    // A delete: gone on the other side too, into its trash.
    std::fs::remove_file(da.join("sub").join("x.txt")).unwrap();
    wait_gone(&db.join("sub").join("x.txt"));
    assert!(trashed(&db, "sub/x.txt"), "not in the trash");

    // A rename: a delete and an add.
    std::fs::rename(da.join("b.txt"), da.join("c.txt")).unwrap();
    wait_content(&db.join("c.txt"), b"from b");
    wait_gone(&db.join("b.txt"));
    assert!(trashed(&db, "b.txt"));
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);
    assert_eq!(
        tree(&db),
        tree(&da)
            .into_iter()
            .filter(|p| p.ends_with(".txt") && !p.starts_with('~'))
            .collect::<Vec<_>>()
    );

    // The files arrived like transfers, marked as synced.
    assert!(b.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::FileReceived { sync: true, name, .. } if name == "a.txt"
    )));
    // Nothing showed up as chat on either side.
    for n in [&a, &b] {
        assert!(
            !n.sink
                .all()
                .iter()
                .any(|e| matches!(e, CoreEvent::MessageReceived(_))),
            "a sync message showed as chat"
        );
    }

    // Removing it stops it on both devices; the files stay.
    a.engine.sync_remove(&id).unwrap();
    assert!(pair_of(&a, &id).is_none());
    assert!(poll(WAIT, || pair_of(&b, &id).is_none().then_some(())).is_some());
    assert!(db.join("a.txt").exists() && da.join("a.txt").exists());
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_conflict_keeps_both_copies_on_both_sides() {
    let (a, b) = linked("conflict");
    let (da, db) = (temp_dir("conflict-a"), temp_dir("conflict-b"));
    std::fs::write(da.join("n.txt"), b"v1").unwrap();
    let id = start_pair(&a, &b, &da, &db, "both");
    wait_content(&db.join("n.txt"), b"v1");
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);

    // Both change it while neither syncs.
    a.engine.sync_pause(&id, true).unwrap();
    b.engine.sync_pause(&id, true).unwrap();
    assert_eq!(pair_of(&a, &id).unwrap().state, "paused");
    std::thread::sleep(Duration::from_secs(2)); // a round under way ends
    std::fs::write(da.join("n.txt"), b"a's version").unwrap();
    std::fs::write(db.join("n.txt"), b"b's version!").unwrap();
    // B's change is the older one: A's keeps the name.
    set_mtime(&da.join("n.txt"), 1_750_000_000); // 2025-06-15
    set_mtime(&db.join("n.txt"), 1_735_800_000); // 2025-01-02
    a.engine.sync_pause(&id, false).unwrap();
    b.engine.sync_pause(&id, false).unwrap();

    let copy = format!(
        "n (conflict from {} 2025-01-02).txt",
        b.engine.identity().name
    );
    for d in [&da, &db] {
        wait_content(&d.join("n.txt"), b"a's version");
        wait_content(&d.join(&copy), b"b's version!");
    }
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);
    assert_eq!(tree(&da), tree(&db));
    assert_eq!(tree(&da).len(), 2, "{:?}", tree(&da));
    assert_eq!(pair_of(&a, &id).unwrap().conflicts, 1);
    assert_eq!(pair_of(&b, &id).unwrap().conflicts, 1);
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn send_only_never_takes_the_other_sides_changes() {
    let (a, b) = linked("oneway");
    let (da, db) = (temp_dir("oneway-a"), temp_dir("oneway-b"));
    std::fs::write(da.join("out.txt"), b"goes").unwrap();
    std::fs::write(db.join("stays.txt"), b"stays on b").unwrap();
    let id = start_pair(&a, &b, &da, &db, "send");
    wait_content(&db.join("out.txt"), b"goes");
    assert!(poll(WAIT, || pair_of(&b, &id).filter(|p| p.mode == "receive")).is_some());
    std::thread::sleep(Duration::from_secs(3)); // a few rounds
    assert!(
        !da.join("stays.txt").exists(),
        "a send-only folder took a file"
    );
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn an_app_too_old_to_sync_never_gets_a_sync_message() {
    let a = node("sync-old-a", true);
    let b = node("sync-old-b", false);
    TestNode::pair(&a, &b);
    let st = learn_status(&a, &b);
    assert!(st.get("sync").is_none(), "{st}");
    assert!(!a.engine.sync_supported(&b.node_id()));
    let da = temp_dir("old-a");
    std::fs::write(da.join("a.txt"), b"x").unwrap();
    let err = a
        .engine
        .sync_create(&b.node_id(), da.to_str().unwrap(), "both")
        .unwrap_err()
        .to_string();
    assert!(err.contains("update"), "{err}");
    assert!(a.engine.sync_pairs().is_empty());
    std::thread::sleep(Duration::from_secs(2));
    // The old app would have shown it as a chat message.
    let chat: Vec<String> = b
        .sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::MessageReceived(m) => Some(m.text),
            _ => None,
        })
        .collect();
    assert!(chat.iter().all(|t| !t.contains("sync_")), "{chat:?}");
    // The new app still talks to it otherwise.
    assert!(learn_status(&b, &a).get("sync").is_some());
    let _ = std::fs::remove_dir_all(&da);
}

#[test]
fn a_big_file_resumes_after_the_link_drops() {
    let (a, b) = linked("resume");
    let (da, db) = (temp_dir("resume-a"), temp_dir("resume-b"));
    let data: Vec<u8> = (0..48 * 1024 * 1024)
        .map(|i: usize| (i * 31 + i / 997) as u8)
        .collect();
    std::fs::write(da.join("movie.bin"), &data).unwrap();
    let id = start_pair(&a, &b, &da, &db, "both");

    // Part way: drop the link.
    let tid = b
        .sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileProgress {
                id,
                done,
                total,
                incoming: true,
                ..
            } if *done > 1_000_000 && done < total => Some(id.clone()),
            _ => None,
        })
        .expect("no partial progress");
    a.engine.disconnect(&b.node_id()).unwrap();

    let got = b
        .sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileReceived {
                id,
                path,
                sync: true,
                ..
            } if *id == tid => Some(PathBuf::from(path)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("not received: {:#?}", pair_of(&b, &id)));
    assert_eq!(got, std::fs::canonicalize(&db).unwrap().join("movie.bin"));
    assert!(std::fs::read(&got).unwrap() == data, "content differs");
    let resumed = b.sink.all().iter().any(|e| {
        matches!(
            e,
            CoreEvent::FileProgress { id, done, resumed: true, incoming: true, .. }
                if *id == tid && *done > 1_000_000
        )
    });
    assert!(resumed, "it started over instead of resuming");
    // It got the sender's modification time.
    let when = |p: &Path| {
        std::fs::metadata(p)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    };
    assert_eq!(when(&got), when(&da.join("movie.bin")));
    wait_in_sync(&a, &id);
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

// --- letter case ---------------------------------------------------------------
//
// A Mac's and Windows' disks take "Notes.txt" and "notes.txt" for one file,
// Linux's don't. `sync_fold_case` makes a side behave as on such a disk
// wherever the test runs (a Linux temp folder tells case apart; a Mac's
// already doesn't, which the core finds out by itself).

/// A synced-folder node whose disk is taken to ignore letter case.
fn folding_node(id: &str) -> TestNode {
    let cfg = CoreConfig {
        sync_fold_case: true,
        ..config(true)
    };
    TestNode::start_full(MockPlatform::new(id), cfg)
}

/// Two linked nodes; `fold_a` / `fold_b`: that side's disk ignores case.
fn linked_case(test: &str, fold_a: bool, fold_b: bool) -> (TestNode, TestNode) {
    let make = |side: &str, fold: bool| {
        let id = format!("sync-{test}-{side}");
        if fold {
            folding_node(&id)
        } else {
            node(&id, true)
        }
    };
    let (a, b) = (make("a", fold_a), make("b", fold_b));
    TestNode::pair(&a, &b);
    assert_eq!(learn_status(&a, &b)["sync"], 1);
    (a, b)
}

/// This system's temp folder tells "x" from "X".
fn temp_tells_case_apart() -> bool {
    let d = temp_dir("case-probe");
    std::fs::write(d.join("probe"), b"x").unwrap();
    let apart = !d.join("PROBE").exists();
    let _ = std::fs::remove_dir_all(&d);
    apart
}

/// Synced files that arrived at `n` so far.
fn received(n: &TestNode) -> usize {
    n.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::FileReceived { sync: true, .. }))
        .count()
}

fn wait_renamed(n: &TestNode, id: &str, want: u32) {
    let ok = poll(WAIT, || {
        pair_of(n, id).filter(|p| p.renamed == want && p.state == "in_sync")
    });
    assert!(
        ok.is_some(),
        "not in sync with {want} renamed: {:#?}",
        pair_of(n, id)
    );
}

#[test]
fn names_that_differ_only_by_case_are_both_kept_on_a_disk_that_ignores_case() {
    let (a, b) = linked_case("case", true, true);
    let (da, db) = (temp_dir("case-a"), temp_dir("case-b"));
    std::fs::write(da.join("notes.txt"), b"a's notes").unwrap();
    std::fs::write(db.join("Notes.txt"), b"b's notes, longer").unwrap();
    std::fs::create_dir_all(da.join("Docs")).unwrap();
    std::fs::write(da.join("Docs").join("x.txt"), b"x").unwrap();
    std::fs::create_dir_all(db.join("docs")).unwrap();
    std::fs::write(db.join("docs").join("y.txt"), b"y").unwrap();
    let id = start_pair(&a, &b, &da, &db, "both");

    let (name_a, name_b) = (a.engine.identity().name, b.engine.identity().name);
    let on_a = format!("Notes (case conflict from {name_b}).txt");
    let on_b = format!("notes (case conflict from {name_a}).txt");
    // Neither overwrote the other: each side keeps its own under its name
    // and the other's under a name of its own; files in a folder spelled
    // differently go in the folder as it's spelled there.
    wait_content(&da.join(&on_a), b"b's notes, longer");
    wait_content(&db.join(&on_b), b"a's notes");
    wait_content(&da.join("Docs").join("y.txt"), b"y");
    wait_content(&db.join("docs").join("x.txt"), b"x");
    wait_renamed(&a, &id, 2);
    wait_renamed(&b, &id, 2);
    assert_eq!(std::fs::read(da.join("notes.txt")).unwrap(), b"a's notes");
    assert_eq!(
        std::fs::read(db.join("Notes.txt")).unwrap(),
        b"b's notes, longer"
    );
    assert_eq!(
        tree(&da),
        ["Docs/x.txt", "Docs/y.txt", on_a.as_str(), "notes.txt"]
    );
    assert_eq!(
        tree(&db),
        ["Notes.txt", "docs/x.txt", "docs/y.txt", on_b.as_str()]
    );
    assert_eq!(pair_of(&a, &id).unwrap().conflicts, 0);

    // Not fetched again every round.
    let (got_a, got_b) = (received(&a), received(&b));
    std::thread::sleep(Duration::from_secs(3)); // a few rounds
    assert_eq!(
        (received(&a), received(&b)),
        (got_a, got_b),
        "fetched again"
    );

    // An edit to the renamed copy goes back to the file it stands for.
    std::fs::write(db.join(&on_b), b"a's notes, edited on b").unwrap();
    wait_content(&da.join("notes.txt"), b"a's notes, edited on b");
    // Both sides take it as agreed before the next edit (else that's a
    // conflict, as for any file).
    std::thread::sleep(Duration::from_secs(3));
    // And the other way round.
    std::fs::write(da.join("notes.txt"), b"edited on a again").unwrap();
    wait_content(&db.join(&on_b), b"edited on a again");
    std::thread::sleep(Duration::from_secs(3));

    // The file in the way goes: the renamed copy takes its own name.
    std::fs::remove_file(db.join("Notes.txt")).unwrap();
    wait_gone(&da.join(&on_a));
    assert!(trashed(&da, &on_a), "not in the trash");
    wait_content(&db.join("notes.txt"), b"edited on a again");
    wait_renamed(&a, &id, 1);
    wait_renamed(&b, &id, 1);
    assert_eq!(tree(&db), ["docs/x.txt", "docs/y.txt", "notes.txt"]);
    assert_eq!(tree(&da), ["Docs/x.txt", "Docs/y.txt", "notes.txt"]);
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn two_files_from_a_disk_that_tells_case_apart_both_arrive() {
    if !temp_tells_case_apart() {
        // Can't hold both names here; the test above covers the rest.
        eprintln!("skipped: this system's temp folder ignores case");
        return;
    }
    let (a, b) = linked_case("both-names", false, true);
    let (da, db) = (temp_dir("both-names-a"), temp_dir("both-names-b"));
    std::fs::write(da.join("Notes.txt"), b"upper").unwrap();
    std::fs::write(da.join("notes.txt"), b"lower").unwrap();
    let id = start_pair(&a, &b, &da, &db, "both");

    let kept = format!(
        "notes (case conflict from {}).txt",
        a.engine.identity().name
    );
    wait_content(&db.join("Notes.txt"), b"upper");
    wait_content(&db.join(&kept), b"lower");
    wait_renamed(&b, &id, 1);
    wait_renamed(&a, &id, 0);
    // The renamed copy isn't sent back as a new file.
    assert_eq!(tree(&da), ["Notes.txt", "notes.txt"]);
    assert_eq!(tree(&db), ["Notes.txt", kept.as_str()]);

    // One of them goes there: the other takes its name here.
    std::fs::remove_file(da.join("Notes.txt")).unwrap();
    wait_content(&db.join("notes.txt"), b"lower");
    wait_renamed(&b, &id, 0);
    assert_eq!(tree(&db), ["notes.txt"]);
    assert!(trashed(&db, "Notes.txt"));
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_rename_only_in_case_is_a_rename_on_a_disk_that_ignores_case() {
    let (a, b) = linked_case("case-rename", false, true);
    let (da, db) = (temp_dir("case-rename-a"), temp_dir("case-rename-b"));
    std::fs::write(da.join("Notes.txt"), b"the only notes").unwrap();
    let id = start_pair(&a, &b, &da, &db, "both");
    wait_content(&db.join("Notes.txt"), b"the only notes");
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);
    let got = received(&b);

    std::fs::rename(da.join("Notes.txt"), da.join("notes.txt")).unwrap();
    let renamed = poll(WAIT, || (tree(&db) == ["notes.txt"]).then_some(()));
    assert!(renamed.is_some(), "not renamed: {:?}", tree(&db));
    wait_in_sync(&a, &id);
    wait_in_sync(&b, &id);
    assert_eq!(
        std::fs::read(db.join("notes.txt")).unwrap(),
        b"the only notes"
    );
    // Renamed in place: nothing fetched again, nothing in the trash.
    assert_eq!(received(&b), got, "fetched again");
    assert!(!trashed(&db, "Notes.txt") && !trashed(&db, "notes.txt"));
    assert_eq!(tree(&da), ["notes.txt"]);
    assert_eq!(pair_of(&b, &id).unwrap().renamed, 0);
    for d in [&da, &db] {
        let _ = std::fs::remove_dir_all(d);
    }
}
