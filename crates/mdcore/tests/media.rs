//! Now playing and media controls: what's playing on one device, seen
//! from another, which can play, pause, skip, seek and set the volume
//! there; an app from before gets only the plain fields and never a
//! command.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::{CoreConfig, CoreEvent};
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

// --- media controls (media_ctl) -------------------------------------------------

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

fn wait_supported(a: &TestNode, b: &TestNode, on: bool) {
    let deadline = Instant::now() + WAIT;
    while a.engine.media_supported(&b.node_id()) != on {
        assert!(Instant::now() < deadline, "never heard media_ctl = {on}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

type Cmd = (String, Option<i64>, Option<String>);

fn command_where(node: &TestNode, from: &TestNode, want: &str) -> Cmd {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::MediaCommand {
                from: f,
                action,
                value,
                session,
            } if *f == from.node_id() && action == want => {
                Some((action.clone(), *value, session.clone()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no media command {want}; got {:#?}", node.sink.all()))
}

fn commands(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::MediaCommand { .. }))
        .count()
}

fn pushes(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::NowPlaying { .. }))
        .count()
}

fn no_media_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("media")
    )));
}

#[test]
fn a_computer_plays_pauses_skips_and_seeks_on_the_phone() {
    let pc = TestNode::start("mc-cmd-pc");
    let phone = TestNode::start("mc-cmd-phone");
    TestNode::pair(&pc, &phone);
    link(&pc, &phone, "hi");
    wait_supported(&pc, &phone, true);
    wait_supported(&phone, &pc, true);

    let phone_id = phone.node_id();
    pc.engine
        .media_command(&phone_id, "play_pause", None, None)
        .unwrap();
    assert_eq!(
        command_where(&phone, &pc, "play_pause"),
        ("play_pause".into(), None, None)
    );
    pc.engine
        .media_command(&phone_id, "next", None, None)
        .unwrap();
    command_where(&phone, &pc, "next");
    pc.engine
        .media_command(&phone_id, "seek", Some(90_000), Some("com.spotify.music"))
        .unwrap();
    assert_eq!(
        command_where(&phone, &pc, "seek"),
        (
            "seek".into(),
            Some(90_000),
            Some("com.spotify.music".into())
        )
    );
    pc.engine
        .media_command(&phone_id, "volume_set", Some(7), None)
        .unwrap();
    assert_eq!(command_where(&phone, &pc, "volume_set").1, Some(7));
    pc.engine
        .media_command(
            &phone_id,
            "session",
            None,
            Some("com.google.android.youtube"),
        )
        .unwrap();
    assert_eq!(
        command_where(&phone, &pc, "session").2.as_deref(),
        Some("com.google.android.youtube")
    );

    // Checked before anything goes.
    let before = commands(&phone);
    assert!(pc
        .engine
        .media_command(&phone_id, "rm -rf", None, None)
        .is_err());
    assert!(pc
        .engine
        .media_command(&phone_id, "seek", None, None)
        .is_err());
    assert!(pc
        .engine
        .media_command(&phone_id, "volume_set", Some(-3), None)
        .is_err());
    assert!(pc
        .engine
        .media_command(&phone_id, "next", Some(1), None)
        .is_err());
    assert!(pc
        .engine
        .media_command(&phone_id, "session", None, None)
        .is_err());
    assert!(pc
        .engine
        .media_command("nobody", "next", None, None)
        .is_err());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(commands(&phone), before);
    no_media_as_chat(&phone);
}

#[test]
fn a_command_makes_the_sender_hear_what_it_did() {
    let pc = TestNode::start("mc-hear-pc");
    let phone = TestNode::start("mc-hear-phone");
    TestNode::pair(&pc, &phone);
    // Linked, but the pc never asked for the phone's status: not watching.
    link(&pc, &phone, "hi");
    wait_supported(&pc, &phone, true);
    assert!(!phone.engine.now_playing_watched());

    pc.engine
        .media_command(&phone.node_id(), "next", None, None)
        .unwrap();
    command_where(&phone, &pc, "next");
    assert!(phone.engine.now_playing_watched());
    phone.engine.set_now_playing(
        r#"{"title":"All Blues","state":"playing","position":0,"duration":690000,"can_next":true}"#,
    );
    let m = now_playing(&pc, &phone, Some("All Blues"));
    assert_eq!(m["duration"], 690_000);
    assert_eq!(m["can_next"], true);
}

