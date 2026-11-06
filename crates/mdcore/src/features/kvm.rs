//! Share mouse and keyboard between computers (like Barrier or Synergy):
//! your computers sit around each other, and when the pointer reaches an
//! edge of this screen that has another computer of yours next to it, this
//! computer's mouse and keyboard drive that one until the pointer comes back
//! across the same edge (or the hotkey, or the link dropping, brings it
//! back). The desktop captures and plays the input; the core carries it and
//! keeps both ends agreeing on who controls whom.
//!
//! ```text
//! kvm_layout { from, side, at }        where the receiver sits around the sender
//!                                     ("left" | "right" | "top" | "bottom" | null)
//! kvm_enter  { from, edge, pos, os, clip? }   the sender controls the receiver now;
//!                                     the pointer comes in at the receiver's `edge`,
//!                                     `pos` ten-thousandths along it
//! kvm_ack    { from, ok, why? }       the receiver's answer ("off", "busy", "ended")
//! kvm_input  { from, events: [KvmEvent] }   buttons, keys, wheel (and motion
//!                                     where there's no DATAGRAM path), in order
//! kvm_ping / kvm_pong { from }        once a second while controlling
//! kvm_leave  { from, edge, pos, clip? }     controlled → controller: the pointer
//!                                     left across `edge` (null: "take back" there)
//! kvm_stop   { from, why }            controller → controlled: it let go
//! kvm_clip   { from, text }           the clipboard follows the active screen
//! ```
//!
//! - Pointer motion adds up and goes every `MOTION_EVERY` (about 125 times
//!   a second), as an unreliable QUIC DATAGRAM (a binary media record, kind
//!   4) when the link is QUIC, else inside `kvm_input`. Buttons, keys and
//!   the wheel go reliably, after any motion still pending, so the order
//!   holds.
//! - Only between your own computers (decrypted with your key, from a
//!   device in this one's list) that say `kvm` in their status and
//!   `addr_update`: an older app never gets a `kvm_*` message or record.
//! - Never left captured: the controller lets go when the controlled
//!   computer stops answering (`CONTROLLER_TIMEOUT`), its link drops, it
//!   refuses, or the pointer comes back; the controlled computer lets go of
//!   everything held when the controller goes quiet (`CONTROLLED_TIMEOUT`)
//!   or its link drops. Each of these ends in `KvmChanged` with role
//!   "idle".
//! - Being controlled can be turned off (`set_allowed`): a `kvm_enter`
//!   then gets `kvm_ack { ok: false, why: "off" }`.
//! - The layout is kept on both ends and agrees: setting where a computer
//!   sits here tells it where this one sits there (the opposite side);
//!   the newer setting (`at`) wins, sent again when a link comes up. One
//!   computer per side: placing another there takes the first one off.
//! - The clipboard follows the active screen (text, while "Clipboard
//!   follows" is on at both ends): the computer giving up control sends
//!   what's copied, unless it must stay private (`Clipboard::keep_private`,
//!   a password manager's concealed copy).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{KvmEvent, KvmNeighbor, KvmState};
use crate::platform::Platform;
use crate::transport::{media, send_control_to};

use super::{ControlHandler, ControlMsg};

pub const FLAG: &str = "kvm";
/// This computer's OS ("macos", "windows", "linux"), said with `FLAG`, for
/// the other end's key mapping (Cmd ↔ Ctrl).
pub const OS_FLAG: &str = "kvm_os";
const VERSION: u64 = 1;

const CAPS_PREFIX: &str = "kvm_caps:";
const LAYOUT_KEY: &str = "kvm_layout";
const ALLOWED_KEY: &str = "kvm_allowed";
const CLIP_KEY: &str = "kvm_clip";

/// How often pending pointer motion goes out (about 125 a second).
pub const MOTION_EVERY: Duration = Duration::from_millis(8);
/// How often the controller asks whether the other end is still there.
pub const PING_EVERY: Duration = Duration::from_millis(1000);
/// The controller lets go after this long without a word from the other end.
pub const CONTROLLER_TIMEOUT: Duration = Duration::from_millis(3500);
/// The controlled computer lets go after this long without a word.
pub const CONTROLLED_TIMEOUT: Duration = Duration::from_millis(5000);
/// `pos` goes from 0 to this along an edge.
pub const POS_SCALE: u16 = 10_000;
/// Most events in one message; more are dropped.
pub const MAX_EVENTS: usize = 128;
/// Largest move or wheel turn in one event, either way.
pub const MAX_STEP: i32 = 4000;
/// Longest clipboard text that follows, in bytes.
pub const MAX_CLIP: usize = 256 * 1024;
/// The motion thread stops after this long with nothing to send.
const IDLE_STOP: Duration = Duration::from_secs(2);
/// How often the watchdog looks.
const WATCH_EVERY: Duration = Duration::from_millis(100);

/// A side of a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "left" => Some(Side::Left),
            "right" => Some(Side::Right),
            "top" => Some(Side::Top),
            "bottom" => Some(Side::Bottom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Top => "top",
            Side::Bottom => "bottom",
        }
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }
}

