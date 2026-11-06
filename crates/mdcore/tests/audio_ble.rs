//! Audio over Bluetooth LE (`features::audio_share` routes,
//! `features::opus_audio`, `engine::ble`): with no network, Opus records go
//! over the Bluetooth link; with one, Auto takes the network; the route can
//! change mid-stream. An in-memory pipe plays the radio, as in `ble.rs`.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::engine::{BleSender, Engine};
use mdcore::error::CoreError;
use mdcore::features::audio_share::RATE_DIRECT;
use mdcore::{CoreConfig, CoreEvent};

#[derive(Default)]
struct Pipe(Mutex<PipeState>);

#[derive(Default)]
struct PipeState {
    to: Option<(Weak<Engine<MockPlatform>>, u64)>,
    waiting: Vec<Vec<u8>>,
    /// Binary media records carried (first byte 1; JSON starts with `{`).
    records: usize,
    bytes: usize,
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
    fn records(&self) -> (usize, usize) {
        let s = self.0.lock().unwrap();
        (s.records, s.bytes)
    }
}

impl BleSender for Pipe {
    fn send(&self, frame: Vec<u8>) {
        let to = {
            let mut s = self.0.lock().unwrap();
            if frame.first() == Some(&1) {
                s.records += 1;
                s.bytes += frame.len();
            }
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

/// Connect `a` and `b` over Bluetooth; returns the pipe from `a` to `b`.
fn radio(a: &TestNode, b: &TestNode) -> Arc<Pipe> {
    let (to_b, to_a) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    let id_a = a.engine.ble_opened(to_b.clone());
    let id_b = b.engine.ble_opened(to_a.clone());
    to_b.connect(&b.engine, id_b);
    to_a.connect(&a.engine, id_a);
    to_b
}

fn offline(opus: bool) -> CoreConfig {
    CoreConfig {
        auto_quic: false,
        rungs: mdcore::ConnectRungs::none(),
        audio_opus: opus,
        ..test_config()
    }
}

fn learn_status(asker: &TestNode, of: &TestNode) {
    asker.engine.request_status(&of.node_id()).unwrap();
    let who = of.node_id();
    asker
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { from, .. } if *from == who => Some(()),
            _ => None,
        })
        .expect("no status");
}

/// `phone` sends, `pc` plays; both know it, and `pc` listens.
fn streaming(phone: &TestNode, pc: &TestNode) {
    phone.engine.set_audio_roles(false, true);
    pc.engine.set_audio_roles(true, false);
    learn_status(pc, phone);
    learn_status(phone, pc);
    pc.engine.audio_listen(&phone.node_id(), true).unwrap();
    let who = pc.node_id();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioListen { from, on: true } if *from == who => Some(()),
            _ => None,
        })
        .expect("never listened");
}

/// 20 ms of a 48 kHz stereo tone, frame `f`.
fn tone(f: usize) -> Vec<u8> {
    (f * 960..(f + 1) * 960)
        .flat_map(|i| {
            let v = ((i as f64 / 48_000.0 * 440.0 * std::f64::consts::TAU).sin() * 9_000.0) as i16;
            [v, v]
        })
        .flat_map(i16::to_le_bytes)
        .collect()
}

/// PCM bytes played on `pc` from `phone` so far.
fn played(pc: &TestNode, phone: &TestNode) -> usize {
    let who = phone.node_id();
    pc.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::AudioFrame { from, pcm, .. } if from == who => Some(pcm.len() * 3 / 4),
            _ => None,
        })
        .sum()
}

/// Send `n` frames from `phone` to `pc` (from frame `at`), paced, and
/// wait until `pc` played at least `want` more bytes.
fn send_and_wait(phone: &TestNode, pc: &TestNode, at: usize, n: usize) {
    let before = played(pc, phone);
    let to = [pc.node_id()];
    for f in at..at + n {
        assert_eq!(
            phone
                .engine
                .audio_frame_to(&tone(f), RATE_DIRECT, 2, &to)
                .unwrap(),
            1,
            "frame {f} went nowhere"
        );
        std::thread::sleep(Duration::from_millis(4));
    }
    // Opus over Bluetooth sends three frames a record: the last one or two
    // may still wait in the encoder.
    let want = (n - 2) * 3840;
    let deadline = Instant::now() + WAIT;
    while played(pc, phone) - before < want && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        played(pc, phone) - before >= want,
        "played {} of {want} bytes",
        played(pc, phone) - before
    );
}

