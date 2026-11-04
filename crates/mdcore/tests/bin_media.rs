//! Screen and audio frames as binary records over QUIC (`transport::media`)
//! between two engines that both read them, and as JSON messages when one
//! side is an older app: the same events either way, and far fewer bytes on
//! the wire the binary way.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use base64::Engine as _;
use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::features::audio_share::RATE_DIRECT;
use mdcore::CoreEvent;

/// Audio frames per run: 20 ms of 48 kHz stereo each (what the apps send).
const AUDIO_FRAMES: usize = 50;
const AUDIO_BYTES: usize = 3840;
/// Screen snapshots per run, of `SCREEN_BYTES` each.
const SCREEN_FRAMES: usize = 8;
const SCREEN_BYTES: usize = 40 * 1024;

fn node(name: &str, bin_media: bool) -> TestNode {
    let mut config = test_config();
    config.bin_media = bin_media;
    TestNode::start_full(MockPlatform::new(name), config)
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
/// QUIC link; each with binary media on or off.
fn linked(test: &str, src_bin: bool, dst_bin: bool) -> (TestNode, TestNode) {
    let src = node(&format!("{test}-src"), src_bin);
    let dst = node(&format!("{test}-dst"), dst_bin);
    TestNode::pair(&src, &dst);
    src.engine.connect(&dst.node_id()).unwrap();
    assert_eq!(
        src.wait_connected(&dst, Duration::from_secs(10)).as_deref(),
        Some("quic"),
        "no QUIC link"
    );
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
                Some((
                    seq,
                    base64::engine::general_purpose::STANDARD
                        .decode(pcm)
                        .unwrap(),
                ))
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
            CoreEvent::ScreenFrame { from, seq, jpeg } if from == who => Some((
                seq,
                base64::engine::general_purpose::STANDARD
                    .decode(jpeg)
                    .unwrap(),
            )),
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

/// Wait until `pred` holds or `WAIT` passes.
fn settle(mut pred: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT * 2;
    while !pred() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Stream `AUDIO_FRAMES` audio frames and `SCREEN_FRAMES` snapshots from
/// `src` to `dst`, check every one arrived intact and in order, and return
/// the UDP bytes `src` sent meanwhile.
fn stream(src: &TestNode, dst: &TestNode) -> u64 {
    let to = [dst.node_id()];
    // Let the link go quiet (handshake, status, listen) before counting.
    std::thread::sleep(Duration::from_millis(300));
    let before = src.engine.link_bytes_sent(&dst.node_id()).unwrap();

    let audio: Vec<Vec<u8>> = (0..AUDIO_FRAMES).map(|i| pattern(AUDIO_BYTES, i)).collect();
    let screens: Vec<Vec<u8>> = (0..SCREEN_FRAMES)
        .map(|i| {
            let mut j = pattern(SCREEN_BYTES, 1000 + i);
            j[0] = 0xff;
            j[1] = 0xd8;
            j
        })
        .collect();
    for (i, pcm) in audio.iter().enumerate() {
        assert_eq!(
            src.engine.audio_frame_to(pcm, RATE_DIRECT, 2, &to).unwrap(),
            1
        );
        if i % (AUDIO_FRAMES / SCREEN_FRAMES) == 0 {
            if let Some(jpeg) = screens.get(i / (AUDIO_FRAMES / SCREEN_FRAMES)) {
                assert_eq!(src.engine.screen_frame_to(jpeg, &to).unwrap(), 1);
            }
        }
    }
    settle(|| {
        audio_from(dst, src).len() >= AUDIO_FRAMES && screens_from(dst, src).len() >= SCREEN_FRAMES
    });

    // Reliable paths: nothing lost, nothing out of order, nothing changed.
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
        got_screens.windows(2).all(|w| w[1].0 > w[0].0),
        "snapshots out of order"
    );
    assert!(
        got_screens.iter().map(|(_, j)| j).eq(screens.iter()),
        "snapshots changed"
    );
    no_chat(dst);
    no_chat(src);
    src.engine.link_bytes_sent(&dst.node_id()).unwrap() - before
}

#[test]
fn binary_frames_cost_far_fewer_bytes_than_json_and_arrive_the_same() {
    let (src, dst) = linked("binmedia-new", true, true);
    let binary = stream(&src, &dst);
    let (src, dst) = linked("binmedia-old-dst", true, false);
    let json = stream(&src, &dst);
    let payload = (AUDIO_FRAMES * AUDIO_BYTES + SCREEN_FRAMES * SCREEN_BYTES) as u64;
    eprintln!(
        "payload {payload} B; on the wire: binary {binary} B ({:.2}x), JSON {json} B ({:.2}x); saved {:.1}%",
        binary as f64 / payload as f64,
        json as f64 / payload as f64,
        100.0 * (1.0 - binary as f64 / json as f64)
    );
    assert!(
        binary * 100 <= json * 65,
        "binary {binary} B is not 35% under JSON {json} B"
    );
    // And close to the payload itself: headers, AEAD and QUIC only.
    assert!(
        binary * 100 <= payload * 110,
        "binary {binary} B for {payload} B"
    );
}

#[test]
fn an_older_sender_still_sends_json_that_arrives() {
    // The sender doesn't do binary (an older app): the receiver, which
    // does, gets its JSON messages as before.
    let (src, dst) = linked("binmedia-old-src", false, true);
    let bytes = stream(&src, &dst);
    let payload = (AUDIO_FRAMES * AUDIO_BYTES + SCREEN_FRAMES * SCREEN_BYTES) as u64;
    assert!(
        bytes * 100 >= payload * 160,
        "{bytes} B for {payload} B: not the JSON path?"
    );
}

#[test]
fn small_audio_frames_go_as_datagrams() {
    let (src, dst) = linked("binmedia-dgram", true, true);
    let to = [dst.node_id()];
    let before = src.engine.link_datagrams_sent(&dst.node_id()).unwrap();
    // 2.5 ms of 48 kHz stereo: fits a datagram with room to spare.
    let frames: Vec<Vec<u8>> = (0..20).map(|i| pattern(480, i)).collect();
    for pcm in &frames {
        assert_eq!(
            src.engine.audio_frame_to(pcm, RATE_DIRECT, 2, &to).unwrap(),
            1
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(|| audio_from(&dst, &src).len() >= frames.len());
    let sent = src.engine.link_datagrams_sent(&dst.node_id()).unwrap() - before;
    assert_eq!(sent, frames.len() as u64, "not sent as datagrams");
    // Unreliable, but nothing is lost on loopback; each arrives intact.
    let got = audio_from(&dst, &src);
    assert!(got.len() >= frames.len() - 2, "only {} arrived", got.len());
    for (_, pcm) in &got {
        assert!(frames.contains(pcm), "a frame changed");
    }
    no_chat(&dst);
}

#[test]
fn binary_frames_respect_the_same_rules_as_json() {
    // A receiver that stopped listening and watching drops binary frames
    // like JSON ones.
    let (src, dst) = linked("binmedia-rules", true, true);
    let to = [dst.node_id()];
    dst.engine.audio_listen(&src.node_id(), false).unwrap();
    // The source still thinks it listens until the stop arrives; send right
    // away so some frames race it.
    for i in 0..5 {
        let _ = src
            .engine
            .audio_frame_to(&pattern(AUDIO_BYTES, i), RATE_DIRECT, 2, &to);
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(audio_from(&dst, &src).is_empty(), "played after stopping");
    no_chat(&dst);
}
