//! Now playing and media controls: what's playing on a device (title,
//! artist, app, artwork, times, playing or paused), shown on your other
//! devices, which can play, pause, skip, seek and set the volume there. The
//! host app reads it from the OS (Android media sessions, `playerctl` on
//! Linux, ...) and hands it to the core with `Engine::set_now_playing`
//! whenever it changes.
//!
//!   now_playing { from, media }        device → devices watching it
//!   media_cmd   { from, action, value?, session? }
//!                                      a watcher → the device playing
//!   media_watch { from }               "I'm looking: keep me posted"
//!
//! A device that asks for this one's status (`status_request`), sends it a
//! `media_watch` or a command is watching it for `WATCH_FOR`: changes are
//! pushed to it while it has a live link (never connecting or waking it
//! just for a song change). The status reply carries the current one too
//! (`"now_playing"`), so a device that just looked never misses it. `media`
//! is `null` when nothing is playing.
//!
//! Media controls (`media_cmd`, `media_watch`, and the fuller now-playing:
//! `duration` / `position` in ms, `can_next` / `can_prev` / `can_seek`,
//! `volume` / `volume_max`, `art` and `icon` as base64 images, `session` and
//! `sessions` when several apps play) only go between devices that said
//! `media_ctl: 1` in their status or `addr_update`; an older app gets the
//! plain `{title, artist, album, app, state}` and never a command.
//!
//! - Pushes are throttled: at most one per `MIN_GAP` (the last change in a
//!   burst still goes, a moment later), and a position that only moved on
//!   with the clock is not a change (a seek is).
//! - The artwork goes once per watcher: after that, while it's the same
//!   picture, the push carries `art_hash` instead and the receiving core
//!   puts the picture it kept back in (the same for `icon`).
//! - The position is brought up to now when it's sent (a status reply
//!   minutes after the last change still says where the song is).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device takes media
/// commands and sends the fuller now-playing, and its version.
pub const FLAG: &str = "media_ctl";
const VERSION: u64 = 1;
/// How long asking for a device's status keeps you hearing its changes.
pub const WATCH_FOR: Duration = Duration::from_secs(10 * 60);
/// At most one push per this long.
pub const MIN_GAP: Duration = Duration::from_secs(1);
/// A position this far (ms) from where the clock says it should be is a
/// seek, worth telling.
pub const DRIFT_MS: u64 = 2_000;
/// Longest kept text field.
const MAX_FIELD: usize = 300;
/// Longest artwork (base64 characters: a 256 px JPEG fits easily).
pub const MAX_ART: usize = 96 * 1024;
/// Longest app icon (base64 characters).
pub const MAX_ICON: usize = 24 * 1024;
/// Most sessions listed.
const MAX_SESSIONS: usize = 8;
/// Longest session id.
const MAX_SESSION_ID: usize = 128;
/// Longest time (ms) taken as a duration or position: a week.
const MAX_MS: u64 = 7 * 24 * 3600 * 1000;
/// Highest volume scale.
const MAX_VOLUME: u64 = 1000;

/// What a `media_cmd` may ask.
pub const ACTIONS: &[&str] = &[
    "play",
    "pause",
    "play_pause",
    "next",
    "previous",
    "seek",
    "volume_up",
    "volume_down",
    "volume_set",
    "session",
];

const OLD_PEER: &str = "this device's app is too old for media controls: update it";

/// At most one send per `gap`: a change inside it waits for the gap's end
/// (only the latest goes then).
#[derive(Debug)]
pub struct Throttle {
    gap: Duration,
    last: Option<Instant>,
    pending: bool,
}

impl Default for Throttle {
    fn default() -> Self {
        Throttle::new(MIN_GAP)
    }
}

impl Throttle {
    pub fn new(gap: Duration) -> Self {
        Throttle {
            gap,
            last: None,
            pending: false,
        }
    }