#[test]
fn with_no_network_audio_goes_over_bluetooth_as_opus() {
    let phone = TestNode::start_full(MockPlatform::new("ble-audio-phone"), offline(true));
    let pc = TestNode::start_full(MockPlatform::new("ble-audio-pc"), offline(true));
    TestNode::pair(&phone, &pc);
    let pipe = radio(&phone, &pc);
    assert_eq!(phone.wait_connected(&pc, WAIT).as_deref(), Some("ble"));
    assert_eq!(pc.wait_connected(&phone, WAIT).as_deref(), Some("ble"));
    streaming(&phone, &pc);
    assert!(phone.engine.audio_opus(&pc.node_id()));
    // The phone captures at 48 kHz for Opus, even over Bluetooth.
    assert_eq!(
        phone.engine.audio_offer(&pc.node_id(), true).unwrap(),
        RATE_DIRECT
    );

    send_and_wait(&phone, &pc, 0, 30);
    // Three 20 ms frames a record, about 32 kbit/s: far below PCM's 1.5
    // Mbit/s.
    let (records, bytes) = pipe.records();
    assert_eq!(records, 10, "30 frames in 10 records");
    assert!(bytes < 30 * 3840 / 15, "{bytes} B over the radio");
    assert_eq!(phone.engine.audio_via(&pc.node_id()), "bluetooth");
    assert_eq!(pc.engine.audio_via(&phone.node_id()), "bluetooth");
    let stats = pc.engine.audio_rx_stats(&phone.node_id());
    assert_eq!(stats.frames, 30);
    assert_eq!(stats.concealed, 0);

    // Wi-Fi only: nothing goes (and the offer says why).
    phone.engine.set_audio_route(&pc.node_id(), "wifi");
    assert_eq!(phone.engine.audio_route(&pc.node_id()), "wifi");
    assert!(matches!(
        phone.engine.audio_offer(&pc.node_id(), true),
        Err(CoreError::Unsupported(_))
    ));
    assert_eq!(
        phone
            .engine
            .audio_frame_to(&tone(0), RATE_DIRECT, 2, &[pc.node_id()])
            .unwrap(),
        0
    );
}

#[test]
fn an_app_without_opus_gets_no_audio_over_bluetooth() {
    let phone = TestNode::start_full(MockPlatform::new("ble-old-phone"), offline(true));
    let pc = TestNode::start_full(MockPlatform::new("ble-old-pc"), offline(false));
    TestNode::pair(&phone, &pc);
    let pipe = radio(&phone, &pc);
    assert_eq!(phone.wait_connected(&pc, WAIT).as_deref(), Some("ble"));
    streaming(&phone, &pc);
    assert!(!phone.engine.audio_opus(&pc.node_id()));
    assert!(matches!(
        phone.engine.audio_offer(&pc.node_id(), true),
        Err(CoreError::Unsupported(_))
    ));
    let to = [pc.node_id()];
    for f in 0..6 {
        assert_eq!(
            phone
                .engine
                .audio_frame_to(&tone(f), RATE_DIRECT, 2, &to)
                .unwrap(),
            0
        );
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(pipe.records().0, 0, "nothing binary to an older app");
    assert_eq!(played(&pc, &phone), 0);
}

#[test]
fn auto_takes_the_network_when_there_is_one_and_the_route_switches_mid_stream() {
    let phone = TestNode::start("ble-switch-phone");
    let pc = TestNode::start("ble-switch-pc");
    TestNode::pair(&phone, &pc);
    let pipe = radio(&phone, &pc);
    assert_eq!(phone.wait_connected(&pc, WAIT).as_deref(), Some("ble"));
    streaming(&phone, &pc);

    // A direct link comes up (Bluetooth stays open): Auto sends on it.
    phone.engine.connect(&pc.node_id()).unwrap();
    phone
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::PeerConnected { node_id, transport }
                if *node_id == pc.node_id() && !transport.is_fallback() =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("no direct link");
    std::thread::sleep(Duration::from_millis(300));
    let over_radio = pipe.records().0;
    send_and_wait(&phone, &pc, 0, 12);
    assert_eq!(phone.engine.audio_via(&pc.node_id()), "network");
    assert_eq!(pc.engine.audio_via(&phone.node_id()), "network");
    assert_eq!(pipe.records().0, over_radio, "nothing more over the radio");
    assert!(!phone.engine.audio_wants_ble(&pc.node_id()));

    // Bluetooth only, chosen on the computer (the sink): it says so in its
    // next listen, and the phone moves the stream back to the radio.
    pc.engine.set_audio_route(&phone.node_id(), "bluetooth");
    pc.engine.audio_listen(&phone.node_id(), true).unwrap();
    let deadline = Instant::now() + WAIT;
    while !phone.engine.audio_wants_ble(&pc.node_id()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(phone.engine.audio_wants_ble(&pc.node_id()));
    assert!(pc.engine.audio_wants_ble(&phone.node_id()));
    send_and_wait(&phone, &pc, 24, 12);
    assert_eq!(phone.engine.audio_via(&pc.node_id()), "bluetooth");
    assert!(pipe.records().0 > over_radio);

    // The phone's own choice wins: Wi-Fi only.
    phone.engine.set_audio_route(&pc.node_id(), "wifi");
    send_and_wait(&phone, &pc, 36, 12);
    assert_eq!(phone.engine.audio_via(&pc.node_id()), "network");

    // Across every switch (PCM on the local link, Opus over Bluetooth) the
    // seqs the player sees only grow: its buffer drops a frame that seems
    // late.
    let who = phone.node_id();
    let seqs: Vec<u64> = pc
        .sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::AudioFrame { from, seq, .. } if from == who => Some(seq),
            _ => None,
        })
        .collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
}
