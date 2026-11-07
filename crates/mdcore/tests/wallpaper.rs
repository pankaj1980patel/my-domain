//! A device's wallpaper look reaches your other devices through its
//! status (as a hash), only when it changes, and is forgotten when it
//! stops sharing.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::model::Wallpaper;
use mdcore::CoreEvent;

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-a"));
    let b = TestNode::start(&format!("{test}-b"));
    TestNode::pair(&a, &b);
    (a, b)
}

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 16, b'J', b'F', b'I', b'F'];

/// The next wallpaper event on `a` for `node`.
fn next_look(a: &TestNode, node: &str) -> Option<Option<Wallpaper>> {
    a.sink.wait_until(WAIT, |e| match e {
        CoreEvent::Wallpaper { node: n, wallpaper } if n == node => Some(wallpaper.clone()),
        _ => None,
    })
}

fn looks(a: &TestNode) -> usize {
    a.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::Wallpaper { .. }))
        .count()
}

#[test]
fn a_look_arrives_with_the_status_then_only_on_change() {
    let (a, b) = pair("wallpaper");
    b.engine.set_wallpaper(
        vec!["#112233".into(), "#AABBCC".into()],
        Some(JPEG.to_vec()),
    );
    let own = b.engine.own_wallpaper().unwrap();

    a.engine.request_status(&b.node_id()).unwrap();
    let got = next_look(&a, &b.node_id()).expect("no look").unwrap();
    assert_eq!(got.hash, own.hash);
    assert_eq!(got.colors, vec!["#112233", "#aabbcc"]);
    assert!(got.image.unwrap().starts_with("data:image/jpeg;base64,"));
    assert_eq!(a.engine.wallpapers().len(), 1);
    assert!(a.engine.wallpaper(&b.node_id()).is_some());

    // The status carries only the hash; the same look isn't sent again.
    a.engine.request_status(&b.node_id()).unwrap();
    let status = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(status["wallpaper"], own.hash.as_str());
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(looks(&a), 1);
    b.engine.set_wallpaper(
        vec!["#112233".into(), "#aabbcc".into()],
        Some(JPEG.to_vec()),
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(looks(&a), 1);

    // A new look reaches a device with a live link at once.
    b.engine.set_wallpaper(vec!["#445566".into()], None);
    let got = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Wallpaper {
                wallpaper: Some(w), ..
            } if w.colors == ["#445566"] => Some(w.clone()),
            _ => None,
        })
        .expect("no new look");
    assert_eq!(got.image, None);

    // Sharing turned off: it's forgotten.
    b.engine.clear_wallpaper();
    a.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::Wallpaper {
                wallpaper: None, ..
            } => Some(()),
            _ => None,
        })
        .expect("not forgotten");
    assert!(a.engine.wallpapers().is_empty());
    // And it says nothing in its status.
    a.engine.request_status(&b.node_id()).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(!a
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn a_device_that_never_says_keeps_nothing() {
    let (a, b) = pair("wallpaper-none");
    a.engine.request_status(&b.node_id()).unwrap();
    let status = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(status["wallpaper"], "");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(looks(&a), 0);
    assert!(a.engine.wallpapers().is_empty());
}
