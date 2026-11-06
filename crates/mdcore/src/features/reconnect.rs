//! Staying reachable: how fast links come back, why they go, and when to
//! leave a sleeping device alone.
//!
//! * Timing. Something that should bring a link up is a *trigger*: the
//!   network changed (`ReconnectTrigger::Network`), a push woke this device
//!   (`Push`, the phone side), or this device pushed another to wake it
//!   (`Wake`, the computer side). The next link to that device measures
//!   trigger → link up; the last `SAMPLES` per device are kept, and a
//!   trigger nothing answered within `PENDING_FOR` is forgotten.
//! * Drops. Each link's lifetime and why it went: `idle` (closed here,
//!   unused in the background), `peer_idle` (the other device closed it
//!   unused: it's asleep, reachable by push), `network_change`, `timeout`
//!   (silence: QUIC's idle timeout), `dead` (it carried nothing:
//!   `transport::outbox`), `manual` (Disconnect), `peer_quit` (its app
//!   quit), `closed` (any other close), `lost` (anything else).
//! * Redial. After a network change, the devices that had a link (or
//!   lost one recently for a reason worth redialling) are dialled again at
//!   once, then with jittered exponential backoff (`backoff`) for up to
//!   `REDIAL_WINDOW`, until a newer change takes over.
//! * Asleep. A push that got no answer means the device sleeps (or is
//!   off): automatic connects (a resend, a background message, a redial)
//!   then try only the direct path, without pushing, for a cooldown that
//!   grows with each unanswered push (`ASLEEP_BASE` doubling to
//!   `ASLEEP_CAP`). Anything the user starts pushes anyway. Any link
//!   resets it.
//! * DeviceReach. What the apps show for a device without a link: `waking` (a
//!   push went, waiting), `asleep`, `unreachable` (the push couldn't be
//!   sent), or `offline` (nothing known).
//! * Local first. A link that goes the long way round to a device that is
//!   now on this network is joined by a direct local QUIC link
//!   (`prefer_lan`, at most every `LAN_RETRY`).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use crate::config::NetKind;
use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport;

/// Reconnect times kept per device.
pub const SAMPLES: usize = 20;
/// A trigger no link answered within this long is forgotten.
pub const PENDING_FOR: Duration = Duration::from_secs(120);
/// A link lost this recently (for a reason worth it) is redialled on a
/// network change.
pub const WANT_FOR: Duration = Duration::from_secs(10 * 60);
/// A push counts as "waking" this long.
pub const WAKING_FOR: Duration = Duration::from_secs(20);
/// Redial after a network change: first retry after about this ...
pub const REDIAL_BASE: Duration = Duration::from_secs(1);
/// ... doubling up to this ...
pub const REDIAL_CAP: Duration = Duration::from_secs(30);
/// ... for at most this long.
pub const REDIAL_WINDOW: Duration = Duration::from_secs(120);
/// Unanswered pushes: automatic connects stop pushing for this long ...
pub const ASLEEP_BASE: Duration = Duration::from_secs(30);
/// ... doubling with each one, up to this.
pub const ASLEEP_CAP: Duration = Duration::from_secs(10 * 60);
/// A push that couldn't be sent counts this long.
const PUSH_FAILED_FOR: Duration = Duration::from_secs(10 * 60);
/// A local dial to a device on a long-way link, at most this often.
pub const LAN_RETRY: Duration = Duration::from_secs(30);

/// What should bring a link up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ReconnectTrigger {
    /// This device's network changed.
    Network,
    /// A push woke this device.
    Push,
    /// This device pushed the other one to wake it.
    Wake,
}

/// Where a device stands, for the apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum DeviceReach {
    Connected,
    /// A push went to wake it; waiting for it.
    Waking,
    /// It didn't answer a push, or closed its link to save battery: it
    /// wakes when something needs it.
    Asleep,
    /// The push couldn't be sent (no server, or it has no push token).
    Unreachable,
    /// Nothing known.
    Offline,
}

