//! Screen and audio frames as binary records over the RELAY
//! (`transport::relay`, `transport::media`): binary WebSocket frames when
//! the server forwards them and both ends read them, the JSON messages when
//! the server is an older one or a peer is an older app. The same events
//! either way, and far fewer bytes through the server the binary way.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use md_testkit::{test_config, FakeRegistry, FakeRelay, SignalHub, TestNode, WAIT};
use mdcore::features::audio_share::RATE_DIRECT;
use mdcore::{ConnectRungs, CoreConfig, CoreEvent};

const AUDIO_FRAMES: usize = 50;
const AUDIO_BYTES: usize = 3840;
const SCREEN_FRAMES: usize = 8;
const SCREEN_BYTES: usize = 40 * 1024;
const PAYLOAD: u64 = (AUDIO_FRAMES * AUDIO_BYTES + SCREEN_FRAMES * SCREEN_BYTES) as u64;

struct Net {
    reg: FakeRegistry,
    relay: FakeRelay,
    hub: Arc<SignalHub>,
}

fn net(relay: FakeRelay) -> Net {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "alice-pw");
    Net {
        reg,
        relay,
        hub: SignalHub::new(),
    }
}

/// A node that only connects through `net`'s relay, with the smaller
/// audio off (0: the PCM arrives byte for byte, which most tests below
/// compare), ADPCM (1, `features::audio_codec`) or Opus too (2,
/// `features::opus_audio`).
fn node_with(net: &Net, id: &str, bin_media: bool, codec: u8) -> TestNode {
    let config = CoreConfig {
        audio_codec: codec >= 1,
        audio_opus: codec >= 2,
        relay_url: Some(net.relay.url()),
        rungs: ConnectRungs {
            relay: true,
            ..ConnectRungs::none()
        },
        auto_quic: false, // keep everything on the relay link
        bin_media,
        ..test_config()
    };
    let n = TestNode::start_on_hub(id, &net.hub, config);
    n.engine
        .auth_login(&net.reg.url(), "alice", "alice-pw")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
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

/// `src` sends audio and its screen, `dst` listens and watches, over a live
/// relayed link; each with binary media on or off.
fn linked(net: &Net, test: &str, src_bin: bool, dst_bin: bool) -> (TestNode, TestNode) {
    linked_with(net, test, src_bin, dst_bin, 0)
}

fn linked_with(
    net: &Net,
    test: &str,
    src_bin: bool,
    dst_bin: bool,
    codec: u8,
) -> (TestNode, TestNode) {
    linked_codecs(net, test, (src_bin, codec), (dst_bin, codec))
}

/// As `linked_with`, each end with its own (binary media, codec).
fn linked_codecs(
    net: &Net,
    test: &str,
    (src_bin, src_codec): (bool, u8),
    (dst_bin, dst_codec): (bool, u8),
) -> (TestNode, TestNode) {
    let src = node_with(net, &format!("{test}-src"), src_bin, src_codec);
    let dst = node_with(net, &format!("{test}-dst"), dst_bin, dst_codec);
    TestNode::pair(&src, &dst);
    src.engine.connect(&dst.node_id()).unwrap();
    assert_eq!(src.wait_connected(&dst, WAIT).as_deref(), Some("relay"));
    assert_eq!(dst.wait_connected(&src, WAIT).as_deref(), Some("relay"));
    src.engine.set_audio_roles(false, true);
    dst.engine.set_audio_roles(true, false);
    learn_status(&dst, &src);
    learn_status(&src, &dst);

    dst.engine.audio_listen(&src.node_id(), true).unwrap();
    src.engine
        .set_peer_permission(&dst.node_id(), "screen", true)
        .unwrap();
    dst.engine.screen_watch(&src.node_id(), true).unwrap();
    let who = dst.node_id();
    let (mut listen, mut watch) = (false, false);
    src.sink
        .wait_until(WAIT, |e| {
            match e {
                CoreEvent::AudioListen { from, on: true } if *from == who => listen = true,
                CoreEvent::ScreenWatch { from, on: true } if *from == who => watch = true,
                _ => {}
            }
            (listen && watch).then_some(())
        })
        .expect("never listened and watched");
    (src, dst)
}

fn pattern(len: usize, salt: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i * 31 + salt * 7) % 251) as u8)
        .collect()
}

