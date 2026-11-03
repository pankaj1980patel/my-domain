//! Now playing: what's playing on one device, seen from another.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::CoreEvent;
use serde_json::{json, Value};

fn status(a: &TestNode, b: &TestNode) -> Value {
    a.engine.request_status(&b.node_id()).unwrap();
    a.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, status } if *from == b.node_id() => {
                Some(status.clone())
            }
            _ => None,
        })
        .expect("no status")
}

fn now_playing(node: &TestNode, from: &TestNode, title: Option<&str>) -> Value {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NowPlaying { from: f, media }
                if *f == from.node_id() && media["title"].as_str() == title =>
            {
                Some(media.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no now-playing {title:?}; got {:#?}", node.sink.all()))
}

#[test]
fn the_status_carries_whats_playing() {
    let a = TestNode::start("np-status-a");
    let b = TestNode::start("np-status-b");
    TestNode::pair(&a, &b);
    assert!(status(&a, &b).get("now_playing").is_none(), "nothing yet");
    b.engine.set_now_playing(
        r#"{"title":"So What","artist":"Miles Davis","app":"Spotify","state":"playing"}"#,
    );
    // The first reply is still in the log: wait for one with the song.
    a.engine.request_status(&b.node_id()).unwrap();
    let s = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } if status.get("now_playing").is_some() => {
                Some(status.clone())
            }
            _ => None,
        })
        .expect("no status with now_playing");
    assert_eq!(
        s["now_playing"],
        json!({"title": "So What", "artist": "Miles Davis", "app": "Spotify", "state": "playing"})
    );
}

#[test]
fn a_device_that_looked_hears_the_song_change() {
    let a = TestNode::start("np-watch-a");
    let b = TestNode::start("np-watch-b");
    let c = TestNode::start("np-watch-c");
    TestNode::pair(&a, &b);
    TestNode::pair(&c, &b);
    // c talks to b but never asks for its status: not watching.
    c.engine.send(&b.node_id(), "hi").unwrap();
    b.sink.wait_for_message("hi");
    assert!(!b.engine.now_playing_watched());
    status(&a, &b);
    assert!(b.engine.now_playing_watched());

    b.engine
        .set_now_playing(r#"{"title":"Blue in Green","state":"playing"}"#);
    let m = now_playing(&a, &b, Some("Blue in Green"));
    assert_eq!(m["state"], "playing");
    b.engine
        .set_now_playing(r#"{"title":"Blue in Green","state":"paused"}"#);
    let paused = a.sink.wait_until(WAIT, |e| match e {
        CoreEvent::NowPlaying { media, .. } if media["state"] == "paused" => Some(()),
        _ => None,
    });
    assert!(paused.is_some(), "pausing is a change too");
    // Nothing playing any more.
    b.engine.set_now_playing("");
    now_playing(&a, &b, None);

    std::thread::sleep(Duration::from_millis(300));
    assert!(!c
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::NowPlaying { .. })));
}

#[test]
fn the_same_song_again_is_not_sent_again() {
    let a = TestNode::start("np-dup-a");
    let b = TestNode::start("np-dup-b");
    TestNode::pair(&a, &b);
    status(&a, &b);
    for _ in 0..3 {
        b.engine
            .set_now_playing(r#"{"title":"Freddie Freeloader","state":"playing"}"#);
    }
    now_playing(&a, &b, Some("Freddie Freeloader"));
    std::thread::sleep(Duration::from_millis(500));
    let count = a
        .sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::NowPlaying { .. }))
        .count();
    assert_eq!(count, 1);
}