/// A device's link over time (part of `diag::Diagnosis`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ReconnectInfo {
    /// The last reconnect, trigger to link up, and the average of those
    /// kept.
    pub last_ms: Option<u32>,
    pub avg_ms: Option<u32>,
    pub samples: u32,
    /// What the last one was after.
    pub last_trigger: Option<ReconnectTrigger>,
    /// Averages by trigger.
    pub network_avg_ms: Option<u32>,
    pub push_avg_ms: Option<u32>,
    pub wake_avg_ms: Option<u32>,
    /// How long links lasted: the last one that ended, and the average.
    pub lifetime_last_s: Option<u32>,
    pub lifetime_avg_s: Option<u32>,
    /// The link up now, for how long.
    pub up_for_s: Option<u32>,
    /// Why the last link went, and how long ago.
    pub last_drop: Option<String>,
    pub last_drop_ago_s: Option<u32>,
    /// Drops by reason, most first.
    pub drops: Vec<DropCount>,
    pub reach: Option<DeviceReach>,
    /// Automatic connects leave it alone for this much longer.
    pub quiet_for_s: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DropCount {
    pub reason: String,
    pub count: u32,
}

/// `base` doubled `attempt` times, at most `cap`, then spread ±20% by
/// `jitter` (0..1), so devices that lost the same network don't all dial
/// at once.
pub fn backoff(attempt: u32, base: Duration, cap: Duration, jitter: f64) -> Duration {
    let factor = 1u32 << attempt.min(16);
    let exp = base.saturating_mul(factor).min(cap);
    exp.mul_f64(0.8 + 0.4 * jitter.clamp(0.0, 1.0))
}

/// A uniformly random number in 0..1.
pub fn jitter() -> f64 {
    use rand_core::RngCore;
    f64::from(rand_core::OsRng.next_u32()) / f64::from(u32::MAX)
}

