//! Side routes for big transfers (Wi-Fi Direct, the phone's hotspot),
//! between two engines. The "platform" here answers the core's requests:
//! hosting succeeds, and joining reports the host at 127.0.0.1, so the side
//! lane is dialed to the other engine's QUIC port for real.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::engine::{BleSender, Engine};
use mdcore::features::wifi_direct::{CAN_HOST, CAN_HOTSPOT, CAN_JOIN, CAN_JOIN_HOTSPOT};
use mdcore::{CoreConfig, CoreEvent};

const BIG_WAIT: Duration = Duration::from_secs(60);
const BOTH: u64 = CAN_HOST | CAN_JOIN;

fn config() -> CoreConfig {
    CoreConfig {
        wifi_direct_on_lan: true,
        wifi_direct_min: 1024 * 1024,
        wifi_direct_setup: Duration::from_secs(2),
        ..test_config()
    }
}

/// No network path at all (no server, no shared Wi-Fi): only Bluetooth.
fn offline() -> CoreConfig {
    CoreConfig {
        auto_quic: false,
        rungs: mdcore::ConnectRungs::none(),
        ..config()
    }
}

fn start(id: &str, config: CoreConfig) -> TestNode {
    TestNode::start_full(MockPlatform::new(id), config)
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("md-wd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 + i / 997) as u8).collect()
}

/// How the played platform answers.
#[derive(Clone, Copy)]
struct Answers {
    /// Host and join at all (false: never answers, like a slow phone).
    answer: bool,
    /// The user's answer to "join the phone's hotspot?" (none: never).
    consent: Option<bool>,
}

const ANSWERS: Answers = Answers {
    answer: true,
    consent: Some(true),
};

/// Plays the platform; stops when the returned flag is set.
fn platform(node: &TestNode, how: Answers) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let (engine, sink, stop2) = (node.engine.clone(), node.sink.clone(), stop.clone());
    std::thread::spawn(move || {
        let mut seen = 0;
        while !stop2.load(Ordering::Relaxed) {
            let all = sink.all();
            for ev in &all[seen..] {
                let CoreEvent::WifiDirectRequest {
                    sid,
                    op,
                    ssid,
                    passphrase,
                    ..
                } = ev
                else {
                    continue;
                };
                match op.as_str() {
                    "consent" => {
                        if let Some(yes) = how.consent {
                            engine.wifi_direct_consent(sid, yes);
                        }
                    }
                    "host" if how.answer => engine.wifi_direct_hosted(
                        sid,
                        ssid.as_deref().unwrap_or("DIRECT-md-hotspot"),
                        passphrase.as_deref().unwrap_or("hotspot-pass"),
                        "127.0.0.1",
                    ),
                    "join" if how.answer => engine.wifi_direct_joined(sid, "127.0.0.1"),
                    _ => {}
                }
            }
            seen = all.len();
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    stop
}

fn wait_flags(a: &TestNode, b: &TestNode) {
    let deadline = std::time::Instant::now() + WAIT;
    while a.engine.wifi_direct_peer_caps(&b.node_id()) == 0
        || b.engine.wifi_direct_peer_caps(&a.node_id()) == 0
    {
        assert!(std::time::Instant::now() < deadline, "flags never arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Two linked engines with these abilities.
fn pair_with(test: &str, caps: (u64, u64)) -> (TestNode, TestNode) {
    let a = start(&format!("{test}-a"), config());
    let b = start(&format!("{test}-b"), config());
    a.engine.set_wifi_direct_caps(caps.0);
    b.engine.set_wifi_direct_caps(caps.1);
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hi").unwrap();
    b.sink.wait_for_message("hi");
    wait_flags(&a, &b);
    (a, b)
}

fn pair(test: &str) -> (TestNode, TestNode) {
    pair_with(test, (BOTH, BOTH))
}

fn send_big(a: &TestNode, b: &TestNode, name: &str, len: usize) -> (String, Vec<u8>, PathBuf) {
    let (src, dst) = (
        temp_dir(&format!("{name}-src")),
        temp_dir(&format!("{name}-dst")),
    );
    b.engine.set_download_dir(dst.to_str().unwrap());
    let data = pattern(len);
    let path = src.join(format!("{name}.bin"));
    std::fs::write(&path, &data).unwrap();
    let id = a
        .engine
        .send_file(&b.node_id(), path.to_str().unwrap())
        .unwrap();
    (id, data, dst)
}

fn received(node: &TestNode, id: &str) -> String {
    node.sink
        .wait_until(BIG_WAIT, |e| match e {
            CoreEvent::FileReceived { id: got, path, .. } if got == id => Some(path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("not received"))
}

/// The side route went up (or down), and how.
fn went_active(node: &TestNode, active: bool, within: Duration) -> Option<String> {
    node.sink.wait_until(within, |e| match e {
        CoreEvent::WifiDirect { active: a, via, .. } if *a == active => Some(via.clone()),
        _ => None,
    })
}

/// The ops the platform was asked, in order.
fn ops(n: &TestNode) -> Vec<String> {
    n.sink
        .all()
        .iter()
        .filter_map(|e| match e {
            CoreEvent::WifiDirectRequest { op, .. } => Some(op.clone()),
            _ => None,
        })
        .collect()
}

/// Every "host" or "join" is followed by a "leave" (a computer always goes
/// back to its own Wi-Fi), within `within`.
fn always_left(n: &TestNode, within: Duration) {
    let deadline = std::time::Instant::now() + within;
    loop {
        let o = ops(n);
        let asked = o.iter().filter(|x| *x == "host" || *x == "join").count();
        let left = o.iter().filter(|x| *x == "leave").count();
        if left >= asked {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "never left: {o:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop(flags: &[&Arc<AtomicBool>]) {
    for f in flags {
        f.store(true, Ordering::Relaxed);
    }
}

#[test]
fn a_big_transfer_moves_to_wifi_direct_and_the_group_goes_after() {
    let (a, b) = pair("wd-up");
    let (sa, sb) = (platform(&a, ANSWERS), platform(&b, ANSWERS));
    let (id, data, dst) = send_big(&a, &b, "up", 24 * 1024 * 1024);

    assert_eq!(went_active(&a, true, WAIT).as_deref(), Some("wifi_direct"));
    assert_eq!(went_active(&b, true, WAIT).as_deref(), Some("wifi_direct"));
    let got = received(&b, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");

    // Done: the lane closes and both leave the group.
    assert!(went_active(&a, false, BIG_WAIT).is_some());
    assert!(went_active(&b, false, BIG_WAIT).is_some());
    always_left(&a, WAIT);
    always_left(&b, WAIT);
    assert!(!a.engine.wifi_direct_active(&b.node_id()));
    // Who hosted: the lower node id; the other joined.
    let (host, joiner) = if a.node_id() < b.node_id() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    assert_eq!(ops(host), vec!["host", "leave"]);
    assert_eq!(ops(joiner), vec!["join", "leave"]);
    stop(&[&sa, &sb]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn a_setup_that_never_finishes_falls_back_to_the_link() {
    let (a, b) = pair("wd-timeout");
    // Nobody answers the platform requests.
    let quiet = Answers {
        answer: false,
        consent: None,
    };
    let (sa, sb) = (platform(&a, quiet), platform(&b, quiet));
    let (id, data, dst) = send_big(&a, &b, "timeout", 8 * 1024 * 1024);
    let got = received(&b, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    // The host was asked, then told to leave after the setup time.
    always_left(&a, Duration::from_secs(10));
    always_left(&b, Duration::from_secs(10));
    assert!(ops(&a).len() + ops(&b).len() >= 2, "the host was asked");
    assert!(went_active(&a, true, Duration::from_millis(200)).is_none());
    stop(&[&sa, &sb]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn small_files_and_the_switch_off_stay_on_the_link() {
    let (a, b) = pair("wd-small");
    let (sa, sb) = (platform(&a, ANSWERS), platform(&b, ANSWERS));
    let (id, _, dst) = send_big(&a, &b, "small", 512 * 1024);
    received(&b, &id);
    let _ = std::fs::remove_dir_all(dst);

    // Off on the receiver: it stops saying it, and says no if asked.
    b.engine.set_wifi_direct_enabled(false);
    let deadline = std::time::Instant::now() + WAIT;
    while a.engine.wifi_direct_peer_caps(&b.node_id()) != 0 {
        assert!(std::time::Instant::now() < deadline, "a never heard");
        std::thread::sleep(Duration::from_millis(20));
    }
    let (id, _, dst) = send_big(&a, &b, "off", 4 * 1024 * 1024);
    received(&b, &id);
    assert!(
        ops(&a).is_empty() && ops(&b).is_empty(),
        "no group for a small file, nor with the switch off"
    );
    stop(&[&sa, &sb]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn an_older_app_never_gets_the_messages() {
    let old = CoreConfig {
        wifi_direct: false,
        ..config()
    };
    let a = start("wd-old-a", config());
    let b = start("wd-old-b", old);
    a.engine.set_wifi_direct_caps(BOTH);
    b.engine.set_wifi_direct_caps(BOTH);
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hi").unwrap();
    b.sink.wait_for_message("hi");
    let sa = platform(&a, ANSWERS);
    let (id, _, dst) = send_big(&a, &b, "old", 4 * 1024 * 1024);
    received(&b, &id);
    std::thread::sleep(Duration::from_millis(300));
    let chat = b
        .sink
        .all()
        .into_iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(m) if m.text.contains("wifi_direct")));
    assert!(!chat, "an older app would show it as a chat message");
    assert_eq!(a.engine.wifi_direct_peer_caps(&b.node_id()), 0);
    assert!(ops(&a).is_empty());
    stop(&[&sa]);
    let _ = std::fs::remove_dir_all(dst);
}

// --- the phone's hotspot -----------------------------------------------------

/// A phone (Wi-Fi Direct and a hotspot) and a Mac (joins a hotspot only).
fn phone_and_mac(test: &str) -> (TestNode, TestNode) {
    pair_with(test, (BOTH | CAN_HOTSPOT, CAN_JOIN_HOTSPOT))
}

#[test]
fn a_mac_joins_the_phones_hotspot_after_saying_yes_and_goes_back() {
    let (phone, mac) = phone_and_mac("hs-up");
    let (sp, sm) = (platform(&phone, ANSWERS), platform(&mac, ANSWERS));
    // The Mac sends to the phone: it asks its user first, then joins.
    let (id, data, dst) = send_big(&mac, &phone, "hs", 16 * 1024 * 1024);
    assert_eq!(went_active(&mac, true, WAIT).as_deref(), Some("hotspot"));
    assert_eq!(went_active(&phone, true, WAIT).as_deref(), Some("hotspot"));
    let got = received(&phone, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    always_left(&mac, BIG_WAIT);
    always_left(&phone, BIG_WAIT);
    assert_eq!(ops(&mac), vec!["consent", "join", "leave"]);
    assert_eq!(ops(&phone), vec!["host", "leave"]);
    stop(&[&sp, &sm]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn a_mac_that_says_no_keeps_its_wifi() {
    let (phone, mac) = phone_and_mac("hs-no");
    let no = Answers {
        answer: true,
        consent: Some(false),
    };
    let (sp, sm) = (platform(&phone, ANSWERS), platform(&mac, no));
    // The phone sends to the Mac: the Mac is asked, says no.
    let (id, _, dst) = send_big(&phone, &mac, "hs-no", 8 * 1024 * 1024);
    received(&mac, &id);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        ops(&mac),
        vec!["consent"],
        "never joined, so nothing to restore"
    );
    assert!(
        ops(&phone).is_empty(),
        "the phone never started its hotspot"
    );
    stop(&[&sp, &sm]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn a_hotspot_that_fails_after_joining_still_restores_the_wifi() {
    let (phone, mac) = phone_and_mac("hs-fail");
    let sp = platform(&phone, ANSWERS);
    // The Mac says yes, then can't join.
    let mac_side = {
        let (engine, sink) = (mac.engine.clone(), mac.sink.clone());
        std::thread::spawn(move || {
            let pick = |want: &'static str| {
                sink.wait_until(WAIT, |e| match e {
                    CoreEvent::WifiDirectRequest { sid, op, .. } if op == want => Some(sid.clone()),
                    _ => None,
                })
                .unwrap()
            };
            let sid = pick("consent");
            engine.wifi_direct_consent(&sid, true);
            let sid = pick("join");
            engine.wifi_direct_failed(&sid, "couldn't join");
        })
    };
    let (id, _, dst) = send_big(&mac, &phone, "hs-fail", 8 * 1024 * 1024);
    mac_side.join().unwrap();
    received(&phone, &id);
    always_left(&mac, WAIT);
    always_left(&phone, WAIT);
    assert_eq!(ops(&mac), vec!["consent", "join", "leave"]);
    stop(&[&sp]);
    let _ = std::fs::remove_dir_all(dst);
}

// --- offline: Bluetooth as the control path ----------------------------------

type Waiting = (Option<(Weak<Engine<MockPlatform>>, u64)>, Vec<Vec<u8>>);

/// One direction of the "radio" (as in tests/ble.rs).
#[derive(Default)]
struct Pipe(Mutex<Waiting>);

impl Pipe {
    fn connect(&self, engine: &Arc<Engine<MockPlatform>>, id: u64) {
        let waiting = {
            let mut s = self.0.lock().unwrap();
            s.0 = Some((Arc::downgrade(engine), id));
            std::mem::take(&mut s.1)
        };
        for frame in waiting {
            engine.ble_frame(id, &frame);
        }
    }
}

impl BleSender for Pipe {
    fn send(&self, frame: Vec<u8>) {
        let to = {
            let mut s = self.0.lock().unwrap();
            if s.0.is_none() {
                s.1.push(frame);
                return;
            }
            s.0.clone()
        };
        if let Some((engine, id)) = to {
            if let Some(engine) = engine.upgrade() {
                engine.ble_frame(id, &frame);
            }
        }
    }
}

fn radio(a: &TestNode, b: &TestNode) {
    let (to_b, to_a) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    let id_a = a.engine.ble_opened(to_b.clone());
    let id_b = b.engine.ble_opened(to_a.clone());
    to_b.connect(&b.engine, id_b);
    to_a.connect(&a.engine, id_a);
}

#[test]
fn offline_the_offer_goes_over_bluetooth_and_the_file_over_wifi_direct() {
    // No server, no relay, no shared network: only Bluetooth links them.
    let a = start("wd-ble-a", offline());
    let b = start("wd-ble-b", offline());
    a.engine.set_wifi_direct_caps(BOTH);
    b.engine.set_wifi_direct_caps(BOTH);
    TestNode::pair(&a, &b);
    radio(&a, &b);
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("ble"));
    wait_flags(&a, &b);
    let (sa, sb) = (platform(&a, ANSWERS), platform(&b, ANSWERS));
    let (id, data, dst) = send_big(&a, &b, "ble", 4 * 1024 * 1024);
    assert_eq!(went_active(&a, true, WAIT).as_deref(), Some("wifi_direct"));
    let got = received(&b, &id);
    assert!(std::fs::read(got).unwrap() == data, "content differs");
    always_left(&a, BIG_WAIT);
    always_left(&b, BIG_WAIT);
    stop(&[&sa, &sb]);
    let _ = std::fs::remove_dir_all(dst);
}

#[test]
fn offline_without_bluetooth_nothing_is_negotiated() {
    let a = start("wd-none-a", offline());
    let b = start("wd-none-b", offline());
    a.engine.set_wifi_direct_caps(BOTH);
    b.engine.set_wifi_direct_caps(BOTH);
    TestNode::pair(&a, &b);
    let (sa, sb) = (platform(&a, ANSWERS), platform(&b, ANSWERS));
    let (_id, _, dst) = send_big(&a, &b, "none", 4 * 1024 * 1024);
    std::thread::sleep(Duration::from_secs(1));
    // No control path: no offer went, no group was made; the transfer waits
    // for a link like any other.
    assert!(ops(&a).is_empty() && ops(&b).is_empty());
    assert_eq!(a.engine.wifi_direct_peer_caps(&b.node_id()), 0);
    stop(&[&sa, &sb]);
    let _ = std::fs::remove_dir_all(dst);
}
