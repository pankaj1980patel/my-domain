//! Browsing another device's shared folders and downloading from them, and
//! sending text to a device, between two engines over a real link.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::model::{RemoteEntry, SharedFolder};
use mdcore::CoreEvent;

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-browse-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn share(node: &TestNode, folders: &[(&str, &Path)]) {
    let folders: Vec<SharedFolder> = folders
        .iter()
        .map(|(name, path)| SharedFolder {
            name: name.to_string(),
            path: path.to_string_lossy().into_owned(),
        })
        .collect();
    node.engine.set_shared_folders(&folders).unwrap();
}

struct Listing {
    entries: Vec<RemoteEntry>,
    next: Option<u64>,
    error: Option<String>,
}

/// Ask `b` for a folder through `a` and wait for the answer to that request.
fn list(a: &TestNode, b: &TestNode, root: &str, path: &str, offset: u64) -> Listing {
    let req = a
        .engine
        .list_folder(&b.node_id(), root, path, offset)
        .unwrap();
    a.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FolderListing {
                req: got,
                from,
                entries,
                next,
                error,
                ..
            } if *got == req => {
                assert_eq!(*from, b.node_id());
                Some(Listing {
                    entries: entries.clone(),
                    next: *next,
                    error: error.clone(),
                })
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no listing; got {:#?}", a.sink.all()))
}

fn names(l: &Listing) -> Vec<String> {
    l.entries.iter().map(|e| e.name.clone()).collect()
}

const BIG_WAIT: Duration = Duration::from_secs(60);

fn received(node: &TestNode, id: &str) -> (String, u64) {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileReceived {
                id: got,
                path,
                size,
                ..
            } if got == id => Some((path.clone(), *size)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("not received; got {:#?}", node.sink.all()))
}

fn failed(node: &TestNode, id: &str) -> String {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileFailed {
                id: got,
                reason,
                incoming: true,
                ..
            } if got == id => Some(reason.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no failure; got {:#?}", node.sink.all()))
}

#[test]
fn a_device_lists_its_shared_folders_and_whats_in_them() {
    let (a, b) = pair("browse-list");
    let (pics, dl) = (temp_dir("list-pics"), temp_dir("list-dl"));
    std::fs::create_dir_all(pics.join("Trip")).unwrap();
    std::fs::write(pics.join("Trip").join("beach.jpg"), b"jpeg").unwrap();
    std::fs::write(pics.join("cat.png"), b"png!!").unwrap();
    std::fs::write(pics.join(".thumbs"), b"hidden").unwrap();
    share(&b, &[("Pictures", &pics), ("Downloads", &dl)]);

    let top = list(&a, &b, "", "", 0);
    assert_eq!(top.error, None);
    assert_eq!(names(&top), ["Pictures", "Downloads"]);
    assert!(top.entries.iter().all(|e| e.dir));

    let inside = list(&a, &b, "Pictures", "", 0);
    assert_eq!(inside.error, None);
    assert_eq!(
        names(&inside),
        ["Trip", "cat.png"],
        "folders first, no hidden files"
    );
    let cat = &inside.entries[1];
    assert_eq!((cat.dir, cat.size), (false, 5));
    assert!(cat.modified > 0);

    let trip = list(&a, &b, "Pictures", "Trip", 0);
    assert_eq!(names(&trip), ["beach.jpg"]);
    for d in [&pics, &dl] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_big_folder_comes_in_pages() {
    let (a, b) = pair("browse-pages");
    let dir = temp_dir("pages");
    let total = mdcore::features::browse::PAGE + 37;
    for i in 0..total {
        std::fs::write(dir.join(format!("f{i:04}.txt")), b"x").unwrap();
    }
    share(&b, &[("Big", &dir)]);
    let first = list(&a, &b, "Big", "", 0);
    assert_eq!(first.entries.len(), mdcore::features::browse::PAGE);
    let next = first.next.expect("there is a second page");
    let second = list(&a, &b, "Big", "", next);
    assert_eq!(second.entries.len(), 37);
    assert_eq!(second.next, None);
    assert_eq!(
        second.entries.last().unwrap().name,
        format!("f{:04}.txt", total - 1)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_outside_a_shared_folder_can_be_listed() {
    let (a, b) = pair("browse-escape");
    let (shared, secret) = (temp_dir("escape-shared"), temp_dir("escape-secret"));
    std::fs::write(secret.join("passwords.txt"), b"hunter2").unwrap();
    share(&b, &[("Shared", &shared)]);

    // The asking side refuses obvious escapes before sending anything.
    assert!(a
        .engine
        .list_folder(&b.node_id(), "Shared", "../x", 0)
        .is_err());

    // A folder that isn't shared, and one that doesn't exist.
    let not_shared = list(&a, &b, "Secret", "", 0);
    assert!(not_shared.error.unwrap().contains("isn't shared"));
    let missing = list(&a, &b, "Shared", "nope", 0);
    assert_eq!(missing.error.as_deref(), Some("not found"));

    // A link inside the shared folder that leads out of it.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&secret, shared.join("way-out")).unwrap();
        let through = list(&a, &b, "Shared", "way-out", 0);
        assert_eq!(through.error.as_deref(), Some("not found"));
        let top = list(&a, &b, "Shared", "", 0);
        assert!(top.entries.is_empty(), "links aren't listed");
    }
    for d in [&shared, &secret] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_device_that_shares_nothing_lists_nothing() {
    let (a, b) = pair("browse-none");
    let top = list(&a, &b, "", "", 0);
    assert!(top.entries.is_empty());
    assert_eq!(top.error, None);
}

#[test]
fn a_file_is_downloaded_from_a_shared_folder() {
    let (a, b) = pair("browse-get");
    let (shared, dst) = (temp_dir("get-shared"), temp_dir("get-dst"));
    std::fs::create_dir_all(shared.join("Docs")).unwrap();
    let data: Vec<u8> = (0..2_500_000u32).map(|i| (i * 7 + i / 991) as u8).collect();
    std::fs::write(shared.join("Docs").join("report.pdf"), &data).unwrap();
    share(&b, &[("Downloads", &shared)]);
    a.engine.set_download_dir(dst.to_str().unwrap());

    let id = a
        .engine
        .download_file(&b.node_id(), "Downloads", "Docs/report.pdf")
        .unwrap();
    // Shown at once as an incoming transfer under that id.
    let first = a.sink.wait_until(WAIT, |e| match e {
        CoreEvent::FileProgress {
            id: got,
            name,
            incoming: true,
            ..
        } if *got == id => Some(name.clone()),
        _ => None,
    });
    assert_eq!(first.as_deref(), Some("report.pdf"));
    let (path, size) = received(&a, &id);
    assert_eq!(PathBuf::from(&path), dst.join("report.pdf"));
    assert_eq!(size, data.len() as u64);
    assert!(std::fs::read(&path).unwrap() == data, "content differs");
    // The sharing device saw it go out and confirmed.
    let sent = b.sink.wait_until(BIG_WAIT, |e| match e {
        CoreEvent::FileSent { id: got, .. } if *got == id => Some(()),
        _ => None,
    });
    assert!(sent.is_some(), "the sharing device hears it was saved");
    for d in [&shared, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// The downloading device goes away half-way (app killed) and comes back:
/// the download carries on from its partial file, like any transfer.
#[test]
fn an_interrupted_download_resumes() {
    use std::sync::Arc;

    let (store, secrets) = (Arc::default(), Arc::default());
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("browse-resume-a")
    };
    let a = TestNode::start_with(device());
    let b = TestNode::start("browse-resume-b");
    TestNode::pair(&a, &b);
    let (shared, dst) = (temp_dir("resume-shared"), temp_dir("resume-dst"));
    let data: Vec<u8> = (0..40 * 1024 * 1024u32)
        .map(|i| (i * 31 + i / 997) as u8)
        .collect();
    std::fs::write(shared.join("movie.mkv"), &data).unwrap();
    share(&b, &[("Videos", &shared)]);
    a.engine.set_download_dir(dst.to_str().unwrap());

    let id = a
        .engine
        .download_file(&b.node_id(), "Videos", "movie.mkv")
        .unwrap();
    a.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileProgress {
                id: got,
                done,
                total,
                ..
            } if *got == id && *done > 1_000_000 && done < total => Some(()),
            _ => None,
        })
        .expect("no partial progress");
    drop(a);

    let a = TestNode::start_with(device());
    a.engine.set_download_dir(dst.to_str().unwrap());
    a.engine.upsert_peer(b.as_peer());
    b.engine.upsert_peer(a.as_peer());
    let resumed_at = a
        .sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileProgress { id: got, done, .. } if *got == id => Some(*done),
            _ => None,
        })
        .expect("the download never came back");
    assert!(resumed_at > 1_000_000, "restarted from {resumed_at}");
    let (path, _) = received(&a, &id);
    assert!(std::fs::read(path).unwrap() == data, "content differs");
    for d in [&shared, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_download_that_cant_happen_fails_with_a_reason() {
    let (a, b) = pair("browse-get-bad");
    let (shared, secret, dst) = (
        temp_dir("getbad-shared"),
        temp_dir("getbad-secret"),
        temp_dir("getbad-dst"),
    );
    std::fs::create_dir_all(shared.join("Folder")).unwrap();
    std::fs::write(secret.join("key.pem"), b"secret").unwrap();
    share(&b, &[("Shared", &shared)]);

    // No downloads folder yet: refused here.
    assert!(a
        .engine
        .download_file(&b.node_id(), "Shared", "x.txt")
        .is_err());
    a.engine.set_download_dir(dst.to_str().unwrap());
    // Not a path inside a shared folder.
    assert!(a
        .engine
        .download_file(&b.node_id(), "Shared", "../x")
        .is_err());
    assert!(a.engine.download_file(&b.node_id(), "Shared", "").is_err());

    let id = a
        .engine
        .download_file(&b.node_id(), "Shared", "missing.txt")
        .unwrap();
    assert_eq!(failed(&a, &id), "not found");
    let id = a
        .engine
        .download_file(&b.node_id(), "Shared", "Folder")
        .unwrap();
    assert_eq!(failed(&a, &id), "not a file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(secret.join("key.pem"), shared.join("key.pem")).unwrap();
        let id = a
            .engine
            .download_file(&b.node_id(), "Shared", "key.pem")
            .unwrap();
        assert_eq!(failed(&a, &id), "not found");
    }
    assert!(
        std::fs::read_dir(&dst)
            .unwrap()
            .flatten()
            .all(|e| e.file_name().to_string_lossy().starts_with('.')),
        "nothing was saved"
    );
    for d in [&shared, &secret, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn shared_folder_names_are_checked() {
    let a = TestNode::start("browse-names");
    let dir = temp_dir("names");
    let f = |name: &str| SharedFolder {
        name: name.into(),
        path: dir.to_string_lossy().into_owned(),
    };
    assert!(a.engine.set_shared_folders(&[f("a/b")]).is_err());
    assert!(a.engine.set_shared_folders(&[f("X"), f("X")]).is_err());
    a.engine.set_shared_folders(&[f("Downloads")]).unwrap();
    assert_eq!(a.engine.shared_folders(), vec![f("Downloads")]);
    let _ = std::fs::remove_dir_all(&dir);
}

// --- sending text ------------------------------------------------------------

fn text_shared(node: &TestNode) -> (String, String, String) {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TextShared {
                from, text, mode, ..
            } => Some((from.clone(), text.clone(), mode.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text; got {:#?}", node.sink.all()))
}

#[test]
fn text_sent_to_the_clipboard_lands_there_and_in_the_history() {
    let (a, b) = pair("share-clip");
    assert!(a.engine.send_text(&b.node_id(), "hi", "shout").is_err());
    assert!(a.engine.send_text(&b.node_id(), "  ", "clipboard").is_err());
    a.engine
        .send_text(&b.node_id(), "221B Baker Street", "clipboard")
        .unwrap();
    let (from, text, mode) = text_shared(&b);
    assert_eq!(from, a.node_id());
    assert_eq!(
        (text.as_str(), mode.as_str()),
        ("221B Baker Street", "clipboard")
    );
    assert_eq!(b.clip().current().as_deref(), Some("221B Baker Street"));
    let history = b.engine.clipboard_history();
    assert_eq!(history[0].text, "221B Baker Street");
    assert_eq!(history[0].from.as_deref(), Some(a.node_id().as_str()));
    // Not shown as a chat message.
    std::thread::sleep(Duration::from_millis(300));
    assert!(!b
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn text_sent_as_a_notification_leaves_the_clipboard_alone() {
    let (a, b) = pair("share-notify");
    a.engine
        .send_text(&b.node_id(), "https://example.com/recipe", "notify")
        .unwrap();
    let (_, text, mode) = text_shared(&b);
    assert_eq!(
        (text.as_str(), mode.as_str()),
        ("https://example.com/recipe", "notify")
    );
    assert_eq!(b.clip().current(), None);
}

#[test]
fn a_device_without_a_clipboard_still_hears_the_text() {
    // The phone: its clipboard is the host app's job.
    let a = TestNode::start("share-noclip-a");
    let b = TestNode::start_with(MockPlatform::new("share-noclip-b").without_clipboard());
    TestNode::pair(&a, &b);
    a.engine
        .send_text(&b.node_id(), "pick up milk", "clipboard")
        .unwrap();
    let (_, text, mode) = text_shared(&b);
    assert_eq!(
        (text.as_str(), mode.as_str()),
        ("pick up milk", "clipboard")
    );
}

#[test]
fn a_password_sent_marked_sensitive_lands_but_stays_out_of_the_history() {
    let (a, b) = pair("share-sensitive");
    a.engine
        .send_text_marked(&b.node_id(), "hunter2", "clipboard", true)
        .unwrap();
    let sensitive = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TextShared {
                text, sensitive, ..
            } if text == "hunter2" => Some(*sensitive),
            _ => None,
        })
        .expect("text arrives");
    assert!(sensitive);
    assert_eq!(b.clip().current().as_deref(), Some("hunter2"));
    assert!(b.engine.clipboard_history().is_empty());
    // An unmarked one (what an older device sends) is kept as before.
    a.engine
        .send_text(&b.node_id(), "not secret", "clipboard")
        .unwrap();
    let sensitive = b
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TextShared {
                text, sensitive, ..
            } if text == "not secret" => Some(*sensitive),
            _ => None,
        })
        .expect("text arrives");
    assert!(!sensitive);
    assert_eq!(b.engine.clipboard_history()[0].text, "not secret");
}
