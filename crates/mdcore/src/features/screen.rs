//! See another device's screen: the viewer asks to watch, the watched
//! device sends small JPEG snapshots of its screen while it's watched. A
//! phone watches a computer (the desktop takes the snapshots with the OS's
//! screenshot tool), and a computer watches a phone (Android's screen
//! capture, after Android's own prompt every time). The core carries the
//! messages; the host apps take and show the snapshots.
//!
//!   screen_watch { from, on }          viewer → watched device
//!   screen_frame { from, seq, data }   watched device → viewer (data: base64 JPEG)
//!   screen_offer { from, on }          phone → computer: "I'm sharing my
//!                                      screen with you, watch it" (`on`),
//!                                      or "I stopped" (`!on`)
//!
//! - A phone's owner can share its screen with a computer from the phone:
//!   once the capture runs, the phone offers it (`Engine::screen_offer`)
//!   and the computer starts watching as usual. Only to a computer whose
//!   status says `"screen_view": true`: an older one doesn't know
//!   `screen_offer` and would show it as a chat message.
//! - A phone shares with the computers its owner chose, not with whoever
//!   watches: `Engine::screen_frame_to` sends only to those.
//! - A watch lasts `WATCH_FOR` unless renewed: the phone renews it every
//!   `RENEW_EVERY` while it shows the screen, so a phone that went away
//!   (crashed, lost its link) stops the snapshots by itself.
//! - Only your own devices can watch (decrypted with your key, from a device
//!   in this one's list), and only while screen sharing is allowed here
//!   (`Engine::set_screen_allowed`, on until turned off). The host shows
//!   when a device starts watching (`CoreEvent::ScreenWatch`).
//! - Frames go only over a live link, never queued: a stale picture of the
//!   screen is worth nothing. One that's too big (`MAX_FRAME_BYTES`) isn't
//!   sent.
//! - To a device whose QUIC link reads binary media frames, a frame goes as
//!   one (the JPEG's bytes, not base64 twice over; `transport::media`); it
//!   arrives as the same `CoreEvent::ScreenFrame`.
//! - A phone's camera rides the same messages with `source: "camera"`
//!   (`features::camera`), to devices that said they know it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::Mutex;
use serde_json::json;

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::media::{self, Sent};

use super::{ControlHandler, ControlMsg};

/// How long a watch lasts without being renewed.
pub const WATCH_FOR: Duration = Duration::from_secs(25);
/// How often a watching phone should renew (`Engine::screen_watch`).
pub const RENEW_EVERY: Duration = Duration::from_secs(10);
/// Largest snapshot sent, in bytes of JPEG.
pub const MAX_FRAME_BYTES: usize = 600 * 1024;

const ALLOWED_KEY: &str = "screen_share";

/// Both sides: who's watching this screen (computer), whether it's
/// allowed, and the snapshot counter.
#[derive(Default)]
pub struct Screen {
    /// Watching device -> when its watch ends.
    watchers: Mutex<HashMap<String, Instant>>,
    /// `None` until read from `kv`.
    allowed: Mutex<Option<bool>>,
    seq: AtomicU64,
}

/// Whether other devices may watch this screen.
pub fn allowed<P: Platform>(ctx: &NetCtx<P>) -> bool {
    let mut a = ctx.extras.screen.allowed.lock();
    *a.get_or_insert_with(|| ctx.platform.kv_get(ALLOWED_KEY).as_deref() != Some("off"))
}

/// Allow or forbid watching; forbidding ends every watch now.
pub fn set_allowed<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    *ctx.extras.screen.allowed.lock() = Some(on);
    ctx.platform
        .kv_set(ALLOWED_KEY, if on { "on" } else { "off" });
    if !on {
        let ended: Vec<String> = ctx
            .extras
            .screen
            .watchers
            .lock()
            .drain()
            .map(|(n, _)| n)
            .collect();
        for from in ended {
            ctx.sink.emit(CoreEvent::ScreenWatch { from, on: false });
        }
    }
}

/// The devices watching this screen now (expired watches are dropped).
pub fn watchers<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let now = Instant::now();
    let mut w = ctx.extras.screen.watchers.lock();
    w.retain(|_, until| *until > now);
    let mut list: Vec<String> = w.keys().cloned().collect();
    list.sort();
    list
}

/// Ask `node` to start (`on`) or stop sending its screen. Over the live link
/// only; with none, fails with `NoLink`.
pub fn watch<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !ctx.ws_conns.lock().contains_key(node) {
        if on {
            crate::engine::start_ladder(ctx, node);
        }
        return Err(CoreError::NoLink("not connected yet: connecting".into()));
    }
    let me = ctx.identity.lock().node_id.clone();
    send(
        ctx,
        node,
        &json!({ "type": "screen_watch", "from": me, "on": on }).to_string(),
    )
}

/// Tell `node` this device shares its screen with it (`on`: watch it), or
/// stopped. Over the live link only; with none, fails with `NoLink`.
pub fn offer<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !ctx.ws_conns.lock().contains_key(node) {
        return Err(CoreError::NoLink("not connected".into()));
    }
    let me = ctx.identity.lock().node_id.clone();
    send(
        ctx,
        node,
        &json!({ "type": "screen_offer", "from": me, "on": on }).to_string(),
    )
}

/// Send a snapshot (`jpeg`) to everyone watching; returns how many got it.
pub fn send_frame<P: Platform>(ctx: &NetCtx<P>, jpeg: &[u8]) -> crate::Result<usize> {
    send_frame_to(ctx, jpeg, None)
}

