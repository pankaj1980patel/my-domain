//! Nearby media mode: while audio, a screen or a phone's camera streams
//! between two of your devices, notice when the other one is near (on this
//! local network, or in Bluetooth range) and, if the stream goes the long
//! way round (the relay, a punched or mapped internet path, WebRTC), open a
//! direct QUIC link on the local network in the background and move the
//! stream onto it without a gap: the new link registers before the old one
//! closes (make before break), and every frame looks up the peer's link as
//! it goes. If that local link drops while the stream runs, the stream
//! falls back to the relay at once (no "disconnected" in between; the relay
//! socket is kept open meanwhile), or, with no relay, the connect ladder
//! looks for the best path.
//!
//! "Near" (`near_by`): the device's LAN address is on this device's subnet
//! (as `addr_update` and mDNS keep it), mDNS saw it here, or a Bluetooth
//! link to it is open. On a local link the receivers use a smaller jitter
//! floor (`audio_jitter::FLOOR_NEARBY_MS`) and the picture lanes keep only
//! the newest frame (`transport::quic`).
//!
//! Latency, end to end: each binary media record carries the time it was
//! sent (`media::Header::ts_ms`); the receiver measures the two clocks'
//! offset with a ping (`nearby_ping { from, t0, lat? }` →
//! `nearby_pong { from, t0, t1 }`) and reports the median of (arrival −
//! sent + offset). The latency before a move is kept, so the apps can show
//! "12 ms (was 140 ms)". The ping also tells the sender the latency
//! measured (`lat`), so both ends show it.
//!
//! Only with devices that say `nearby: 1` in their status and
//! `addr_update` (an older app gets no ping, and is never moved: its QUIC
//! link rules could close both links). The user's switch "Use nearby mode
//! for media" (`set_enabled`, on by default, in the settings backup) turns
//! it off.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::{Peer, Source};
use crate::platform::Platform;
use crate::transport::{self, media::now_ms, TransportKind};

use super::diag::is_local_addr;
use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag, and its version.
pub const FLAG: &str = "nearby";
const VERSION: u64 = 1;
/// The user's switch in the platform's store ("0": off).
pub const SETTING_KEY: &str = "nearby_media";
/// A stream counts as running while a frame went either way this recently.
pub const ACTIVE_FOR: Duration = Duration::from_secs(3);
/// How often a running stream's link is looked at.
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// After a local dial that failed.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// After a local link that was lost mid-stream.
const LOST_BACKOFF: Duration = Duration::from_secs(30);
/// How often the receiver pings for the clock offset.
const PING_EVERY: Duration = Duration::from_secs(2);
/// How often a running stream's state is reported.
const REPORT_EVERY: Duration = Duration::from_secs(2);
/// Arrival samples kept for the median.
const SAMPLES: usize = 50;

/// Where a stream to one device stands (`Engine::nearby_status`,
/// `CoreEvent::NearbyMedia`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct NearbyStatus {
    pub node_id: String,
    /// A stream runs with it now (either way).
    pub media: bool,
    /// It's near: "lan" (on this network) or "ble" (in Bluetooth range).
    pub near: Option<String>,
    /// The stream is on a local link now (the "Nearby · low latency" badge).
    pub low_latency: bool,
    /// How the link is carried (`links_map::how`), "" without one.
    pub path: String,
    /// End-to-end media latency now, in ms (none: not measured yet).
    pub latency_ms: Option<u32>,
    /// The latency on the link before the last move.
    pub before_ms: Option<u32>,
    /// The ping's round trip.
    pub rtt_ms: Option<u32>,
}

#[derive(Default)]
struct PeerMedia {
    media_at: Option<Instant>,
    checked_at: Option<Instant>,
    tried_at: Option<Instant>,
    dialing: bool,
    lost_at: Option<Instant>,
    /// Clock offset (theirs − ours, ms) from the ping with the lowest
    /// round trip since the last move, and that round trip.
    offset: Option<(i64, u32)>,
    pinged_at: Option<Instant>,
    /// Arrival − sent, ms, theirs and ours mixed: offset added later.
    samples: VecDeque<i64>,
    /// The latency the other end measured (it receives).
    remote_ms: Option<u32>,
    path: String,
    before_ms: Option<u32>,
    reported_at: Option<Instant>,
    last: Option<NearbyStatus>,
}