/// A link lost for `reason` is worth redialling on a network change (not
/// one closed on purpose by either side).
pub fn redialable(reason: &str) -> bool {
    matches!(
        reason,
        "network_change" | "timeout" | "lost" | "dead" | "closed"
    )
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

fn secs(d: Duration) -> u32 {
    u32::try_from(d.as_secs()).unwrap_or(u32::MAX)
}

fn avg(xs: impl Iterator<Item = u32>) -> Option<u32> {
    let (sum, n) = xs.fold((0u64, 0u64), |(s, n), x| (s + u64::from(x), n + 1));
    (n > 0).then(|| u32::try_from(sum / n).unwrap_or(u32::MAX))
}

#[derive(Default)]
struct Track {
    pending: Option<(ReconnectTrigger, Instant)>,
    samples: VecDeque<(ReconnectTrigger, u32)>,
    up_at: Option<Instant>,
    lifetimes: VecDeque<u32>,
    /// Why the link is being closed (set just before; read when it goes).
    closing: Option<String>,
    last_drop: Option<(String, Instant)>,
    drops: BTreeMap<String, u32>,
    /// Pushes in a row that got no answer.
    misses: u32,
    asleep_until: Option<Instant>,
    asleep_at: Option<Instant>,
    waking_at: Option<Instant>,
    push_failed_at: Option<Instant>,
}

/// The bookkeeping, apart from the engine (times are passed in).
#[derive(Default)]
pub struct Tracker {
    peers: HashMap<String, Track>,
}

impl Tracker {
    fn t(&mut self, node: &str) -> &mut Track {
        self.peers.entry(node.to_string()).or_default()
    }

    /// Something that should bring a link to `node` up happened at `at`.
    /// An earlier one still waiting is kept (the wait started then).
    pub fn trigger(&mut self, node: &str, kind: ReconnectTrigger, at: Instant) {
        let t = self.t(node);
        let stale = t
            .pending
            .is_none_or(|(_, was)| at.saturating_duration_since(was) >= PENDING_FOR);
        if stale {
            t.pending = Some((kind, at));
        }
    }

    /// Forget what's waiting (the link survived, say).
    pub fn clear_pending(&mut self, node: &str) {
        if let Some(t) = self.peers.get_mut(node) {
            t.pending = None;
        }
    }

    /// A link to `node` came up: the reconnect it ends, if one was waiting.
    pub fn link_up(&mut self, node: &str, at: Instant) -> Option<(ReconnectTrigger, u32)> {
        let t = self.t(node);
        t.up_at = Some(at);
        t.misses = 0;
        t.asleep_until = None;
        t.asleep_at = None;
        t.waking_at = None;
        t.push_failed_at = None;
        t.closing = None;
        let (kind, since) = t.pending.take()?;
        let took = at.saturating_duration_since(since);
        if took >= PENDING_FOR {
            return None;
        }
        let sample = (kind, ms(took));
        if t.samples.len() >= SAMPLES {
            t.samples.pop_front();
        }
        t.samples.push_back(sample);
        Some(sample)
    }

    /// The link to `node` is being closed for `why` (it goes in a moment);
    /// `""`: it isn't after all.
    pub fn closing(&mut self, node: &str, why: &str) {
        self.t(node).closing = (!why.is_empty()).then(|| why.to_string());
    }

    /// The link to `node` went at `at`: why (as `closing` said, else
    /// `lost`).
    pub fn link_down(&mut self, node: &str, at: Instant) -> String {
        let t = self.t(node);
        let why = t.closing.take().unwrap_or_else(|| "lost".into());
        if let Some(up) = t.up_at.take() {
            if t.lifetimes.len() >= SAMPLES {
                t.lifetimes.pop_front();
            }
            t.lifetimes
                .push_back(secs(at.saturating_duration_since(up)));
        }
        *t.drops.entry(why.clone()).or_default() += 1;
        t.last_drop = Some((why.clone(), at));
        if why == "peer_idle" {
            t.asleep_at = Some(at);
        }
        why
    }

    /// A push went to `node` to wake it.
    pub fn waking(&mut self, node: &str, at: Instant) {
        let t = self.t(node);
        t.waking_at = Some(at);
        t.push_failed_at = None;
    }

    /// The push to `node` got no answer: automatic connects leave it alone
    /// for the returned time.
    pub fn no_answer(&mut self, node: &str, at: Instant, jitter: f64) -> Duration {
        let t = self.t(node);
        t.misses = t.misses.saturating_add(1);
        let quiet = backoff(t.misses - 1, ASLEEP_BASE, ASLEEP_CAP, jitter);
        t.asleep_until = Some(at + quiet);
        t.asleep_at = Some(at);
        t.waking_at = None;
        quiet
    }

    /// The push to `node` couldn't be sent.
    pub fn push_failed(&mut self, node: &str, at: Instant) {
        let t = self.t(node);
        t.push_failed_at = Some(at);
        t.waking_at = None;
    }

    /// How much longer automatic connects leave `node` alone.
    pub fn cooldown(&self, node: &str, now: Instant) -> Option<Duration> {
        let until = self.peers.get(node)?.asleep_until?;
        (until > now).then(|| until - now)
    }

    /// Devices without a link worth redialling on a network change: their
    /// link went within `WANT_FOR`, for a reason worth it.
    pub fn wanted(&self, now: Instant) -> Vec<String> {
        let mut out: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, t)| t.up_at.is_none())
            .filter(|(_, t)| {
                t.last_drop.as_ref().is_some_and(|(why, at)| {
                    redialable(why) && now.saturating_duration_since(*at) < WANT_FOR
                })
            })
            .map(|(k, _)| k.clone())
            .collect();
        out.sort();
        out
    }

    /// Where `node` stands (`live`: it has a link).
    pub fn reach(&self, node: &str, live: bool, now: Instant) -> DeviceReach {
        if live {
            return DeviceReach::Connected;
        }
        let Some(t) = self.peers.get(node) else {
            return DeviceReach::Offline;
        };
        let recent = |at: Option<Instant>, within: Duration| {
            at.filter(|a| now.saturating_duration_since(*a) < within)
        };
        let waking = recent(t.waking_at, WAKING_FOR);
        let failed = recent(t.push_failed_at, PUSH_FAILED_FOR);
        let asleep = t.asleep_at;
        // The newest of the three says it.
        let newest = [
            (waking, DeviceReach::Waking),
            (failed, DeviceReach::Unreachable),
            (asleep, DeviceReach::Asleep),
        ]
        .into_iter()
        .filter_map(|(at, r)| at.map(|a| (a, r)))
        .max_by_key(|(a, _)| *a);
        if let Some((_, r)) = newest {
            return r;
        }
        DeviceReach::Offline
    }

    /// The report for `node`.
    pub fn report(&self, node: &str, live: bool, now: Instant) -> ReconnectInfo {
        let reach = Some(self.reach(node, live, now));
        let Some(t) = self.peers.get(node) else {
            return ReconnectInfo {
                reach,
                ..Default::default()
            };
        };
        let of = |k: ReconnectTrigger| avg(t.samples.iter().filter(|s| s.0 == k).map(|s| s.1));
        let mut drops: Vec<DropCount> = t
            .drops
            .iter()
            .map(|(r, n)| DropCount {
                reason: r.clone(),
                count: *n,
            })
            .collect();
        drops.sort_by(|a, b| b.count.cmp(&a.count).then(a.reason.cmp(&b.reason)));
        ReconnectInfo {
            last_ms: t.samples.back().map(|s| s.1),
            avg_ms: avg(t.samples.iter().map(|s| s.1)),
            samples: u32::try_from(t.samples.len()).unwrap_or(u32::MAX),
            last_trigger: t.samples.back().map(|s| s.0),
            network_avg_ms: of(ReconnectTrigger::Network),
            push_avg_ms: of(ReconnectTrigger::Push),
            wake_avg_ms: of(ReconnectTrigger::Wake),
            lifetime_last_s: t.lifetimes.back().copied(),
            lifetime_avg_s: avg(t.lifetimes.iter().copied()),
            up_for_s: t
                .up_at
                .filter(|_| live)
                .map(|u| secs(now.saturating_duration_since(u))),
            last_drop: t.last_drop.as_ref().map(|d| d.0.clone()),
            last_drop_ago_s: t
                .last_drop
                .as_ref()
                .map(|d| secs(now.saturating_duration_since(d.1))),
            drops,
            reach,
            quiet_for_s: self.cooldown(node, now).map(secs),
        }
    }
}

