//! Envelope version 2 (`transport::envelope`): sender, recipient, a message
//! id and the send time are bound to the encryption, so a recorded envelope
//! sent again (by the relay server, or over Bluetooth) is refused, and so
//! are stale, misaddressed and misattributed ones, and version 1 from a
//! device seen sending version 2. An older app (`envelope_v2: false`)
//! still talks version 1 both ways.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use md_testkit::{
    test_config, FakeRegistry, FakeRelay, MockPlatform, SignalHub, TestNode, TEST_PASSPHRASE,
    TEST_USER, WAIT,
};
use mdcore::engine::{BleSender, Engine};
use mdcore::model::{Plaintext, WsFrame};
use mdcore::transport::envelope::{self, EnvelopeInfo, WINDOW_MS};
use mdcore::{ConnectRungs, CoreConfig, CoreEvent};

const LONG: Duration = Duration::from_secs(15);

// --- a Bluetooth "radio" that records what it carries -------------------------

#[derive(Default)]
struct Pipe(Mutex<PipeState>);

#[derive(Default)]
struct PipeState {
    to: Option<(Weak<Engine<MockPlatform>>, u64)>,
    waiting: Vec<Vec<u8>>,
    /// Every frame carried, in order (what a radio sniffer records).
    log: Vec<Vec<u8>>,
}

impl Pipe {
    fn connect(&self, engine: &Arc<Engine<MockPlatform>>, id: u64) {
        let waiting = {
            let mut s = self.0.lock().unwrap();
            s.to = Some((Arc::downgrade(engine), id));
            std::mem::take(&mut s.waiting)
        };
        for frame in waiting {
            engine.ble_frame(id, &frame);
        }
    }

    fn log(&self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().log.clone()
    }
}

impl BleSender for Pipe {
    fn send(&self, frame: Vec<u8>) {
        let to = {
            let mut s = self.0.lock().unwrap();
            s.log.push(frame.clone());
            if s.to.is_none() {
                s.waiting.push(frame);
                return;
            }
            s.to.clone()
        };
        if let Some((engine, id)) = to {
            if let Some(engine) = engine.upgrade() {
                engine.ble_frame(id, &frame);
            }
        }
    }
}

struct Radio {
    /// What `a` sent `b`.
    a_to_b: Arc<Pipe>,
    /// `a`'s end of the link.
    id_a: u64,
    /// `b`'s end of the link (frames handed to it come "from a").
    id_b: u64,
}

fn radio(a: &TestNode, b: &TestNode) -> Radio {
    let (to_b, to_a) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    let id_a = a.engine.ble_opened(to_b.clone());
    let id_b = b.engine.ble_opened(to_a.clone());
    to_b.connect(&b.engine, id_b);
    to_a.connect(&a.engine, id_a);
    Radio {
        a_to_b: to_b,
        id_a,
        id_b,
    }
}

fn offline() -> CoreConfig {
    CoreConfig {
        auto_quic: false,
        rungs: ConnectRungs::none(),
        ..test_config()
    }
}

fn old_app() -> CoreConfig {
    CoreConfig {
        envelope_v2: false,
        ..offline()
    }
}

// --- helpers ------------------------------------------------------------------

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + LONG;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn info(on: &TestNode, about: &TestNode) -> EnvelopeInfo {
    on.engine.envelope_info(&about.node_id())
}

fn refused(i: &EnvelopeInfo) -> u32 {
    i.replays + i.stale + i.downgrades + i.wrong_recipient + i.spoofed
}

fn received(node: &TestNode, text: &str) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::MessageReceived(m) if m.text == text))
        .count()
}

/// Both know the other reads version 2, and `b` has taken one from `a`.
fn negotiated(a: &TestNode, b: &TestNode) {
    wait_for("env: 2 both ways", || {
        info(a, b).version == 2 && info(b, a).version == 2
    });
    a.engine.send(&b.node_id(), "warm up").unwrap();
    b.sink.wait_for_message("warm up");
    wait_for("a version 2 envelope", || info(b, a).v2_only);
}

fn v2_frames(frames: &[Vec<u8>]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .filter(|f| {
            serde_json::from_slice::<serde_json::Value>(f)
                .is_ok_and(|v| v["type"] == "msg" && v["v"] == 2)
        })
        .cloned()
        .collect()
}

fn key() -> [u8; 32] {
    mdcore::crypto::derive_key(TEST_PASSPHRASE, TEST_USER).unwrap()
}

fn body(text: &str) -> Vec<u8> {
    serde_json::to_vec(&Plaintext {
        from: "forged".into(),
        text: text.into(),
        ..Default::default()
    })
    .unwrap()
}

/// A version 2 frame as a device would make it, with the fields chosen.
fn crafted(src: &str, dst: &str, ts: u64, text: &str) -> Vec<u8> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut mid = [0u8; 16];
    mid[..8].copy_from_slice(&ts.to_be_bytes());
    mid[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    let env = envelope::seal_with(&key(), src, dst, mid, ts, &body(text)).unwrap();
    serde_json::to_vec(&WsFrame::Msg(env)).unwrap()
}

