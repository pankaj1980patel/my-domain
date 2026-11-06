//! Countdown timers shared between your devices: start one on any of them,
//! every device that understands timers shows it counting down, and the
//! devices you picked ring when it ends. Pause, resume, add a minute,
//! cancel or stop the ringing from any of them.
//!
//!   timers { from, timers: [ {id, label, duration_ms, state, ends_at,
//!                             remaining_ms, ring_on, created_at,
//!                             updated_at, by, by_name} ] }
//!
//! The core keeps a small replicated map (`kv`, key `timers_map`): timer
//! id -> the timer as last changed. A running timer carries the time it
//! ends (`ends_at`, unix ms), so each device counts down by itself and no
//! tick ever crosses the network (devices' clocks are assumed to be set by
//! the network, as phones' and computers' are); a paused one carries what
//! was left. Merging is last-writer-wins per timer by (`updated_at`, `by`,
//! then the rest), so every device ends with the same map whatever order
//! the messages came in. Cancelling, and stopping a timer that ended
//! ("dismiss"), keep a finished entry (a tombstone) for `KEEP_GONE_MS`, so
//! an older copy never brings it back.
//!
//! The whole map goes (it's small): when a link comes up, when a device is
//! first heard to understand it, and to every linked device when a timer
//! is started or changed here. A device that has something newer than what
//! it was sent answers with its own map once; the answer brings nothing
//! newer back, so it ends there.
//!
//! Only devices that said `timers: 1` (in their status, or in the
//! `addr_update` every link starts with) get the message: older apps would
//! show it as a chat message. A `timers` message arriving says the same.
//!
//! What arrives and changes something makes a `CoreEvent::TimersChanged`
//! with every timer the apps show; a change made here makes no event (the
//! app made it). Ringing is the apps' part: each counts down to `ends_at`
//! and rings if its own node id is in `ring_on`, until the timer is
//! dismissed (here or on any device) or it gives up by itself.

use std::collections::{BTreeMap, HashMap};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::Timer;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device understands
/// `timers`, and its version.
pub const FLAG: &str = "timers";
const VERSION: u64 = 1;

const MAP_KEY: &str = "timers_map";
const CAPS_PREFIX: &str = "timers_caps:";

pub const RUNNING: &str = "running";
pub const PAUSED: &str = "paused";
pub const DONE: &str = "done";
pub const CANCELLED: &str = "cancelled";

/// The longest label, in characters.
pub const MAX_LABEL: usize = 60;
/// The shortest timer.
pub const MIN_DURATION_MS: u64 = 1_000;
/// The longest timer (and the most time left after adding some).
pub const MAX_DURATION_MS: u64 = 7 * 24 * 3600 * 1000;
/// The most timers running or paused at once.
pub const MAX_ACTIVE: usize = 50;
/// The most kept, finished ones included.
const MAX_KEPT: usize = 200;
/// The most devices one timer rings on.
const MAX_RING: usize = 32;
/// The longest id or node id.
const MAX_ID: usize = 128;
/// A cancelled or dismissed timer (and one that ended and was never
/// dismissed) is forgotten this long after; until then it stays finished
/// on every device.
pub const KEEP_GONE_MS: u64 = 30 * 24 * 3600 * 1000;
/// A timer that ended and was never dismissed is shown (as ended) this
/// long, then no more.
pub const ENDED_SHOWN_MS: u64 = 24 * 3600 * 1000;

/// Timer id -> the timer.
pub type TimerMap = BTreeMap<String, Timer>;

fn state_rank(s: &str) -> u8 {
    match s {
        RUNNING => 1,
        PAUSED => 2,
        DONE => 3,
        CANCELLED => 4,
        _ => 0,
    }
}

/// Running or paused (not cancelled or dismissed).
pub fn is_active(t: &Timer) -> bool {
    t.state == RUNNING || t.state == PAUSED
}

/// Running and its time is up (it rings until dismissed).
pub fn has_ended(t: &Timer, now: u64) -> bool {
    t.state == RUNNING && t.ends_at <= now
}

