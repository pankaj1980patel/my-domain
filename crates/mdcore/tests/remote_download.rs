//! "Download on <device>" (features::remote_download): a device asks
//! another of yours to download a link; the other asks its user the first
//! time, downloads it from a local test server (no network), saves it and
//! tells the asker; downloads queue, either side cancels, settings refuse,
//! and an app from before is never sent the request.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::{RemoteDownloadItem, RemoteDownloadSettings};
use mdcore::{CoreConfig, CoreEvent};

/// Serves `/file.bin` (`size` bytes, named by its header) and `/slow.bin`
/// (sent slowly, for queueing and cancelling) on 127.0.0.1.
fn server(size: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            std::thread::spawn(move || {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap_or(0);
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                }
                let mut parts = line.split_whitespace();
                let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
                let (name, slow) = match path {
                    "/file.bin" => ("report.zip", false),
                    "/slow.bin" => ("slow.zip", true),
                    _ => {
                        let _ = conn.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        return;
                    }
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if conn.write_all(head.as_bytes()).is_err() || method == "HEAD" {
                    return;
                }
                if slow {
                    for chunk in body.chunks(4096) {
                        if conn.write_all(chunk).is_err() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(40));
                    }
                } else {
                    let _ = conn.write_all(&body);
                }
            });
        }
    });
    base
}

/// A linked, paired pair: `phone` asks, `mac` downloads into a temp folder
/// (LAN links allowed: the test server is local), with `policy`.
fn pair(tag: &str, policy: &str) -> (TestNode, TestNode, std::path::PathBuf) {
    let phone = TestNode::start(&format!("rd-phone-{tag}"));
    let mac = TestNode::start(&format!("rd-mac-{tag}"));
    TestNode::pair(&phone, &mac);
    let dir = std::env::temp_dir().join(format!("mdcore-rd-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    mac.engine.set_download_dir(dir.to_str().unwrap());
    mac.engine
        .set_remote_download_settings(RemoteDownloadSettings {
            policy: policy.into(),
            allow_lan: true,
            ..RemoteDownloadSettings::default()
        })
        .unwrap();
    phone.engine.send(&mac.node_id(), "hello").unwrap();
    mac.sink.wait_for_message("hello");
    let deadline = Instant::now() + WAIT;
    while !phone.engine.remote_download_supported(&mac.node_id()) {
        assert!(Instant::now() < deadline, "never heard it downloads links");
        std::thread::sleep(Duration::from_millis(50));
    }
    (phone, mac, dir)
}

/// Wait for a `RemoteDownload` event on `node` for `id` matching `f`.
fn wait_state(node: &TestNode, id: &str, state: &str) -> RemoteDownloadItem {
    node.sink
        .wait_until(WAIT * 2, |e| match e {
            CoreEvent::RemoteDownload { item } if item.id == id && item.state == state => {
                Some(item.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "{id} never got to {state}: {:#?}",
                node.engine.remote_downloads()
            )
        })
}

fn none_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("dl_")
    )));
}

#[test]
fn asked_allowed_downloaded_and_reported_back() {
    let base = server(300_000);
    let (phone, mac, dir) = pair("ask", "ask");
    let asked = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("look: {base}/file.bin."))
        .unwrap();
    assert_eq!(asked.state, "requested");
    assert_eq!(asked.name, "file.bin");
    assert!(asked.id.starts_with("dl"));

    // The mac asks its user, with the name and size from HEAD.
    let ask = wait_state(&mac, &asked.id, "asking");
    assert!(ask.here);
    assert_eq!(ask.node_id, phone.node_id());
    assert_eq!(ask.name, "report.zip");
    assert_eq!(ask.size, 300_000);
    assert_eq!(ask.host, "127.0.0.1");
    wait_state(&phone, &asked.id, "asking");
    mac.engine
        .answer_remote_download(&asked.id, true, true)
        .unwrap();

    let done = wait_state(&mac, &asked.id, "done");
    let path = std::path::PathBuf::from(done.path.clone().unwrap());
    assert_eq!(path, dir.join("report.zip"));
    let got = std::fs::read(&path).unwrap();
    assert_eq!(got.len(), 300_000);
    assert!(got.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
    // As a transfer on both sides.
    mac.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileReceived { id, name, size, .. } if *id == asked.id => {
                Some((name.clone(), *size))
            }
            _ => None,
        })
        .filter(|(n, s)| n == "report.zip" && *s == 300_000)
        .expect("a FileReceived");
    let there = wait_state(&phone, &asked.id, "done");
    assert_eq!((there.name.as_str(), there.size), ("report.zip", 300_000));
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileSent { id, .. } if *id == asked.id => Some(()),
            _ => None,
        })
        .expect("a FileSent");
    // "Always": the phone is allowed from now on.
    assert_eq!(
        mac.engine.remote_download_settings().allowed,
        vec![phone.node_id()]
    );
    let again = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
        .unwrap();
    let second = wait_state(&mac, &again.id, "done");
    assert_eq!(second.path.as_deref(), dir.join("report (1).zip").to_str());
    assert!(!mac
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::RemoteDownload { item } if item.id == again.id && item.state == "asking")));
    none_as_chat(&mac);
    none_as_chat(&phone);
}

