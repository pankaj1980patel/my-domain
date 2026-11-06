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
//! - To a device that says `audio_codec: 2`, Opus instead
//!   (`features::opus_audio`): 20 ms frames, about 96 kbit/s stereo, with
//!   in-band FEC set from the loss measured on the link (QUIC's count, and
//!   the `loss` the sink says in its `audio_play` renewals), RED over QUIC,
//!   and lost frames filled by the copy, FEC or Opus's concealment. Opus
//!   at any link rate: the source captures at 48 kHz (`offer`).
//! - Over Bluetooth LE, only to such a device: Opus records (three frames,
//!   60 ms, a record; about 32 kbit/s) go as binary frames on the
//!   Bluetooth link (`engine::ble`). Which way a stream goes is the
//!   route ([`AudioRoute`], per device, `Engine::set_audio_route`): Auto
//!   (a network link if there is one, else Bluetooth), Wi-Fi only, or
//!   Bluetooth only (kept even with a network link). The sink says its
//!   choice in `audio_play` (`route`); the source's own choice wins when
//!   it made one. `Engine::audio_via` says which way it went last.

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

use super::opus_audio::Profile;
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
/// `audio_codec` from a device that reads Opus too.
const OPUS_VERSION: u64 = 2;
/// The route chosen for a device, kept across restarts.
const ROUTE_PREFIX: &str = "audio_route:";
/// How long `audio_via` remembers the way frames went.
const VIA_FOR: Duration = Duration::from_secs(2);

/// Which way audio with a device goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioRoute {
    /// A network link (Wi-Fi, direct, relay) when there is one, else
    /// Bluetooth.
    #[default]
    Auto,
    /// Never over Bluetooth.
    Network,
    /// Over Bluetooth even with a network link.
    Bluetooth,
}

impl AudioRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            AudioRoute::Auto => "auto",
            AudioRoute::Network => "wifi",
            AudioRoute::Bluetooth => "bluetooth",
        }
    }
    pub fn parse(s: &str) -> AudioRoute {
        match s {
            "wifi" | "network" => AudioRoute::Network,
            "bluetooth" | "ble" => AudioRoute::Bluetooth,
            _ => AudioRoute::Auto,
        }
    }
}

/// Where a stream's frames go now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioPath {
    /// Over the network link (`ws_conns`' route).
    Network,
    /// Over the Bluetooth link, as binary Opus records.
    Bluetooth,
}

/// The way frames go with `choice`, given a network link (`ip`) and a
/// Bluetooth link that carries audio (`ble`).
pub fn choose_path(choice: AudioRoute, ip: bool, ble: bool) -> Option<AudioPath> {
    match choice {
        AudioRoute::Auto if ip => Some(AudioPath::Network),
        AudioRoute::Auto | AudioRoute::Bluetooth => ble.then_some(AudioPath::Bluetooth),
        AudioRoute::Network => ip.then_some(AudioPath::Network),
    }
}
/// Seqs remembered per source, to drop repeats.
const SEEN: usize = 128;

/// What arrived from one source (`Engine::audio_rx_stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AudioRxStats {
    /// Records played (each 10 or 20 ms).
    pub frames: u32,
    /// Lost ones recovered from the redundant copy in the next (or,
    /// with Opus, its in-band FEC).
    pub recovered: u32,
    /// Repeats dropped.
    pub duplicates: u32,
    /// Lost ones Opus made up (packet-loss concealment).
    pub concealed: u32,
}

#[derive(Default)]
struct Rx {
    session: u32,
    seen: std::collections::VecDeque<u64>,
    stats: AudioRxStats,
    /// The newest seq delivered (any codec): Opus frames are numbered on
    /// after it, so the players' buffers see seqs only grow.
    last_out: Option<u64>,
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
        let v = if ctx.config.audio_opus {
            OPUS_VERSION
        } else {
            CODEC_VERSION
        };
        status[CODEC_FLAG] = v.into();
    }
}

/// `node` said (`flag`) whether it reads the smaller audio.
pub fn remember_codec<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &serde_json::Value) {
    if node.is_empty() {
        return;
    }
    let v = flag.as_u64().unwrap_or(0);
    ctx.extras.audio.codec.lock().insert(node.to_string(), v);
}

fn codec_version<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    ctx.extras
        .audio
        .codec
        .lock()
        .get(node)
        .copied()
        .unwrap_or(0)
}

/// `node` reads Opus (and this build sends it).
pub fn opus_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.audio_codec && ctx.config.audio_opus && codec_version(ctx, node) >= OPUS_VERSION
}

/// `node` reads the smaller audio (and this build sends it).
pub fn codec_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.audio_codec && codec_version(ctx, node) >= CODEC_VERSION
}