/// What's left at `now`, in ms (0 once it has ended).
pub fn remaining(t: &Timer, now: u64) -> u64 {
    match t.state.as_str() {
        RUNNING => t.ends_at.saturating_sub(now),
        PAUSED => t.remaining_ms,
        _ => 0,
    }
}

/// The apps show it: paused, running, or ended less than
/// `ENDED_SHOWN_MS` ago and not dismissed.
pub fn is_shown(t: &Timer, now: u64) -> bool {
    match t.state.as_str() {
        PAUSED => true,
        RUNNING => t.ends_at.saturating_add(ENDED_SHOWN_MS) > now,
        _ => false,
    }
}

/// Since when it's been over (none: it isn't): when it was cancelled or
/// dismissed, or when it ended.
fn over_since(t: &Timer) -> Option<u64> {
    match t.state.as_str() {
        DONE | CANCELLED => Some(t.updated_at),
        RUNNING => Some(t.ends_at),
        _ => None,
    }
}

/// Over for `KEEP_GONE_MS`: forgotten.
fn forgotten(t: &Timer, now: u64) -> bool {
    over_since(t).is_some_and(|at| at.saturating_add(KEEP_GONE_MS) < now)
}

/// The order merges go by: the newest change wins; the same time goes by
/// the device that made it, then by what it says, so every device agrees.
#[allow(clippy::type_complexity)]
fn order(t: &Timer) -> (u64, &str, u8, u64, u64, u64, &str, &[String]) {
    (
        t.updated_at,
        &t.by,
        state_rank(&t.state),
        t.ends_at,
        t.remaining_ms,
        t.duration_ms,
        &t.label,
        &t.ring_on,
    )
}

/// `a` replaces `b` in a merge.
pub fn wins_over(a: &Timer, b: &Timer) -> bool {
    order(a) > order(b)
}

fn valid_id(id: &str) -> bool {
    !id.trim().is_empty() && id.len() <= MAX_ID && !id.chars().any(char::is_control)
}

/// A label tidied: spaces collapsed, control characters dropped, cut to
/// `MAX_LABEL`.
pub fn tidy_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The devices to ring on: real node ids, each once, at most `MAX_RING`.
pub fn tidy_ring(nodes: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in nodes {
        let n = n.trim();
        if valid_id(n) && !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
        if out.len() == MAX_RING {
            break;
        }
    }
    out
}

/// A timer as it's kept: tidied, or `None` if it can't be kept.
pub fn sanitize(mut t: Timer) -> Option<Timer> {
    if !valid_id(&t.id) || !valid_id(&t.by) || state_rank(&t.state) == 0 {
        return None;
    }
    t.label = tidy_label(&t.label);
    t.by_name = tidy_label(&t.by_name);
    t.ring_on = tidy_ring(&t.ring_on);
    t.duration_ms = t.duration_ms.clamp(MIN_DURATION_MS, MAX_DURATION_MS);
    match t.state.as_str() {
        RUNNING => {
            if t.ends_at == 0 {
                return None;
            }
            t.remaining_ms = 0;
        }
        PAUSED => {
            t.ends_at = 0;
            t.remaining_ms = t.remaining_ms.min(MAX_DURATION_MS);
        }
        _ => {}
    }
    Some(t)
}

/// Timers as they arrived (a JSON array), each tidied; the ones that
/// can't be read are left out.
pub fn parse(v: &Value) -> TimerMap {
    let Some(arr) = v.as_array() else {
        return TimerMap::new();
    };
    arr.iter()
        .take(MAX_KEPT * 2)
        .filter_map(|t| sanitize(serde_json::from_value::<Timer>(t.clone()).ok()?))
        .map(|t| (t.id.clone(), t))
        .collect()
}

/// Make room for one more in a full map: drop the finished one over the
/// longest. False if there's none.
fn make_room(map: &mut TimerMap) -> bool {
    if map.len() < MAX_KEPT {
        return true;
    }
    let oldest = map
        .values()
        .filter(|t| !is_active(t))
        .min_by_key(|t| (over_since(t).unwrap_or(0), t.id.clone()))
        .map(|t| t.id.clone());
    match oldest {
        Some(id) => {
            map.remove(&id);
            true
        }
        None => false,
    }
}