/// The state, in `Extras`.
#[derive(Default)]
pub struct NearbyState {
    caps: Mutex<HashMap<String, bool>>,
    setting: Mutex<Option<bool>>,
    peers: Mutex<HashMap<String, PeerMedia>>,
    ticking: AtomicBool,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &NearbyState {
    &ctx.extras.nearby
}

// --- pure parts -------------------------------------------------------------

/// Two IPv4 addresses on the same local network: both loopback, or both
/// private (or link-local) in the same /24.
pub fn same_subnet(mine: &str, theirs: &str) -> bool {
    let (Ok(a), Ok(b)) = (mine.parse::<Ipv4Addr>(), theirs.parse::<Ipv4Addr>()) else {
        return false;
    };
    if a.is_loopback() && b.is_loopback() {
        return true;
    }
    let local = |x: Ipv4Addr| x.is_private() || x.is_link_local();
    local(a) && local(b) && a.octets()[..3] == b.octets()[..3]
}

/// Why a device is near, if it is: "lan" (its address is on this subnet,
/// or mDNS saw it here with a local address), else "ble" (a Bluetooth link
/// is open to it).
pub fn near_by(my_ip: &str, peer_ip: &str, seen_here: bool, ble: bool) -> Option<&'static str> {
    let local = peer_ip
        .parse::<IpAddr>()
        .is_ok_and(|ip| is_local_addr(ip) && !ip.is_unspecified());
    if same_subnet(my_ip, peer_ip) || (seen_here && local) {
        Some("lan")
    } else if ble {
        Some("ble")
    } else {
        None
    }
}

/// The local addresses to dial `peer` at: its LAN IPv4 (and a unique-local
/// IPv6), with its QUIC port.
pub fn lan_targets(peer: &Peer) -> Vec<SocketAddr> {
    if peer.quic_port == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(v4) = peer.ip.parse::<Ipv4Addr>() {
        if !v4.is_unspecified() && is_local_addr(IpAddr::V4(v4)) {
            out.push(SocketAddr::new(v4.into(), peer.quic_port));
        }
    }
    if let Some(v6) = peer
        .ipv6
        .as_deref()
        .and_then(|s| s.parse::<std::net::Ipv6Addr>().ok())
    {
        // Unique-local only: a link-local one needs its interface.
        if (v6.segments()[0] & 0xfe00) == 0xfc00 {
            out.push(SocketAddr::new(v6.into(), peer.quic_port));
        }
    }
    out
}

/// A link of `kind` to `addr` is a local, low-latency one.
pub fn is_local_link(kind: TransportKind, addr: Option<SocketAddr>) -> bool {
    matches!(kind, TransportKind::Quic | TransportKind::Punch) && super::diag::is_lan(addr)
}

/// The two clocks' offset (theirs − ours) and the round trip, from a ping
/// sent at `t0`, answered at `t1` (their clock), back at `t3`.
pub fn clock_offset(t0: u64, t1: u64, t3: u64) -> (i64, u32) {
    let rtt = t3.saturating_sub(t0);
    let mid = t0 as i64 + (rtt / 2) as i64;
    (t1 as i64 - mid, u32::try_from(rtt).unwrap_or(u32::MAX))
}

/// End-to-end latency from arrival samples (arrival − sent, mixed clocks)
/// and the clock offset; without an offset, half the round trip.
pub fn latency_from(samples: &[i64], offset: Option<i64>, rtt_ms: Option<u32>) -> Option<u32> {
    match offset {
        Some(off) if !samples.is_empty() => {
            let mut s = samples.to_vec();
            s.sort_unstable();
            let m = s[s.len() / 2] + off;
            Some(u32::try_from(m.max(0)).unwrap_or(u32::MAX))
        }
        _ => rtt_ms.map(|r| r / 2),
    }
}