/// One computer's place in this one's layout. `side: None` is a removal,
/// kept (with its time) so an older placement can't come back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Place {
    side: Option<Side>,
    /// Unix ms when it was set (on either end).
    at: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum Session {
    Idle,
    Controlling {
        node: String,
        /// The edge of this screen the pointer went out at.
        edge: Side,
        id: u64,
        last_heard: Instant,
        last_ping: Instant,
    },
    Controlled {
        node: String,
        /// The edge of this screen it came in at (and leaves by).
        edge: Side,
        id: u64,
        last_heard: Instant,
    },
}

#[derive(Default)]
struct Motion {
    dx: i32,
    dy: i32,
    flushing: bool,
}

/// The state on both ends.
pub struct Kvm {
    session: Mutex<Session>,
    next_id: AtomicU64,
    seq: AtomicU64,
    motion: Mutex<Motion>,
    /// node → its OS ("" when it doesn't say `kvm`).
    caps: Mutex<HashMap<String, String>>,
    /// `None` until read from `kv`.
    layout: Mutex<Option<HashMap<String, Place>>>,
    allowed: Mutex<Option<bool>>,
    clip: Mutex<Option<bool>>,
    watching: AtomicBool,
}

impl Default for Kvm {
    fn default() -> Self {
        Kvm {
            session: Mutex::new(Session::Idle),
            next_id: AtomicU64::new(1),
            seq: AtomicU64::new(0),
            motion: Mutex::new(Motion::default()),
            caps: Mutex::new(HashMap::new()),
            layout: Mutex::new(None),
            allowed: Mutex::new(None),
            clip: Mutex::new(None),
            watching: AtomicBool::new(false),
        }
    }
}

fn state<P: Platform>(ctx: &NetCtx<P>) -> &Kvm {
    &ctx.extras.kvm
}

/// Whether this device takes part at all.
pub fn available<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.config.kvm && ctx.platform.shares_input()
}

/// This OS, as said in `OS_FLAG`.
pub fn this_os() -> &'static str {
    std::env::consts::OS
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if available(ctx) {
        status[FLAG] = VERSION.into();
        status[OS_FLAG] = this_os().into();
    }
}

/// `node` said (`flag`, `os`: its status's or `addr_update`'s `kvm` and
/// `kvm_os`) whether it shares mouse and keyboard. Kept across restarts;
/// newly understood, it gets this end's layout for it.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value, os: &Value) {
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    let os = if on {
        os.as_str()
            .filter(|s| !s.is_empty() && s.len() <= 16)
            .unwrap_or("unknown")
            .to_string()
    } else {
        String::new()
    };
    if node.is_empty() || node.starts_with("manual:") || peer_os(ctx, node) == os {
        return;
    }
    state(ctx).caps.lock().insert(node.to_string(), os.clone());
    ctx.platform.kv_set(&format!("{CAPS_PREFIX}{node}"), &os);
    if on {
        send_layout_to(ctx, node);
    }
}

/// `node`'s OS, "" when it doesn't share mouse and keyboard.
pub fn peer_os<P: Platform>(ctx: &NetCtx<P>, node: &str) -> String {
    if let Some(os) = state(ctx).caps.lock().get(node) {
        return os.clone();
    }
    let os = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .unwrap_or_default();
    state(ctx).caps.lock().insert(node.to_string(), os.clone());
    os
}

/// `node` said it shares mouse and keyboard.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    !peer_os(ctx, node).is_empty()
}