/// Merge `incoming` (tidied: `parse`) into `map`, last writer wins per
/// timer. Returns the timers that changed.
pub fn merge(map: &mut TimerMap, incoming: &TimerMap) -> Vec<String> {
    let mut changed = Vec::new();
    for (id, t) in incoming {
        let fits = match map.get(id) {
            Some(cur) => wins_over(t, cur),
            None => make_room(map),
        };
        if fits {
            map.insert(id.clone(), t.clone());
            changed.push(id.clone());
        }
    }
    changed
}

/// `ours` has a timer newer than `theirs` has (or one it lacks): worth
/// answering with.
pub fn has_newer(ours: &TimerMap, theirs: &TimerMap) -> bool {
    ours.iter()
        .any(|(id, t)| theirs.get(id).is_none_or(|o| wins_over(t, o)))
}

/// Forget what's been over for `KEEP_GONE_MS`. True if any went.
pub fn prune(map: &mut TimerMap, now: u64) -> bool {
    let before = map.len();
    map.retain(|_, t| !forgotten(t, now));
    map.len() != before
}

/// The timers the apps show at `now`: running ones by when they end
/// (ended ones first), then paused ones, oldest first.
pub fn shown(map: &TimerMap, now: u64) -> Vec<Timer> {
    let mut out: Vec<Timer> = map.values().filter(|t| is_shown(t, now)).cloned().collect();
    out.sort_by(|a, b| {
        (state_rank(&a.state), a.ends_at, a.created_at, &a.id).cmp(&(
            state_rank(&b.state),
            b.ends_at,
            b.created_at,
            &b.id,
        ))
    });
    out
}

/// Who makes a change: this device's node id and name.
pub struct Author<'a> {
    pub node: &'a str,
    pub name: &'a str,
}

/// A new timer `id` for `duration_ms`, running from `now`.
pub fn start(
    map: &mut TimerMap,
    id: &str,
    label: &str,
    duration_ms: u64,
    ring_on: &[String],
    who: &Author,
    now: u64,
) -> crate::Result<Timer> {
    if !(MIN_DURATION_MS..=MAX_DURATION_MS).contains(&duration_ms) {
        return Err(CoreError::InvalidInput(
            "a timer is from a second to a week long".into(),
        ));
    }
    if !valid_id(id) || map.contains_key(id) {
        return Err(CoreError::InvalidInput("that timer already exists".into()));
    }
    if map.values().filter(|t| is_shown(t, now)).count() >= MAX_ACTIVE {
        return Err(CoreError::InvalidInput(format!(
            "at most {MAX_ACTIVE} timers at once"
        )));
    }
    if !make_room(map) {
        return Err(CoreError::InvalidInput("too many timers".into()));
    }
    let t = Timer {
        id: id.to_string(),
        label: tidy_label(label),
        duration_ms,
        state: RUNNING.into(),
        ends_at: now.saturating_add(duration_ms),
        remaining_ms: 0,
        ring_on: tidy_ring(ring_on),
        created_at: now,
        updated_at: now,
        by: who.node.to_string(),
        by_name: tidy_label(who.name),
    };
    map.insert(t.id.clone(), t.clone());
    Ok(t)
}

/// A change to a timer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Pause,
    Resume,
    /// Stop it and forget it, on every device.
    Cancel,
    /// It ended: stop the ringing everywhere.
    Dismiss,
    /// Add this many ms (an ended one starts again for that long).
    Add(u64),
    /// Start it again from the top, for as long as it was set for.
    Restart,
    /// Ring on these devices when it ends.
    Ring(Vec<String>),
}