    /// A change at `now`: `None` to send it now (counted as sent), else how
    /// long until [`Throttle::flush`] may send it.
    pub fn offer(&mut self, now: Instant) -> Option<Duration> {
        match self.last {
            Some(t) if now.saturating_duration_since(t) < self.gap => {
                self.pending = true;
                Some(self.gap - now.saturating_duration_since(t))
            }
            _ => {
                self.last = Some(now);
                self.pending = false;
                None
            }
        }
    }

    /// Whether the change that waited may go now (counted as sent).
    pub fn flush(&mut self, now: Instant) -> bool {
        let ready = self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) >= self.gap);
        if self.pending && ready {
            self.pending = false;
            self.last = Some(now);
            return true;
        }
        false
    }
}

/// The (art, icon) hashes a watcher has from here.
type SentHashes = (Option<String>, Option<String>);

#[derive(Default)]
struct Now {
    media: Value,
    /// When `media` was set (its position is as of then).
    at: Option<Instant>,
}

/// What's playing here, who's watching, and what the others said.
#[derive(Default)]
pub struct Media {
    now: Mutex<Now>,
    watchers: Mutex<HashMap<String, Instant>>,
    /// Device -> whether it said `media_ctl`.
    caps: Mutex<HashMap<String, bool>>,
    /// Watcher -> the (art, icon) hashes it has from here.
    sent: Mutex<HashMap<String, SentHashes>>,
    /// Device -> the artwork and icon it sent last, by hash.
    seen: Mutex<HashMap<String, Pictures>>,
    throttle: Mutex<Throttle>,
    flushing: AtomicBool,
}

#[derive(Default, Clone)]
struct Pictures {
    art: Option<(String, String)>,
    icon: Option<(String, String)>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &Media {
    &ctx.media
}

/// A short, stable hash of a picture (FNV-1a, 64 bits, as hex): the same
/// on every device and build.
pub fn picture_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn clean_text(v: Option<&Value>, max: usize) -> Option<String> {
    v.and_then(Value::as_str)
        .map(|s| {
            s.trim()
                .chars()
                .filter(|c| !c.is_control())
                .take(max)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
}

fn is_base64(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
}

fn picture(v: Option<&Value>, max: usize) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= max && is_base64(s))
        .map(String::from)
}

fn ms(v: Option<&Value>) -> Option<u64> {
    let n = v?;
    let n = n
        .as_u64()
        .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))?;
    Some(n.min(MAX_MS))
}

fn state_of(v: Option<&Value>) -> &'static str {
    match v.and_then(Value::as_str) {
        Some("playing") => "playing",
        Some("paused") => "paused",
        _ => "stopped",
    }
}

/// The fields other devices get, cleaned up: `title`, `artist`, `album`,
/// `app` (short strings, left out when empty) and `state` ("playing" or
/// "paused"), and when the host has them `duration` / `position` (ms),
/// `can_next` / `can_prev` / `can_seek`, `volume` / `volume_max`, `art` /
/// `icon` (base64 pictures), `session` and `sessions`. `Null` when there's
/// nothing to show.
pub fn normalize(raw: &Value) -> Value {
    let Some(o) = raw.as_object() else {
        return Value::Null;
    };
    let state = state_of(o.get("state"));
    let title = clean_text(o.get("title"), MAX_FIELD);
    if title.is_none() || state == "stopped" {
        return Value::Null;
    }
    let mut out = json!({ "title": title, "state": state });
    for k in ["artist", "album", "app"] {
        if let Some(v) = clean_text(o.get(k), MAX_FIELD) {
            out[k] = v.into();
        }
    }
    let duration = ms(o.get("duration")).filter(|d| *d > 0);
    if let Some(d) = duration {
        out["duration"] = d.into();
    }
    if let Some(p) = ms(o.get("position")) {
        out["position"] = duration.map_or(p, |d| p.min(d)).into();
    }
    for k in ["can_next", "can_prev", "can_seek"] {
        if let Some(b) = o.get(k).and_then(Value::as_bool) {
            out[k] = b.into();
        }
    }
    let vmax = o
        .get("volume_max")
        .and_then(Value::as_u64)
        .filter(|m| (1..=MAX_VOLUME).contains(m));
    if let (Some(m), Some(v)) = (vmax, o.get("volume").and_then(Value::as_u64)) {
        out["volume"] = v.min(m).into();
        out["volume_max"] = m.into();
    }
    if let Some(a) = picture(o.get("art"), MAX_ART) {
        out["art"] = a.into();
    }
    if let Some(i) = picture(o.get("icon"), MAX_ICON) {
        out["icon"] = i.into();
    }
    if let Some(s) = clean_text(o.get("session"), MAX_SESSION_ID) {
        out["session"] = s.into();
    }
    if let Some(list) = o.get("sessions").and_then(Value::as_array) {
        let sessions: Vec<Value> = list
            .iter()
            .filter_map(|s| {
                let id = clean_text(s.get("id"), MAX_SESSION_ID)?;
                let mut e = json!({ "id": id, "state": state_of(s.get("state")) });
                for k in ["app", "title"] {
                    if let Some(v) = clean_text(s.get(k), MAX_FIELD) {
                        e[k] = v.into();
                    }
                }
                Some(e)
            })
            .take(MAX_SESSIONS)
            .collect();
        if sessions.len() > 1 {
            out["sessions"] = sessions.into();
        }
    }
    out
}

