//! A phone's media audio (music, videos, podcasts; never calls) played on
//! a computer's speakers, like a Bluetooth speaker but over the encrypted
//! link. The phone captures what its apps play (Android's playback
//! capture, after its own prompt every time) and sends it as raw PCM; the
//! computer buffers a little and plays it. The core carries the messages;
//! the host apps capture and play.
//!
//!   audio_play  { from, on }                       computer → phone: "play
//!                                                  your audio here" (`on`,
//!                                                  renewed), or "stop"
//!   audio_offer { from, on, rate, ch }             phone → computer: "I'm
//!                                                  playing my audio to you"
//!                                                  (`on`), or "I stopped"
//!   audio_frame { from, seq, rate, ch, data }      phone → computer (data:
//!                                                  base64 PCM, s16le,
//!                                                  interleaved)
//!
//! - Only to devices that said they understand it: a computer says
//!   `"audio_sink": true` in its status, a phone `"audio_source": true`
//!   (`Engine::set_audio_roles`; the core adds them to every status
//!   reply and remembers what each peer said, or showed by sending one of
//!   these messages). An older app doesn't know
//!   these messages and would show them as chat, so the core refuses to
//!   send them to a device that didn't say so (`Unsupported`).
//! - A listen lasts `LISTEN_FOR` unless renewed: the computer renews it
//!   every `RENEW_EVERY` while it plays, so a computer that went away
//!   (crashed, lost its link) stops the stream by itself.
//! - The phone sends only to the computers its owner chose that listen
//!   (`Engine::audio_frame_to`); a computer plays only what comes from
//!   a phone it listens to (`Engine::audio_listen`).
//! - Frames go only over a live link, never queued (late audio is
//!   noise). 48 kHz stereo over a direct link, 24 kHz through the relay
//!   (half the bytes), none over Bluetooth (far too slow): see
//!   `rate_for`. Each frame says its rate and channels, so the computer
//!   follows a change.
//! - Only your own devices (decrypted with your key, in this device's
//!   list).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::json;

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::TransportKind;

use super::{ControlHandler, ControlMsg};

/// How long a listen lasts without being renewed.
pub const LISTEN_FOR: Duration = Duration::from_secs(25);
/// How often a listening computer should renew (`Engine::audio_listen`).
pub const RENEW_EVERY: Duration = Duration::from_secs(10);
/// The rate over a direct link (Hz).
pub const RATE_DIRECT: u32 = 48_000;
/// The rate through the relay (Hz): half the bytes.
pub const RATE_RELAY: u32 = 24_000;
/// Rates a frame may have.
pub const RATES: [u32; 5] = [16_000, 22_050, 24_000, 44_100, 48_000];
/// Largest frame sent, in bytes of PCM (100 ms of 48 kHz stereo).
pub const MAX_FRAME_BYTES: usize = 48_000 * 2 * 2 / 10;

/// What a computer says in its status: it plays a phone's audio.
pub const SINK_FLAG: &str = "audio_sink";
/// What a phone says in its status: it sends its audio.
pub const SOURCE_FLAG: &str = "audio_source";

const CAPS_PREFIX: &str = "audio_caps:";

/// Both sides' state.
#[derive(Default)]
pub struct AudioShare {
    sink: AtomicBool,
    source: AtomicBool,
    /// Phone: listening computer -> when its listen ends.
    listeners: Mutex<HashMap<String, Instant>>,
    /// Computer: the phones it listens to.
    listening: Mutex<HashSet<String>>,
    /// What each peer said it understands: (sink, source). `None` until
    /// read from `kv`.
    caps: Mutex<HashMap<String, (bool, bool)>>,
    seq: AtomicU64,
}

/// This device plays (`sink`) or sends (`source`) audio; said in its
/// status.
pub fn set_roles<P: Platform>(ctx: &NetCtx<P>, sink: bool, source: bool) {
    let a = &ctx.extras.audio;
    a.sink.store(sink, Ordering::Relaxed);
    a.source.store(source, Ordering::Relaxed);
}

/// Add this device's roles to a status reply.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut serde_json::Value) {
    let a = &ctx.extras.audio;
    if a.sink.load(Ordering::Relaxed) {
        status[SINK_FLAG] = true.into();
    }
    if a.source.load(Ordering::Relaxed) {
        status[SOURCE_FLAG] = true.into();
    }
}

/// Remember what `node` said it understands (kept across restarts).
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, status: &serde_json::Value) {
    let now = (
        status[SINK_FLAG].as_bool() == Some(true),
        status[SOURCE_FLAG].as_bool() == Some(true),
    );
    if caps_of(ctx, node) == now {
        return;
    }
    ctx.extras.audio.caps.lock().insert(node.to_string(), now);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), &caps_text(now));
}