fn decode(b64: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap()
}

fn audio_from(dst: &TestNode, src: &TestNode) -> Vec<(u64, Vec<u8>)> {
    let who = src.node_id();
    dst.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::AudioFrame {
                from,
                seq,
                rate,
                channels,
                pcm,
            } if from == who => {
                assert_eq!((rate, channels), (RATE_DIRECT, 2));
                Some((seq, decode(&pcm)))
            }
            _ => None,
        })
        .collect()
}

fn screens_from(dst: &TestNode, src: &TestNode) -> Vec<(u64, Vec<u8>)> {
    let who = src.node_id();
    dst.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::ScreenFrame { from, seq, jpeg } if from == who => Some((seq, decode(&jpeg))),
            _ => None,
        })
        .collect()
}

fn no_chat(n: &TestNode) {
    let chat: Vec<_> = n
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::MessageReceived(_)))
        .collect();
    assert!(chat.is_empty(), "media showed as chat: {chat:?}");
}

/// Stream the frames from `src` to `dst` through the relay, check every one
/// arrived intact and in order, and return the bytes `src` sent the relay
/// meanwhile.
fn stream(relay: &FakeRelay, src: &TestNode, dst: &TestNode) -> u64 {
    let to = [dst.node_id()];
    // Let the link go quiet (handshake, status, listen) before counting.
    std::thread::sleep(Duration::from_millis(300));
    let before = relay.bytes_from(&src.node_id());

    let audio: Vec<Vec<u8>> = (0..AUDIO_FRAMES).map(|i| pattern(AUDIO_BYTES, i)).collect();
    let screens: Vec<Vec<u8>> = (0..SCREEN_FRAMES)
        .map(|i| {
            let mut j = pattern(SCREEN_BYTES, 1000 + i);
            j[0] = 0xff;
            j[1] = 0xd8;
            j
        })
        .collect();
    let step = AUDIO_FRAMES / SCREEN_FRAMES;
    for (i, pcm) in audio.iter().enumerate() {
        assert_eq!(
            src.engine.audio_frame_to(pcm, RATE_DIRECT, 2, &to).unwrap(),
            1
        );
        if i % step == 0 {
            if let Some(jpeg) = screens.get(i / step) {
                assert_eq!(src.engine.screen_frame_to(jpeg, &to).unwrap(), 1);
            }
        }
    }
    let deadline = Instant::now() + WAIT * 2;
    while (audio_from(dst, src).len() < AUDIO_FRAMES
        || screens_from(dst, src).len() < SCREEN_FRAMES)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }

    let got_audio = audio_from(dst, src);
    assert_eq!(got_audio.len(), AUDIO_FRAMES, "audio frames lost");
    assert!(
        got_audio.windows(2).all(|w| w[1].0 > w[0].0),
        "audio out of order"
    );
    assert!(
        got_audio.iter().map(|(_, p)| p).eq(audio.iter()),
        "audio changed"
    );
    let got_screens = screens_from(dst, src);
    assert_eq!(got_screens.len(), SCREEN_FRAMES, "snapshots lost");
    assert!(
        got_screens.iter().map(|(_, j)| j).eq(screens.iter()),
        "snapshots changed"
    );
    no_chat(dst);
    no_chat(src);
    relay.bytes_from(&src.node_id()) - before
}