/// The route this device's owner chose for `node` (kept across restarts).
pub fn route_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> AudioRoute {
    if let Some(r) = ctx.extras.audio.routes.lock().get(node) {
        return *r;
    }
    let r = AudioRoute::parse(
        &ctx.platform
            .kv_get(&format!("{ROUTE_PREFIX}{node}"))
            .unwrap_or_default(),
    );
    ctx.extras.audio.routes.lock().insert(node.to_string(), r);
    r
}

/// Choose the route for audio with `node`.
pub fn set_route<P: Platform>(ctx: &NetCtx<P>, node: &str, route: AudioRoute) {
    ctx.extras
        .audio
        .routes
        .lock()
        .insert(node.to_string(), route);
    ctx.platform
        .kv_set(&format!("{ROUTE_PREFIX}{node}"), route.as_str());
}

/// The route that counts for `node`: this device's choice, else the one
/// the other device said in its `audio_play`.
pub fn effective_route<P: Platform>(ctx: &NetCtx<P>, node: &str) -> AudioRoute {
    match route_of(ctx, node) {
        AudioRoute::Auto => ctx
            .extras
            .audio
            .their_route
            .lock()
            .get(node)
            .copied()
            .unwrap_or_default(),
        r => r,
    }
}

/// Whether the link to `node` now is a network one (not Bluetooth).
fn network_link<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    let Some(id) = ctx.ws_conns.lock().get(node).map(|(id, _)| *id) else {
        return false;
    };
    !crate::engine::is_ble_link(ctx, id)
}

/// Where audio with `node` goes now (`None`: no way it can).
pub fn path_for<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<AudioPath> {
    let ble = opus_of(ctx, node) && crate::engine::ble_linked(ctx, node);
    choose_path(effective_route(ctx, node), network_link(ctx, node), ble)
}

/// A stream with `node` wants its Bluetooth link kept (Bluetooth only was
/// chosen, and it plays or listens), even with a network link.
pub fn wants_ble<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    effective_route(ctx, node) == AudioRoute::Bluetooth
        && opus_of(ctx, node)
        && (ctx.extras.audio.listening.lock().contains(node)
            || listeners(ctx).iter().any(|n| n == node))
}

/// Frames went to or came from `node` over Bluetooth (`ble`) or not.
pub(crate) fn note_via<P: Platform>(ctx: &NetCtx<P>, node: &str, ble: bool) {
    ctx.extras
        .audio
        .via
        .lock()
        .insert(node.to_string(), (ble, Instant::now()));
}

/// Which way audio with `node` went in the last moments: "bluetooth",
/// "network", or "" (none lately).
pub fn via<P: Platform>(ctx: &NetCtx<P>, node: &str) -> &'static str {
    match ctx.extras.audio.via.lock().get(node) {
        Some((ble, at)) if at.elapsed() < VIA_FOR => {
            if *ble {
                "bluetooth"
            } else {
                "network"
            }
        }
        _ => "",
    }
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
    /// What each device said in `audio_codec` (1 ADPCM, 2 Opus too).
    codec: Mutex<HashMap<String, u64>>,
    coder: Mutex<Coder>,
    /// Sink: per source, what arrived.
    rx: Mutex<HashMap<String, Rx>>,
    /// Source: an Opus encoder per sink.
    opus_enc: Mutex<HashMap<String, super::opus_audio::Enc>>,
    /// Sink: an Opus decoder per source (with the sender's run).
    opus_dec: Mutex<HashMap<String, (u32, super::opus_audio::Dec)>>,
    /// Source: the loss each sink said it saw (percent).
    their_loss: Mutex<HashMap<String, f32>>,
    /// The route chosen here per device (read from `kv` once).
    routes: Mutex<HashMap<String, AudioRoute>>,
    /// The route each device said in its `audio_play`.
    their_route: Mutex<HashMap<String, AudioRoute>>,
    /// Which way frames last went or came, per device, and when.
    via: Mutex<HashMap<String, (bool, Instant)>>,
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
    let Some(route) = ctx.ws_conns.lock().get(node).map(|(id, _)| *id) else {
        return Err(CoreError::NoLink("not connected".into()));
    };
    let kind = if crate::engine::is_ble_link(ctx, route) {
        Some(TransportKind::Ble)
    } else {
        kind
    };
    // A route with no link yet (the relay's, before the peer answered):
    // treat as the relay.
    rate_for(kind.unwrap_or(TransportKind::Relay)).ok_or(CoreError::Unsupported(
        "the link is Bluetooth only, too slow for audio: get both on the same Wi-Fi or online",
    ))
}