// --- capability and the switch ----------------------------------------------

/// Say in this device's status (and `addr_update`) that it knows nearby
/// mode.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.nearby {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`) whether it knows nearby mode.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    st(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` knows nearby mode (and this build does).
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.nearby && st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

/// The user's switch "Use nearby mode for media" (on unless turned off).
pub fn enabled<P: Platform>(ctx: &NetCtx<P>) -> bool {
    let mut s = st(ctx).setting.lock();
    *s.get_or_insert_with(|| ctx.platform.kv_get(SETTING_KEY).as_deref() != Some("0"))
}

pub fn set_enabled<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    *st(ctx).setting.lock() = Some(on);
    ctx.platform.kv_set(SETTING_KEY, if on { "1" } else { "0" });
}

// --- the stream -------------------------------------------------------------

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Whether `node` is near now, and why.
pub fn near<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<&'static str> {
    let peer = ctx.peers.lock().get(node).cloned()?;
    let mine = ctx.identity.lock().ip.clone();
    let ble = ctx.p2p_conns.lock().get(node).map(|c| c.kind) == Some(TransportKind::Ble)
        || crate::engine::ble_linked(ctx, node);
    near_by(&mine, &peer.ip, peer.source == Source::Lan, ble)
}

/// The link to `node` now: (kind, address).
fn link<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<(TransportKind, Option<SocketAddr>)> {
    ctx.p2p_conns.lock().get(node).map(|c| (c.kind, c.addr))
}

/// The stream to `node` is on a local link now.
pub fn low_latency<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    link(ctx, node).is_some_and(|(k, a)| is_local_link(k, a))
}

/// A media frame went to `node`.
pub(crate) fn sent<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    touch(ctx, node, None);
}

/// A media frame came from `node`, sent at `ts_ms` (their clock; 0: not
/// said).
pub(crate) fn received<P: Platform>(ctx: &NetCtx<P>, node: &str, ts_ms: u64) {
    touch(ctx, node, (ts_ms > 0).then_some(ts_ms));
}

fn touch<P: Platform>(ctx: &NetCtx<P>, node: &str, ts_ms: Option<u64>) {
    if !ctx.config.nearby {
        return;
    }
    let now = Instant::now();
    let (check, ping) = {
        let mut peers = st(ctx).peers.lock();
        let p = peers.entry(node.to_string()).or_default();
        p.media_at = Some(now);
        if let Some(ts) = ts_ms {
            p.samples.push_back(now_ms() as i64 - ts as i64);
            while p.samples.len() > SAMPLES {
                p.samples.pop_front();
            }
        }
        let check = p
            .checked_at
            .is_none_or(|t| now.duration_since(t) >= CHECK_EVERY);
        if check {
            p.checked_at = Some(now);
        }
        let ping = ts_ms.is_some()
            && p.pinged_at
                .is_none_or(|t| now.duration_since(t) >= PING_EVERY);
        if ping {
            p.pinged_at = Some(now);
        }
        (check, ping)
    };
    if ping && supports(ctx, node) {
        let lat = status(ctx, node).latency_ms;
        let mut m = json!({ "type": "nearby_ping", "from": me(ctx), "t0": now_ms() });
        if let Some(l) = lat {
            m["lat"] = l.into();
        }
        transport::send_control_to(ctx, node, &m.to_string());
    }
    if check {
        consider(ctx, node);
        ensure_ticker(ctx);
    }
}