/// A version 1 frame (an older app's).
fn crafted_v1(text: &str) -> Vec<u8> {
    let env = mdcore::crypto::encrypt(&key(), &body(text)).unwrap();
    serde_json::to_vec(&WsFrame::Msg(env)).unwrap()
}

fn settle() {
    std::thread::sleep(Duration::from_millis(400));
}

// --- Bluetooth ----------------------------------------------------------------

#[test]
fn a_recorded_bluetooth_envelope_sent_again_is_refused() {
    let a = TestNode::start_full(MockPlatform::new("env-ble-a"), offline());
    let b = TestNode::start_full(MockPlatform::new("env-ble-b"), offline());
    TestNode::pair(&a, &b);
    let r = radio(&a, &b);
    negotiated(&a, &b);

    let before = r.a_to_b.log().len();
    a.engine.send(&b.node_id(), "lock the screen").unwrap();
    b.sink.wait_for_message("lock the screen");
    settle();
    let copies = v2_frames(&r.a_to_b.log()[before..]);
    assert!(!copies.is_empty(), "the message went as version 2");
    let refused_before = info(&b, &a).replays;
    for f in &copies {
        b.engine.ble_frame(r.id_b, f);
    }
    wait_for("the copies refused", || {
        info(&b, &a).replays >= refused_before + copies.len() as u32
    });
    assert_eq!(received(&b, "lock the screen"), 1);
}

#[test]
fn stale_misaddressed_and_misattributed_envelopes_are_refused() {
    let a = TestNode::start_full(MockPlatform::new("env-chk-a"), offline());
    let b = TestNode::start_full(MockPlatform::new("env-chk-b"), offline());
    TestNode::pair(&a, &b);
    let r = radio(&a, &b);
    negotiated(&a, &b);
    let (ida, idb) = (a.node_id(), b.node_id());
    let now = envelope::now_ms();

    // A good one, made the same way, is taken (the checks below are real).
    b.engine
        .ble_frame(r.id_b, &crafted(&ida, &idb, now, "crafted fine"));
    b.sink.wait_for_message("crafted fine");

    // Sent over 5 minutes ago, or over 5 minutes ahead.
    b.engine.ble_frame(
        r.id_b,
        &crafted(&ida, &idb, now - WINDOW_MS - 60_000, "too old"),
    );
    b.engine.ble_frame(
        r.id_b,
        &crafted(&ida, &idb, now + WINDOW_MS + 60_000, "too new"),
    );
    // Within the window (clock skew) is fine.
    b.engine.ble_frame(
        r.id_b,
        &crafted(&ida, &idb, now - WINDOW_MS + 30_000, "a slow clock"),
    );
    b.sink.wait_for_message("a slow clock");
    // Sealed for another device.
    b.engine
        .ble_frame(r.id_b, &crafted(&ida, "env-chk-c", now, "not for b"));
    // Sealed as another device's, on a's link.
    b.engine
        .ble_frame(r.id_b, &crafted("env-chk-c", &idb, now, "not from a"));

    wait_for("the refusals counted", || refused(&info(&b, &a)) >= 4);
    let i = info(&b, &a);
    assert_eq!(i.stale, 2, "{i:?}");
    assert_eq!(i.wrong_recipient, 1, "{i:?}");
    assert_eq!(i.spoofed, 1, "{i:?}");
    assert!(i.skew_ms.is_some());
    for text in ["too old", "too new", "not for b", "not from a"] {
        assert_eq!(received(&b, text), 0, "{text} was taken");
    }
    // The report carries it too.
    let report = b.engine.diagnose(&ida).unwrap();
    assert_eq!(report.envelope.stale, 2);
}

#[test]
fn version_1_after_version_2_is_a_downgrade() {
    let a = TestNode::start_full(MockPlatform::new("env-dg-a"), offline());
    let b = TestNode::start_full(MockPlatform::new("env-dg-b"), offline());
    TestNode::pair(&a, &b);
    let r = radio(&a, &b);
    negotiated(&a, &b);
    b.engine.ble_frame(r.id_b, &crafted_v1("downgraded"));
    wait_for("the downgrade refused", || info(&b, &a).downgrades == 1);
    assert_eq!(received(&b, "downgraded"), 0);
}

#[test]
fn an_older_app_still_talks_version_1_both_ways() {
    let new = TestNode::start_full(MockPlatform::new("env-new"), offline());
    let old = TestNode::start_full(MockPlatform::new("env-old"), old_app());
    TestNode::pair(&new, &old);
    let r = radio(&new, &old);
    assert_eq!(new.wait_connected(&old, WAIT).as_deref(), Some("ble"));
    // Its addr_update (no `env`) comes and goes; then talk.
    settle();
    new.engine.send(&old.node_id(), "to the old app").unwrap();
    old.sink.wait_for_message("to the old app");
    old.engine.send(&new.node_id(), "from the old app").unwrap();
    new.sink.wait_for_message("from the old app");
    settle();

    let i = info(&new, &old);
    assert_eq!(i.version, 1);
    assert!(!i.v2_only);
    assert_eq!(refused(&i), 0);
    assert!(
        v2_frames(&r.a_to_b.log()).is_empty(),
        "never version 2 to an app that didn't say it reads it"
    );
}