/// Send `msg` to `node` only if both ends take part.
fn send_kvm<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value) {
    if available(ctx) && supports(ctx, node) {
        send_control_to(ctx, node, &msg.to_string());
    }
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

fn name_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<String> {
    ctx.peers.lock().get(node).map(|p| p.name.clone())
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Whether other computers may control this one (on until turned off).
pub fn allowed<P: Platform>(ctx: &NetCtx<P>) -> bool {
    *state(ctx)
        .allowed
        .lock()
        .get_or_insert_with(|| ctx.platform.kv_get(ALLOWED_KEY).as_deref() != Some("off"))
}

/// Turning it off while controlled lets go at once.
pub fn set_allowed<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    *state(ctx).allowed.lock() = Some(on);
    ctx.platform
        .kv_set(ALLOWED_KEY, if on { "on" } else { "off" });
    if !on && matches!(*state(ctx).session.lock(), Session::Controlled { .. }) {
        stop(ctx, "off");
    }
}

/// Whether the clipboard follows the active screen (on until turned off).
pub fn clip_follows<P: Platform>(ctx: &NetCtx<P>) -> bool {
    *state(ctx)
        .clip
        .lock()
        .get_or_insert_with(|| ctx.platform.kv_get(CLIP_KEY).as_deref() != Some("off"))
}

pub fn set_clip_follows<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    *state(ctx).clip.lock() = Some(on);
    ctx.platform.kv_set(CLIP_KEY, if on { "on" } else { "off" });
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

fn with_layout<P: Platform, R>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut HashMap<String, Place>) -> (R, bool),
) -> R {
    let mut g = state(ctx).layout.lock();
    let map = g.get_or_insert_with(|| {
        ctx.platform
            .kv_get(LAYOUT_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    let (r, changed) = f(map);
    if changed {
        if let Ok(s) = serde_json::to_string(&*map) {
            ctx.platform.kv_set(LAYOUT_KEY, &s);
        }
    }
    r
}

/// The computers placed around this one.
pub fn layout<P: Platform>(ctx: &NetCtx<P>) -> Vec<KvmNeighbor> {
    let mut out: Vec<KvmNeighbor> = with_layout(ctx, |m| {
        let list = m
            .iter()
            .filter_map(|(node, p)| {
                p.side.map(|s| KvmNeighbor {
                    node_id: node.clone(),
                    side: s.as_str().to_string(),
                })
            })
            .collect();
        (list, false)
    });
    out.sort_by(|a, b| a.side.cmp(&b.side));
    out
}

/// The computer at `side` of this screen, if any.
pub fn neighbor_at<P: Platform>(ctx: &NetCtx<P>, side: Side) -> Option<String> {
    with_layout(ctx, |m| {
        let n = m
            .iter()
            .find(|(_, p)| p.side == Some(side))
            .map(|(n, _)| n.clone());
        (n, false)
    })
}

/// Put `node` at `side` of `map` (or take it off) as of `at`, unless what's
/// there is as new or newer. Whoever was at that side comes off (returned,
/// to be told). Returns whether it applied.
fn place_in(
    map: &mut HashMap<String, Place>,
    node: &str,
    side: Option<Side>,
    at: u64,
) -> (bool, Vec<String>) {
    if map.get(node).is_some_and(|p| p.at >= at) {
        return (false, Vec::new());
    }
    let mut displaced = Vec::new();
    if let Some(s) = side {
        for (other, p) in map.iter_mut() {
            if other != node && p.side == Some(s) {
                p.side = None;
                p.at = at.max(p.at + 1);
                displaced.push(other.clone());
            }
        }
    }
    map.insert(node.to_string(), Place { side, at });
    (true, displaced)
}

/// Place `node` at `side` of this screen (`None`: take it off). It's told
/// where this computer sits there (the opposite side), now if linked, else
/// when a link comes up.
pub fn set_neighbor<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    side: Option<Side>,
) -> crate::Result<Vec<KvmNeighbor>> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    let now = media::now_ms();
    let displaced = with_layout(ctx, |m| {
        // Newer than anything this pair had, even with a clock behind.
        let at = m.get(node).map_or(now, |p| now.max(p.at + 1));
        let (_, displaced) = place_in(m, node, side, at);
        (displaced, true)
    });
    send_layout_to(ctx, node);
    for other in displaced {
        send_layout_to(ctx, &other);
    }
    Ok(layout(ctx))
}

/// Tell `node` where it sits around this computer, as this end knows it.
fn send_layout_to<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let place = with_layout(ctx, |m| (m.get(node).copied(), false));
    let Some(p) = place else {
        return;
    };
    // Only over a live link (the next one sends it again), but then held
    // until it's taken: a link replaced just then mustn't lose it.
    if !available(ctx) || !supports(ctx, node) || !ctx.ws_conns.lock().contains_key(node) {
        return;
    }
    let msg = json!({
        "type": "kvm_layout",
        "from": me(ctx),
        "side": p.side.map(Side::as_str),
        "at": p.at,
    });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
}

/// A link to `node` came up: make sure it agrees on the layout.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if supports(ctx, node) {
        send_layout_to(ctx, node);
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

fn idle_state(reason: Option<&str>, edge: Option<Side>, pos: Option<u16>) -> KvmState {
    KvmState {
        role: "idle".into(),
        edge: edge.map(|e| e.as_str().to_string()),
        pos,
        reason: reason.map(str::to_string),
        ..KvmState::default()
    }
}

/// What the session is now.
pub fn current<P: Platform>(ctx: &NetCtx<P>) -> KvmState {
    let s = state(ctx).session.lock().clone();
    match s {
        Session::Idle => idle_state(None, None, None),
        Session::Controlling { node, edge, .. } => KvmState {
            role: "controlling".into(),
            name: name_of(ctx, &node),
            node_id: Some(node),
            edge: Some(edge.as_str().into()),
            ..KvmState::default()
        },
        Session::Controlled { node, edge, .. } => KvmState {
            role: "controlled".into(),
            name: name_of(ctx, &node),
            node_id: Some(node),
            edge: Some(edge.as_str().into()),
            ..KvmState::default()
        },
    }
}

fn emit<P: Platform>(ctx: &NetCtx<P>, state: KvmState) {
    ctx.sink.emit(CoreEvent::KvmChanged { state });
}

/// End the session (whatever it was) and say why; returns what it was.
fn end<P: Platform>(ctx: &NetCtx<P>) -> Session {
    let was = std::mem::replace(&mut *state(ctx).session.lock(), Session::Idle);
    let mut m = state(ctx).motion.lock();
    m.dx = 0;
    m.dy = 0;
    was
}