/// Change timer `id` (running or paused) at `now`. Answers with it as
/// changed. Always newer than what the map had, whatever the clock says.
pub fn change(
    map: &mut TimerMap,
    id: &str,
    op: &Op,
    who: &Author,
    now: u64,
) -> crate::Result<Timer> {
    let cur = map
        .get(id)
        .filter(|t| is_active(t))
        .cloned()
        .ok_or_else(|| CoreError::InvalidInput("that timer has gone".into()))?;
    let mut t = cur.clone();
    match op {
        Op::Pause => {
            if t.state == RUNNING {
                if has_ended(&t, now) {
                    return Err(CoreError::InvalidInput("that timer has ended".into()));
                }
                t.remaining_ms = remaining(&t, now);
                t.ends_at = 0;
                t.state = PAUSED.into();
            }
        }
        Op::Resume => {
            if t.state == PAUSED {
                t.ends_at = now.saturating_add(t.remaining_ms.max(1));
                t.remaining_ms = 0;
                t.state = RUNNING.into();
            }
        }
        Op::Cancel => t.state = CANCELLED.into(),
        Op::Dismiss => t.state = DONE.into(),
        Op::Add(ms) => {
            let ms = *ms;
            if ms == 0 {
                return Err(CoreError::InvalidInput("add some time".into()));
            }
            let left = remaining(&t, now).saturating_add(ms);
            if left > MAX_DURATION_MS {
                return Err(CoreError::InvalidInput(
                    "a timer is at most a week long".into(),
                ));
            }
            if has_ended(&t, now) {
                t.duration_ms = ms.max(MIN_DURATION_MS);
                t.ends_at = now.saturating_add(ms);
            } else if t.state == RUNNING {
                t.ends_at = t.ends_at.saturating_add(ms);
                t.duration_ms = t.duration_ms.saturating_add(ms).min(MAX_DURATION_MS);
            } else {
                t.remaining_ms = left;
                t.duration_ms = t.duration_ms.saturating_add(ms).min(MAX_DURATION_MS);
            }
        }
        Op::Restart => {
            t.state = RUNNING.into();
            t.ends_at = now.saturating_add(t.duration_ms);
            t.remaining_ms = 0;
        }
        Op::Ring(nodes) => t.ring_on = tidy_ring(nodes),
    }
    if t == cur {
        return Ok(t);
    }
    t.updated_at = now.max(cur.updated_at.saturating_add(1));
    t.by = who.node.to_string();
    t.by_name = tidy_label(who.name);
    map.insert(t.id.clone(), t.clone());
    Ok(t)
}

/// The replicated map, and which devices understand `timers`.
#[derive(Default)]
pub struct Timers {
    /// `None` until read from `kv`.
    map: Mutex<Option<TimerMap>>,
    caps: Mutex<HashMap<String, bool>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run `f` on the map (read from `kv` the first time, with what's been
/// over long enough forgotten); saved when that or `f` changed it.
fn with_map<P: Platform, R>(ctx: &NetCtx<P>, f: impl FnOnce(&mut TimerMap) -> (R, bool)) -> R {
    let mut slot = ctx.extras.timers.map.lock();
    let map = slot.get_or_insert_with(|| {
        ctx.platform
            .kv_get(MAP_KEY)
            .and_then(|j| serde_json::from_str::<Value>(&j).ok())
            .map(|v| parse(&v))
            .unwrap_or_default()
    });
    let pruned = prune(map, now_ms());
    let (r, changed) = f(map);
    if changed || pruned {
        let list: Vec<&Timer> = map.values().collect();
        if let Ok(json) = serde_json::to_string(&list) {
            ctx.platform.kv_set(MAP_KEY, &json);
        }
    }
    r
}

fn author<P: Platform>(ctx: &NetCtx<P>) -> (String, String) {
    let id = ctx.identity.lock();
    (id.node_id.clone(), id.name.clone())
}

/// The timers the apps show (see `shown`).
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> Vec<Timer> {
    with_map(ctx, |m| (shown(m, now_ms()), false))
}

/// Start a timer here: kept, and sent to your linked devices that
/// understand timers (the others get it at their next link).
pub fn start_timer<P: Platform>(
    ctx: &NetCtx<P>,
    label: &str,
    duration_ms: u64,
    ring_on: &[String],
) -> crate::Result<Timer> {
    if !ctx.config.timers {
        return Err(CoreError::InvalidInput("timers are off".into()));
    }
    let (node, name) = author(ctx);
    let who = Author {
        node: &node,
        name: &name,
    };
    let id = uuid::Uuid::new_v4().simple().to_string();
    let t = with_map(ctx, |m| {
        match start(m, &id, label, duration_ms, ring_on, &who, now_ms()) {
            Ok(t) => (Ok(t), true),
            Err(e) => (Err(e), false),
        }
    })?;
    send_all(ctx);
    Ok(t)
}

/// Change a timer here (see `Op`): kept, and sent like a new one.
pub fn change_timer<P: Platform>(ctx: &NetCtx<P>, id: &str, op: &Op) -> crate::Result<Timer> {
    let (node, name) = author(ctx);
    let who = Author {
        node: &node,
        name: &name,
    };
    let (t, changed) = with_map(ctx, |m| {
        let before = m.get(id).cloned();
        match change(m, id, op, &who, now_ms()) {
            Ok(t) => {
                let changed = m.get(id) != before.as_ref();
                (Ok((t, changed)), changed)
            }
            Err(e) => (Err(e), false),
        }
    })?;
    if changed {
        send_all(ctx);
    }
    Ok(t)
}

/// Say in this device's status that it understands `timers`.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.timers {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `timers` value of its status or
/// `addr_update`) whether it understands `timers`. Kept across restarts;
/// newly understood, it gets the timers now.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    if set_caps(ctx, node, on) && on {
        send_to(ctx, node);
    }
}