// --- restart --------------------------------------------------------------------

#[test]
fn a_restart_does_not_reopen_the_window() {
    let store = Arc::default();
    let secrets = Arc::default();
    let device = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("env-rs-b")
    };
    let a = TestNode::start_full(MockPlatform::new("env-rs-a"), offline());
    let b = TestNode::start_full(device(), offline());
    TestNode::pair(&a, &b);
    let r = radio(&a, &b);
    negotiated(&a, &b);
    let before = r.a_to_b.log().len();
    a.engine.send(&b.node_id(), "open the door").unwrap();
    b.sink.wait_for_message("open the door");
    settle();
    let copies = v2_frames(&r.a_to_b.log()[before..]);
    assert!(!copies.is_empty());
    a.engine.ble_closed(r.id_a);
    drop(b);
    std::thread::sleep(Duration::from_millis(300));

    // Same device, same store: it remembers a sends version 2 ...
    let b = TestNode::start_full(device(), offline());
    TestNode::pair(&a, &b);
    assert!(info(&b, &a).v2_only, "downgrade protection kept");
    let r = radio(&a, &b);
    assert_eq!(b.wait_connected(&a, WAIT).as_deref(), Some("ble"));
    // ... and the ids it took: the copies are still refused.
    for f in &copies {
        b.engine.ble_frame(r.id_b, f);
    }
    b.engine.ble_frame(r.id_b, &crafted_v1("old style"));
    wait_for("the copies refused after a restart", || {
        let i = info(&b, &a);
        i.replays >= copies.len() as u32 && i.downgrades == 1
    });
    assert_eq!(received(&b, "open the door"), 0);
    assert_eq!(received(&b, "old style"), 0);
    // Fresh messages still go.
    a.engine.send(&b.node_id(), "after the restart").unwrap();
    b.sink.wait_for_message("after the restart");
}

// --- relay ---------------------------------------------------------------------

fn relay_node(id: &str, reg: &FakeRegistry, hub: &Arc<SignalHub>, relay: &FakeRelay) -> TestNode {
    let config = CoreConfig {
        relay_url: Some(relay.url()),
        rungs: ConnectRungs {
            relay: true,
            ..ConnectRungs::none()
        },
        auto_quic: false,
        ..test_config()
    };
    let n = TestNode::start_on_hub(id, hub, config);
    n.engine
        .auth_login(&reg.url(), "alice", "alice-pw")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

#[test]
fn the_relay_cannot_replay_or_reattribute_an_envelope() {
    let (reg, relay, hub) = (FakeRegistry::start(), FakeRelay::start(), SignalHub::new());
    reg.add_user("alice", "alice-pw");
    let a = relay_node("env-rl-a", &reg, &hub, &relay);
    let b = relay_node("env-rl-b", &reg, &hub, &relay);
    TestNode::pair(&a, &b);
    a.engine.connect(&b.node_id()).unwrap();
    assert_eq!(a.wait_connected(&b, LONG).as_deref(), Some("relay"));
    negotiated(&a, &b);

    let before = relay.forwarded().len();
    assert_eq!(
        a.engine.send(&b.node_id(), "type and press enter").unwrap(),
        "relay"
    );
    b.sink.wait_for_message("type and press enter");
    settle();
    let copies: Vec<String> = relay.forwarded()[before..]
        .iter()
        .filter(|f| f["to"] == b.node_id().as_str())
        .filter_map(|f| f["data"].as_str().map(String::from))
        .filter(|d| v2_frames(&[d.clone().into_bytes()]).len() == 1)
        .collect();
    assert!(!copies.is_empty(), "the relay carried version 2");

    // Sent again as a's.
    for d in &copies {
        assert!(relay.inject(&b.node_id(), &a.node_id(), d));
    }
    wait_for("the copies refused", || {
        info(&b, &a).replays >= copies.len() as u32
    });
    // Passed off as another device of the account's.
    for d in &copies {
        assert!(relay.inject(&b.node_id(), "env-rl-c", d));
    }
    wait_for("the reattributed copies refused", || {
        b.engine.envelope_info("env-rl-c").spoofed >= copies.len() as u32
    });
    assert_eq!(received(&b, "type and press enter"), 1);
}

// --- QUIC -------------------------------------------------------------------------

#[test]
fn quic_links_use_version_2_too() {
    let a = TestNode::start("env-q-a");
    let b = TestNode::start("env-q-b");
    TestNode::pair(&a, &b);
    assert_eq!(a.engine.send(&b.node_id(), "first").unwrap(), "QUIC");
    b.sink.wait_for_message("first");
    negotiated(&a, &b);
    a.engine.send(&b.node_id(), "over quic").unwrap();
    b.sink.wait_for_message("over quic");
    let i = info(&b, &a);
    assert!(i.accepted >= 2 && i.v2_only, "{i:?}");
    assert_eq!(refused(&i), 0);
}