fn clamp_pos(pos: u64) -> u16 {
    u16::try_from(pos.min(u64::from(POS_SCALE))).unwrap_or(POS_SCALE)
}

/// What's copied here, to follow the pointer: only while "Clipboard
/// follows" is on, never what must stay private, at most `MAX_CLIP`.
fn clip_to_send<P: Platform>(ctx: &NetCtx<P>) -> Option<String> {
    if !clip_follows(ctx) {
        return None;
    }
    let clip = ctx.platform.clipboard()?;
    if clip.keep_private() {
        return None;
    }
    clip.get()
        .filter(|t| !t.trim().is_empty() && t.len() <= MAX_CLIP)
}

/// Text that followed the pointer here.
fn take_clip<P: Platform>(ctx: &NetCtx<P>, text: &Value) {
    let Some(text) = text.as_str() else {
        return;
    };
    if text.trim().is_empty() || text.len() > MAX_CLIP || !clip_follows(ctx) {
        return;
    }
    let Some(clip) = ctx.platform.clipboard() else {
        return;
    };
    if clip.get().as_deref() == Some(text) {
        return;
    }
    // Not news for clipboard sharing: it came from your own device.
    *ctx.clip_last.lock() = text.to_string();
    clip.set(text);
}

/// Start controlling `node`: the pointer went out at `edge` of this screen,
/// `pos` ten-thousandths along it. `node` must sit at `edge` in the layout,
/// share mouse and keyboard, and have a live link. Emits `KvmChanged`
/// ("controlling"); the desktop then captures its input until "idle".
pub fn start<P: Platform>(ctx: &NetCtx<P>, node: &str, edge: Side, pos: u16) -> crate::Result<()> {
    if !available(ctx) {
        return Err(CoreError::Unsupported(
            "can't share mouse and keyboard here",
        ));
    }
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported("that computer's app is too old"));
    }
    if neighbor_at(ctx, edge).as_deref() != Some(node) {
        return Err(CoreError::InvalidInput(
            "that computer isn't on that side".into(),
        ));
    }
    if !ctx.ws_conns.lock().contains_key(node) {
        crate::engine::start_ladder(ctx, node);
        return Err(CoreError::NoLink("not connected yet: connecting".into()));
    }
    let id = state(ctx).next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut s = state(ctx).session.lock();
        if *s != Session::Idle {
            return Err(CoreError::InvalidInput("already sharing".into()));
        }
        let now = Instant::now();
        *s = Session::Controlling {
            node: node.to_string(),
            edge,
            id,
            last_heard: now,
            last_ping: now,
        };
    }
    let pos = pos.min(POS_SCALE);
    let mut msg = json!({
        "type": "kvm_enter",
        "from": me(ctx),
        "edge": edge.opposite().as_str(),
        "pos": pos,
        "os": this_os(),
    });
    if let Some(text) = clip_to_send(ctx) {
        msg["clip"] = text.into();
    }
    send_kvm(ctx, node, &msg);
    emit(ctx, current(ctx));
    watch(ctx);
    Ok(())
}

/// Let go: as the controller (the hotkey, Stop, the app quitting), the
/// other computer is told; as the controlled computer ("take back"), the
/// controller is told and lets go. Nothing to do when idle.
pub fn stop<P: Platform>(ctx: &NetCtx<P>, why: &str) {
    match end(ctx) {
        Session::Idle => {}
        Session::Controlling { node, .. } => {
            send_kvm(
                ctx,
                &node,
                &json!({ "type": "kvm_stop", "from": me(ctx), "why": why }),
            );
            emit(ctx, idle_state(Some(why), None, None));
        }
        Session::Controlled { node, .. } => {
            let mut msg =
                json!({ "type": "kvm_leave", "from": me(ctx), "edge": null, "pos": null });
            if let Some(text) = clip_to_send(ctx) {
                msg["clip"] = text.into();
            }
            send_kvm(ctx, &node, &msg);
            emit(ctx, idle_state(Some(why), None, None));
        }
    }
}

/// Controlled: the pointer was pushed out across `edge` of this screen,
/// `pos` along it. Only the edge it came in at gives control back.
/// Returns whether it did.
pub fn leave<P: Platform>(ctx: &NetCtx<P>, edge: Side, pos: u16) -> bool {
    let node = {
        let mut s = state(ctx).session.lock();
        match &*s {
            Session::Controlled { node, edge: e, .. } if *e == edge => {
                let node = node.clone();
                *s = Session::Idle;
                node
            }
            _ => return false,
        }
    };
    let mut msg = json!({
        "type": "kvm_leave",
        "from": me(ctx),
        "edge": edge.as_str(),
        "pos": pos.min(POS_SCALE),
    });
    if let Some(text) = clip_to_send(ctx) {
        msg["clip"] = text.into();
    }
    send_kvm(ctx, &node, &msg);
    emit(ctx, idle_state(Some("returned"), None, None));
    true
}