/// Everything this module keeps (in the engine's `Extras`).
#[derive(Default)]
pub struct ReconnectState {
    tracker: Mutex<Tracker>,
    /// Bumped by every network change: a redial from an older one stops.
    gen: AtomicU64,
    net: Mutex<NetKind>,
    lan_tried: Mutex<HashMap<String, Instant>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &ReconnectState {
    &ctx.extras.reconnect
}

fn progress<P: Platform>(ctx: &NetCtx<P>, node: &str, stage: &str) {
    ctx.sink.emit(CoreEvent::ConnectProgress {
        node_id: node.to_string(),
        stage: stage.into(),
        detail: None,
    });
}

/// Something that should bring a link to `node` up happened just now
/// (`ago` earlier: a push that waited for the app to start).
pub fn trigger<P: Platform>(ctx: &NetCtx<P>, node: &str, kind: ReconnectTrigger, ago: Duration) {
    let at = Instant::now().checked_sub(ago).unwrap_or_else(Instant::now);
    st(ctx).tracker.lock().trigger(node, kind, at);
}

/// A link to `node` came up (`transport::mark_live`).
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let took = st(ctx).tracker.lock().link_up(node, Instant::now());
    if let Some((kind, ms)) = took {
        let kind = serde_json::to_value(kind).ok();
        let detail = format!(
            "{}:{ms}",
            kind.as_ref().and_then(|v| v.as_str()).unwrap_or("")
        );
        super::diag::note(ctx, node, "reconnected", Some(&detail));
    }
}

/// The link to `node` is about to be closed for `why`.
pub fn closing<P: Platform>(ctx: &NetCtx<P>, node: &str, why: &str) {
    st(ctx).tracker.lock().closing(node, why);
}

/// The link to `node` went (`transport::drop_live`): why.
pub fn link_down<P: Platform>(ctx: &NetCtx<P>, node: &str) -> String {
    let why = st(ctx).tracker.lock().link_down(node, Instant::now());
    if why == "peer_idle" {
        // It closed an unused link to save battery: asleep, not gone.
        progress(ctx, node, "asleep");
    }
    why
}

/// A push went to `node` to wake it (it has no link).
pub fn waking<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if transport::is_live(ctx, node) {
        return;
    }
    {
        let mut t = st(ctx).tracker.lock();
        t.trigger(node, ReconnectTrigger::Wake, Instant::now());
        t.waking(node, Instant::now());
    }
    progress(ctx, node, "waking");
}

/// The push to `node` got no answer.
pub fn no_answer<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    st(ctx)
        .tracker
        .lock()
        .no_answer(node, Instant::now(), jitter());
}