#[test]
fn refused_by_the_user_by_the_setting_or_the_link() {
    let base = server(1_000);
    let (phone, mac, _dir) = pair("refuse", "ask");
    let a = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
        .unwrap();
    wait_state(&mac, &a.id, "asking");
    mac.engine
        .answer_remote_download(&a.id, false, false)
        .unwrap();
    let r = wait_state(&phone, &a.id, "refused");
    assert!(r.error.unwrap().contains("wasn't allowed"));
    assert!(mac.engine.remote_download_settings().allowed.is_empty());

    // LAN links off: the local server is refused before anything's asked.
    mac.engine
        .set_remote_download_settings(RemoteDownloadSettings {
            policy: "always".into(),
            allow_lan: false,
            ..RemoteDownloadSettings::default()
        })
        .unwrap();
    let b = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
        .unwrap();
    let r = wait_state(&phone, &b.id, "refused");
    assert!(r.error.unwrap().contains("local network"));

    // Turned off.
    mac.engine
        .set_remote_download_settings(RemoteDownloadSettings {
            policy: "never".into(),
            ..RemoteDownloadSettings::default()
        })
        .unwrap();
    let c = phone
        .engine
        .request_remote_download(&mac.node_id(), "https://example.com/a.zip")
        .unwrap();
    let r = wait_state(&phone, &c.id, "refused");
    assert!(r.error.unwrap().contains("turned off"));
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileFailed {
                id,
                incoming: false,
                ..
            } if *id == c.id => Some(()),
            _ => None,
        })
        .expect("a FileFailed on the asker");

    // Only the three choices (the size limit is checked in link_fetch's
    // tests).
    assert!(mac
        .engine
        .set_remote_download_settings(RemoteDownloadSettings {
            policy: "sometimes".into(),
            ..RemoteDownloadSettings::default()
        })
        .is_err());
    // Not a web link: refused on the asker already.
    assert!(phone
        .engine
        .request_remote_download(&mac.node_id(), "ftp://example.com/a")
        .is_err());
    assert!(phone
        .engine
        .request_remote_download(&mac.node_id(), "no link here")
        .is_err());
}

#[test]
fn one_at_a_time_and_cancelled_from_either_side() {
    let base = server(200_000);
    let (phone, mac, dir) = pair("queue", "always");
    let slow = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/slow.bin"))
        .unwrap();
    wait_state(&mac, &slow.id, "downloading");
    let next = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
        .unwrap();
    // Queued behind the slow one, on both sides.
    wait_state(&mac, &next.id, "queued");
    wait_state(&phone, &next.id, "queued");

    // The asker cancels the queued one; the downloader cancels the slow one.
    phone.engine.cancel_file(&next.id);
    wait_state(&mac, &next.id, "cancelled");
    mac.engine.cancel_file(&slow.id);
    wait_state(&mac, &slow.id, "cancelled");
    wait_state(&phone, &slow.id, "cancelled");
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::FileFailed { id, reason, .. } if *id == slow.id => Some(reason.clone()),
            _ => None,
        })
        .filter(|r| r == "cancelled")
        .expect("cancelled as a transfer");
    std::thread::sleep(Duration::from_millis(300));
    // Nothing left behind: no file, no partial file.
    let partial = dir.join(".mydomain-partial");
    let left: Vec<_> = std::fs::read_dir(&partial)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
    assert!(!dir.join("slow.zip").exists());
    assert!(!dir.join("report.zip").exists());
}

#[test]
fn an_app_from_before_never_gets_the_request() {
    let new = TestNode::start("rd-new");
    let old = TestNode::start_full(
        MockPlatform::new("rd-old"),
        CoreConfig {
            remote_download: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    new.engine.send(&old.node_id(), "hello").unwrap();
    old.sink.wait_for_message("hello");
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("remote_download").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without remote_download");
    assert!(!new.engine.remote_download_supported(&old.node_id()));
    assert!(new
        .engine
        .request_remote_download(&old.node_id(), "https://example.com/a.zip")
        .is_err());
    std::thread::sleep(Duration::from_millis(500));
    none_as_chat(&old);
    assert!(new.engine.remote_downloads().is_empty());
    // And the old one doesn't offer it either.
    assert!(!old.engine.remote_download_supported(&new.node_id()));
}

#[test]
fn a_device_cannot_pile_up_requests() {
    let base = server(1_000);
    let (phone, mac, _dir) = pair("flood", "ask");
    let limit = mdcore::features::remote_download::MAX_OPEN_PER_DEVICE;
    let mut ids = Vec::new();
    for _ in 0..limit + 3 {
        let it = phone
            .engine
            .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
            .unwrap();
        ids.push(it.id);
    }
    // The ones past the limit are refused, and say why.
    for id in &ids[limit..] {
        let r = wait_state(&phone, id, "refused");
        assert!(r.error.unwrap().contains("too many"), "{id}");
    }
    let open = mac
        .engine
        .remote_downloads()
        .into_iter()
        .filter(|i| i.here && !i.finished())
        .count();
    assert!(open <= limit, "{open} open");
    // Answering one makes room again.
    mac.engine
        .answer_remote_download(&ids[0], false, false)
        .unwrap();
    wait_state(&phone, &ids[0], "refused");
    let again = phone
        .engine
        .request_remote_download(&mac.node_id(), &format!("{base}/file.bin"))
        .unwrap();
    wait_state(&mac, &again.id, "asking");
}