/// What an app from before media controls understands of `media`.
pub fn basic(media: &Value) -> Value {
    if media.is_null() {
        return Value::Null;
    }
    let mut out = json!({ "title": media["title"], "state": media["state"] });
    for k in ["artist", "album", "app"] {
        if let Some(v) = media.get(k) {
            out[k] = v.clone();
        }
    }
    out
}

/// `media` as of `elapsed` after it was set: a playing position moved on
/// (never past the end).
pub fn position_now(media: &Value, elapsed: Duration) -> Value {
    let mut m = media.clone();
    if m["state"] != "playing" {
        return m;
    }
    if let Some(p) = m["position"].as_u64() {
        let mut p = p.saturating_add(elapsed.as_millis().min(u128::from(MAX_MS)) as u64);
        if let Some(d) = m["duration"].as_u64() {
            p = p.min(d);
        }
        m["position"] = p.into();
    }
    m
}

/// Whether `new` is worth telling the watchers after `old`, set `elapsed`
/// ago: anything but a position that only moved on with the clock.
pub fn worth_telling(old: &Value, new: &Value, elapsed: Duration) -> bool {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("position");
        }
        v
    };
    if strip(old) != strip(new) {
        return true;
    }
    match (old["position"].as_u64(), new["position"].as_u64()) {
        (None, None) => false,
        (Some(_), None) | (None, Some(_)) => true,
        (Some(_), Some(p)) => {
            let expected = position_now(old, elapsed)["position"].as_u64().unwrap_or(0);
            p.abs_diff(expected) > DRIFT_MS
        }
    }
}

/// A command as it goes on the wire (without `type` / `from`), checked:
/// `value` is the position in ms for "seek" and the level for
/// "volume_set" (needed there, refused elsewhere); `session` names the
/// session for "session" (needed there; elsewhere it picks which one,
/// when given).
pub fn command(action: &str, value: Option<i64>, session: Option<&str>) -> crate::Result<Value> {
    if !ACTIONS.contains(&action) {
        return Err(CoreError::InvalidInput(format!(
            "not a media command: {action}"
        )));
    }
    let mut cmd = json!({ "action": action });
    match (action, value) {
        ("seek", Some(v)) if (0..=MAX_MS as i64).contains(&v) => cmd["value"] = v.into(),
        ("volume_set", Some(v)) if (0..=MAX_VOLUME as i64).contains(&v) => cmd["value"] = v.into(),
        ("seek" | "volume_set", _) => {
            return Err(CoreError::InvalidInput(format!(
                "{action} needs a value in range"
            )))
        }
        (_, Some(_)) => return Err(CoreError::InvalidInput(format!("{action} takes no value"))),
        _ => {}
    }
    let session = session.map(str::trim).filter(|s| !s.is_empty());
    match session {
        Some(s) if s.len() <= MAX_SESSION_ID && !s.chars().any(char::is_control) => {
            cmd["session"] = s.into()
        }
        Some(_) => return Err(CoreError::InvalidInput("not a session id".into())),
        None if action == "session" => {
            return Err(CoreError::InvalidInput("which session?".into()))
        }
        None => {}
    }
    Ok(cmd)
}