/// Send a snapshot (`jpeg`) to those watching, and only to those in `only`
/// when given; returns how many got it.
pub fn send_frame_to<P: Platform>(
    ctx: &NetCtx<P>,
    jpeg: &[u8],
    only: Option<&[String]>,
) -> crate::Result<usize> {
    check_frame(jpeg)?;
    let mut to = watchers(ctx);
    if let Some(only) = only {
        to.retain(|n| only.contains(n));
    }
    if to.is_empty() {
        return Ok(0);
    }
    let seq = ctx.extras.screen.seq.fetch_add(1, Ordering::Relaxed);
    Ok(send_jpeg(ctx, &to, jpeg, seq, false))
}

/// A frame is 1 to `MAX_FRAME_BYTES` bytes of JPEG.
pub(super) fn check_frame(jpeg: &[u8]) -> crate::Result<()> {
    if jpeg.is_empty() || jpeg.len() > MAX_FRAME_BYTES {
        return Err(CoreError::InvalidInput(format!(
            "a snapshot must be 1 to {MAX_FRAME_BYTES} bytes, not {}",
            jpeg.len()
        )));
    }
    Ok(())
}

/// Send frame `seq` (`jpeg`) to each of `to`: a binary record where the
/// link reads them, else the JSON message. `camera`: a camera frame
/// (`features::camera`), its own record kind and `source` in the JSON.
/// Returns how many it went to.
pub(super) fn send_jpeg<P: Platform>(
    ctx: &NetCtx<P>,
    to: &[String],
    jpeg: &[u8],
    seq: u64,
    camera: bool,
) -> usize {
    let (width, height, quality) = (0, 0, 0);
    let header = media::Header {
        session: ctx.quic.media_session,
        seq,
        ts_ms: media::now_ms(),
        format: if camera {
            media::Format::Camera {
                width,
                height,
                quality,
            }
        } else {
            media::Format::Jpeg {
                width,
                height,
                quality,
            }
        },
    };
    // The JSON message, made only if a peer needs it.
    let mut msg: Option<String> = None;
    to.iter()
        .inspect(|n| super::nearby::sent(ctx, n))
        .filter(|n| match media::send(ctx, n, &header, jpeg) {
            Sent::Yes => true,
            Sent::Dropped => false,
            Sent::NotBinary => {
                let msg = msg.get_or_insert_with(|| {
                    let me = ctx.identity.lock().node_id.clone();
                    let mut m = json!({
                        "type": "screen_frame", "from": me, "seq": seq, "ts": header.ts_ms,
                        "data": base64::engine::general_purpose::STANDARD.encode(jpeg),
                    });
                    if camera {
                        m["source"] = super::camera::SOURCE.into();
                    }
                    m.to_string()
                });
                send(ctx, n, msg).is_ok()
            }
        })
        .count()
}

/// A snapshot that came as a binary frame (`transport::media`) from `from`,
/// the link's peer: checked as the JSON one, the same event.
pub(crate) fn deliver<P: Platform>(ctx: &NetCtx<P>, from: &str, seq: u64, jpeg: &[u8]) {
    if jpeg.is_empty() || jpeg.len() > MAX_FRAME_BYTES || !ctx.peers.lock().contains_key(from) {
        return;
    }
    ctx.sink.emit(CoreEvent::ScreenFrame {
        from: from.to_string(),
        seq,
        jpeg: base64::engine::general_purpose::STANDARD.encode(jpeg),
    });
}

fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &str) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(ctx, &name, node, msg)
}

pub struct ScreenHandler;

impl<P: Platform> ControlHandler<P> for ScreenHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["screen_watch", "screen_frame", "screen_offer"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own devices only: decrypted with your key (or it wouldn't be
        // here), and in this device's list.
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        // The phone's camera (`features::camera`): its own watchers, offers
        // and frames. (An app from before reads these as about the screen,
        // which is why they only ever go to a device that says it knows.)
        if msg.kind() == "screen_frame" {
            super::nearby::received(ctx, &from, msg.json("ts").as_u64().unwrap_or(0));
        }
        if ctx.config.camera && msg.str("source") == super::camera::SOURCE {
            return super::camera::handle_screen(ctx, from, msg);
        }
        match msg.kind() {
            "screen_watch" => {
                let on = msg.json("on").as_bool().unwrap_or(false);
                if on && !allowed(ctx) {
                    return;
                }
                let started = {
                    let mut w = ctx.extras.screen.watchers.lock();
                    let now = Instant::now();
                    let was = w.get(&from).is_some_and(|until| *until > now);
                    if on {
                        w.insert(from.clone(), now + WATCH_FOR);
                    } else {
                        w.remove(&from);
                    }
                    was != on
                };
                // A renewal is not news.
                if started {
                    ctx.sink.emit(CoreEvent::ScreenWatch { from, on });
                }
            }
            // Whether to watch is the host's call (the desktop does, and
            // says so).
            "screen_offer" => ctx.sink.emit(CoreEvent::ScreenOffer {
                from,
                on: msg.json("on").as_bool().unwrap_or(false),
            }),
            "screen_frame" => {
                let data = msg.str("data");
                if data.is_empty() || data.len() > MAX_FRAME_BYTES * 4 / 3 + 4 {
                    return;
                }
                ctx.sink.emit(CoreEvent::ScreenFrame {
                    from,
                    seq: msg.json("seq").as_u64().unwrap_or(0),
                    jpeg: data,
                });
            }
            _ => {}
        }
    }
}
