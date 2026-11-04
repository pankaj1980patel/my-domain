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

/// A node that only connects through `net`'s relay.
fn node(net: &Net, id: &str, bin_media: bool) -> TestNode {
    let config = CoreConfig {
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
    let src = node(net, &format!("{test}-src"), src_bin);
    let dst = node(net, &format!("{test}-dst"), dst_bin);
    TestNode::pair(&src, &dst);
    src.engine.connect(&dst.node_id()).unwrap();
    assert_eq!(src.wait_connected(&dst, WAIT).as_deref(), Some("relay"));
    assert_eq!(dst.wait_connected(&src, WAIT).as_deref(), Some("relay"));
    src.engine.set_audio_roles(false, true);
    dst.engine.set_audio_roles(true, false);
    learn_status(&dst, &src);
    learn_status(&src, &dst);

    dst.engine.audio_listen(&src.node_id(), true).unwrap();
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