fn caps_text((sink, source): (bool, bool)) -> String {
    let mut v = Vec::new();
    if sink {
        v.push("sink");
    }
    if source {
        v.push("source");
    }
    v.join(",")
}

/// `node` sent one of these messages, so it understands them: a computer
/// that asks to listen plays audio, a phone that offers sends it (before
/// this device ever asked for its status).
fn learned<P: Platform>(ctx: &NetCtx<P>, node: &str, sink: bool, source: bool) {
    let (was_sink, was_source) = caps_of(ctx, node);
    let now = (was_sink || sink, was_source || source);
    if now == (was_sink, was_source) {
        return;
    }
    ctx.extras.audio.caps.lock().insert(node.to_string(), now);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), &caps_text(now));
}

/// What `node` last said it understands: (plays audio, sends audio).
pub fn caps_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> (bool, bool) {
    if let Some(c) = ctx.extras.audio.caps.lock().get(node) {
        return *c;
    }
    let saved = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .unwrap_or_default();
    let c = (
        saved.split(',').any(|s| s == "sink"),
        saved.split(',').any(|s| s == "source"),
    );
    ctx.extras.audio.caps.lock().insert(node.to_string(), c);
    c
}

/// The rate to capture at for a link of `kind` (`None`: too slow for
/// audio).
pub fn rate_for(kind: TransportKind) -> Option<u32> {
    match kind {
        TransportKind::Ble => None,
        TransportKind::Relay => Some(RATE_RELAY),
        _ => Some(RATE_DIRECT),
    }
}

/// The live link's rate to `node`: `NoLink` without one, `Unsupported`
/// over Bluetooth.
fn link_rate<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<u32> {
    let kind = ctx.p2p_conns.lock().get(node).map(|c| c.kind);
    if !ctx.ws_conns.lock().contains_key(node) {
        return Err(CoreError::NoLink("not connected".into()));
    }
    // A route with no link yet (the relay's, before the peer answered):
    // treat as the relay.
    rate_for(kind.unwrap_or(TransportKind::Relay)).ok_or(CoreError::Unsupported(
        "the link is Bluetooth only, too slow for audio: get both on the same Wi-Fi or online",
    ))
}

const OLD_SINK: &str = "that computer's my-domain doesn't play a phone's audio yet: update it";
const OLD_SOURCE: &str =
    "that phone's my-domain can't send its audio (Android 10 or later, and an up-to-date app)";

/// Computer: ask phone `node` to play its audio here (`on`, renew every
/// `RENEW_EVERY`), or to stop. Only to a phone that said `audio_source`.
/// Over the live link only; with none, fails with `NoLink` (and starts
/// connecting).
pub fn listen<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !on {
        ctx.extras.audio.listening.lock().remove(node);
    }
    if !caps_of(ctx, node).1 {
        return Err(CoreError::Unsupported(OLD_SOURCE));
    }
    if !ctx.ws_conns.lock().contains_key(node) {
        if on {
            crate::engine::start_ladder(ctx, node);
        }
        return Err(CoreError::NoLink("not connected yet: connecting".into()));
    }
    if on {
        ctx.extras.audio.listening.lock().insert(node.to_string());
    }
    let me = ctx.identity.lock().node_id.clone();
    send(
        ctx,
        node,
        &json!({ "type": "audio_play", "from": me, "on": on }).to_string(),
    )
}

/// Phone: the computers listening now (expired listens are dropped).
pub fn listeners<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let now = Instant::now();
    let mut l = ctx.extras.audio.listeners.lock();
    l.retain(|_, until| *until > now);
    let mut list: Vec<String> = l.keys().cloned().collect();
    list.sort();
    list
}

/// Phone: tell computer `node` this phone plays its audio to it (`on`),
/// or stopped. Returns the rate to capture at for its link (0 when
/// stopping). Only to a computer that said `audio_sink`; over a live link
/// only.
pub fn offer<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> crate::Result<u32> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !caps_of(ctx, node).0 {
        return Err(CoreError::Unsupported(OLD_SINK));
    }
    let rate = if on {
        link_rate(ctx, node)?
    } else if ctx.ws_conns.lock().contains_key(node) {
        0
    } else {
        return Err(CoreError::NoLink("not connected".into()));
    };
    let me = ctx.identity.lock().node_id.clone();
    send(
        ctx,
        node,
        &json!({ "type": "audio_offer", "from": me, "on": on, "rate": rate, "ch": 2 }).to_string(),
    )?;
    Ok(rate)
}

