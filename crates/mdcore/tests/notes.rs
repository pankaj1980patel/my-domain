//! Notes shared between your devices (features::notes): two devices end
//! with the same notes, edits made apart merge at the next link, an edit
//! made on both at once keeps both texts, and an app from before never
//! gets a notes message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::Note;
use mdcore::{CoreConfig, CoreEvent};

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Close the link between `a` and `b`, and wait until both ends know.
fn unlink(a: &TestNode, b: &TestNode) {
    a.engine.disconnect(&b.node_id()).unwrap();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let a_up = a.engine.connected_peers().contains(&b.node_id());
        let b_up = b.engine.connected_peers().contains(&a.node_id());
        if !a_up && !b_up {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the link stayed up");
}

/// Wait until `a` and `b` have the same notes; those notes.
fn converged(a: &TestNode, b: &TestNode) -> Vec<Note> {
    let deadline = Instant::now() + WAIT * 2;
    loop {
        let (x, y) = (a.engine.notes(), b.engine.notes());
        if x == y {
            return x;
        }
        assert!(Instant::now() < deadline, "never the same:\n{x:#?}\n{y:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Wait for a `NotesChanged` on `node` with a note titled `title`.
fn wait_note(node: &TestNode, title: &str) -> Note {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotesChanged { notes, .. } => {
                notes.iter().find(|n| n.title == title).cloned()
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no note {title:?}"))
}

fn note_events(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::NotesChanged { .. }))
        .count()
}

fn no_notes_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("notes")
    )));
}

fn save(node: &TestNode, id: Option<&str>, title: &str, body: &str) -> Note {
    node.engine
        .save_note(id, title, body, false, false, None)
        .unwrap()
}

#[test]
fn two_devices_end_with_the_same_notes() {
    let phone = TestNode::start("notes-phone");
    let mac = TestNode::start("notes-mac");
    TestNode::pair(&phone, &mac);

    // Written before there's a link: it goes when the link comes up.
    let list = phone
        .engine
        .save_note(
            None,
            "Shopping",
            "[ ] milk\n[x] eggs",
            true,
            true,
            Some("green"),
        )
        .unwrap();
    save(&phone, None, "Wi-Fi", "the guest password");
    link(&phone, &mac, "hi");
    let got = wait_note(&mac, "Shopping");
    assert_eq!(got.id, list.id);
    assert!(got.pinned && got.checklist);
    assert_eq!(got.color.as_deref(), Some("green"));
    assert_eq!(got.by, phone.node_id());
    assert!(phone.engine.notes_supported(&mac.node_id()));
    assert!(mac.engine.notes_supported(&phone.node_id()));
    assert_eq!(converged(&phone, &mac).len(), 2);

    // The other way, over the live link.
    save(
        &mac,
        Some(&list.id),
        "Shopping",
        "[ ] milk\n[x] eggs\n[ ] tea",
    );
    let edited = wait_note(&phone, "Shopping");
    assert!(edited.body.ends_with("tea"));
    // Unpinned by the edit above (the apps send every field).
    assert!(!edited.pinned);

    // A delete goes too.
    let wifi = phone
        .engine
        .notes()
        .into_iter()
        .find(|n| n.title == "Wi-Fi")
        .unwrap();
    mac.engine.delete_note(&wifi.id).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NotesChanged { removed, .. } => removed.contains(&wifi.id).then_some(()),
            _ => None,
        })
        .expect("the delete arrives");
    assert_eq!(converged(&phone, &mac).len(), 1);
    assert!(phone.engine.note(&wifi.id).is_none());

    // Saving a note makes no event where it was saved (nothing echoes).
    assert!(!mac.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::NotesChanged { notes, .. } if notes.iter().any(|n| n.by == mac.node_id())
    )));
    no_notes_as_chat(&phone);
    no_notes_as_chat(&mac);
}

#[test]
fn edits_made_apart_merge_at_the_next_link() {
    let a = TestNode::start("notes-apart-a");
    let b = TestNode::start("notes-apart-b");
    TestNode::pair(&a, &b);
    link(&a, &b, "one");
    let shared = save(&a, None, "Trip", "passport");
    wait_note(&b, "Trip");
    unlink(&a, &b);

    // Apart: a edits the shared note, b writes a new one and deletes nothing.
    save(&a, Some(&shared.id), "Trip", "passport, tickets");
    save(&b, None, "Books", "Dune");
    std::thread::sleep(Duration::from_millis(300));
    assert!(b.engine.note(&shared.id).unwrap().body == "passport");
    assert!(a.engine.notes().iter().all(|n| n.title != "Books"));

    link(&b, &a, "two");
    let notes = converged(&a, &b);
    assert_eq!(notes.len(), 2);
    let trip = notes.iter().find(|n| n.id == shared.id).unwrap();
    assert_eq!(trip.body, "passport, tickets");
    assert!(notes.iter().any(|n| n.title == "Books"));
    // An edit of the version the other had is no conflict.
    assert!(notes.iter().all(|n| n.conflict_of.is_none()));
}

#[test]
fn an_edit_on_both_at_once_keeps_both_texts() {
    let a = TestNode::start("notes-both-a");
    let b = TestNode::start("notes-both-b");
    TestNode::pair(&a, &b);
    link(&a, &b, "one");
    let shared = save(&a, None, "Plan", "start");
    wait_note(&b, "Plan");
    unlink(&a, &b);

    save(&a, Some(&shared.id), "Plan", "the mac's plan");
    std::thread::sleep(Duration::from_millis(20));
    save(&b, Some(&shared.id), "Plan", "the phone's plan");

    link(&a, &b, "two");
    let notes = converged(&a, &b);
    assert_eq!(notes.len(), 2, "{notes:#?}");
    // The newer edit (b's) keeps the note; a's text is a copy, on both.
    let plan = notes.iter().find(|n| n.id == shared.id).unwrap();
    assert_eq!(plan.body, "the phone's plan");
    let copy = notes.iter().find(|n| n.id != shared.id).unwrap();
    assert_eq!(copy.body, "the mac's plan");
    assert_eq!(copy.conflict_of.as_deref(), Some(shared.id.as_str()));
    assert!(
        copy.title.starts_with("Plan (conflict from "),
        "{}",
        copy.title
    );
    // Nothing more after a while: it doesn't go round and round.
    std::thread::sleep(Duration::from_millis(500));
    let (ea, eb) = (note_events(&a), note_events(&b));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!((note_events(&a), note_events(&b)), (ea, eb));
    assert_eq!(a.engine.notes(), b.engine.notes());
}

#[test]
fn an_app_from_before_never_gets_notes() {
    let new = TestNode::start("notes-new");
    let old = TestNode::start_full(
        MockPlatform::new("notes-old"),
        CoreConfig {
            notes: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    save(&new, None, "Secret plan", "only for the new ones");
    link(&new, &old, "hello");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("notes").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without notes");
    // A note saved over the live link doesn't go either.
    save(&new, None, "Another", "x");
    std::thread::sleep(Duration::from_millis(1_000));

    assert!(!new.engine.notes_supported(&old.node_id()));
    no_notes_as_chat(&old);
    assert_eq!(note_events(&old), 0);
    assert!(old.engine.notes().is_empty());
    assert_eq!(note_events(&new), 0);
}