/// The rate to capture at for `node`: 48 kHz for Opus whichever way it
/// goes (a binary path on the network, or Bluetooth), else the link's.
fn offer_rate<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<u32> {
    if !opus_of(ctx, node) {
        return link_rate(ctx, node);
    }
    match path_for(ctx, node) {
        Some(AudioPath::Bluetooth) => Ok(RATE_DIRECT),
        Some(AudioPath::Network) if media::binary_path(ctx, node).is_some() => Ok(RATE_DIRECT),
        Some(AudioPath::Network) => link_rate(ctx, node),
        None if !ctx.ws_conns.lock().contains_key(node) => {
            Err(CoreError::NoLink("not connected".into()))
        }
        None => Err(CoreError::Unsupported(match effective_route(ctx, node) {
            AudioRoute::Bluetooth => {
                "Bluetooth only was chosen, and there's no Bluetooth link to that device"
            }
            _ => "the link is Bluetooth only: choose Auto or Bluetooth for this device's audio",
        })),
    }
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
    let mut msg = json!({ "type": "audio_play", "from": me, "on": on });
    // Newer sources read these; older ones ignore them.
    if ctx.config.audio_opus {
        msg["route"] = route_of(ctx, node).as_str().into();
        let loss = ctx
            .extras
            .audio
            .opus_dec
            .lock()
            .get_mut(node)
            .and_then(|(_, d)| d.take_loss());
        if let Some(l) = loss {
            msg["loss"] = (f64::from((l * 10.0).round()) / 10.0).into();
        }
    }
    send(ctx, node, &msg.to_string())
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
        offer_rate(ctx, node)?
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
            let way = if opus_of(ctx, n) {
                path_for(ctx, n)
            } else {
                // Never over Bluetooth to an app without Opus.
                network_link(ctx, n).then_some(AudioPath::Network)
            };
            match way {
                None => return false,
                Some(AudioPath::Bluetooth) => {
                    note_via(ctx, n, true);
                    return send_opus(ctx, n, &header, pcm, Profile::Bluetooth).unwrap_or(false);
                }
                Some(AudioPath::Network) => note_via(ctx, n, false),
            }
            crate::features::nearby::sent(ctx, n);
            let path = media::binary_path(ctx, n);
            let low = crate::features::nearby::low_latency(ctx, n);
            if path.is_some() && !low && opus_of(ctx, n) {
                let profile = Profile::Network {
                    lossy: path == Some(true),
                };
                if let Some(sent) = send_opus(ctx, n, &header, pcm, profile) {
                    return sent;
                }
            } else {
                flush_opus(ctx, n);
            }
            if path.is_some() && codec_of(ctx, n) && !low {
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

/// The loss (percent) to expect sending to `node`: the larger of what it
/// said and QUIC's own count.
fn loss_to<P: Platform>(ctx: &NetCtx<P>, node: &str) -> f32 {
    let said = ctx
        .extras
        .audio
        .their_loss
        .lock()
        .get(node)
        .copied()
        .unwrap_or(0.0);
    let quic = ctx
        .quic
        .path_stats(node)
        .and_then(|(_, lost, sent)| crate::transport::quality::loss_pct(lost, sent))
        .unwrap_or(0.0);
    said.max(quic)
}

/// Code `pcm` as Opus for `node` and send what's ready (over Bluetooth
/// for that profile). `None`: Opus can't take this format (send it
/// another way).
fn send_opus<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    header: &media::Header,
    pcm: &[u8],
    profile: Profile,
) -> Option<bool> {
    use super::opus_audio::{Enc, FLAG_OPUS};
    let media::Format::Pcm { rate, channels, .. } = header.format else {
        return None;
    };
    let loss = loss_to(ctx, node);
    let ready = {
        let mut encs = ctx.extras.audio.opus_enc.lock();
        let next = encs.get(node).map_or(0, Enc::next);
        let fits = encs
            .get(node)
            .is_some_and(|e| e.format() == (rate, channels, profile));
        if !fits {
            encs.insert(node.to_string(), Enc::new(rate, channels, profile, next)?);
        }
        let enc = encs.get_mut(node)?;
        enc.set_loss(loss);
        enc.push(pcm)
    };
    let ble = profile == Profile::Bluetooth;
    let mut any = ready.is_empty();
    for (_, payload) in ready {
        let h = media::Header {
            format: media::Format::Pcm {
                rate,
                channels,
                flags: FLAG_OPUS,
            },
            ..*header
        };
        let sent = if ble {
            crate::engine::ble_send_media(ctx, node, &h, &payload)
        } else {
            media::send(ctx, node, &h, &payload)
        };
        any |= sent == Sent::Yes;
    }
    Some(any)
}

/// The stream to `node` went another way for now: what the Opus encoder
/// holds is stale (its frame numbers carry on).
fn flush_opus<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if let Some(e) = ctx.extras.audio.opus_enc.lock().get_mut(node) {
        e.flush();
    }
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
    if flags == super::opus_audio::FLAG_OPUS {
        deliver_opus(ctx, from, h, payload);
        return;
    }
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

/// An Opus record from `from`: decode it, fill what was lost before it
/// (the copy, FEC or concealment), drop repeats, deliver in order.
fn deliver_opus<P: Platform>(ctx: &NetCtx<P>, from: &str, h: &media::Header, payload: &[u8]) {
    use super::opus_audio::{Dec, Fill};
    let media::Format::Pcm { rate, channels, .. } = h.format else {
        return;
    };
    if !ctx.config.audio_opus
        || !ctx.extras.audio.listening.lock().contains(from)
        || !ctx.peers.lock().contains_key(from)
    {
        return;
    }
    let out = {
        let mut decs = ctx.extras.audio.opus_dec.lock();
        let fresh = !decs
            .get(from)
            .is_some_and(|(s, d)| *s == h.session && d.format() == (rate, channels));
        if fresh {
            let Some(d) = Dec::new(rate, channels) else {
                return;
            };
            decs.insert(from.to_string(), (h.session, d));
        }
        let Some((_, dec)) = decs.get_mut(from) else {
            return;
        };
        let dups = dec.stats.duplicates;
        let Some(out) = dec.receive(payload) else {
            return;
        };
        let dup = dec.stats.duplicates != dups;
        let mut rx = ctx.extras.audio.rx.lock();
        let r = rx.entry(from.to_string()).or_default();
        if r.session != h.session {
            *r = Rx {
                session: h.session,
                ..Default::default()
            };
        }
        let st = &mut r.stats;
        if dup {
            st.duplicates = st.duplicates.saturating_add(1);
        }
        for (_, how, _) in &out {
            match how {
                Fill::Concealed => st.concealed = st.concealed.saturating_add(1),
                Fill::Received => st.frames = st.frames.saturating_add(1),
                Fill::Redundant | Fill::Fec => {
                    st.frames = st.frames.saturating_add(1);
                    st.recovered = st.recovered.saturating_add(1);
                }
            }
        }
        // Seqs for the players: up to the record's own, after the last.
        let count = out.len() as u64;
        let mut seq = h.seq.saturating_sub(count.saturating_sub(1));
        if let Some(l) = r.last_out {
            seq = seq.max(l.saturating_add(1));
        }
        if count > 0 {
            r.last_out = Some(seq.saturating_add(count - 1));
        }
        (seq, out)
    };
    let (first, out) = out;
    for (i, (_, _, pcm)) in out.into_iter().enumerate() {
        deliver(ctx, from, first + i as u64, rate, channels, &pcm);
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
    {
        let mut rx = ctx.extras.audio.rx.lock();
        let r = rx.entry(from.to_string()).or_default();
        r.last_out = Some(r.last_out.map_or(seq, |l| l.max(seq)));
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
                if let Some(r) = msg.json("route").as_str() {
                    a.their_route
                        .lock()
                        .insert(from.clone(), AudioRoute::parse(r));
                }
                if let Some(l) = msg.json("loss").as_f64() {
                    let l = (l as f32).clamp(0.0, 100.0);
                    a.their_loss.lock().insert(from.clone(), l);
                }
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
                duplicates: 2,
                concealed: 0,
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
    fn the_route_picks_the_network_first_on_auto() {
        use AudioPath as P;
        use AudioRoute as R;
        assert_eq!(choose_path(R::Auto, true, true), Some(P::Network));
        assert_eq!(choose_path(R::Auto, false, true), Some(P::Bluetooth));
        assert_eq!(choose_path(R::Auto, true, false), Some(P::Network));
        assert_eq!(choose_path(R::Auto, false, false), None);
        assert_eq!(choose_path(R::Network, false, true), None);
        assert_eq!(choose_path(R::Network, true, true), Some(P::Network));
        assert_eq!(choose_path(R::Bluetooth, true, true), Some(P::Bluetooth));
        assert_eq!(choose_path(R::Bluetooth, true, false), None);
        for r in [R::Auto, R::Network, R::Bluetooth] {
            assert_eq!(AudioRoute::parse(r.as_str()), r);
        }
        assert_eq!(AudioRoute::parse("anything"), R::Auto);
    }

    #[test]
    fn caps_text_round_trips() {
        assert_eq!(caps_text((true, false)), "sink");
        assert_eq!(caps_text((true, true)), "sink,source");
        assert_eq!(caps_text((false, false)), "");
    }
}
