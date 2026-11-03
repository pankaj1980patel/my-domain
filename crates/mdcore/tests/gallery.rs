//! A computer looks at the phone's recent photos and fetches one, between
//! two engines over a real link. The test plays the phone's host app: it
//! answers each `GalleryRequest` with `gallery_reply`, like the Android app
//! does from MediaStore.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::time::Duration;

use md_testkit::{TestNode, WAIT};
use mdcore::model::{GalleryEntry, GalleryItem};
use mdcore::CoreEvent;

fn pair(test: &str) -> (TestNode, TestNode) {
    let a = TestNode::start(&format!("{test}-computer"));
    let b = TestNode::start(&format!("{test}-phone"));
    TestNode::pair(&a, &b);
    (a, b)
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-gallery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The phone's host: wait for request `req` and return (offset, limit, thumbs).
fn request(phone: &TestNode, req: &str) -> (u64, u32, bool) {
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::GalleryRequest {
                req: got,
                offset,
                limit,
                thumbs,
                ..
            } if got == req => Some((*offset, *limit, *thumbs)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no gallery request; got {:#?}", phone.sink.all()))
}

struct Listing {
    items: Vec<GalleryEntry>,
    next: Option<u64>,
    error: Option<String>,
}

fn listing(computer: &TestNode, req: &str) -> Listing {
    computer
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::GalleryListing {
                req: got,
                items,
                next,
                error,
                ..
            } if got == req => Some(Listing {
                items: items.clone(),
                next: *next,
                error: error.clone(),
            }),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no listing; got {:#?}", computer.sink.all()))
}

fn photo(id: &str, path: &PathBuf, taken: u64, thumb: Option<Vec<u8>>) -> GalleryItem {
    GalleryItem {
        entry: GalleryEntry {
            id: id.into(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            mime: "image/jpeg".into(),
            size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            taken,
            width: 4000,
            height: 3000,
            duration: 0,
            video: false,
        },
        path: path.to_string_lossy().into_owned(),
        thumb,
    }
}

#[test]
fn a_computer_sees_the_newest_photos_with_thumbnails() {
    let (a, b) = pair("gallery-list");
    b.engine.set_gallery_enabled(true);
    let dir = temp_dir("list");
    let (p1, p2) = (dir.join("IMG_2.jpg"), dir.join("VID_1.mp4"));
    std::fs::write(&p1, b"new photo").unwrap();
    std::fs::write(&p2, b"older video").unwrap();

    let req = a.engine.list_gallery(&b.node_id(), 0, 2, true).unwrap();
    assert_eq!(request(&b, &req), (0, 2, true));
    let mut video = photo("101", &p2, 1_700_000_000, None);
    video.entry.video = true;
    video.entry.duration = 12_000;
    b.engine.gallery_reply(
        &a.node_id(),
        &req,
        vec![
            photo("102", &p1, 1_700_000_500, Some(vec![0xFF, 0xD8, 1, 2, 3])),
            video,
        ],
        Some(2),
        None,
    );

    let l = listing(&a, &req);
    assert_eq!(l.error, None);
    assert_eq!(l.next, Some(2));
    let ids: Vec<&str> = l.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["102", "101"], "in the phone's order (newest first)");
    assert_eq!(l.items[0].name, "IMG_2.jpg");
    assert_eq!(l.items[0].size, 9);
    assert!(l.items[1].video && l.items[1].duration == 12_000);

    let thumb = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::GalleryThumb {
                req: r, id, jpeg, ..
            } if *r == req => Some((id.clone(), jpeg.clone())),
            _ => None,
        })
        .expect("no thumbnail");
    assert_eq!(thumb, ("102".to_string(), "/9gBAgM=".to_string()));

    // The next page: the asker's offset reaches the host.
    let req2 = a.engine.list_gallery(&b.node_id(), 2, 500, false).unwrap();
    assert_eq!(request(&b, &req2), (2, 100, false), "limit capped");
    b.engine
        .gallery_reply(&a.node_id(), &req2, vec![], None, None);
    let l2 = listing(&a, &req2);
    assert!(l2.items.is_empty() && l2.next.is_none() && l2.error.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_phone_says_why_there_is_no_listing() {
    let (a, b) = pair("gallery-noaccess");
    // A device whose host never offered a gallery (a computer) answers
    // by itself.
    let req = a.engine.list_gallery(&b.node_id(), 0, 10, true).unwrap();
    let l = listing(&a, &req);
    assert_eq!(l.error.as_deref(), Some("this device has no photo gallery"));

    // Photo access not allowed: the host says so.
    b.engine.set_gallery_enabled(true);
    let req = a.engine.list_gallery(&b.node_id(), 0, 10, true).unwrap();
    request(&b, &req);
    b.engine.gallery_reply(
        &a.node_id(),
        &req,
        vec![],
        None,
        Some("photo access isn't allowed on the phone"),
    );
    let l = listing(&a, &req);
    assert_eq!(
        l.error.as_deref(),
        Some("photo access isn't allowed on the phone")
    );
}

#[test]
fn a_listed_photo_downloads_and_an_unlisted_one_does_not() {
    let (a, b) = pair("gallery-get");
    b.engine.set_gallery_enabled(true);
    let (src, dst) = (temp_dir("get-src"), temp_dir("get-dst"));
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7) as u8).collect();
    let file = src.join("PXL_20261103.jpg");
    std::fs::write(&file, &data).unwrap();
    a.engine.set_download_dir(dst.to_str().unwrap());

    // Not listed yet: the phone doesn't know which file that is.
    let id = a
        .engine
        .download_gallery_item(&b.node_id(), "555", "PXL_20261103.jpg")
        .unwrap();
    let reason = a
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileFailed {
                id: got, reason, ..
            } if *got == id => Some(reason.clone()),
            _ => None,
        })
        .expect("no failure");
    assert!(reason.contains("list it again"), "{reason}");
    assert!(a
        .engine
        .download_gallery_item(&b.node_id(), "../etc", "x")
        .is_err());

    let req = a.engine.list_gallery(&b.node_id(), 0, 10, false).unwrap();
    request(&b, &req);
    b.engine.gallery_reply(
        &a.node_id(),
        &req,
        vec![photo("555", &file, 1_700_000_000, None)],
        None,
        None,
    );
    listing(&a, &req);
    let id = a
        .engine
        .download_gallery_item(&b.node_id(), "555", "PXL_20261103.jpg")
        .unwrap();
    let path = a
        .sink
        .wait_until(Duration::from_secs(30), |e| match e {
            CoreEvent::FileReceived { id: got, path, .. } if *got == id => Some(path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("not received; got {:#?}", a.sink.all()));
    assert!(path.ends_with("PXL_20261103.jpg"), "{path}");
    assert!(std::fs::read(&path).unwrap() == data, "content differs");
    for d in [&src, &dst] {
        let _ = std::fs::remove_dir_all(d);
    }
}