/// A received command, checked the same way: (action, value, session).
pub fn parse_command(v: &Value) -> Option<(String, Option<i64>, Option<String>)> {
    let action = v["action"].as_str()?;
    let value = v["value"].as_i64();
    let session = v["session"].as_str();
    let ok = command(action, value, session).ok()?;
    Some((
        action.to_string(),
        ok["value"].as_i64(),
        ok["session"].as_str().map(String::from),
    ))
}

// --- capability ---------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it takes media
/// commands.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.media_ctl {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `media_ctl` value of its status or
/// `addr_update`) whether it takes them.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    st(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` said it takes media commands.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.media_ctl && st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

/// Whether `node` gets the fuller now-playing: it said so, or hasn't said
/// either way yet (an older app just keeps the fields it knows).
fn full_for<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.media_ctl && st(ctx).caps.lock().get(node).copied().unwrap_or(true)
}

// --- this device's side ---------------------------------------------------------

/// `node` asked for our status: it hears changes for a while.
pub fn watch<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let mut w = st(ctx).watchers.lock();
    w.retain(|_, at| at.elapsed() < WATCH_FOR);
    w.insert(node.to_string(), Instant::now());
}

/// Somebody looked at this device lately (a host can skip polling the OS
/// for what's playing when nobody would hear it).
pub fn watched<P: Platform>(ctx: &NetCtx<P>) -> bool {
    st(ctx)
        .watchers
        .lock()
        .values()
        .any(|at| at.elapsed() < WATCH_FOR)
}

/// What's playing now (`Null` if nothing), its position brought up to now.
pub fn current<P: Platform>(ctx: &NetCtx<P>) -> Value {
    let now = st(ctx).now.lock();
    position_now(&now.media, now.at.map(|t| t.elapsed()).unwrap_or_default())
}

/// What's playing, as `node` gets it in a status reply: everything (the
/// pictures too) for a device with media controls, else the plain fields.
pub fn current_for<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Value {
    media_for(ctx, node, false)
}

/// What's playing as `node` gets it; with `dedupe`, a picture it already
/// has from here goes as its hash.
fn media_for<P: Platform>(ctx: &NetCtx<P>, node: &str, dedupe: bool) -> Value {
    let media = current(ctx);
    if media.is_null() {
        return media;
    }
    if !full_for(ctx, node) {
        return basic(&media);
    }
    let mut media = media;
    let art = media["art"].as_str().map(picture_hash);
    let icon = media["icon"].as_str().map(picture_hash);
    let mut sent = st(ctx).sent.lock();
    let had = sent.get(node).cloned().unwrap_or_default();
    if dedupe {
        if art.is_some() && had.0 == art {
            media.as_object_mut().map(|o| o.remove("art"));
            media["art_hash"] = art.clone().into();
        }
        if icon.is_some() && had.1 == icon {
            media.as_object_mut().map(|o| o.remove("icon"));
            media["icon_hash"] = icon.clone().into();
        }
    }
    sent.insert(node.to_string(), (art, icon));
    media
}

fn send_to<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &str) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(ctx, &name, node, msg)
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Tell `node` what's playing (live link only).
fn tell<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !crate::transport::is_live(ctx, node) {
        return;
    }
    let media = media_for(ctx, node, true);
    let msg = json!({ "type": "now_playing", "from": me(ctx), "media": media }).to_string();
    let _ = send_to(ctx, node, &msg);
}