/// Look at a running stream's link: move it to a local one if it can,
/// and report.
fn consider<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    note_path(ctx, node);
    report(ctx, node, false);
    if !enabled(ctx) || !ctx.config.rungs.quic || !supports(ctx, node) {
        return;
    }
    let Some((kind, addr)) = link(ctx, node) else {
        return;
    };
    if is_local_link(kind, addr) || near(ctx, node).is_none() {
        return;
    }
    let Some(peer) = ctx.peers.lock().get(node).cloned() else {
        return;
    };
    let targets = lan_targets(&peer);
    if targets.is_empty() {
        return;
    }
    {
        let now = Instant::now();
        let mut peers = st(ctx).peers.lock();
        let p = peers.entry(node.to_string()).or_default();
        if p.dialing
            || p.tried_at
                .is_some_and(|t| now.duration_since(t) < RETRY_AFTER)
            || p.lost_at
                .is_some_and(|t| now.duration_since(t) < LOST_BACKOFF)
        {
            return;
        }
        p.dialing = true;
        p.tried_at = Some(now);
    }
    super::diag::note(ctx, node, "nearby_try", None);
    let (ctx, node) = (ctx.clone(), node.to_string());
    std::thread::spawn(move || {
        let ok = ctx.quic.connect_any(ctx.clone(), &targets).is_ok();
        if ok {
            // Registered a moment after the handshake.
            for _ in 0..20 {
                if low_latency(&ctx, &node) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if let Some(p) = st(&ctx).peers.lock().get_mut(&node) {
            p.dialing = false;
        }
        let up = low_latency(&ctx, &node);
        super::diag::note(
            &ctx,
            &node,
            if up { "nearby_up" } else { "nearby_failed" },
            None,
        );
        note_path(&ctx, &node);
        report(&ctx, &node, true);
    });
}

/// The link changed how it's carried: keep the latency it had, and measure
/// the new one afresh.
fn note_path<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let how = link(ctx, node)
        .map(|(k, a)| super::links_map::how(k, super::diag::is_lan(a)).to_string())
        .unwrap_or_default();
    let before = {
        let peers = st(ctx).peers.lock();
        match peers.get(node) {
            Some(p) if p.path != how => Some(p.path.is_empty()),
            _ => None,
        }
    };
    let Some(first) = before else {
        return;
    };
    let latency = (!first).then(|| status(ctx, node).latency_ms).flatten();
    let mut peers = st(ctx).peers.lock();
    if let Some(p) = peers.get_mut(node) {
        p.path = how;
        if !first {
            p.before_ms = latency.or(p.before_ms);
            p.samples.clear();
            p.offset = None;
            p.remote_ms = None;
            p.pinged_at = None;
        }
    }
}

/// Where the stream to `node` stands.
pub fn status<P: Platform>(ctx: &NetCtx<P>, node: &str) -> NearbyStatus {
    let rtt_quic = ctx
        .quic
        .path_stats(node)
        .map(|(r, _, _)| u32::try_from(r.as_millis()).unwrap_or(u32::MAX));
    let l = link(ctx, node);
    let path = l
        .map(|(k, a)| super::links_map::how(k, super::diag::is_lan(a)).to_string())
        .unwrap_or_default();
    let low = l.is_some_and(|(k, a)| is_local_link(k, a));
    let near = near(ctx, node).map(str::to_string);
    let peers = st(ctx).peers.lock();
    let p = peers.get(node);
    let media = p
        .and_then(|p| p.media_at)
        .is_some_and(|t| t.elapsed() < ACTIVE_FOR);
    let (latency_ms, before_ms, rtt_ms) = match p {
        Some(p) => {
            let samples: Vec<i64> = p.samples.iter().copied().collect();
            let rtt = p.offset.map(|(_, r)| r).or(rtt_quic);
            let measured = if samples.is_empty() {
                p.remote_ms
            } else {
                latency_from(&samples, p.offset.map(|(o, _)| o), rtt)
            };
            (measured, p.before_ms, rtt)
        }
        None => (None, None, rtt_quic),
    };
    NearbyStatus {
        node_id: node.to_string(),
        media,
        near,
        low_latency: low,
        path,
        latency_ms,
        before_ms,
        rtt_ms,
    }
}