/// The push to `node` couldn't be sent.
pub fn push_failed<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    st(ctx).tracker.lock().push_failed(node, Instant::now());
}

/// How much longer automatic connects leave `node` alone (it didn't
/// answer a push).
pub fn cooldown<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<Duration> {
    st(ctx).tracker.lock().cooldown(node, Instant::now())
}

pub fn report<P: Platform>(ctx: &NetCtx<P>, node: &str) -> ReconnectInfo {
    let live = transport::is_live(ctx, node);
    st(ctx).tracker.lock().report(node, live, Instant::now())
}

pub fn reach<P: Platform>(ctx: &NetCtx<P>, node: &str) -> DeviceReach {
    let live = transport::is_live(ctx, node);
    st(ctx).tracker.lock().reach(node, live, Instant::now())
}

/// Devices to redial on a network change (see `Tracker::wanted`).
pub fn wanted<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    st(ctx).tracker.lock().wanted(Instant::now())
}

/// Forget a waiting trigger for `node`.
pub fn clear_pending<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    st(ctx).tracker.lock().clear_pending(node);
}

/// The network changed (to `net`; `Unknown` keeps what was said before):
/// the new generation, and the network now.
pub fn network_changed<P: Platform>(ctx: &NetCtx<P>, net: NetKind) -> (u64, NetKind) {
    let net = {
        let mut cur = st(ctx).net.lock();
        if net != NetKind::Unknown {
            *cur = net;
        }
        *cur
    };
    // Local dials may be tried again on the new network.
    st(ctx).lan_tried.lock().clear();
    (st(ctx).gen.fetch_add(1, Ordering::SeqCst) + 1, net)
}

/// The current network-change generation.
pub fn generation<P: Platform>(ctx: &NetCtx<P>) -> u64 {
    st(ctx).gen.load(Ordering::SeqCst)
}

/// The network the platform said this device is on.
pub fn net<P: Platform>(ctx: &NetCtx<P>) -> NetKind {
    *st(ctx).net.lock()
}

