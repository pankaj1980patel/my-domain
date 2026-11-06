//! Nearby media mode (`features::nearby`): two devices linked through the
//! relay start streaming audio, find they're on the same network, and the
//! stream moves to a local QUIC link without a gap (no "disconnected" in
//! between); when that link drops mid-stream it falls back to the relay at
//! once and keeps flowing. An app from before is never moved.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use md_testkit::{test_config, FakeRegistry, FakeRelay, SignalHub, TestNode, WAIT};
use mdcore::features::audio_share::RATE_DIRECT;
use mdcore::{ConnectRungs, CoreConfig, CoreEvent};

struct Net {
    reg: FakeRegistry,
    relay: FakeRelay,
    hub: Arc<SignalHub>,
}

fn net() -> Net {
    let reg = FakeRegistry::start();
    reg.add_user("alice", "alice-pw");
    Net {
        reg,
        relay: FakeRelay::start(),
        hub: SignalHub::new(),
    }
}

/// A node that connects through the relay first (the direct rung is on,
/// but its peers' QUIC ports aren't known yet: only `addr_update` tells).
fn node(net: &Net, id: &str, nearby: bool) -> TestNode {
    let config = CoreConfig {
        relay_url: Some(net.relay.url()),
        rungs: ConnectRungs {
            relay: true,
            quic: true,
            ..ConnectRungs::none()
        },
        auto_quic: false,
        nearby,
        ..test_config()
    };
    let n = TestNode::start_on_hub(id, &net.hub, config);
    n.engine
        .auth_login(&net.reg.url(), "alice", "alice-pw")
        .unwrap();
    n.engine.set_encryption_key("family key").unwrap();
    n
}

/// Know each other, without a QUIC port: the ladder ends on the relay.
fn pair_without_ports(a: &TestNode, b: &TestNode) {
    let mut pa = a.as_peer();
    pa.quic_port = 0;
    let mut pb = b.as_peer();
    pb.quic_port = 0;
    a.engine.upsert_peer(pb);
    b.engine.upsert_peer(pa);
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

/// `src` plays its audio to `dst` over a relayed link.
fn relayed(net: &Net, test: &str, dst_nearby: bool) -> (TestNode, TestNode) {
    let src = node(net, &format!("{test}-src"), true);
    let dst = node(net, &format!("{test}-dst"), dst_nearby);
    pair_without_ports(&src, &dst);
    src.engine.connect(&dst.node_id()).unwrap();
    assert_eq!(src.wait_connected(&dst, WAIT).as_deref(), Some("relay"));
    assert_eq!(dst.wait_connected(&src, WAIT).as_deref(), Some("relay"));
    src.engine.set_audio_roles(false, true);
    dst.engine.set_audio_roles(true, false);
    learn_status(&dst, &src);
    learn_status(&src, &dst);
    dst.engine.audio_listen(&src.node_id(), true).unwrap();
    let who = dst.node_id();
    src.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::AudioListen { from, on: true } if *from == who => Some(()),
            _ => None,
        })
        .expect("never listened");
    (src, dst)
}

fn frames_from(dst: &TestNode, src: &TestNode) -> usize {
    let who = src.node_id();
    dst.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::AudioFrame { from, .. } if *from == who))
        .count()
}

fn transports(n: &TestNode, peer: &TestNode) -> Vec<String> {
    let id = peer.node_id();
    n.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::PeerConnected { node_id, transport } if node_id == id => {
                serde_json::to_value(transport)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
            }
            _ => None,
        })
        .collect()
}

fn disconnects(n: &TestNode) -> usize {
    n.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::PeerDisconnected { .. }))
        .count()
}