/// The link to `node` dropped: a session with it ends.
pub fn link_down<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let with = matches!(
        &*state(ctx).session.lock(),
        Session::Controlling { node: n, .. } | Session::Controlled { node: n, .. } if n == node
    );
    if with {
        end(ctx);
        emit(ctx, idle_state(Some("link_lost"), None, None));
    }
}

/// The controller's node, while this computer is controlled by it.
fn controller<P: Platform>(ctx: &NetCtx<P>) -> Option<String> {
    match &*state(ctx).session.lock() {
        Session::Controlled { node, .. } => Some(node.clone()),
        _ => None,
    }
}

/// The controlled node, while this computer controls it.
fn controlled<P: Platform>(ctx: &NetCtx<P>) -> Option<String> {
    match &*state(ctx).session.lock() {
        Session::Controlling { node, .. } => Some(node.clone()),
        _ => None,
    }
}

/// Heard from `node`, the other end of the session.
fn heard<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    match &mut *state(ctx).session.lock() {
        Session::Controlling {
            node: n,
            last_heard,
            ..
        }
        | Session::Controlled {
            node: n,
            last_heard,
            ..
        } if n == node => {
            *last_heard = Instant::now();
            true
        }
        _ => false,
    }
}

/// One watchdog thread while a session lasts: pings as the controller, and
/// lets go when the other end goes quiet.
fn watch<P: Platform>(ctx: &NetCtx<P>) {
    if state(ctx).watching.swap(true, Ordering::AcqRel) {
        return;
    }
    let c = ctx.clone();
    let started = std::thread::Builder::new()
        .name("kvm-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(WATCH_EVERY);
                if !watch_once(&c) {
                    break;
                }
            }
            state(&c).watching.store(false, Ordering::Release);
            // A session that started just as this one stopped looking.
            if *state(&c).session.lock() != Session::Idle {
                watch(&c);
            }
        })
        .is_ok();
    if !started {
        state(ctx).watching.store(false, Ordering::Release);
    }
}