/// Tell every watcher with a live link.
fn push<P: Platform>(ctx: &NetCtx<P>) {
    let watchers: Vec<String> = {
        let mut w = st(ctx).watchers.lock();
        w.retain(|_, at| at.elapsed() < WATCH_FOR);
        w.keys().cloned().collect()
    };
    for node in watchers {
        tell(ctx, &node);
    }
}

/// The host reports what's playing. When it changed (more than the clock
/// moving the position on), watchers with a live link are told, at most
/// once per `MIN_GAP`.
pub fn set<P: Platform>(ctx: &NetCtx<P>, raw: &Value) {
    let media = normalize(raw);
    {
        let mut now = st(ctx).now.lock();
        let elapsed = now.at.map(|t| t.elapsed()).unwrap_or_default();
        let tell = worth_telling(&now.media, &media, elapsed);
        now.media = media;
        now.at = Some(Instant::now());
        if !tell {
            return;
        }
    }
    let wait = st(ctx).throttle.lock().offer(Instant::now());
    match wait {
        None => push(ctx),
        Some(wait) => {
            if st(ctx).flushing.swap(true, Ordering::AcqRel) {
                return; // a flush is already on its way: it sends the latest
            }
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let mut wait = wait;
                loop {
                    std::thread::sleep(wait);
                    let now = Instant::now();
                    let mut t = st(&ctx).throttle.lock();
                    if t.flush(now) {
                        break;
                    }
                    if !t.pending {
                        st(&ctx).flushing.store(false, Ordering::Release);
                        return;
                    }
                    wait = t.offer_wait(now);
                }
                st(&ctx).flushing.store(false, Ordering::Release);
                push(&ctx);
            });
        }
    }
}

impl Throttle {
    /// How long until a waiting change may go.
    fn offer_wait(&self, now: Instant) -> Duration {
        self.last
            .map(|t| self.gap.saturating_sub(now.saturating_duration_since(t)))
            .unwrap_or_default()
            .max(Duration::from_millis(10))
    }
}

// --- the watching side ----------------------------------------------------------

/// Ask `node` to keep this device posted on what it plays (`media_watch`),
/// over a live link only. `Ok(false)`: no live link now (nothing sent).
pub fn ask_watch<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<bool> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported(OLD_PEER));
    }
    if !crate::transport::is_live(ctx, node) {
        return Ok(false);
    }
    let msg = json!({ "type": "media_watch", "from": me(ctx) }).to_string();
    send_to(ctx, node, &msg)?;
    Ok(true)
}

/// Send `node` a media command (see [`command`]). Only over a live link: a
/// "play" that arrived minutes later would surprise; without one this
/// starts connecting and says so.
pub fn send_command<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    action: &str,
    value: Option<i64>,
    session: Option<&str>,
) -> crate::Result<()> {
    let mut cmd = command(action, value, session)?;
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported(OLD_PEER));
    }
    if !crate::transport::is_live(ctx, node) {
        crate::engine::start_ladder(ctx, node);
        return Err(CoreError::NoLink("not connected yet: connecting".into()));
    }
    cmd["type"] = "media_cmd".into();
    cmd["from"] = me(ctx).into();
    send_to(ctx, node, &cmd.to_string())
}

/// A status reply from `from` carried `media` (its `now_playing`): keep its
/// pictures, so later pushes may send just their hashes.
pub fn saw<P: Platform>(ctx: &NetCtx<P>, from: &str, media: &Value) {
    let _ = fill_in(ctx, from, media);
}

/// `media` from `from` with the pictures it sent as hashes put back (from
/// what it sent before), and the pictures it sent kept.
fn fill_in<P: Platform>(ctx: &NetCtx<P>, from: &str, media: &Value) -> Value {
    let mut media = media.clone();
    if !media.is_object() {
        return media;
    }
    let mut seen = st(ctx).seen.lock();
    let pics = seen.entry(from.to_string()).or_default();
    for (key, hash_key, slot) in [
        ("art", "art_hash", &mut pics.art),
        ("icon", "icon_hash", &mut pics.icon),
    ] {
        if let Some(p) = media[key].as_str() {
            *slot = Some((picture_hash(p), p.to_string()));
        } else if let Some(h) = media[hash_key].as_str() {
            match slot {
                Some((have, p)) if have == h => media[key] = p.clone().into(),
                _ => {}
            }
        }
    }
    media
}

