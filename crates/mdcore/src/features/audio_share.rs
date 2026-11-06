//! One device's audio played on another, over the encrypted link, like a
//! Bluetooth speaker. Both ways round:
//!
//! - a phone's media audio (music, videos, podcasts; never calls) on a
//!   computer's speakers: the phone captures what its apps play (Android's
//!   playback capture, after its own prompt every time);
//! - a computer's sound on a phone (and the earbuds on it): the computer
//!   captures what it plays (ScreenCaptureKit on macOS, the default
//!   output's monitor on Linux, WASAPI loopback on Windows).
//!
//! The side that sends is the *source*, the side that plays the *sink*.
//! The source sends raw PCM; the sink buffers a little and plays it. The
//! core carries the messages; the host apps capture and play.
//!
//!   audio_play  { from, on }                       sink → source: "play
//!                                                  your audio here" (`on`,
//!                                                  renewed), or "stop" (also
//!                                                  the sink's "no" to an
//!                                                  offer)
//!   audio_offer { from, on, rate, ch }             source → sink: "I'm
//!                                                  playing my audio to you"
//!                                                  (`on`), or "I stopped"
//!                                                  (also its "no" to a
//!                                                  listen)
//!   audio_frame { from, seq, rate, ch, data }      source → sink (data:
//!                                                  base64 PCM, s16le,
//!                                                  interleaved)
//!
//! - Only to devices that said they understand it: a device that plays
//!   says `"audio_sink": true` in its status, one that sends
//!   `"audio_source": true`, and a device may say both
//!   (`Engine::set_audio_roles`; the core adds them to every status reply
//!   and remembers what each peer said, or showed by sending one of these
//!   messages). The first apps had one direction only (computers said
//!   `audio_sink`, phones `audio_source`); an app older still doesn't know
//!   these messages and would show them as chat. So the core sends
//!   `audio_play` only to a device that said `audio_source`, and
//!   `audio_offer` / `audio_frame` only to one that said `audio_sink`
//!   (`Unsupported` otherwise): a one-direction app never gets a message
//!   for the direction it doesn't have.
//! - A listen lasts `LISTEN_FOR` unless renewed: the sink renews it every
//!   `RENEW_EVERY` while it plays, so a sink that went away (crashed, lost
//!   its link) stops the stream by itself.
//! - The source sends only to the sinks its owner chose that listen
//!   (`Engine::audio_frame_to`); a sink plays only what comes from a
//!   source it listens to (`Engine::audio_listen`). Whoever starts it, the
//!   other side's owner says yes there (a phone never starts playing a
//!   computer's sound without a tap; a computer says, every time, that its
//!   sound is playing on a phone).
//! - Frames go only over a live link, never queued (late audio is
//!   noise). 48 kHz stereo over a direct link, 24 kHz through the relay
//!   (half the bytes), none over Bluetooth (far too slow): see
//!   `rate_for`. Each frame says its rate and channels, so the sink
//!   follows a change.
//! - Only your own devices (decrypted with your key, in this device's
//!   list).
//! - To a device whose QUIC link reads binary media frames, a frame goes as
//!   one (the PCM itself, as a DATAGRAM when it fits; `transport::media`),
//!   not as base64 twice over: about 1.5 Mbit/s for 48 kHz stereo instead
//!   of 2.8. It arrives as the same `CoreEvent::AudioFrame`.
//! - To a device that says `audio_codec: 1` and isn't on a local link
//!   (`features::nearby`), a frame goes as two 10 ms IMA ADPCM records (a
//!   quarter of the bytes; `features::audio_codec`), and over QUIC each
//!   also carries the one before it, so a lost DATAGRAM is recovered from
//!   the next (counted in `rx_stats`). Repeats are dropped by `seq`. On a
//!   local link it stays PCM. The JSON message carries the time it was
//!   sent (`ts`), as a binary record does, for the latency
//!   (`features::nearby`).

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
use crate::transport::media::{self, Sent};
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

/// What a device says in its status: it plays another device's audio.
pub const SINK_FLAG: &str = "audio_sink";
/// What a device says in its status: it sends its own audio.
pub const SOURCE_FLAG: &str = "audio_source";

const CAPS_PREFIX: &str = "audio_caps:";

/// What a device says in its status and `addr_update`: it reads ADPCM and
/// redundant audio records (`features::audio_codec`).
pub const CODEC_FLAG: &str = "audio_codec";
const CODEC_VERSION: u64 = 1;
/// Seqs remembered per source, to drop repeats.
const SEEN: usize = 128;