#[test]
fn media_watch_keeps_a_device_posted() {
    let pc = TestNode::start("mc-watch-pc");
    let phone = TestNode::start("mc-watch-phone");
    TestNode::pair(&pc, &phone);
    link(&pc, &phone, "hi");
    wait_supported(&pc, &phone, true);
    phone
        .engine
        .set_now_playing(r#"{"title":"Flamenco Sketches","state":"paused"}"#);
    assert_eq!(pushes(&pc), 0, "not watching yet");
    // Watching: it hears what plays right away, and the changes after.
    assert!(pc.engine.media_watch(&phone.node_id()).unwrap());
    let m = now_playing(&pc, &phone, Some("Flamenco Sketches"));
    assert_eq!(m["state"], "paused");
    phone
        .engine
        .set_now_playing(r#"{"title":"Flamenco Sketches","state":"playing"}"#);
    let playing = pc.sink.wait_until(WAIT, |e| match e {
        CoreEvent::NowPlaying { media, .. } if media["state"] == "playing" => Some(()),
        _ => None,
    });
    assert!(playing.is_some());
    assert!(pc.engine.media_watch("nobody").is_err());
}

#[test]
fn the_fuller_now_playing_and_the_artwork_once() {
    let pc = TestNode::start("mc-art-pc");
    let phone = TestNode::start("mc-art-phone");
    TestNode::pair(&pc, &phone);
    link(&pc, &phone, "hi");
    wait_supported(&phone, &pc, true);
    status(&pc, &phone);
    let art = "QUJD".repeat(500);
    let song = |state: &str| {
        json!({
            "title": "So What", "artist": "Miles Davis", "app": "Spotify", "state": state,
            "duration": 562_000, "position": 1_000,
            "can_next": true, "can_prev": true, "can_seek": true,
            "volume": 5, "volume_max": 15, "art": art, "icon": "SUNPTg==",
            "session": "com.spotify.music",
            "sessions": [
                {"id": "com.spotify.music", "app": "Spotify", "state": "playing"},
                {"id": "com.google.android.youtube", "app": "YouTube", "state": "paused"},
            ],
        })
        .to_string()
    };
    phone.engine.set_now_playing(&song("playing"));
    let m = now_playing(&pc, &phone, Some("So What"));
    assert_eq!(m["art"], art.as_str());
    assert_eq!(m["icon"], "SUNPTg==");
    assert_eq!(
        (m["volume"].clone(), m["volume_max"].clone()),
        (json!(5), json!(15))
    );
    assert_eq!(m["can_seek"], true);
    assert_eq!(m["session"], "com.spotify.music");
    assert_eq!(m["sessions"].as_array().map(Vec::len), Some(2));
    assert!(m["position"].as_u64().unwrap() >= 1_000);

    // The same picture, paused: it went as its hash and the core put the
    // kept picture back.
    phone.engine.set_now_playing(&song("paused"));
    let paused = pc
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NowPlaying { media, .. } if media["state"] == "paused" => {
                Some(media.clone())
            }
            _ => None,
        })
        .expect("no paused");
    assert_eq!(paused["art"], art.as_str(), "the picture is put back");
    assert_eq!(paused["icon"], "SUNPTg==");
    assert!(paused.get("art_hash").is_none());
}

#[test]
fn a_burst_of_changes_ends_on_the_last() {
    let pc = TestNode::start("mc-burst-pc");
    let phone = TestNode::start("mc-burst-phone");
    TestNode::pair(&pc, &phone);
    status(&pc, &phone);
    for i in 0..6 {
        phone
            .engine
            .set_now_playing(&format!(r#"{{"title":"Track {i}","state":"playing"}}"#));
    }
    now_playing(&pc, &phone, Some("Track 5"));
    let n = pushes(&pc);
    assert!(n <= 3, "throttled: {n} pushes for 6 changes");
}

#[test]
fn an_app_from_before_gets_the_plain_fields_and_never_a_command() {
    let pc = TestNode::start("mc-old-pc");
    let old = TestNode::start_full(
        MockPlatform::new("mc-old-phone"),
        CoreConfig {
            media_ctl: false,
            ..test_config()
        },
    );
    TestNode::pair(&pc, &old);
    let s = status(&pc, &old);
    assert!(s.get("media_ctl").is_none());
    assert!(!pc.engine.media_supported(&old.node_id()));
    assert!(pc
        .engine
        .media_command(&old.node_id(), "pause", None, None)
        .is_err());
    assert!(pc.engine.media_watch(&old.node_id()).is_err());

    // The old one looking at the new one gets only what it knows.
    let st = status(&old, &pc);
    assert_eq!(st["media_ctl"], 1);
    pc.engine.set_now_playing(
        r#"{"title":"Kind of Blue","state":"playing","duration":1000,"position":10,"art":"QUJD","can_next":true}"#,
    );
    let m = now_playing(&old, &pc, Some("Kind of Blue"));
    assert_eq!(m, json!({"title": "Kind of Blue", "state": "playing"}));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(commands(&old), 0);
    no_media_as_chat(&old);
}