pub struct MediaHandler;

impl<P: Platform> ControlHandler<P> for MediaHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["now_playing", "media_cmd", "media_watch"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        match msg.kind() {
            "now_playing" => {
                let media = fill_in(ctx, &from, &msg.json("media"));
                ctx.sink.emit(CoreEvent::NowPlaying {
                    from,
                    media: normalize(&media),
                });
            }
            // Only from your own devices, and only while this end takes them.
            _ if !ctx.config.media_ctl || !ctx.peers.lock().contains_key(&from) => {}
            "media_watch" => {
                watch(ctx, &from);
                // It may have lost the pictures: they go again.
                st(ctx).sent.lock().remove(&from);
                tell(ctx, &from);
            }
            "media_cmd" => {
                let raw = json!({
                    "action": msg.json("action"),
                    "value": msg.json("value"),
                    "session": msg.json("session"),
                });
                let Some((action, value, session)) = parse_command(&raw) else {
                    return;
                };
                // It's looking: it hears what the command did.
                watch(ctx, &from);
                ctx.sink.emit(CoreEvent::MediaCommand {
                    from,
                    action,
                    value,
                    session,
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
    fn only_known_short_fields_are_kept() {
        let m = normalize(&json!({
            "title": "  So What ", "artist": "Miles Davis", "album": "",
            "app": "Spotify", "state": "playing", "art": "data:...",
        }));
        assert_eq!(
            m,
            json!({"title": "So What", "artist": "Miles Davis", "app": "Spotify", "state": "playing"})
        );
        let long = normalize(&json!({"title": "x".repeat(1000), "state": "paused"}));
        assert_eq!(long["title"].as_str().unwrap_or("").len(), MAX_FIELD);
    }

    #[test]
    fn nothing_playing_is_null() {
        assert_eq!(normalize(&json!({})), Value::Null);
        assert_eq!(normalize(&Value::Null), Value::Null);
        assert_eq!(
            normalize(&json!({"title": "a", "state": "stopped"})),
            Value::Null
        );
        assert_eq!(
            normalize(&json!({"title": " ", "state": "playing"})),
            Value::Null
        );
    }

    #[test]
    fn the_fuller_fields_are_checked() {
        let m = normalize(&json!({
            "title": "So What", "state": "playing",
            "duration": 562_000, "position": 999_999,
            "can_next": true, "can_prev": false, "can_seek": "yes",
            "volume": 20, "volume_max": 15,
            "art": "QUJD", "icon": "not base64!",
            "session": "com.spotify.music",
            "sessions": [
                {"id": "com.spotify.music", "app": "Spotify", "state": "playing"},
                {"id": "com.google.android.youtube", "app": "YouTube", "title": "x", "state": "paused"},
                {"app": "no id"},
            ],
        }));
        assert_eq!(m["duration"], 562_000);
        assert_eq!(m["position"], 562_000, "never past the end");
        assert_eq!(m["can_next"], true);
        assert_eq!(m["can_prev"], false);
        assert!(m.get("can_seek").is_none());
        assert_eq!(
            (m["volume"].clone(), m["volume_max"].clone()),
            (json!(15), json!(15))
        );
        assert_eq!(m["art"], "QUJD");
        assert!(m.get("icon").is_none());
        assert_eq!(m["sessions"].as_array().map(Vec::len), Some(2));
        // One session isn't a choice.
        let one = normalize(&json!({"title": "a", "state": "paused",
            "sessions": [{"id": "a", "state": "paused"}]}));
        assert!(one.get("sessions").is_none());
        // Too big a picture is left out.
        let big =
            normalize(&json!({"title": "a", "state": "paused", "art": "A".repeat(MAX_ART + 1)}));
        assert!(big.get("art").is_none());
        assert_eq!(basic(&m), json!({"title": "So What", "state": "playing"}));
    }

    #[test]
    fn the_position_moves_on_while_playing() {
        let m = json!({"title": "a", "state": "playing", "position": 1000, "duration": 5000});
        assert_eq!(
            position_now(&m, Duration::from_millis(1500))["position"],
            2500
        );
        assert_eq!(position_now(&m, Duration::from_secs(60))["position"], 5000);
        let paused = json!({"title": "a", "state": "paused", "position": 1000});
        assert_eq!(
            position_now(&paused, Duration::from_secs(9))["position"],
            1000
        );
    }

    #[test]
    fn only_real_changes_are_worth_telling() {
        let a = json!({"title": "a", "state": "playing", "position": 10_000, "duration": 100_000});
        // The clock moved it on: not news.
        let later =
            json!({"title": "a", "state": "playing", "position": 15_200, "duration": 100_000});
        assert!(!worth_telling(&a, &later, Duration::from_secs(5)));
        // A seek is.
        let seek =
            json!({"title": "a", "state": "playing", "position": 60_000, "duration": 100_000});
        assert!(worth_telling(&a, &seek, Duration::from_secs(5)));
        // So is pausing, or another song.
        let mut paused = later.clone();
        paused["state"] = "paused".into();
        assert!(worth_telling(&a, &paused, Duration::from_secs(5)));
        let mut other = later.clone();
        other["title"] = "b".into();
        assert!(worth_telling(&a, &other, Duration::from_secs(5)));
        assert!(worth_telling(&a, &Value::Null, Duration::ZERO));
        assert!(!worth_telling(&Value::Null, &Value::Null, Duration::ZERO));
    }

    #[test]
    fn the_throttle_lets_one_through_per_gap_and_the_last_one_later() {
        let mut t = Throttle::new(Duration::from_secs(1));
        let t0 = Instant::now();
        assert_eq!(t.offer(t0), None, "the first goes at once");
        let wait = t.offer(t0 + Duration::from_millis(300));
        assert_eq!(wait, Some(Duration::from_millis(700)));
        assert_eq!(
            t.offer(t0 + Duration::from_millis(600)),
            Some(Duration::from_millis(400))
        );
        assert!(!t.flush(t0 + Duration::from_millis(900)), "not yet");
        assert!(t.flush(t0 + Duration::from_millis(1000)), "the latest goes");
        assert!(
            !t.flush(t0 + Duration::from_millis(3000)),
            "nothing waiting"
        );
        assert_eq!(
            t.offer(t0 + Duration::from_millis(1500)),
            Some(Duration::from_millis(500))
        );
        assert_eq!(t.offer(t0 + Duration::from_millis(5000)), None);
    }

    #[test]
    fn commands_are_checked() {
        assert_eq!(
            command("next", None, None).unwrap(),
            json!({"action": "next"})
        );
        assert_eq!(
            command("seek", Some(30_000), Some("com.spotify.music")).unwrap(),
            json!({"action": "seek", "value": 30_000, "session": "com.spotify.music"})
        );
        assert!(command("seek", None, None).is_err());
        assert!(command("seek", Some(-1), None).is_err());
        assert!(command("volume_set", Some(1001), None).is_err());
        assert!(command("volume_set", Some(7), None).is_ok());
        assert!(command("next", Some(1), None).is_err());
        assert!(command("session", None, None).is_err());
        assert!(command("session", None, Some("a\nb")).is_err());
        assert!(command("rm -rf", None, None).is_err());
        assert_eq!(
            parse_command(&json!({"action": "volume_up", "value": null, "session": null})),
            Some(("volume_up".into(), None, None))
        );
        assert_eq!(parse_command(&json!({"action": "seek"})), None);
    }

    #[test]
    fn picture_hashes_are_stable() {
        assert_eq!(picture_hash(""), "cbf29ce484222325");
        assert_eq!(picture_hash("a"), "af63dc4c8601ec8c");
        assert_ne!(picture_hash("QUJD"), picture_hash("QUJE"));
    }
}