/// What arrived from one source (`Engine::audio_rx_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AudioRxStats {
    /// Records played (each 10 or 20 ms).
    pub frames: u32,
    /// Lost ones recovered from the redundant copy in the next.
    pub recovered: u32,
    /// Repeats dropped.
    pub duplicates: u32,
}

#[derive(Default)]
struct Rx {
    session: u32,
    seen: std::collections::VecDeque<u64>,
    stats: AudioRxStats,
}

impl Rx {
    fn seen(&self, seq: u64) -> bool {
        self.seen.contains(&seq)
    }
    fn mark(&mut self, seq: u64) {
        self.seen.push_back(seq);
        while self.seen.len() > SEEN {
            self.seen.pop_front();
        }
    }
    fn oldest(&self) -> Option<u64> {
        self.seen.iter().min().copied()
    }

    /// Record `seq` (of sender run `session`), carrying a redundant copy
    /// of `red` if any: (play the redundant copy first, play this one).
    /// The copy is played only when its record never came and isn't older
    /// than everything remembered (then it's too late anyway); a repeat is
    /// dropped.
    fn accept(&mut self, session: u32, seq: u64, red: Option<u64>) -> (bool, bool) {
        if self.session != session {
            *self = Rx {
                session,
                ..Default::default()
            };
        }
        if self.seen(seq) {
            self.stats.duplicates = self.stats.duplicates.saturating_add(1);
            return (false, false);
        }
        let recover = red.is_some_and(|rs| self.oldest().is_some_and(|o| rs > o) && !self.seen(rs));
        if let (true, Some(rs)) = (recover, red) {
            self.mark(rs);
            self.stats.recovered = self.stats.recovered.saturating_add(1);
            self.stats.frames = self.stats.frames.saturating_add(1);
        }
        self.mark(seq);
        self.stats.frames = self.stats.frames.saturating_add(1);
        (recover, true)
    }
}

/// The sender's ADPCM coder (for one format) and its last 10 ms record
/// (its seq and payload), for the redundant copy in the next.
#[derive(Default)]
struct Coder {
    format: (u32, u8),
    enc: super::audio_codec::Encoder,
    last: Option<(u64, Vec<u8>)>,
}

/// Say this device reads the smaller audio.
pub fn extend_codec<P: Platform>(ctx: &NetCtx<P>, status: &mut serde_json::Value) {
    if ctx.config.audio_codec {
        status[CODEC_FLAG] = CODEC_VERSION.into();
    }
}

/// `node` said (`flag`) whether it reads the smaller audio.
pub fn remember_codec<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &serde_json::Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= CODEC_VERSION);
    ctx.extras.audio.codec.lock().insert(node.to_string(), on);
}

/// `node` reads the smaller audio (and this build sends it).
pub fn codec_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.audio_codec
        && ctx
            .extras
            .audio
            .codec
            .lock()
            .get(node)
            .copied()
            .unwrap_or(false)
}

/// What arrived from source `node` so far.
pub fn rx_stats<P: Platform>(ctx: &NetCtx<P>, node: &str) -> AudioRxStats {
    ctx.extras
        .audio
        .rx
        .lock()
        .get(node)
        .map(|r| r.stats)
        .unwrap_or_default()
}

/// Both sides' state.
#[derive(Default)]
pub struct AudioShare {
    sink: AtomicBool,
    source: AtomicBool,
    /// Source: listening sink -> when its listen ends.
    listeners: Mutex<HashMap<String, Instant>>,
    /// Sink: the sources it listens to.
    listening: Mutex<HashSet<String>>,
    /// What each peer said it understands: (sink, source). `None` until
    /// read from `kv`.
    caps: Mutex<HashMap<String, (bool, bool)>>,
    seq: AtomicU64,
    /// Who said `audio_codec`.
    codec: Mutex<HashMap<String, bool>>,
    coder: Mutex<Coder>,
    /// Sink: per source, what arrived.
    rx: Mutex<HashMap<String, Rx>>,
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

/// `node` sent one of these messages, so it understands them: a device
/// that asks to listen plays audio, one that offers sends it (before this
/// device ever asked for its status).
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

const OLD_SINK: &str = "that device's my-domain can't play another device's audio yet: update it";
const OLD_SOURCE: &str = "that device's my-domain can't send its audio (an up-to-date app is needed; on a phone, Android 10 or later)";

/// Sink: ask source `node` to play its audio here (`on`, renew every
/// `RENEW_EVERY`), or to stop (also the answer "no" to its offer). Only to
/// a device that said `audio_source`.
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

/// Source: the sinks listening now (expired listens are dropped).
pub fn listeners<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let now = Instant::now();
    let mut l = ctx.extras.audio.listeners.lock();
    l.retain(|_, until| *until > now);
    let mut list: Vec<String> = l.keys().cloned().collect();
    list.sort();
    list
}