/// One look; false when there's nothing more to watch.
fn watch_once<P: Platform>(ctx: &NetCtx<P>) -> bool {
    enum Do {
        Nothing,
        Ping(String),
        Timeout,
    }
    let what = {
        let mut s = state(ctx).session.lock();
        match &mut *s {
            Session::Idle => return false,
            Session::Controlling {
                node,
                last_heard,
                last_ping,
                ..
            } => {
                if last_heard.elapsed() >= CONTROLLER_TIMEOUT {
                    Do::Timeout
                } else if last_ping.elapsed() >= PING_EVERY {
                    *last_ping = Instant::now();
                    Do::Ping(node.clone())
                } else {
                    Do::Nothing
                }
            }
            Session::Controlled { last_heard, .. } => {
                if last_heard.elapsed() >= CONTROLLED_TIMEOUT {
                    Do::Timeout
                } else {
                    Do::Nothing
                }
            }
        }
    };
    match what {
        Do::Nothing => {}
        Do::Ping(node) => send_kvm(ctx, &node, &json!({ "type": "kvm_ping", "from": me(ctx) })),
        Do::Timeout => stop(ctx, "timeout"),
    }
    true
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Events made safe to play: at most `MAX_EVENTS`, moves and wheel turns
/// within `MAX_STEP`, empty ones dropped, buttons 1 to 5, keys that are
/// keyboard usages (not 0).
pub fn clean(events: impl IntoIterator<Item = KvmEvent>) -> Vec<KvmEvent> {
    events
        .into_iter()
        .take(MAX_EVENTS)
        .filter_map(|e| match e {
            KvmEvent::Move { dx: 0, dy: 0 } | KvmEvent::Wheel { dx: 0, dy: 0 } => None,
            KvmEvent::Move { dx, dy } => Some(KvmEvent::Move {
                dx: dx.clamp(-MAX_STEP, MAX_STEP),
                dy: dy.clamp(-MAX_STEP, MAX_STEP),
            }),
            KvmEvent::Wheel { dx, dy } => Some(KvmEvent::Wheel {
                dx: dx.clamp(-MAX_STEP, MAX_STEP),
                dy: dy.clamp(-MAX_STEP, MAX_STEP),
            }),
            KvmEvent::Button { button, .. } if !(1..=5).contains(&button) => None,
            KvmEvent::Key { code: 0, .. } => None,
            KvmEvent::Key { code, .. } if code > 0xFF => None,
            other => Some(other),
        })
        .collect()
}

/// Pointer motion while controlling: adds up, goes within `MOTION_EVERY`.
pub fn motion<P: Platform>(ctx: &NetCtx<P>, dx: i32, dy: i32) {
    if (dx == 0 && dy == 0) || controlled(ctx).is_none() {
        return;
    }
    let start = {
        let mut m = state(ctx).motion.lock();
        m.dx = m.dx.saturating_add(dx);
        m.dy = m.dy.saturating_add(dy);
        !std::mem::replace(&mut m.flushing, true)
    };
    if start {
        let c = ctx.clone();
        let ok = std::thread::Builder::new()
            .name("kvm-motion".into())
            .spawn(move || motion_loop(&c))
            .is_ok();
        if !ok {
            state(ctx).motion.lock().flushing = false;
        }
    }
}

fn take_motion<P: Platform>(ctx: &NetCtx<P>) -> Option<(i32, i32)> {
    let mut m = state(ctx).motion.lock();
    if m.dx == 0 && m.dy == 0 {
        return None;
    }
    Some((std::mem::take(&mut m.dx), std::mem::take(&mut m.dy)))
}

fn motion_loop<P: Platform>(ctx: &NetCtx<P>) {
    let mut last = Instant::now();
    loop {
        std::thread::sleep(MOTION_EVERY);
        let due = take_motion(ctx);
        let Some((dx, dy)) = due else {
            let mut m = state(ctx).motion.lock();
            if (m.dx == 0 && m.dy == 0) && last.elapsed() >= IDLE_STOP {
                m.flushing = false;
                return;
            }
            continue;
        };
        last = Instant::now();
        if let Some(node) = controlled(ctx) {
            send_motion(ctx, &node, dx, dy);
        }
    }
}

/// Motion as a DATAGRAM when the link is QUIC, else in `kvm_input`.
fn send_motion<P: Platform>(ctx: &NetCtx<P>, node: &str, dx: i32, dy: i32) {
    if !supports(ctx, node) {
        return;
    }
    let (dx, dy) = (dx.clamp(-MAX_STEP, MAX_STEP), dy.clamp(-MAX_STEP, MAX_STEP));
    if media::binary_path(ctx, node) == Some(true) {
        let h = media::Header {
            session: 0,
            seq: state(ctx).seq.fetch_add(1, Ordering::Relaxed),
            ts_ms: media::now_ms(),
            format: media::Format::Kvm,
        };
        if media::send(ctx, node, &h, &motion_payload(dx, dy)) == media::Sent::Yes {
            return;
        }
    }
    send_events_now(ctx, node, &[KvmEvent::Move { dx, dy }]);
}

/// `dx`, `dy` as i16 big-endian (clamped).
pub fn motion_payload(dx: i32, dy: i32) -> [u8; 4] {
    let c = |v: i32| i16::try_from(v.clamp(i16::MIN.into(), i16::MAX.into())).unwrap_or(0);
    let (x, y) = (c(dx).to_be_bytes(), c(dy).to_be_bytes());
    [x[0], x[1], y[0], y[1]]
}

pub fn parse_motion(payload: &[u8]) -> Option<(i32, i32)> {
    let b: &[u8; 4] = payload.get(..4)?.try_into().ok()?;
    Some((
        i32::from(i16::from_be_bytes([b[0], b[1]])),
        i32::from(i16::from_be_bytes([b[2], b[3]])),
    ))
}

fn send_events_now<P: Platform>(ctx: &NetCtx<P>, node: &str, events: &[KvmEvent]) {
    send_kvm(
        ctx,
        node,
        &json!({ "type": "kvm_input", "from": me(ctx), "events": events }),
    );
}

/// Buttons, keys and wheel turns while controlling: reliably, after any
/// motion still pending.
pub fn send<P: Platform>(ctx: &NetCtx<P>, events: Vec<KvmEvent>) -> crate::Result<()> {
    let Some(node) = controlled(ctx) else {
        return Err(CoreError::InvalidInput("not sharing".into()));
    };
    let mut all = Vec::new();
    if let Some((dx, dy)) = take_motion(ctx) {
        all.push(KvmEvent::Move { dx, dy });
    }
    all.extend(events);
    let all = clean(all);
    if !all.is_empty() {
        send_events_now(ctx, &node, &all);
    }
    Ok(())
}

/// A motion record arrived from `from` (a DATAGRAM).
pub(crate) fn on_motion<P: Platform>(ctx: &NetCtx<P>, from: &str, payload: &[u8]) {
    let Some((dx, dy)) = parse_motion(payload) else {
        return;
    };
    if controller(ctx).as_deref() != Some(from) {
        return;
    }
    heard(ctx, from);
    let events = clean([KvmEvent::Move { dx, dy }]);
    if !events.is_empty() {
        ctx.sink.emit(CoreEvent::KvmInput {
            from: from.to_string(),
            events,
        });
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

pub struct KvmHandler;

impl<P: Platform> ControlHandler<P> for KvmHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "kvm_layout",
            "kvm_enter",
            "kvm_ack",
            "kvm_input",
            "kvm_ping",
            "kvm_pong",
            "kvm_leave",
            "kvm_stop",
            "kvm_clip",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own computers only: decrypted with your key (or it wouldn't
        // be here), and in this device's list.
        if from.is_empty() || !available(ctx) || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        match msg.kind() {
            "kvm_layout" => on_layout(ctx, &from, msg),
            "kvm_enter" => on_enter(ctx, &from, msg),
            "kvm_ack" => {
                if controlled(ctx).as_deref() != Some(from.as_str()) {
                    return;
                }
                if msg.flag("ok") {
                    heard(ctx, &from);
                } else {
                    let why = msg.opt("why").unwrap_or_else(|| "ended".into());
                    let why = match why.as_str() {
                        "off" | "busy" | "ended" => why,
                        _ => "ended".into(),
                    };
                    end(ctx);
                    emit(ctx, idle_state(Some(&why), None, None));
                }
            }
            "kvm_input" => {
                if controller(ctx).as_deref() != Some(from.as_str()) {
                    return;
                }
                heard(ctx, &from);
                let events: Vec<KvmEvent> = match msg.json("events") {
                    Value::Array(list) => list
                        .into_iter()
                        .take(MAX_EVENTS)
                        .filter_map(|e| serde_json::from_value(e).ok())
                        .collect(),
                    _ => return,
                };
                let events = clean(events);
                if !events.is_empty() {
                    ctx.sink.emit(CoreEvent::KvmInput { from, events });
                }
            }
            "kvm_ping" => {
                if heard(ctx, &from) && controller(ctx).as_deref() == Some(from.as_str()) {
                    send_kvm(ctx, &from, &json!({ "type": "kvm_pong", "from": me(ctx) }));
                } else {
                    // It thinks it controls this computer, but this one
                    // doesn't (restarted, or let go): it must let go too.
                    send_kvm(
                        ctx,
                        &from,
                        &json!({ "type": "kvm_ack", "from": me(ctx), "ok": false, "why": "ended" }),
                    );
                }
            }
            "kvm_pong" => {
                heard(ctx, &from);
            }
            "kvm_leave" => {
                if controlled(ctx).as_deref() != Some(from.as_str()) {
                    return;
                }
                let my_edge = match &*state(ctx).session.lock() {
                    Session::Controlling { edge, .. } => Some(*edge),
                    _ => None,
                };
                // "take back" there: no edge; else it came back across the
                // edge it went out at, here.
                let back = Side::parse(&msg.str("edge")).is_some();
                let pos = msg.json("pos").as_u64().map(clamp_pos);
                end(ctx);
                take_clip(ctx, &msg.json("clip"));
                if back {
                    emit(ctx, idle_state(Some("returned"), my_edge, pos));
                } else {
                    emit(ctx, idle_state(Some("stopped"), None, None));
                }
            }
            "kvm_stop" => {
                if controller(ctx).as_deref() != Some(from.as_str()) {
                    return;
                }
                end(ctx);
                let why = match msg.str("why").as_str() {
                    "timeout" | "link_lost" => "link_lost",
                    _ => "stopped",
                };
                emit(ctx, idle_state(Some(why), None, None));
                // Control is back there: so is what's copied here.
                if let Some(text) = clip_to_send(ctx) {
                    send_kvm(
                        ctx,
                        &from,
                        &json!({ "type": "kvm_clip", "from": me(ctx), "text": text }),
                    );
                }
            }
            "kvm_clip" if supports(ctx, &from) => take_clip(ctx, &msg.json("text")),
            _ => {}
        }
    }
}

/// How far ahead of this clock a layout setting's time may be.
const MAX_CLOCK_AHEAD_MS: u64 = 24 * 3600 * 1000;

fn on_layout<P: Platform>(ctx: &NetCtx<P>, from: &str, msg: &ControlMsg) {
    let side_v = msg.json("side");
    let side = match &side_v {
        Value::Null => None,
        Value::String(s) => match Side::parse(s) {
            Some(s) => Some(s),
            None => return,
        },
        _ => return,
    };
    let Some(at) = msg.json("at").as_u64() else {
        return;
    };
    // A setting from the future would win over every later one here.
    if at > media::now_ms().saturating_add(MAX_CLOCK_AHEAD_MS) {
        return;
    }
    // It says where this computer sits around it: it sits on the other
    // side here.
    let (applied, displaced) = with_layout(ctx, |m| {
        let (applied, displaced) = place_in(m, from, side.map(Side::opposite), at);
        ((applied, displaced), applied)
    });
    if !applied {
        return;
    }
    for other in displaced {
        send_layout_to(ctx, &other);
    }
    ctx.sink.emit(CoreEvent::KvmLayout {
        layout: layout(ctx),
    });
}

fn on_enter<P: Platform>(ctx: &NetCtx<P>, from: &str, msg: &ControlMsg) {
    let refuse = |why: &str| {
        send_kvm(
            ctx,
            from,
            &json!({ "type": "kvm_ack", "from": me(ctx), "ok": false, "why": why }),
        );
    };
    let Some(edge) = Side::parse(&msg.str("edge")) else {
        return;
    };
    if !allowed(ctx) {
        refuse("off");
        return;
    }
    // Only the computer placed on that side here may bring the pointer in
    // across it (as the controller itself checks before it starts).
    if neighbor_at(ctx, edge).as_deref() != Some(from) {
        refuse("off");
        return;
    }
    let pos = clamp_pos(msg.json("pos").as_u64().unwrap_or(u64::from(POS_SCALE / 2)));
    if let Some(os) = msg.opt("os") {
        if !os.is_empty() && os.len() <= 16 && peer_os(ctx, from) != os {
            state(ctx).caps.lock().insert(from.to_string(), os.clone());
            ctx.platform.kv_set(&format!("{CAPS_PREFIX}{from}"), &os);
        }
    }
    {
        let mut s = state(ctx).session.lock();
        match &*s {
            Session::Idle => {}
            Session::Controlled { node, .. } if node == from => {}
            _ => {
                drop(s);
                refuse("busy");
                return;
            }
        }
        let id = state(ctx).next_id.fetch_add(1, Ordering::Relaxed);
        *s = Session::Controlled {
            node: from.to_string(),
            edge,
            id,
            last_heard: Instant::now(),
        };
    }
    take_clip(ctx, &msg.json("clip"));
    send_kvm(
        ctx,
        from,
        &json!({ "type": "kvm_ack", "from": me(ctx), "ok": true }),
    );
    let mut st = current(ctx);
    st.pos = Some(pos);
    emit(ctx, st);
    watch(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sides_and_their_opposites() {
        for s in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            assert_eq!(Side::parse(s.as_str()), Some(s));
            assert_eq!(s.opposite().opposite(), s);
            assert_ne!(s.opposite(), s);
        }
        assert_eq!(Side::Left.opposite(), Side::Right);
        assert_eq!(Side::Top.opposite(), Side::Bottom);
        assert_eq!(Side::parse("Left"), None);
    }

    #[test]
    fn a_newer_placement_wins_and_takes_the_side() {
        let mut m = HashMap::new();
        assert!(place_in(&mut m, "a", Some(Side::Left), 10).0);
        // Older or as old: ignored.
        assert!(!place_in(&mut m, "a", Some(Side::Right), 10).0);
        assert!(!place_in(&mut m, "a", None, 5).0);
        assert_eq!(m["a"].side, Some(Side::Left));
        // Another computer at the same side takes it.
        let (ok, displaced) = place_in(&mut m, "b", Some(Side::Left), 20);
        assert!(ok);
        assert_eq!(displaced, vec!["a".to_string()]);
        assert_eq!(m["a"], Place { side: None, at: 20 });
        // Taken off later than it was placed, even with a clock behind.
        let mut n = HashMap::new();
        place_in(&mut n, "a", Some(Side::Top), 50);
        place_in(&mut n, "b", Some(Side::Top), 40);
        assert_eq!(n["a"], Place { side: None, at: 51 });
        // A removal is kept, so an older placement can't come back.
        assert!(!place_in(&mut m, "a", Some(Side::Top), 15).0);
        assert!(place_in(&mut m, "a", Some(Side::Top), 21).0);
        assert_eq!(m["b"].side, Some(Side::Left));
    }

    #[test]
    fn events_are_made_safe() {
        let got = clean([
            KvmEvent::Move { dx: 0, dy: 0 },
            KvmEvent::Move { dx: 99_999, dy: -3 },
            KvmEvent::Wheel { dx: 0, dy: 0 },
            KvmEvent::Wheel { dx: 0, dy: -2 },
            KvmEvent::Button {
                button: 0,
                down: true,
            },
            KvmEvent::Button {
                button: 9,
                down: true,
            },
            KvmEvent::Button {
                button: 1,
                down: true,
            },
            KvmEvent::Key {
                code: 0,
                down: true,
            },
            KvmEvent::Key {
                code: 0x1FF,
                down: true,
            },
            KvmEvent::Key {
                code: 0x04,
                down: false,
            },
        ]);
        assert_eq!(
            got,
            [
                KvmEvent::Move {
                    dx: MAX_STEP,
                    dy: -3
                },
                KvmEvent::Wheel { dx: 0, dy: -2 },
                KvmEvent::Button {
                    button: 1,
                    down: true
                },
                KvmEvent::Key {
                    code: 0x04,
                    down: false
                },
            ]
        );
        let many = vec![
            KvmEvent::Key {
                code: 4,
                down: true
            };
            MAX_EVENTS * 2
        ];
        assert_eq!(clean(many).len(), MAX_EVENTS);
    }

    #[test]
    fn motion_payload_round_trips_and_clamps() {
        assert_eq!(parse_motion(&motion_payload(12, -7)), Some((12, -7)));
        assert_eq!(
            parse_motion(&motion_payload(100_000, -100_000)),
            Some((i32::from(i16::MAX), i32::from(i16::MIN)))
        );
        assert_eq!(parse_motion(&[1, 2, 3]), None);
    }

    #[test]
    fn the_wire_form_is_tagged_by_kind() {
        assert_eq!(
            serde_json::to_value(KvmEvent::Key {
                code: 0xE3,
                down: true
            })
            .unwrap_or_default(),
            json!({"kind": "key", "code": 0xE3, "down": true})
        );
        assert_eq!(
            serde_json::from_value::<KvmEvent>(
                json!({"kind": "button", "button": 2, "down": false})
            )
            .ok(),
            Some(KvmEvent::Button {
                button: 2,
                down: false
            })
        );
        assert!(serde_json::from_value::<KvmEvent>(json!({"kind": "text", "text": "x"})).is_err());
    }
}
