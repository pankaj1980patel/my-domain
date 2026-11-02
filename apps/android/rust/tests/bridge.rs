//! The UniFFI bridge, from the Rust side: what Kotlin gets from `MdEngine`.
//! Events must reach the listener (this replaced polling), and failures must
//! arrive as typed `MdError`s (Kotlin exceptions), not "ERROR: …" strings.
//!
//! One `MdEngine` per process (it's a process-wide singleton, like on the
//! phone), so everything is checked in one test.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md_testkit::{TestNode, TEST_PASSPHRASE, TEST_USER};
use mdcore::model::{Peer, Source};
use mydomain_net::{running_engine, EventListener, MdEngine, MdError, SecretStore};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<(String, serde_json::Value)>>>);

impl EventListener for Recorder {
    fn on_event(&self, name: String, payload_json: String) {
        let v = serde_json::from_str(&payload_json).unwrap();
        self.0.lock().unwrap().push((name, v));
    }
}

impl Recorder {
    fn wait_for(&self, name: &str, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some((_, v)) = self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|(n, v)| n == name && pred(v))
            {
                return v.clone();
            }
            assert!(Instant::now() < deadline, "no {name} event");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The Keystore-backed store Kotlin passes in, kept in memory.
#[derive(Default)]
struct MemSecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for MemSecrets {
    fn get(&self, key: String) -> Option<String> {
        self.0.lock().unwrap().get(&key).cloned()
    }
    fn set(&self, key: String, value: Option<String>) {
        let mut m = self.0.lock().unwrap();
        match value {
            Some(v) => m.insert(key, v),
            None => m.remove(&key),
        };
    }
}

#[test]
fn the_bridge_pushes_events_and_types_its_errors() {
    let events = Recorder::default();
    let dir = std::env::temp_dir().join(format!("md-bridge-{}", std::process::id()));
    let phone = MdEngine::start(
        "bridge-phone".into(),
        "Bridge phone".into(),
        "127.0.0.1".into(),
        dir.to_string_lossy().into_owned(),
        Box::new(events.clone()),
        Box::new(MemSecrets::default()),
    )
    .unwrap();

    // A second start returns the running engine, as when a push woke the app.
    let again = MdEngine::start(
        "ignored".into(),
        "ignored".into(),
        String::new(),
        String::new(),
        Box::new(events.clone()),
        Box::new(MemSecrets::default()),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&phone, &again));
    assert!(Arc::ptr_eq(&phone, &running_engine().unwrap()));

    // Typed errors.
    assert!(matches!(
        phone.send("nobody".into(), "hi".into()),
        Err(MdError::NoKey)
    ));
    phone
        .dev_login(TEST_USER.into(), TEST_PASSPHRASE.into())
        .unwrap();
    // Offline (LAN-only) login: has a key, but isn't signed in to a server.
    assert!(!phone.is_ready());
    let session = phone.session();
    assert_eq!(session.username.as_deref(), Some(TEST_USER));
    assert!(session.has_key && session.server_url.is_none());
    assert!(matches!(
        phone.send("nobody".into(), "hi".into()),
        Err(MdError::PeerNotFound)
    ));

    // A desktop messages the phone: the listener hears it, kind-tagged.
    // (The phone knows the desktop only by address, as typed by hand.)
    let desktop = TestNode::start("bridge-desktop");
    phone
        .add_manual_peer(
            "Desk".into(),
            "127.0.0.1".into(),
            desktop.engine.identity().quic_port,
        )
        .unwrap();
    let id = phone.identity();
    desktop.engine.upsert_peer(Peer {
        node_id: id.node_id.clone(),
        name: id.name.clone(),
        ip: "127.0.0.1".into(),
        tcp_port: 0,
        udp_port: 0,
        ws_port: 0,
        quic_port: id.quic_port,
        source: Source::Manual,
        ws_open: false,
        inbound_blocked: false,
        reflexive_ip: None,
        reflexive_udp_port: None,
        nat: Default::default(),
        ipv6: None,
        mapped_quic: None,
    });
    desktop
        .engine
        .send(&id.node_id, "hello through the bridge")
        .unwrap();
    let msg = events.wait_for("message-received", |v| {
        v["text"] == "hello through the bridge"
    });
    assert_eq!(msg["kind"], "message_received");
    assert_eq!(msg["protocol"], "QUIC");
    events.wait_for("peer-connected", |v| {
        v["node_id"] == desktop.node_id().as_str()
    });

    // The peer list carries the live-link flag, under the real node id.
    let listed = phone
        .peers()
        .into_iter()
        .find(|p| p.node_id == desktop.node_id())
        .expect("the desktop is re-listed under its real id");
    assert!(listed.connected);
    assert_eq!(listed.link.as_deref(), Some("quic"));
    // ...and its quality, measured by QUIC (loopback: excellent).
    let q = phone.link_stats();
    let q = q
        .iter()
        .find(|s| s.node_id == desktop.node_id())
        .expect("rated");
    assert!(q.rtt_ms.is_some());
    assert_eq!(q.quality, "excellent");
    assert_eq!(listed.name, "Desk");

    // And the phone answers over the same link.
    assert_eq!(
        phone
            .send(desktop.node_id(), "reply from the phone".into())
            .unwrap(),
        "QUIC"
    );
    desktop.sink.wait_for_message("reply from the phone");

    // With an app-chosen id, the desktop's acknowledgement comes back as a
    // `message-delivered` event carrying that id.
    assert_eq!(
        phone
            .send_with_id(
                desktop.node_id(),
                "receipt please".into(),
                "bridge-msg-1".into()
            )
            .unwrap(),
        "QUIC"
    );
    desktop.sink.wait_for_message("receipt please");
    let delivered = events.wait_for("message-delivered", |v| v["id"] == "bridge-msg-1");
    assert_eq!(delivered["kind"], "message_delivered");
    assert_eq!(delivered["node_id"], desktop.node_id().as_str());

    // Quick device actions. The desktop rings the phone: a `ring-event`.
    desktop.engine.ring(&id.node_id).unwrap();
    let ring = events.wait_for("ring-event", |_| true);
    assert_eq!(ring["kind"], "ring");
    assert_eq!(ring["from"], desktop.node_id().as_str());

    // ...and sends it a link: an `open-url-event`.
    desktop
        .engine
        .open_url(&id.node_id, "https://example.com/x")
        .unwrap();
    let open = events.wait_for("open-url-event", |_| true);
    assert_eq!(open["kind"], "open_url");
    assert_eq!(open["url"], "https://example.com/x");
    // Only web links leave the phone.
    assert!(matches!(
        phone.open_url(desktop.node_id(), "file:///etc/passwd".into()),
        Err(MdError::Failed(_))
    ));

    // The desktop asks the phone's status: the battery the app pushed.
    phone.set_device_status(r#"{"battery":42,"charging":true}"#.into());
    desktop.engine.request_status(&id.node_id).unwrap();
    let status = desktop
        .sink
        .wait_until(Duration::from_secs(10), |e| match e {
            mdcore::events::CoreEvent::DeviceStatus { status, .. } => Some(status.clone()),
            _ => None,
        })
        .expect("a device-status event");
    assert_eq!(status["battery"], 42);
    assert_eq!(status["charging"], true);
    assert_eq!(status["platform"], "android");

    // And the phone asks the desktop's.
    phone.request_status(desktop.node_id()).unwrap();
    let st = events.wait_for("device-status-event", |_| true);
    assert_eq!(st["kind"], "device_status");
    assert_eq!(st["from"], desktop.node_id().as_str());
    let _ = std::fs::remove_dir_all(dir);
}