/// Source: tell sink `node` this device plays its audio to it (`on`), or
/// stopped (also the answer "no" to its listen). Returns the rate to
/// capture at for its link (0 when stopping). Only to a device that said
/// `audio_sink`; over a live link only.
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

/// Source: send `pcm` (s16le, interleaved, `channels` of `rate` Hz) to
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
    // Two seqs a frame: the ADPCM path sends it as two halves.
    let seq = ctx.extras.audio.seq.fetch_add(2, Ordering::Relaxed);
    let ts_ms = media::now_ms();
    let header = media::Header {
        session: ctx.quic.media_session,
        seq,
        ts_ms,
        format: media::Format::Pcm {
            rate,
            channels,
            flags: 0,
        },
    };
    // The ADPCM halves, coded once if a peer takes them.
    let mut coded: Option<Halves> = None;
    // The JSON message, made only if a peer needs it.
    let mut msg: Option<String> = None;
    let sent = to
        .iter()
        .filter(|n| {
            crate::features::nearby::sent(ctx, n);
            let path = media::binary_path(ctx, n);
            if path.is_some() && codec_of(ctx, n) && !crate::features::nearby::low_latency(ctx, n) {
                let halves =
                    coded.get_or_insert_with(|| code_halves(ctx, pcm, rate, channels, seq));
                // Redundancy only where a record can be lost (QUIC
                // DATAGRAMs); the relay is a reliable stream.
                return send_halves(ctx, n, &header, halves, path == Some(true));
            }
            match media::send(ctx, n, &header, pcm) {
                Sent::Yes => true,
                Sent::Dropped => false,
                Sent::NotBinary => {
                    let msg = msg.get_or_insert_with(|| {
                        let me = ctx.identity.lock().node_id.clone();
                        json!({
                            "type": "audio_frame", "from": me, "seq": seq, "rate": rate,
                            "ch": channels, "ts": ts_ms,
                            "data": base64::engine::general_purpose::STANDARD.encode(pcm),
                        })
                        .to_string()
                    });
                    send(ctx, n, msg).is_ok()
                }
            }
        })
        .count();
    if let Some([_, (s, data, _)]) = coded {
        ctx.extras.audio.coder.lock().last = Some((s, data));
    }
    Ok(sent)
}

/// A frame as two ADPCM records: (seq, payload, RED payload with the
/// record before it, when there is one).
type Halves = [(u64, Vec<u8>, Option<Vec<u8>>); 2];

/// Code a frame as two ADPCM halves, seqs `seq` and `seq + 1`.
fn code_halves<P: Platform>(
    ctx: &NetCtx<P>,
    pcm: &[u8],
    rate: u32,
    channels: u8,
    seq: u64,
) -> Halves {
    use super::audio_codec::pack_red;
    let mut c = ctx.extras.audio.coder.lock();
    if c.format != (rate, channels) {
        *c = Coder {
            format: (rate, channels),
            ..Default::default()
        };
    }
    let bytes_per = 2 * usize::from(channels);
    let frames = pcm.len() / bytes_per;
    let cut = frames.div_ceil(2) * bytes_per;
    let a = c.enc.encode(&pcm[..cut], channels);
    let b = c.enc.encode(&pcm[cut..], channels);
    let prev = c.last.take().filter(|(s, _)| s + 1 == seq);
    let red_a = prev.map(|(_, p)| pack_red(&a, 1, &p));
    let red_b = Some(pack_red(&b, 1, &a));
    [(seq, a, red_a), (seq + 1, b, red_b)]
}

/// Send both halves to `node`; true if any went.
fn send_halves<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    header: &media::Header,
    halves: &Halves,
    red: bool,
) -> bool {
    use super::audio_codec::{FLAG_ADPCM, FLAG_RED};
    let media::Format::Pcm { rate, channels, .. } = header.format else {
        return false;
    };
    let mut any = false;
    for (seq, data, with_prev) in halves {
        let (payload, flags) = match (red, with_prev) {
            (true, Some(p)) => (p, FLAG_ADPCM | FLAG_RED),
            _ => (data, FLAG_ADPCM),
        };
        let h = media::Header {
            seq: *seq,
            format: media::Format::Pcm {
                rate,
                channels,
                flags,
            },
            ..*header
        };
        any |= media::send(ctx, node, &h, payload) == Sent::Yes;
    }
    any
}