#[test]
fn binary_through_the_relay_costs_far_fewer_bytes_than_json() {
    let n = net(FakeRelay::start());
    let (src, dst) = linked(&n, "relaybin-new", true, true);
    let frames = n.relay.binary_frames();
    let binary = stream(&n.relay, &src, &dst);
    assert_eq!(
        n.relay.binary_frames() - frames,
        (AUDIO_FRAMES + SCREEN_FRAMES) as u64,
        "not every frame went binary"
    );
    // The same relay, an older receiver: JSON.
    let (src, dst) = linked(&n, "relaybin-old-dst", true, false);
    let frames = n.relay.binary_frames();
    let json = stream(&n.relay, &src, &dst);
    assert_eq!(n.relay.binary_frames(), frames, "binary to an older app");
    eprintln!(
        "payload {PAYLOAD} B; through the relay: binary {binary} B ({:.2}x), JSON {json} B ({:.2}x); saved {:.1}%",
        binary as f64 / PAYLOAD as f64,
        json as f64 / PAYLOAD as f64,
        100.0 * (1.0 - binary as f64 / json as f64)
    );
    assert!(
        binary * 100 <= json * 65,
        "binary {binary} B is not 35% under JSON {json} B"
    );
    assert!(
        binary * 100 <= PAYLOAD * 105,
        "binary {binary} B for {PAYLOAD} B"
    );
}

#[test]
fn an_older_sender_through_the_relay_still_sends_json() {
    let n = net(FakeRelay::start());
    let (src, dst) = linked(&n, "relaybin-old-src", false, true);
    let bytes = stream(&n.relay, &src, &dst);
    assert_eq!(n.relay.binary_frames(), 0);
    assert!(
        bytes * 100 >= PAYLOAD * 160,
        "{bytes} B: not the JSON path?"
    );
}

#[test]
fn an_older_relay_server_gets_json_only() {
    // A server from before binary frames closes a socket that sends one:
    // the clients must see it doesn't say "bin" and keep to JSON, and the
    // link must stay up throughout.
    let n = net(FakeRelay::start_old());
    let (src, dst) = linked(&n, "relaybin-old-server", true, true);
    let bytes = stream(&n.relay, &src, &dst);
    assert_eq!(n.relay.binary_frames(), 0);
    assert!(
        bytes * 100 >= PAYLOAD * 160,
        "{bytes} B: not the JSON path?"
    );
    assert!(n.relay.connected(&src.node_id()) && n.relay.connected(&dst.node_id()));
    // And messages still flow both ways after.
    src.engine.send(&dst.node_id(), "still here").unwrap();
    dst.sink.wait_for_message("still here");
}

