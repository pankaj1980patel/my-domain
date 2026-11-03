//! See a computer's screen on the phone: the phone asks to watch, the
//! computer sends small JPEG snapshots of its screen while it's watched.
//! The core carries the messages; the computer's host app takes the
//! snapshots (desktop: the OS's screenshot tool).
//!
//!   screen_watch { from, on }          phone → computer
//!   screen_frame { from, seq, data }   computer → phone (data: base64 JPEG)
//!
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

/// Send a snapshot (`jpeg`) to everyone watching; returns how many got it.
pub fn send_frame<P: Platform>(ctx: &NetCtx<P>, jpeg: &[u8]) -> crate::Result<usize> {
    if jpeg.is_empty() || jpeg.len() > MAX_FRAME_BYTES {
        return Err(CoreError::InvalidInput(format!(
            "a snapshot must be 1 to {MAX_FRAME_BYTES} bytes, not {}",
            jpeg.len()
        )));
    }
    let to = watchers(ctx);
    if to.is_empty() {
        return Ok(0);
    }
    let me = ctx.identity.lock().node_id.clone();
    let seq = ctx.extras.screen.seq.fetch_add(1, Ordering::Relaxed);
    let msg = json!({
        "type": "screen_frame", "from": me, "seq": seq,
        "data": base64::engine::general_purpose::STANDARD.encode(jpeg),
    })
    .to_string();
    Ok(to.iter().filter(|n| send(ctx, n, &msg).is_ok()).count())
}

fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &str) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(&ctx.ws_conns, &ctx.p2p_conns, &ctx.key, &name, node, msg)
}

pub struct ScreenHandler;

impl<P: Platform> ControlHandler<P> for ScreenHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["screen_watch", "screen_frame"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own devices only: decrypted with your key (or it wouldn't be
        // here), and in this device's list.
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
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