/// Phone: send `pcm` (s16le, interleaved, `channels` of `rate` Hz) to
/// those of `nodes` that listen and understand it; returns how many got it.
pub fn send_frame_to<P: Platform>(
    ctx: &NetCtx<P>,
    pcm: &[u8],
    rate: u32,
    channels: u8,
    nodes: &[String],
) -> crate::Result<usize> {
    if pcm.is_empty() || pcm.len() > MAX_FRAME_BYTES {
        return Err(CoreError::InvalidInput(format!(
            "an audio frame must be 1 to {MAX_FRAME_BYTES} bytes, not {}",
            pcm.len()
        )));
    }
    if !RATES.contains(&rate) || !(1..=2).contains(&channels) {
        return Err(CoreError::InvalidInput(format!(
            "unsupported audio format: {rate} Hz, {channels} channels"
        )));
    }
    if !pcm.len().is_multiple_of(2 * usize::from(channels)) {
        return Err(CoreError::InvalidInput(
            "an audio frame must hold whole samples".into(),
        ));
    }
    let mut to = listeners(ctx);
    to.retain(|n| nodes.contains(n) && caps_of(ctx, n).0);
    if to.is_empty() {
        return Ok(0);
    }
    let me = ctx.identity.lock().node_id.clone();
    let seq = ctx.extras.audio.seq.fetch_add(1, Ordering::Relaxed);
    let msg = json!({
        "type": "audio_frame", "from": me, "seq": seq, "rate": rate, "ch": channels,
        "data": base64::engine::general_purpose::STANDARD.encode(pcm),
    })
    .to_string();
    Ok(to.iter().filter(|n| send(ctx, n, &msg).is_ok()).count())
}

fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &str) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(&ctx.ws_conns, &ctx.p2p_conns, &ctx.key, &name, node, msg)
}

pub struct AudioShareHandler;

impl<P: Platform> ControlHandler<P> for AudioShareHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["audio_play", "audio_offer", "audio_frame"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own devices only.
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        let a = &ctx.extras.audio;
        match msg.kind() {
            "audio_play" => {
                if !a.source.load(Ordering::Relaxed) {
                    return;
                }
                let on = msg.json("on").as_bool().unwrap_or(false);
                learned(ctx, &from, true, false);
                let changed = {
                    let mut l = a.listeners.lock();
                    let now = Instant::now();
                    let was = l.get(&from).is_some_and(|until| *until > now);
                    if on {
                        l.insert(from.clone(), now + LISTEN_FOR);
                    } else {
                        l.remove(&from);
                    }
                    was != on
                };
                // A renewal is not news.
                if changed {
                    ctx.sink.emit(CoreEvent::AudioListen { from, on });
                }
            }
            // Whether to play is the host's call.
            "audio_offer" => {
                if !a.sink.load(Ordering::Relaxed) {
                    return;
                }
                let on = msg.json("on").as_bool().unwrap_or(false);
                learned(ctx, &from, false, true);
                if !on {
                    a.listening.lock().remove(&from);
                }
                ctx.sink.emit(CoreEvent::AudioOffer {
                    from,
                    on,
                    rate: msg
                        .json("rate")
                        .as_u64()
                        .and_then(|r| u32::try_from(r).ok())
                        .unwrap_or(0),
                });
            }
            "audio_frame" => {
                if !a.listening.lock().contains(&from) {
                    return;
                }
                let rate = msg.json("rate").as_u64().unwrap_or(0);
                let ch = msg.json("ch").as_u64().unwrap_or(0);
                let (Some(rate), Ok(ch)) = (
                    u32::try_from(rate).ok().filter(|r| RATES.contains(r)),
                    u8::try_from(ch),
                ) else {
                    return;
                };
                if !(1..=2).contains(&ch) {
                    return;
                }
                let data = msg.str("data");
                if data.is_empty() || data.len() > MAX_FRAME_BYTES * 4 / 3 + 4 {
                    return;
                }
                ctx.sink.emit(CoreEvent::AudioFrame {
                    from,
                    seq: msg.json("seq").as_u64().unwrap_or(0),
                    rate,
                    channels: ch,
                    pcm: data,
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_follows_the_link() {
        assert_eq!(rate_for(TransportKind::Quic), Some(RATE_DIRECT));
        assert_eq!(rate_for(TransportKind::Punch), Some(RATE_DIRECT));
        assert_eq!(rate_for(TransportKind::Relay), Some(RATE_RELAY));
        assert_eq!(rate_for(TransportKind::Ble), None);
    }

    #[test]
    fn caps_text_round_trips() {
        assert_eq!(caps_text((true, false)), "sink");
        assert_eq!(caps_text((true, true)), "sink,source");
        assert_eq!(caps_text((false, false)), "");
    }
}