#[test]
fn binary_through_the_relay_respects_the_same_rules_as_json() {
    // A receiver that stopped listening drops binary frames like JSON ones.
    let n = net(FakeRelay::start());
    let (src, dst) = linked(&n, "relaybin-rules", true, true);
    let to = [dst.node_id()];
    dst.engine.audio_listen(&src.node_id(), false).unwrap();
    for i in 0..5 {
        let _ = src
            .engine
            .audio_frame_to(&pattern(AUDIO_BYTES, i), RATE_DIRECT, 2, &to);
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(audio_from(&dst, &src).is_empty(), "played after stopping");
    no_chat(&dst);
}

#[test]
fn audio_through_the_relay_goes_as_adpcm_when_both_read_it() {
    // Both say `audio_codec`: each 20 ms frame goes as two 10 ms ADPCM
    // records, a quarter of the bytes, and plays as the same PCM length.
    let n = net(FakeRelay::start());
    let (src, dst) = linked_with(&n, "relaybin-adpcm", true, true, 1);
    std::thread::sleep(Duration::from_millis(300));
    let to = [dst.node_id()];
    let before = n.relay.bytes_from(&src.node_id());
    for i in 0..AUDIO_FRAMES {
        let pcm = pattern(AUDIO_BYTES, i);
        assert_eq!(
            src.engine
                .audio_frame_to(&pcm, RATE_DIRECT, 2, &to)
                .unwrap(),
            1
        );
        // Paced (two records a frame; the relay queue holds 64).
        std::thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + WAIT;
    while audio_from(&dst, &src).len() < AUDIO_FRAMES * 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let got = audio_from(&dst, &src);
    assert_eq!(got.len(), AUDIO_FRAMES * 2, "records lost");
    assert!(got.windows(2).all(|w| w[1].0 > w[0].0), "out of order");
    let samples: usize = got.iter().map(|(_, p)| p.len()).sum();
    assert_eq!(samples, AUDIO_FRAMES * AUDIO_BYTES);
    let bytes = n.relay.bytes_from(&src.node_id()) - before;
    let pcm = (AUDIO_FRAMES * AUDIO_BYTES) as u64;
    assert!(bytes * 100 <= pcm * 40, "{bytes} B for {pcm} B of PCM");
    assert_eq!(
        dst.engine.audio_rx_stats(&src.node_id()).frames,
        (AUDIO_FRAMES * 2) as u32
    );
    no_chat(&dst);
}

/// `frames` 20 ms frames of a 48 kHz stereo tone.
fn tone(frames: usize) -> Vec<Vec<u8>> {
    let n = 960;
    (0..frames)
        .map(|f| {
            (f * n..(f + 1) * n)
                .flat_map(|i| {
                    let t = i as f64 / 48_000.0;
                    let v = ((t * 440.0 * std::f64::consts::TAU).sin() * 10_000.0) as i16;
                    [v, v]
                })
                .flat_map(i16::to_le_bytes)
                .collect()
        })
        .collect()
}

#[test]
fn audio_through_the_relay_goes_as_opus_when_both_read_it() {
    // Both say `audio_codec: 2`: each 20 ms frame goes as one Opus record,
    // about a tenth of ADPCM's bytes, and plays as the same PCM length.
    let n = net(FakeRelay::start());
    let (src, dst) = linked_with(&n, "relaybin-opus", true, true, 2);
    std::thread::sleep(Duration::from_millis(300));
    // Opus captures at 48 kHz on the relay too.
    assert_eq!(
        src.engine.audio_offer(&dst.node_id(), true).unwrap(),
        RATE_DIRECT
    );
    let to = [dst.node_id()];
    let before = n.relay.bytes_from(&src.node_id());
    for pcm in tone(AUDIO_FRAMES) {
        assert_eq!(
            src.engine
                .audio_frame_to(&pcm, RATE_DIRECT, 2, &to)
                .unwrap(),
            1
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + WAIT;
    while audio_from(&dst, &src).len() < AUDIO_FRAMES && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let got = audio_from(&dst, &src);
    assert_eq!(got.len(), AUDIO_FRAMES, "frames lost");
    assert!(got.iter().all(|(_, p)| p.len() == AUDIO_BYTES));
    assert!(got.windows(2).all(|w| w[1].0 > w[0].0), "in order");
    let bytes = n.relay.bytes_from(&src.node_id()) - before;
    let pcm = (AUDIO_FRAMES * AUDIO_BYTES) as u64;
    assert!(bytes * 100 <= pcm * 12, "{bytes} B for {pcm} B of PCM");
    let stats = dst.engine.audio_rx_stats(&src.node_id());
    assert_eq!(stats.frames, AUDIO_FRAMES as u32);
    assert_eq!(stats.concealed, 0);
    assert_eq!(dst.engine.audio_via(&src.node_id()), "network");
    no_chat(&dst);
}

#[test]
fn an_adpcm_only_app_still_gets_adpcm_from_an_opus_one() {
    // The sink says `audio_codec: 1` (the app before Opus): the source
    // sends it ADPCM halves as before, never an Opus record.
    let n = net(FakeRelay::start());
    let (src, dst) = linked_codecs(&n, "relaybin-mixed", (true, 2), (true, 1));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!src.engine.audio_opus(&dst.node_id()));
    assert!(!dst.engine.audio_opus(&src.node_id()));
    let to = [dst.node_id()];
    for pcm in tone(10) {
        src.engine
            .audio_frame_to(&pcm, RATE_DIRECT, 2, &to)
            .unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let deadline = Instant::now() + WAIT;
    while audio_from(&dst, &src).len() < 20 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let got = audio_from(&dst, &src);
    assert_eq!(got.len(), 20, "two ADPCM halves a frame");
    assert!(got.iter().all(|(_, p)| p.len() == AUDIO_BYTES / 2));
}
