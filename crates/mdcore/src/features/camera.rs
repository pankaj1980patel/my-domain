//! See a phone's camera on a computer: look across the room, or use the
//! phone as a document camera, and take a photo from the computer. The same
//! messages and frame path as the phone's screen (`features::screen`), with
//! a camera source:
//!
//!   screen_watch { from, on, source: "camera" }      computer → phone
//!   screen_offer { from, on, source: "camera",       phone → computer: the
//!                  front, torch?, reason? }          camera is on (its owner
//!                                                    said yes; which camera,
//!                                                    the torch), or it's off
//!                                                    (`reason`: "declined",
//!                                                    "stopped", "locked", ...)
//!   screen_frame { from, seq, data, source: "camera" }  phone → computer
//!   camera_ctl   { from, op, on }                    computer → phone: "switch"
//!                                                    camera, "torch" on/off,
//!                                                    take a "photo"
//!
//! - Only between devices that said `camera_view: 1` (in their status, or
//!   in the `addr_update` every link starts with): an older phone would take
//!   a camera watch for a screen watch, and an older computer would take a
//!   camera offer for a screen offer (and `camera_ctl` shows as chat). This
//!   end never sends any of them to a device that didn't say it.
//! - The core only carries them. The phone asks its owner every time
//!   ("Show your camera to …?") and streams only to the computers its
//!   owner allowed (`Engine::camera_frame_to`); a watch lasts
//!   `screen::WATCH_FOR` unless renewed, as for the screen.
//! - A photo goes to the computer as an ordinary file transfer.
//! - Frames go over a live link only, never queued; to a link that reads
//!   binary media they go as binary records of their own kind
//!   (`transport::media`, kind 3), never mixed with the screen's.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::screen::{self, MAX_FRAME_BYTES, WATCH_FOR};
use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device understands the
/// camera messages, and its version.
pub const FLAG: &str = "camera_view";
const VERSION: u64 = 1;
/// The `source` of the screen messages that are about the camera.
pub const SOURCE: &str = "camera";
/// What a computer may ask a phone's camera to do (`camera_ctl`'s `op`).
pub const OPS: &[&str] = &["switch", "torch", "photo"];

const OLD_PEER: &str = "this device's app is too old to show its camera: update it";

/// Both sides: the phone's watchers, the computer's views, what each
/// device said, and the frame counter.
#[derive(Default)]
pub struct Camera {
    /// Phone: watching computer -> when its watch ends.
    watchers: Mutex<HashMap<String, Instant>>,
    /// Computer: the phones whose camera it asked to see.
    viewing: Mutex<HashSet<String>>,
    /// Device -> whether it said `camera_view`.
    caps: Mutex<HashMap<String, bool>>,
    seq: AtomicU64,
}

fn cam<P: Platform>(ctx: &NetCtx<P>) -> &Camera {
    &ctx.extras.camera
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it understands the
/// camera messages.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.camera {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `camera_view` value of its status or
/// `addr_update`) whether it understands them.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    cam(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` said it understands the camera messages.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.camera && cam(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

/// A known device that understands the camera messages, with a live link.
fn ready<P: Platform>(ctx: &NetCtx<P>, node: &str, connect: bool) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported(OLD_PEER));
    }
    if !ctx.ws_conns.lock().contains_key(node) {
        if connect {
            crate::engine::start_ladder(ctx, node);
        }
        return Err(CoreError::NoLink("not connected yet: connecting".into()));
    }
    Ok(())
}

fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(ctx, &name, node, &msg.to_string())
}

// --- the computer's side ------------------------------------------------------

/// Ask phone `node` to show its camera here (`on`; it asks its owner, and
/// renewing every `screen::RENEW_EVERY` keeps it going), or to stop.
pub fn watch<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> crate::Result<()> {
    if !on {
        cam(ctx).viewing.lock().remove(node);
    }
    ready(ctx, node, on)?;
    if on {
        cam(ctx).viewing.lock().insert(node.to_string());
    }
    send(
        ctx,
        node,
        &json!({ "type": "screen_watch", "from": me(ctx), "on": on, "source": SOURCE }),
    )
}

/// Ask phone `node`'s camera to `op` ("switch", "torch" with `on`, "photo"),
/// while this computer sees it.
pub fn control<P: Platform>(ctx: &NetCtx<P>, node: &str, op: &str, on: bool) -> crate::Result<()> {
    if !OPS.contains(&op) {
        return Err(CoreError::InvalidInput(format!(
            "not a camera action: {op}"
        )));
    }
    ready(ctx, node, false)?;
    send(
        ctx,
        node,
        &json!({ "type": "camera_ctl", "from": me(ctx), "op": op, "on": on }),
    )
}

/// A camera frame from `from` (the JSON message's data, or a binary
/// record's bytes as base64): shown only from a phone this computer asked.
fn frame<P: Platform>(ctx: &NetCtx<P>, from: &str, seq: u64, jpeg: String) {
    if !cam(ctx).viewing.lock().contains(from) {
        return;
    }
    ctx.sink.emit(CoreEvent::CameraFrame {
        from: from.to_string(),
        seq,
        jpeg,
    });
}