/// True if it changed.
fn set_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> bool {
    if node.is_empty() || supports(ctx, node) == on {
        return false;
    }
    ctx.extras.timers.caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
    true
}

/// `node` said it understands `timers`.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if let Some(on) = ctx.extras.timers.caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    ctx.extras.timers.caps.lock().insert(node.to_string(), on);
    on
}

/// A link to `node` just came up: send it the timers.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if supports(ctx, node) {
        send_to(ctx, node);
    }
}

fn message<P: Platform>(ctx: &NetCtx<P>) -> String {
    let me = ctx.identity.lock().node_id.clone();
    let timers = with_map(ctx, |m| {
        let list: Vec<&Timer> = m.values().collect();
        (serde_json::to_value(list).unwrap_or_default(), false)
    });
    json!({ "type": "timers", "from": me, "timers": timers }).to_string()
}

/// Send the whole map to `node`, if it understands it (and it's a real
/// device, not a hand-entered address).
fn send_to<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !ctx.config.timers || node.starts_with("manual:") || !supports(ctx, node) {
        return;
    }
    send_control_to(ctx, node, &message(ctx));
}

/// Send the whole map to every linked device that understands it.
fn send_all<P: Platform>(ctx: &NetCtx<P>) {
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    for node in linked {
        send_to(ctx, &node);
    }
}

pub struct TimersHandler;