/// Tell the apps how the stream to `node` stands: when it changed (not
/// counting the latency) or every `REPORT_EVERY` while it runs.
fn report<P: Platform>(ctx: &NetCtx<P>, node: &str, force: bool) {
    let s = status(ctx, node);
    let send = {
        let mut peers = st(ctx).peers.lock();
        let Some(p) = peers.get_mut(node) else {
            return;
        };
        let shape = |x: &NearbyStatus| (x.media, x.near.clone(), x.low_latency, x.path.clone());
        let changed = p.last.as_ref().is_none_or(|l| shape(l) != shape(&s));
        let due = s.media && p.reported_at.is_none_or(|t| t.elapsed() >= REPORT_EVERY);
        let go = force || changed || (due && p.last.as_ref() != Some(&s));
        if go {
            p.reported_at = Some(Instant::now());
            p.last = Some(s.clone());
        }
        (go, changed)
    };
    if send.1 {
        // The map shows the badge too.
        super::links_map::changed(ctx);
    }
    if send.0 {
        ctx.sink.emit(CoreEvent::NearbyMedia(s));
    }
}

/// While any stream runs: look at the links once a second, and say when a
/// stream stopped.
fn ensure_ticker<P: Platform>(ctx: &NetCtx<P>) {
    if st(ctx).ticking.swap(true, Ordering::AcqRel) {
        return;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        while ctx.shutdown.wait(CHECK_EVERY) {
            let nodes: Vec<(String, bool)> = st(&ctx)
                .peers
                .lock()
                .iter()
                .filter(|(_, p)| p.last.as_ref().is_some_and(|l| l.media))
                .map(|(n, p)| {
                    (
                        n.clone(),
                        p.media_at.is_some_and(|t| t.elapsed() < ACTIVE_FOR),
                    )
                })
                .collect();
            if nodes.is_empty() {
                break;
            }
            for (n, running) in nodes {
                if running {
                    consider(&ctx, &n);
                } else {
                    report(&ctx, &n, false);
                }
            }
        }
        st(&ctx).ticking.store(false, Ordering::Release);
    });
}

/// A stream runs with `node` now.
pub fn media_running<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    st(ctx)
        .peers
        .lock()
        .get(node)
        .and_then(|p| p.media_at)
        .is_some_and(|t| t.elapsed() < ACTIVE_FOR)
}

/// `node`'s local QUIC link (to `remote`) just closed. Mid-stream, fall
/// back to the relay at once (route it, greet it, relabel the link: no
/// `PeerDisconnected`) and let the ladder look for the best path. Returns
/// whether it did (else the caller drops the link as usual).
pub(crate) fn link_lost<P: Platform>(ctx: &NetCtx<P>, node: &str, remote: SocketAddr) -> bool {
    if !ctx.config.nearby
        || !is_local_addr(remote.ip())
        || !media_running(ctx, node)
        || ctx.shutdown.load(Ordering::Relaxed)
    {
        return false;
    }
    if let Some(p) = st(ctx).peers.lock().get_mut(node) {
        p.lost_at = Some(Instant::now());
    }
    super::diag::note(ctx, node, "nearby_lost", None);
    let relayed = transport::relay::is_open(ctx) && transport::relay::add_relay_link(ctx, node);
    if relayed {
        transport::relay::send_hello(ctx, node);
        transport::relabel(ctx, node, TransportKind::Relay, None);
    } else {
        // No relay to fall back on: find the best path there is.
        crate::engine::start_ladder(ctx, node);
    }
    note_path(ctx, node);
    report(ctx, node, true);
    relayed
}

// --- the clock ping -----------------------------------------------------------

pub struct NearbyHandler;