/// Send 20 ms frames, paced, for `ms`, until `done` (checked each frame).
fn stream_until(src: &TestNode, dst: &TestNode, ms: u64, mut done: impl FnMut() -> bool) -> bool {
    let to = [dst.node_id()];
    let pcm = vec![7u8; (RATE_DIRECT / 50) as usize * 4];
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        let _ = src.engine.audio_frame_to(&pcm, RATE_DIRECT, 2, &to);
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

#[test]
fn a_stream_moves_to_the_local_network_and_falls_back_to_the_relay() {
    let n = net();
    let (src, dst) = relayed(&n, "nearby-move", true);
    assert!(src.engine.nearby_supported(&dst.node_id()));

    // Streaming over the relay: the local link comes up and takes over.
    let moved = stream_until(&src, &dst, 8_000, || {
        transports(&src, &dst).last().map(String::as_str) == Some("quic")
            && transports(&dst, &src).last().map(String::as_str) == Some("quic")
    });
    assert!(moved, "never moved: {:?}", transports(&src, &dst));
    let st = src.engine.nearby_status(&dst.node_id());
    assert!(st.low_latency && st.media, "{st:?}");
    assert_eq!(st.path, "lan");
    assert_eq!(disconnects(&src) + disconnects(&dst), 0, "a gap");

    // Audio keeps flowing on the new link.
    let before = frames_from(&dst, &src);
    assert!(stream_until(&src, &dst, 5_000, || frames_from(&dst, &src) >= before + 20));
    // The badge, and a latency, reached the receiver too.
    let who = src.node_id();
    dst.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::NearbyMedia(s) if s.node_id == who && s.low_latency => Some(()),
            _ => None,
        })
        .expect("no nearby status");
    let measured = stream_until(&src, &dst, 6_000, || {
        dst.engine
            .nearby_status(&src.node_id())
            .latency_ms
            .is_some()
    });
    assert!(measured, "no latency");
    let map = src.engine.links_map();
    let link = map
        .devices
        .iter()
        .find(|d| d.node_id == dst.node_id())
        .and_then(|d| d.link.clone())
        .unwrap();
    assert!(link.nearby_media, "{link:?}");

    // The local link drops mid-stream: back on the relay, no gap.
    assert!(src.engine.drop_quic_link(&dst.node_id()));
    let fell = stream_until(&src, &dst, 8_000, || {
        transports(&src, &dst).last().map(String::as_str) == Some("relay")
            && transports(&dst, &src).last().map(String::as_str) == Some("relay")
    });
    assert!(fell, "no fall back: {:?}", transports(&src, &dst));
    assert_eq!(disconnects(&src) + disconnects(&dst), 0, "a gap");
    let before = frames_from(&dst, &src);
    assert!(
        stream_until(&src, &dst, 5_000, || frames_from(&dst, &src) >= before + 20),
        "nothing after the fall back"
    );
    let st = src.engine.nearby_status(&dst.node_id());
    assert!(!st.low_latency && st.path == "relay", "{st:?}");
}

#[test]
fn an_older_app_is_never_moved() {
    let n = net();
    let (src, dst) = relayed(&n, "nearby-old", false);
    assert!(!src.engine.nearby_supported(&dst.node_id()));
    let before = frames_from(&dst, &src);
    stream_until(&src, &dst, 2_500, || false);
    assert!(frames_from(&dst, &src) > before, "no audio");
    assert_eq!(transports(&src, &dst), ["relay"]);
    // And it never got a clock ping (it would show it as chat).
    let chat: Vec<_> = dst
        .sink
        .all()
        .into_iter()
        .filter(|e| matches!(e, CoreEvent::MessageReceived(_)))
        .collect();
    assert!(chat.is_empty(), "{chat:?}");
}

#[test]
fn the_switch_turns_nearby_mode_off() {
    let n = net();
    let (src, dst) = relayed(&n, "nearby-off", true);
    src.engine.set_nearby_media(false);
    dst.engine.set_nearby_media(false);
    assert!(!src.engine.nearby_media());
    stream_until(&src, &dst, 2_500, || false);
    assert_eq!(transports(&src, &dst), ["relay"]);
}