/// A link to a device that goes the long way round (the relay, Bluetooth,
/// an internet path) while that device is on this network: dial it
/// locally (the local link takes over; see `features::nearby`). Only to
/// devices that say `nearby` (an older app's link rules could close both).
/// Called on a timer and after a network change.
pub fn prefer_lan<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.config.nearby || !ctx.config.auto_quic || net(ctx) == NetKind::Offline {
        return;
    }
    let far: Vec<String> = ctx
        .p2p_conns
        .lock()
        .iter()
        .filter(|(_, c)| !super::nearby::is_local_link(c.kind, c.addr))
        .map(|(k, _)| k.clone())
        .collect();
    for node in far {
        if !super::nearby::supports(ctx, &node) || super::nearby::near(ctx, &node) != Some("lan") {
            continue;
        }
        let Some(peer) = ctx.peers.lock().get(&node).cloned() else {
            continue;
        };
        let targets = super::nearby::lan_targets(&peer);
        if targets.is_empty() {
            continue;
        }
        {
            let mut tried = st(ctx).lan_tried.lock();
            if tried.get(&node).is_some_and(|t| t.elapsed() < LAN_RETRY) {
                continue;
            }
            tried.insert(node.clone(), Instant::now());
        }
        super::diag::note(ctx, &node, "lan_try", None);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = ctx.quic.connect_any(ctx.clone(), &targets);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn backoff_doubles_up_to_the_cap_with_twenty_percent_jitter() {
        let mid = |a| backoff(a, REDIAL_BASE, REDIAL_CAP, 0.5);
        assert_eq!(mid(0), S);
        assert_eq!(mid(1), 2 * S);
        assert_eq!(mid(2), 4 * S);
        assert_eq!(mid(4), 16 * S);
        assert_eq!(mid(5), 30 * S);
        assert_eq!(mid(60), 30 * S);
        assert_eq!(backoff(0, S, REDIAL_CAP, 0.0), S.mul_f64(0.8));
        assert_eq!(backoff(0, S, REDIAL_CAP, 1.0), S.mul_f64(1.2));
        // Out-of-range jitter is clamped.
        assert_eq!(backoff(0, S, REDIAL_CAP, 7.0), S.mul_f64(1.2));
        for _ in 0..100 {
            let d = backoff(3, S, REDIAL_CAP, jitter());
            assert!(
                d >= (8 * S).mul_f64(0.8) && d <= (8 * S).mul_f64(1.2),
                "{d:?}"
            );
        }
    }

    #[test]
    fn the_redial_schedule_fits_its_window() {
        let total: Duration = (0..6)
            .map(|a| backoff(a, REDIAL_BASE, REDIAL_CAP, 1.0))
            .sum();
        assert!(total <= REDIAL_WINDOW, "{total:?}");
    }

    #[test]
    fn a_trigger_is_timed_to_the_next_link() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        t.trigger("pc", ReconnectTrigger::Network, t0);
        // A later trigger doesn't restart the clock.
        t.trigger("pc", ReconnectTrigger::Push, t0 + S);
        assert_eq!(
            t.link_up("pc", t0 + Duration::from_millis(1500)),
            Some((ReconnectTrigger::Network, 1500))
        );
        // Used up.
        assert_eq!(t.link_up("pc", t0 + 2 * S), None);
        t.trigger("pc", ReconnectTrigger::Push, t0 + 10 * S);
        t.link_up("pc", t0 + 10 * S + Duration::from_millis(500));
        let r = t.report("pc", true, t0 + 11 * S);
        assert_eq!(r.last_ms, Some(500));
        assert_eq!(r.avg_ms, Some(1000));
        assert_eq!(r.samples, 2);
        assert_eq!(r.last_trigger, Some(ReconnectTrigger::Push));
        assert_eq!(r.network_avg_ms, Some(1500));
        assert_eq!(r.push_avg_ms, Some(500));
        assert_eq!(r.wake_avg_ms, None);
        assert_eq!(r.up_for_s, Some(0));
        assert_eq!(r.reach, Some(DeviceReach::Connected));
    }

    #[test]
    fn a_stale_trigger_measures_nothing() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        t.trigger("pc", ReconnectTrigger::Wake, t0);
        assert_eq!(t.link_up("pc", t0 + PENDING_FOR + S), None);
        // A new trigger replaces a stale one.
        t.trigger("pc", ReconnectTrigger::Wake, t0);
        t.trigger("pc", ReconnectTrigger::Network, t0 + PENDING_FOR + S);
        assert_eq!(
            t.link_up("pc", t0 + PENDING_FOR + 2 * S),
            Some((ReconnectTrigger::Network, 1000))
        );
        t.trigger("pc", ReconnectTrigger::Network, t0);
        t.clear_pending("pc");
        assert_eq!(t.link_up("pc", t0 + S), None);
    }

    #[test]
    fn samples_are_capped() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        for i in 0..(SAMPLES as u64 + 5) {
            let at = t0 + Duration::from_secs(i * 10);
            t.trigger("pc", ReconnectTrigger::Network, at);
            t.link_up("pc", at + Duration::from_millis(i));
        }
        let r = t.report("pc", true, t0);
        assert_eq!(r.samples as usize, SAMPLES);
        assert_eq!(r.last_ms, Some(SAMPLES as u32 + 4));
    }

    #[test]
    fn drops_keep_their_reason_and_the_links_lifetime() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        t.link_up("pc", t0);
        t.closing("pc", "idle");
        assert_eq!(t.link_down("pc", t0 + 180 * S), "idle");
        t.link_up("pc", t0 + 200 * S);
        // Nobody said why: lost.
        assert_eq!(t.link_down("pc", t0 + 260 * S), "lost");
        t.link_up("pc", t0 + 300 * S);
        assert_eq!(t.link_down("pc", t0 + 302 * S), "lost");
        let r = t.report("pc", false, t0 + 312 * S);
        assert_eq!(r.lifetime_last_s, Some(2));
        assert_eq!(r.lifetime_avg_s, Some(80)); // (180 + 60 + 2) / 3
        assert_eq!(r.up_for_s, None);
        assert_eq!(r.last_drop.as_deref(), Some("lost"));
        assert_eq!(r.last_drop_ago_s, Some(10));
        assert_eq!(
            r.drops,
            vec![
                DropCount {
                    reason: "lost".into(),
                    count: 2
                },
                DropCount {
                    reason: "idle".into(),
                    count: 1
                },
            ]
        );
        // A close reason is used once.
        t.link_up("pc", t0 + 400 * S);
        t.closing("pc", "manual");
        t.link_up("pc", t0 + 401 * S);
        assert_eq!(t.link_down("pc", t0 + 402 * S), "lost");
    }

    #[test]
    fn only_links_lost_by_accident_recently_are_redialled() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        for (node, why) in [
            ("a", "network_change"),
            ("b", "timeout"),
            ("c", "idle"),
            ("d", "manual"),
            ("e", "peer_idle"),
            ("f", "lost"),
            ("g", "peer_quit"),
        ] {
            t.link_up(node, t0);
            t.closing(node, why);
            t.link_down(node, t0 + S);
        }
        t.link_up("live", t0);
        assert_eq!(t.wanted(t0 + 2 * S), vec!["a", "b", "f"]);
        assert!(t.wanted(t0 + S + WANT_FOR).is_empty());
        // Back up: not wanted.
        t.link_up("a", t0 + 3 * S);
        assert_eq!(t.wanted(t0 + 4 * S), vec!["b", "f"]);
    }

    #[test]
    fn unanswered_pushes_back_off_and_a_link_resets_it() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        assert_eq!(t.cooldown("phone", t0), None);
        assert_eq!(t.no_answer("phone", t0, 0.5), ASLEEP_BASE);
        assert_eq!(t.cooldown("phone", t0 + 10 * S), Some(20 * S));
        assert_eq!(t.cooldown("phone", t0 + ASLEEP_BASE), None);
        assert_eq!(t.no_answer("phone", t0, 0.5), 2 * ASLEEP_BASE);
        assert_eq!(t.no_answer("phone", t0, 0.5), 4 * ASLEEP_BASE);
        for _ in 0..10 {
            t.no_answer("phone", t0, 0.5);
        }
        assert_eq!(t.cooldown("phone", t0), Some(ASLEEP_CAP));
        assert_eq!(t.reach("phone", false, t0), DeviceReach::Asleep);
        t.link_up("phone", t0 + S);
        assert_eq!(t.cooldown("phone", t0 + S), None);
        assert_eq!(t.no_answer("phone", t0 + S, 0.5), ASLEEP_BASE);
    }

    #[test]
    fn reach_follows_the_newest_news() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        assert_eq!(t.reach("phone", false, t0), DeviceReach::Offline);
        assert_eq!(t.reach("phone", true, t0), DeviceReach::Connected);
        t.waking("phone", t0);
        assert_eq!(t.reach("phone", false, t0 + S), DeviceReach::Waking);
        // Waking only lasts so long.
        assert_eq!(
            t.reach("phone", false, t0 + WAKING_FOR),
            DeviceReach::Offline
        );
        t.waking("phone", t0 + 30 * S);
        t.no_answer("phone", t0 + 38 * S, 0.5);
        assert_eq!(t.reach("phone", false, t0 + 39 * S), DeviceReach::Asleep);
        // Pushed again: waking again.
        t.waking("phone", t0 + 100 * S);
        assert_eq!(t.reach("phone", false, t0 + 101 * S), DeviceReach::Waking);
        t.push_failed("phone", t0 + 102 * S);
        assert_eq!(
            t.reach("phone", false, t0 + 103 * S),
            DeviceReach::Unreachable
        );
        t.link_up("phone", t0 + 200 * S);
        assert_eq!(t.reach("phone", true, t0 + 201 * S), DeviceReach::Connected);
        // It closed the link itself to save battery: asleep.
        t.closing("phone", "peer_idle");
        t.link_down("phone", t0 + 400 * S);
        assert_eq!(t.reach("phone", false, t0 + 401 * S), DeviceReach::Asleep);
        // Its app quit: nothing known.
        t.link_up("phone", t0 + 500 * S);
        t.closing("phone", "peer_quit");
        t.link_down("phone", t0 + 501 * S);
        assert_eq!(t.reach("phone", false, t0 + 502 * S), DeviceReach::Offline);
    }

    #[test]
    fn the_report_of_a_device_never_seen_is_empty() {
        let t = Tracker::default();
        let r = t.report("x", false, Instant::now());
        assert_eq!(
            r,
            ReconnectInfo {
                reach: Some(DeviceReach::Offline),
                ..Default::default()
            }
        );
    }
}