/// A coded audio record (`flags`: `audio_codec::FLAG_*`) from `from`:
/// recover the one before it from its redundant copy if that was lost,
/// drop repeats, decode, deliver.
pub(crate) fn deliver_coded<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    h: &media::Header,
    payload: &[u8],
) {
    use super::audio_codec::{decode, unpack_red, FLAG_ADPCM, FLAG_RED, KNOWN_FLAGS};
    let media::Format::Pcm {
        rate,
        channels,
        flags,
    } = h.format
    else {
        return;
    };
    if flags & !KNOWN_FLAGS != 0 || !ctx.extras.audio.listening.lock().contains(from) {
        return;
    }
    let (primary, red) = if flags & FLAG_RED != 0 {
        match unpack_red(payload) {
            Some((p, back, r)) => (p, h.seq.checked_sub(u64::from(back)).map(|s| (s, r))),
            None => return,
        }
    } else {
        (payload, None)
    };
    let pcm_of = |data: &[u8]| -> Option<Vec<u8>> {
        if flags & FLAG_ADPCM != 0 {
            decode(data, channels)
        } else {
            Some(data.to_vec())
        }
    };
    // Decoded before it counts: a damaged record isn't played.
    let Some(main) = pcm_of(primary) else {
        return;
    };
    let before = match red {
        Some((rs, r)) => pcm_of(r).map(|p| (rs, p)),
        None => None,
    };
    let (recover, play) = ctx
        .extras
        .audio
        .rx
        .lock()
        .entry(from.to_string())
        .or_default()
        .accept(h.session, h.seq, before.as_ref().map(|(rs, _)| *rs));
    let mut out: Vec<(u64, Vec<u8>)> = Vec::new();
    if let (true, Some(b)) = (recover, before) {
        out.push(b);
    }
    if play {
        out.push((h.seq, main));
    }
    for (seq, pcm) in out {
        deliver(ctx, from, seq, rate, channels, &pcm);
    }
}

/// Audio that came as a binary frame (`transport::media`) from `from`, the
/// link's peer: checked as the JSON one, the same event.
pub(crate) fn deliver<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    seq: u64,
    rate: u32,
    channels: u8,
    pcm: &[u8],
) {
    if pcm.is_empty()
        || pcm.len() > MAX_FRAME_BYTES
        || !RATES.contains(&rate)
        || !(1..=2).contains(&channels)
        || !ctx.peers.lock().contains_key(from)
        || !ctx.extras.audio.listening.lock().contains(from)
    {
        return;
    }
    ctx.sink.emit(CoreEvent::AudioFrame {
        from: from.to_string(),
        seq,
        rate,
        channels,
        pcm: base64::engine::general_purpose::STANDARD.encode(pcm),
    });
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
                // A renewal is not news. A stop always is: it's also the
                // sink's "no" to an offer this device made.
                if changed || !on {
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
                crate::features::nearby::received(ctx, &from, msg.json("ts").as_u64().unwrap_or(0));
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
    fn a_lost_record_is_recovered_from_the_next_once() {
        let mut rx = Rx::default();
        assert_eq!(rx.accept(7, 10, None), (false, true));
        assert_eq!(rx.accept(7, 11, Some(10)), (false, true), "10 came");
        // 12 lost: 13 carries it.
        assert_eq!(rx.accept(7, 13, Some(12)), (true, true));
        // 12 turns up late after all: a repeat.
        assert_eq!(rx.accept(7, 12, Some(11)), (false, false));
        assert_eq!(rx.accept(7, 13, Some(12)), (false, false), "duplicate");
        assert_eq!(
            rx.stats,
            AudioRxStats {
                frames: 4,
                recovered: 1,
                duplicates: 2
            }
        );
        // Too old to matter: not played.
        let mut rx = Rx::default();
        for s in 100..300 {
            rx.accept(1, s, None);
        }
        assert_eq!(rx.accept(1, 301, Some(5)), (false, true));
        // The sender restarted: counting starts over.
        assert_eq!(rx.accept(2, 0, None), (false, true));
        assert_eq!(rx.stats.frames, 1);
    }

    #[test]
    fn caps_text_round_trips() {
        assert_eq!(caps_text((true, false)), "sink");
        assert_eq!(caps_text((true, true)), "sink,source");
        assert_eq!(caps_text((false, false)), "");
    }
}