/// A frame that came as a binary record (`transport::media`, kind 3).
pub(crate) fn deliver<P: Platform>(ctx: &NetCtx<P>, from: &str, seq: u64, jpeg: &[u8]) {
    if jpeg.is_empty() || jpeg.len() > MAX_FRAME_BYTES || !ctx.peers.lock().contains_key(from) {
        return;
    }
    use base64::Engine as _;
    frame(
        ctx,
        from,
        seq,
        base64::engine::general_purpose::STANDARD.encode(jpeg),
    );
}

// --- the phone's side ---------------------------------------------------------

/// The computers asking to see this camera now (expired watches dropped).
pub fn watchers<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let now = Instant::now();
    let mut w = cam(ctx).watchers.lock();
    w.retain(|_, until| *until > now);
    let mut list: Vec<String> = w.keys().cloned().collect();
    list.sort();
    list
}

/// Send a camera frame (`jpeg`) to those of `only` watching (the computers
/// its owner allowed); returns how many got it.
pub fn send_frame_to<P: Platform>(
    ctx: &NetCtx<P>,
    jpeg: &[u8],
    only: &[String],
) -> crate::Result<usize> {
    screen::check_frame(jpeg)?;
    let mut to = watchers(ctx);
    to.retain(|n| only.contains(n) && supports(ctx, n));
    if to.is_empty() {
        return Ok(0);
    }
    let seq = cam(ctx).seq.fetch_add(1, Ordering::Relaxed);
    Ok(screen::send_jpeg(ctx, &to, jpeg, seq, true))
}

/// What the phone tells a computer about its camera.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CameraState {
    /// On (streaming to it), or off.
    pub on: bool,
    /// The front camera (else the back one).
    pub front: bool,
    /// The torch: on or off, `None` where this camera has none.
    pub torch: Option<bool>,
    /// Why it's off: "declined", "stopped", "locked", "no_permission",
    /// "busy", "error" (a short word; the computer says what it knows).
    pub reason: Option<String>,
}

/// Tell computer `node` the camera is on for it, its state now, or off.
pub fn offer<P: Platform>(ctx: &NetCtx<P>, node: &str, state: &CameraState) -> crate::Result<()> {
    ready(ctx, node, false)?;
    let mut msg = json!({
        "type": "screen_offer", "from": me(ctx), "on": state.on,
        "source": SOURCE, "front": state.front,
    });
    if let Some(t) = state.torch {
        msg["torch"] = t.into();
    }
    if let Some(r) = state.reason.as_deref().filter(|r| is_reason(r)) {
        msg["reason"] = r.into();
    }
    send(ctx, node, &msg)
}

/// A reason as the offer may carry it: a short lowercase word.
fn is_reason(r: &str) -> bool {
    !r.is_empty() && r.len() <= 24 && r.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

// --- receiving ------------------------------------------------------------------

/// A `screen_*` message with the camera source, from `from` (one of your
/// devices). `features::screen` hands them here.
pub(super) fn handle_screen<P: Platform>(ctx: &NetCtx<P>, from: String, msg: &ControlMsg) {
    match msg.kind() {
        "screen_watch" => {
            let on = msg.flag("on");
            let started = {
                let mut w = cam(ctx).watchers.lock();
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
                ctx.sink.emit(CoreEvent::CameraWatch { from, on });
            }
        }
        "screen_offer" => {
            let on = msg.flag("on");
            if !on {
                // Off: the phone stopped (its owner said no, or Stop).
                cam(ctx).viewing.lock().remove(&from);
            }
            ctx.sink.emit(CoreEvent::CameraOffer {
                from,
                on,
                front: msg.flag("front"),
                torch: msg.json("torch").as_bool(),
                reason: msg.opt("reason").filter(|r| is_reason(r)),
            });
        }
        "screen_frame" => {
            let data = msg.str("data");
            if data.is_empty() || data.len() > MAX_FRAME_BYTES * 4 / 3 + 4 {
                return;
            }
            frame(ctx, &from, msg.json("seq").as_u64().unwrap_or(0), data);
        }
        _ => {}
    }
}

/// `camera_ctl`, from a computer watching this camera now.
pub struct CameraHandler;

impl<P: Platform> ControlHandler<P> for CameraHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["camera_ctl"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        // Only a computer that sees this camera now may turn it about.
        if !watchers(ctx).contains(&from) {
            return;
        }
        let op = msg.str("op");
        if !OPS.contains(&op.as_str()) {
            return;
        }
        ctx.sink.emit(CoreEvent::CameraControl {
            from,
            op,
            on: msg.flag("on"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_short_words() {
        assert!(is_reason("declined"));
        assert!(is_reason("no_permission"));
        assert!(!is_reason(""));
        assert!(!is_reason("Declined"));
        assert!(!is_reason("<b>hi</b>"));
        assert!(!is_reason(&"a".repeat(25)));
    }
}