impl<P: Platform> ControlHandler<P> for TimersHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["timers"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return; // only your own devices start your timers
        }
        // Sending it says it understands it.
        set_caps(ctx, &from, true);
        let now = now_ms();
        let mut theirs = parse(&msg.json("timers"));
        // What's been over long enough is forgotten here: never taken back.
        let all_theirs = theirs.clone();
        prune(&mut theirs, now);
        let (shown_now, answer) = with_map(ctx, |m| {
            let changed = merge(m, &theirs);
            let answer = has_newer(m, &all_theirs);
            let dirty = !changed.is_empty();
            ((dirty.then(|| shown(m, now)), answer), dirty)
        });
        if let Some(timers) = shown_now {
            ctx.sink.emit(CoreEvent::TimersChanged { timers });
        }
        if answer {
            send_to(ctx, &from);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const ME: Author = Author {
        node: "a",
        name: "Mac",
    };
    const THEM: Author = Author {
        node: "b",
        name: "Pixel",
    };

    fn started(now: u64) -> TimerMap {
        let mut m = TimerMap::new();
        start(&mut m, "t1", " Pasta \n", 5 * MIN, &["a".into()], &ME, now).unwrap();
        m
    }

    #[test]
    fn a_timer_counts_down_pauses_and_resumes() {
        let now = 1_000_000;
        let mut m = started(now);
        let t = &m["t1"];
        assert_eq!((t.label.as_str(), t.state.as_str()), ("Pasta", RUNNING));
        assert_eq!(remaining(t, now + MIN), 4 * MIN);

        let p = change(&mut m, "t1", &Op::Pause, &THEM, now + MIN).unwrap();
        assert_eq!(
            (p.state.as_str(), p.remaining_ms, p.ends_at),
            (PAUSED, 4 * MIN, 0)
        );
        assert_eq!(p.by, "b");
        // Time passes while paused: nothing's taken off.
        assert_eq!(remaining(&p, now + 60 * MIN), 4 * MIN);

        let r = change(&mut m, "t1", &Op::Resume, &ME, now + 10 * MIN).unwrap();
        assert_eq!(r.ends_at, now + 14 * MIN);
        assert!(!has_ended(&r, now + 14 * MIN - 1));
        assert!(has_ended(&r, now + 14 * MIN));
        // Pausing an ended one makes no sense; dismissing it does.
        assert!(change(&mut m, "t1", &Op::Pause, &ME, now + 15 * MIN).is_err());
        let d = change(&mut m, "t1", &Op::Dismiss, &ME, now + 15 * MIN).unwrap();
        assert_eq!(d.state, DONE);
        assert!(shown(&m, now + 15 * MIN).is_empty());
        // Gone: no more changes.
        assert!(change(&mut m, "t1", &Op::Resume, &ME, now + 16 * MIN).is_err());
    }

    #[test]
    fn adding_time_and_restarting() {
        let now = 1_000_000;
        let mut m = started(now);
        let t = change(&mut m, "t1", &Op::Add(MIN), &ME, now).unwrap();
        assert_eq!((t.ends_at, t.duration_ms), (now + 6 * MIN, 6 * MIN));
        // Ended: a minute more starts it again for a minute.
        let late = now + 10 * MIN;
        let t = change(&mut m, "t1", &Op::Add(MIN), &ME, late).unwrap();
        assert_eq!((t.ends_at, t.duration_ms), (late + MIN, MIN));
        let t = change(&mut m, "t1", &Op::Restart, &ME, late + 5 * MIN).unwrap();
        assert_eq!(t.ends_at, late + 6 * MIN);
        // Paused: added to what's left.
        change(&mut m, "t1", &Op::Pause, &ME, late + 5 * MIN).unwrap();
        let t = change(&mut m, "t1", &Op::Add(2 * MIN), &ME, late + 5 * MIN).unwrap();
        assert_eq!(t.remaining_ms, 3 * MIN);
        assert!(change(&mut m, "t1", &Op::Add(MAX_DURATION_MS), &ME, late).is_err());
        assert!(change(&mut m, "t1", &Op::Add(0), &ME, late).is_err());
    }

    #[test]
    fn starting_checks_what_it_is_given() {
        let mut m = TimerMap::new();
        assert!(start(&mut m, "x", "", 500, &[], &ME, 1).is_err());
        assert!(start(&mut m, "x", "", MAX_DURATION_MS + 1, &[], &ME, 1).is_err());
        let ring: Vec<String> = vec!["a".into(), "a".into(), " ".into(), "b".into()];
        let t = start(&mut m, "x", &"y".repeat(100), MIN, &ring, &ME, 1).unwrap();
        assert_eq!(t.ring_on, ["a", "b"]);
        assert_eq!(t.label.chars().count(), MAX_LABEL);
        assert!(start(&mut m, "x", "", MIN, &[], &ME, 1).is_err(), "same id");
        for i in 1..MAX_ACTIVE {
            start(&mut m, &format!("t{i}"), "", MIN, &[], &ME, 1).unwrap();
        }
        assert!(start(&mut m, "one-more", "", MIN, &[], &ME, 1).is_err());
    }

    #[test]
    fn the_last_change_wins_in_any_order() {
        let now = 1_000_000;
        let base = started(now);
        let mut a = base.clone();
        let mut b = base.clone();
        change(&mut a, "t1", &Op::Pause, &ME, now + MIN).unwrap();
        change(&mut b, "t1", &Op::Cancel, &THEM, now + 2 * MIN).unwrap();
        let mut x = base.clone();
        merge(&mut x, &a);
        merge(&mut x, &b);
        let mut y = base.clone();
        merge(&mut y, &b);
        merge(&mut y, &a);
        assert_eq!(x, y);
        assert_eq!(x["t1"].state, CANCELLED);
        // The cancel stays: an older copy never brings it back.
        assert!(merge(&mut x, &base).is_empty());
        assert!(has_newer(&x, &base));
        assert!(!has_newer(&base, &x));
    }

    #[test]
    fn a_local_change_beats_what_was_there_whatever_the_clock() {
        let mut m = started(5_000_000);
        let t = change(&mut m, "t1", &Op::Pause, &THEM, 1_000).unwrap();
        assert_eq!(t.updated_at, 5_000_001);
    }

    #[test]
    fn what_is_over_is_forgotten_after_a_while() {
        let now = 1_000_000;
        let mut m = started(now);
        start(&mut m, "p", "", MIN, &[], &ME, now).unwrap();
        change(&mut m, "p", &Op::Pause, &ME, now).unwrap();
        start(&mut m, "c", "", MIN, &[], &ME, now).unwrap();
        change(&mut m, "c", &Op::Cancel, &ME, now).unwrap();
        // Ended and never dismissed: shown for a day, then not.
        let later = now + 5 * MIN + ENDED_SHOWN_MS - 1;
        assert_eq!(shown(&m, later).len(), 2);
        assert_eq!(shown(&m, later + 1).len(), 1, "only the paused one");
        assert!(!prune(&mut m, now + KEEP_GONE_MS));
        assert!(prune(&mut m, now + 5 * MIN + KEEP_GONE_MS + 1));
        assert_eq!(m.keys().collect::<Vec<_>>(), ["p"], "paused stays");
    }

    #[test]
    fn what_arrives_is_tidied_and_round_trips() {
        let v = json!([
            {"id": "ok", "label": " Tea\n", "duration_ms": 10, "state": "running",
             "ends_at": 99, "remaining_ms": 5, "ring_on": ["a", "a"],
             "updated_at": 3, "by": "b"},
            {"id": "noend", "duration_ms": 1000, "state": "running", "updated_at": 3, "by": "b"},
            {"id": "weird", "duration_ms": 1000, "state": "exploded", "ends_at": 9,
             "updated_at": 3, "by": "b"},
            {"id": "nobody", "duration_ms": 1000, "state": "paused", "updated_at": 3},
            "not a timer",
        ]);
        let m = parse(&v);
        assert_eq!(m.len(), 1);
        let t = &m["ok"];
        assert_eq!(t.label, "Tea");
        assert_eq!(t.duration_ms, MIN_DURATION_MS);
        assert_eq!(t.remaining_ms, 0);
        assert_eq!(t.ring_on, ["a"]);
        let back = parse(&serde_json::to_value(m.values().collect::<Vec<_>>()).unwrap());
        assert_eq!(back, m);
        assert!(!has_newer(&m, &back), "tidied the same on both sides");
    }

    #[test]
    fn shown_in_order_ended_first_then_soonest_then_paused() {
        let now = 1_000_000;
        let mut m = TimerMap::new();
        start(&mut m, "long", "", 30 * MIN, &[], &ME, now).unwrap();
        start(&mut m, "short", "", MIN, &[], &ME, now).unwrap();
        start(&mut m, "paused", "", MIN, &[], &ME, now).unwrap();
        change(&mut m, "paused", &Op::Pause, &ME, now).unwrap();
        let ids: Vec<String> = shown(&m, now + 2 * MIN).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["short", "long", "paused"]);
    }
}
