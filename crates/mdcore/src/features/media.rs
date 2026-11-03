//! Now playing: what's playing on a device (title, artist, app, playing or
//! paused), shown on your other devices. The host app reads it from the OS
//! (Android media sessions, `playerctl` on Linux, ...) and hands it to the
//! core with `Engine::set_now_playing` whenever it changes.
//!
//!   now_playing { from, media }   device → devices watching it
//!
//! A device that asks for this one's status (`status_request`) is watching
//! it for `WATCH_FOR`: changes are pushed to it while it has a live link
//! (never connecting or waking it just for a song change). The status reply
//! carries the current one too (`"now_playing"`), so a device that just
//! looked never misses it. `media` is `null` when nothing is playing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// How long asking for a device's status keeps you hearing its changes.
pub const WATCH_FOR: Duration = Duration::from_secs(10 * 60);
/// Longest kept text field.
const MAX_FIELD: usize = 300;

/// What's playing here, and who's watching.
#[derive(Default)]
pub struct Media {
    now: Mutex<Value>,
    watchers: Mutex<HashMap<String, Instant>>,
}

/// The fields other devices get, cleaned up: `title`, `artist`, `album`,
/// `app` (short strings, left out when empty) and `state` ("playing",
/// "paused" or "stopped"). `Null` when there's nothing to show.
pub fn normalize(raw: &Value) -> Value {
    let Some(o) = raw.as_object() else {
        return Value::Null;
    };
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().chars().take(MAX_FIELD).collect::<String>())
            .filter(|s| !s.is_empty())
    };
    let state = match o.get("state").and_then(Value::as_str) {
        Some("playing") => "playing",
        Some("paused") => "paused",
        _ => "stopped",
    };
    let title = text("title");
    if title.is_none() || state == "stopped" {
        return Value::Null;
    }
    let mut out = json!({ "title": title, "state": state });
    for k in ["artist", "album", "app"] {
        if let Some(v) = text(k) {
            out[k] = v.into();
        }
    }
    out
}

/// `node` asked for our status: it hears changes for a while.
pub fn watch<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let mut w = ctx.media.watchers.lock();
    w.retain(|_, at| at.elapsed() < WATCH_FOR);
    w.insert(node.to_string(), Instant::now());
}

/// Somebody looked at this device lately (a host can skip polling the OS
/// for what's playing when nobody would hear it).
pub fn watched<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.media
        .watchers
        .lock()
        .values()
        .any(|at| at.elapsed() < WATCH_FOR)
}

/// What's playing now (`Null` if nothing).
pub fn current<P: Platform>(ctx: &NetCtx<P>) -> Value {
    ctx.media.now.lock().clone()
}

/// The host reports what's playing. When it changed, watchers with a live
/// link are told.
pub fn set<P: Platform>(ctx: &NetCtx<P>, raw: &Value) {
    let media = normalize(raw);
    {
        let mut now = ctx.media.now.lock();
        if *now == media {
            return;
        }
        *now = media.clone();
    }
    let watchers: Vec<String> = {
        let mut w = ctx.media.watchers.lock();
        w.retain(|_, at| at.elapsed() < WATCH_FOR);
        w.keys().cloned().collect()
    };
    if watchers.is_empty() {
        return;
    }
    let (me, name) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.name.clone())
    };
    let msg = json!({ "type": "now_playing", "from": me, "media": media }).to_string();
    for node in watchers {
        if crate::transport::is_live(ctx, &node) {
            let _ = crate::transport::send_payload(
                &ctx.ws_conns,
                &ctx.p2p_conns,
                &ctx.key,
                &name,
                &node,
                &msg,
            );
        }
    }
}

pub struct MediaHandler;

impl<P: Platform> ControlHandler<P> for MediaHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["now_playing"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        ctx.sink.emit(CoreEvent::NowPlaying {
            from,
            media: normalize(&msg.json("media")),
        });
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
}