impl<P: Platform> ControlHandler<P> for NearbyHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["nearby_ping", "nearby_pong"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        // It sends these, so it knows them.
        st(ctx).caps.lock().insert(from.clone(), true);
        let Some(t0) = msg.json("t0").as_u64() else {
            return;
        };
        match msg.kind() {
            "nearby_ping" => {
                let lat = msg
                    .json("lat")
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .filter(|v| *v < 60_000);
                if let Some(l) = lat {
                    st(ctx)
                        .peers
                        .lock()
                        .entry(from.clone())
                        .or_default()
                        .remote_ms = Some(l);
                }
                let pong =
                    json!({ "type": "nearby_pong", "from": me(ctx), "t0": t0, "t1": now_ms() });
                transport::send_control_to(ctx, &from, &pong.to_string());
            }
            "nearby_pong" => {
                let Some(t1) = msg.json("t1").as_u64() else {
                    return;
                };
                let t3 = now_ms();
                if t0 > t3 || t3 - t0 > 30_000 {
                    return;
                }
                let (off, rtt) = clock_offset(t0, t1, t3);
                let mut peers = st(ctx).peers.lock();
                let p = peers.entry(from).or_default();
                if p.offset.is_none_or(|(_, best)| rtt <= best) {
                    p.offset = Some((off, rtt));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_subnet_means_the_same_local_network() {
        assert!(same_subnet("192.168.1.2", "192.168.1.77"));
        assert!(same_subnet("127.0.0.1", "127.0.0.1"));
        assert!(same_subnet("10.0.0.2", "10.0.0.9"));
        assert!(!same_subnet("192.168.1.2", "192.168.2.2"));
        assert!(!same_subnet("8.8.8.1", "8.8.8.8"), "public");
        assert!(!same_subnet("", "192.168.1.2"));
        assert!(!same_subnet("192.168.1.2", "fe80::1"));
    }

    #[test]
    fn near_by_the_network_or_bluetooth() {
        assert_eq!(
            near_by("192.168.1.2", "192.168.1.9", false, false),
            Some("lan")
        );
        // mDNS saw it here with a local address on another /24.
        assert_eq!(near_by("10.0.1.2", "10.0.2.9", true, false), Some("lan"));
        assert_eq!(near_by("10.0.1.2", "8.8.8.8", true, false), None);
        assert_eq!(near_by("192.168.1.2", "10.0.0.9", false, true), Some("ble"));
        assert_eq!(near_by("192.168.1.2", "10.0.0.9", false, false), None);
        assert_eq!(near_by("192.168.1.2", "0.0.0.0", true, false), None);
    }

    #[test]
    fn only_local_addresses_are_dialled() {
        let mut p = Peer {
            node_id: "n".into(),
            name: "n".into(),
            ip: "192.168.1.9".into(),
            tcp_port: 0,
            udp_port: 0,
            ws_port: 0,
            quic_port: 4433,
            source: Source::Manual,
            ws_open: false,
            inbound_blocked: false,
            reflexive_ip: None,
            reflexive_udp_port: None,
            nat: Default::default(),
            ipv6: Some("fd00::9".into()),
            mapped_quic: None,
        };
        assert_eq!(
            lan_targets(&p),
            vec![
                "192.168.1.9:4433".parse::<SocketAddr>().unwrap(),
                "[fd00::9]:4433".parse().unwrap()
            ]
        );
        p.ip = "8.8.8.8".into();
        p.ipv6 = Some("2001:db8::1".into());
        assert!(lan_targets(&p).is_empty());
        p.ip = "192.168.1.9".into();
        p.quic_port = 0;
        assert!(lan_targets(&p).is_empty());
    }

    #[test]
    fn local_links_are_quic_or_punched_to_a_local_address() {
        let lan = Some("192.168.1.9:1".parse().unwrap());
        let public = Some("8.8.8.8:1".parse().unwrap());
        assert!(is_local_link(TransportKind::Quic, lan));
        assert!(is_local_link(TransportKind::Punch, lan));
        assert!(!is_local_link(TransportKind::Quic, public));
        assert!(!is_local_link(TransportKind::Relay, None));
        assert!(!is_local_link(TransportKind::WebRtc, lan));
    }

    #[test]
    fn the_clock_offset_and_latency() {
        // Their clock 1000 ms ahead; 40 ms round trip.
        let (off, rtt) = clock_offset(10_000, 11_020, 10_040);
        assert_eq!((off, rtt), (1000, 40));
        // Frames sent at their time, arriving 25 ms later at ours: raw
        // samples are 25 − 1000.
        let s = vec![-975, -976, -974, -900, -975];
        assert_eq!(latency_from(&s, Some(off), Some(rtt)), Some(25));
        assert_eq!(latency_from(&s, None, Some(40)), Some(20));
        assert_eq!(latency_from(&[], Some(off), None), None);
        assert_eq!(latency_from(&[-2000], Some(off), None), Some(0));
    }
}
