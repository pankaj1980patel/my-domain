//! Extra routes for big file transfers between devices that are near each
//! other but not on the same network (or only linked through the relay, a
//! punched path or WebRTC):
//!
//! - Wi-Fi Direct (`Method::WifiDirect`): phone to phone, and phone to a
//!   Windows PC;
//! - the phone's own hotspot (`Method::Hotspot`, Android's local-only
//!   hotspot): phone to a Mac, a Linux PC, or a Windows PC without Wi-Fi
//!   Direct. The computer leaves its Wi-Fi for the transfer and rejoins it
//!   after, so it asks its user first (`CoreEvent::WifiDirectRequest` "consent").
//!
//! The transfer itself doesn't change: it starts on the normal link, and
//! once the group (or hotspot) is up and a QUIC side lane runs over it,
//! `features::files` sends its messages on the lane instead (a chunk that
//! arrives out of order is answered with a gap, and the sender rewinds, as
//! on any link). Any failure or a setup slower than
//! `CoreConfig::wifi_direct_setup` (twice that for a hotspot) simply leaves
//! it on the normal link.
//!
//! Which devices can: the platform says (`Engine::set_wifi_direct_caps`)
//! whether it can host a Wi-Fi Direct group (`CAN_HOST`), join one
//! (`CAN_JOIN`), start a hotspot (`CAN_HOTSPOT`) or join a phone's
//! (`CAN_JOIN_HOTSPOT`): Android 10+ host and join, older Android only host,
//! Android 8+ hotspot; Windows hosts Wi-Fi Direct; computers join a
//! hotspot. With the user's switch on (`Engine::set_wifi_direct_enabled`),
//! the status and `addr_update` say `wifi_direct: <caps>`; only devices
//! that said it ever get these messages, so an older app never sees them.
//! Wi-Fi Direct is preferred when both ends can.
//!
//! The negotiation, over the existing (encrypted) link:
//!
//!   wifi_direct_ask   { from, sid, method, host, caps }  sender → receiver
//!   wifi_direct_ok    { from, sid }               receiver → sender (the sender hosts)
//!   wifi_direct_group { from, sid, ssid, passphrase, port, host_ip? }  host → joiner
//!   wifi_direct_no    { from, sid }               either way: not now
//!   wifi_direct_end   { from, sid }               either way: torn down
//!
//! The method and host are chosen the same way on both ends (`choose`).
//! The core asks the platform with `CoreEvent::WifiDirectRequest` (its
//! `method`, and `op`): "consent" (a hotspot's joiner asks its user first;
//! answer `Engine::wifi_direct_consent`), "host" (with a suggested name and
//! passphrase; answer `Engine::wifi_direct_hosted` with what it used),
//! "join" (the host's name and passphrase; answer
//! `Engine::wifi_direct_joined` with the host's address), "leave" when
//! done; either fails with `Engine::wifi_direct_failed`. A "leave" always
//! follows a "host" or "join", whatever happened, so a computer always
//! goes back to its own Wi-Fi. The group message goes reliably: a phone
//! whose Wi-Fi drops as its hotspot starts sends it once a link is back
//! (over mobile data and the relay).
//!
//! The joiner then dials the host's QUIC port at that address as a side
//! lane (`transport::quic::dial_lane`): the same handshake and device-key
//! check as a link, and the messages on it are encrypted the same way.
//! Only a device this one is negotiating with may open one. The name and
//! passphrase are never logged or kept.
//!
//! The group is torn down a little after the last transfer with that
//! device ends (`LINGER`), when the lane carried nothing for `IDLE`, when
//! the lane closes, or when the user turns the switch off.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::Plaintext;
use crate::platform::Platform;
use crate::transport::{self, TransportKind};

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag: what this device can do.
pub const FLAG: &str = "wifi_direct";
/// It can host a Wi-Fi Direct group (be the group owner).
pub const CAN_HOST: u64 = 1;
/// It can join a Wi-Fi Direct group another device hosts.
pub const CAN_JOIN: u64 = 2;
/// It can start a local hotspot (a phone).
pub const CAN_HOTSPOT: u64 = 4;
/// It can leave its Wi-Fi to join a phone's hotspot (a computer).
pub const CAN_JOIN_HOTSPOT: u64 = 8;
const ALL_CAPS: u64 = CAN_HOST | CAN_JOIN | CAN_HOTSPOT | CAN_JOIN_HOTSPOT;
/// The smallest transfer worth it (what's left to send), by default.
pub const MIN_SIZE: u64 = 20 * 1024 * 1024;
/// How long setting it up may take, by default.
pub const SETUP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a computer's user has to say yes to joining a hotspot.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Torn down once nothing went over the lane for this long.
pub const IDLE: Duration = Duration::from_secs(60);
/// Torn down this long after the last transfer with the device ended
/// (another file of the same batch may follow).
pub const LINGER: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_millis(250);

/// This end's part in the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Join,
}

/// Which route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    WifiDirect,
    Hotspot,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::WifiDirect => "wifi_direct",
            Method::Hotspot => "hotspot",
        }
    }

    fn parse(s: &str) -> Option<Method> {
        match s {
            "wifi_direct" => Some(Method::WifiDirect),
            "hotspot" => Some(Method::Hotspot),
            _ => None,
        }
    }
}

/// Where a session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for this device's user to agree to join a hotspot.
    Consent,
    /// Asked the other device; waiting for its answer.
    Asking,
    /// The platform is making the group.
    Hosting,
    /// Waiting for the group's name, or the platform is joining it.
    Joining,
    /// The group is up; the side lane is being dialed.
    Connecting,
    /// The side lane is up: transfers go over it.
    Active,
}

/// Who hosts a Wi-Fi Direct group, the same on both ends: a device that
/// can only host (or only join) gets that part; when both can do both, the
/// lower node id hosts. `None`: they can't make a group together.
pub fn choose_role(me: &str, my_caps: u64, peer: &str, peer_caps: u64) -> Option<Role> {
    pick(me, peer, (my_caps, peer_caps), CAN_HOST, CAN_JOIN)
}

fn pick(me: &str, peer: &str, (mine, theirs): (u64, u64), host: u64, join: u64) -> Option<Role> {
    let can = |caps: u64, bit: u64| caps & bit != 0;
    let i_host = can(mine, host) && can(theirs, join);
    let they_host = can(theirs, host) && can(mine, join);
    match (i_host, they_host) {
        (true, true) => Some(if me < peer { Role::Host } else { Role::Join }),
        (true, false) => Some(Role::Host),
        (false, true) => Some(Role::Join),
        (false, false) => None,
    }
}

/// The route and this end's part, the same on both ends: Wi-Fi Direct when
/// both can (it leaves everyone's Wi-Fi alone), else the phone's hotspot.
pub fn choose(me: &str, my_caps: u64, peer: &str, peer_caps: u64) -> Option<(Method, Role)> {
    if let Some(r) = choose_role(me, my_caps, peer, peer_caps) {
        return Some((Method::WifiDirect, r));
    }
    pick(
        me,
        peer,
        (my_caps, peer_caps),
        CAN_HOTSPOT,
        CAN_JOIN_HOTSPOT,
    )
    .map(|r| (Method::Hotspot, r))
}

/// Whether the current link is one these routes would beat: anything but
/// a direct link on the same local network (`on_lan`: tests take a local
/// link too).
pub fn link_worth_it(kind: TransportKind, addr: Option<SocketAddr>, on_lan: bool) -> bool {
    if on_lan {
        return true;
    }
    match kind {
        TransportKind::Quic => !super::diag::is_lan(addr),
        TransportKind::Punch
        | TransportKind::WebRtc
        | TransportKind::Relay
        | TransportKind::Ble => true,
    }
}

/// Everything the decision to try takes, apart from the link.
#[derive(Clone, Copy, Debug)]
pub struct Want {
    /// The user's switch.
    pub enabled: bool,
    pub my_caps: u64,
    pub peer_caps: u64,
    /// Bytes still to send.
    pub left: u64,
    pub min_size: u64,
    /// A session with the device is already going.
    pub busy: bool,
}

/// Try one of the routes for this transfer: which, and this end's part.
pub fn should_try(me: &str, peer: &str, w: Want) -> Option<(Method, Role)> {
    if !w.enabled || w.busy || w.left < w.min_size {
        return None;
    }
    choose(me, w.my_caps, peer, w.peer_caps)
}

/// A suggested group name: Wi-Fi Direct groups are called `DIRECT-xy-…`.
pub fn group_name(sid: &str) -> String {
    let tail: String = sid
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    format!("DIRECT-md-{tail}")
}

/// A random passphrase (WPA2 takes 8 to 63 characters).
fn passphrase() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..16].to_string()
}

/// How long a phase may last before the session is given up.
pub fn phase_limit(phase: Phase, method: Method, setup: Duration) -> Option<Duration> {
    match phase {
        Phase::Active => None,
        Phase::Consent => Some(CONSENT_TIMEOUT),
        _ if method == Method::Hotspot => Some(setup * 2),
        _ => Some(setup),
    }
}

struct Session {
    node: String,
    method: Method,
    role: Role,
    phase: Phase,
    /// This end asked (it sends the transfer).
    asker: bool,
    /// When the current stage began (setup timeouts count from here).
    started: Instant,
    /// When a transfer with the device was last going.
    busy_at: Instant,
    /// The platform was asked to host or join (so it's told to leave).
    platform_asked: bool,
    /// The host's QUIC port (joiner).
    port: u16,
}

/// What this feature keeps.
pub struct WifiDirectState {
    caps: Mutex<HashMap<String, u64>>,
    my_caps: AtomicU64,
    /// The user's switch (on unless turned off).
    off: AtomicBool,
    /// sid -> session.
    sessions: Mutex<HashMap<String, Session>>,
    /// node -> when something last went over its lane.
    lane_used: Mutex<HashMap<String, Instant>>,
}

impl Default for WifiDirectState {
    fn default() -> Self {
        WifiDirectState {
            caps: Mutex::default(),
            my_caps: AtomicU64::new(0),
            off: AtomicBool::new(false),
            sessions: Mutex::default(),
            lane_used: Mutex::default(),
        }
    }
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &WifiDirectState {
    &ctx.extras.wifi_direct
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// What this device can do, as it says it (0 with the switch off).
fn advertised<P: Platform>(ctx: &NetCtx<P>) -> u64 {
    if !ctx.config.wifi_direct || st(ctx).off.load(Ordering::Relaxed) {
        return 0;
    }
    st(ctx).my_caps.load(Ordering::Relaxed)
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) what it can do.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    let caps = advertised(ctx);
    if caps != 0 {
        status[FLAG] = caps.into();
    }
}

/// `node` said (`flag`: the `wifi_direct` value of its status or
/// `addr_update`) what it can do; nothing: an app from before, or off.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let caps = flag.as_u64().unwrap_or(0) & ALL_CAPS;
    st(ctx).caps.lock().insert(node.to_string(), caps);
}

/// What `node` said it can do (0: nothing, or an app from before).
pub fn peer_caps<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    if !ctx.config.wifi_direct {
        return 0;
    }
    st(ctx).caps.lock().get(node).copied().unwrap_or(0)
}

/// `node` takes the negotiation messages.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    peer_caps(ctx, node) != 0
}

/// The platform says what it can do (the `CAN_*` bits, 0: nothing).
pub fn set_caps<P: Platform>(ctx: &NetCtx<P>, caps: u64) {
    let caps = caps & ALL_CAPS;
    if st(ctx).my_caps.swap(caps, Ordering::Relaxed) != caps {
        tell_peers(ctx);
    }
}

/// The user's switch.
pub fn set_enabled<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    if st(ctx).off.swap(!on, Ordering::Relaxed) == !on {
        return;
    }
    if !on {
        let sids: Vec<String> = st(ctx).sessions.lock().keys().cloned().collect();
        for sid in sids {
            end(ctx, &sid, true);
        }
    }
    tell_peers(ctx);
}

pub fn enabled<P: Platform>(ctx: &NetCtx<P>) -> bool {
    !st(ctx).off.load(Ordering::Relaxed)
}

/// The linked devices hear the new flag.
fn tell_peers<P: Platform>(ctx: &NetCtx<P>) {
    let ctx = ctx.clone();
    std::thread::spawn(move || super::addr_update::announce(&ctx, None));
}

// --- state -----------------------------------------------------------------

/// How transfers with `node` go right now: "wifi_direct" or "hotspot";
/// none: over the link.
pub fn active_via<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<&'static str> {
    if !ctx.quic.has_lane(node) {
        return None;
    }
    st(ctx)
        .sessions
        .lock()
        .values()
        .find(|s| s.node == node && s.phase == Phase::Active)
        .map(|s| s.method.as_str())
}

/// Transfers to or from `node` go over a side route now.
pub fn active<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    active_via(ctx, node).is_some()
}

/// A side lane from `node` is welcome now (`transport::quic`).
pub fn lane_wanted<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    enabled(ctx)
        && st(ctx)
            .sessions
            .lock()
            .values()
            .any(|s| s.node == node && matches!(s.phase, Phase::Connecting | Phase::Active))
}

/// The side lane to `node` came up.
pub fn lane_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let now_active = {
        let mut ss = st(ctx).sessions.lock();
        let mut changed = None;
        for s in ss.values_mut().filter(|s| s.node == node) {
            if s.phase != Phase::Active {
                s.phase = Phase::Active;
                s.busy_at = Instant::now();
                changed = Some(s.method);
            }
        }
        changed
    };
    st(ctx)
        .lane_used
        .lock()
        .insert(node.to_string(), Instant::now());
    if let Some(method) = now_active {
        ctx.sink.emit(CoreEvent::WifiDirect {
            node_id: node.to_string(),
            active: true,
            via: method.as_str().into(),
        });
        super::links_map::changed(ctx);
    }
}

/// The side lane to `node` closed (the other end, or the group went).
pub fn lane_down<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let sids: Vec<String> = st(ctx)
        .sessions
        .lock()
        .iter()
        .filter(|(_, s)| s.node == node && s.phase == Phase::Active)
        .map(|(k, _)| k.clone())
        .collect();
    for sid in sids {
        end(ctx, &sid, true);
    }
}

/// Send `text` to `node` over its side lane, if it has one up. False:
/// it goes the normal way.
pub fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, text: &str) -> bool {
    if !ctx.quic.has_lane(node) {
        return false;
    }
    let pt = Plaintext {
        from: ctx.identity.lock().name.clone(),
        text: text.to_string(),
        ..Default::default()
    };
    let Ok(frame) = transport::seal(ctx, node, &pt) else {
        return false;
    };
    if !ctx.quic.lane_send(node, frame) {
        return false;
    }
    st(ctx)
        .lane_used
        .lock()
        .insert(node.to_string(), Instant::now());
    // The link stays in use too (it isn't closed as idle meanwhile).
    transport::note_use(&ctx.p2p_conns, node);
    true
}

// --- starting --------------------------------------------------------------

/// A transfer to `node` with `left` bytes to go just started (or resumed):
/// ask for a side route if it's worth it.
pub fn transfer_started<P: Platform>(ctx: &NetCtx<P>, node: &str, left: u64) {
    let link = ctx.p2p_conns.lock().get(node).map(|c| (c.kind, c.addr));
    let Some((kind, addr)) = link else { return };
    if !link_worth_it(kind, addr, ctx.config.wifi_direct_on_lan) {
        return;
    }
    let me = me(ctx);
    let my_caps = advertised(ctx);
    let busy = st(ctx).sessions.lock().values().any(|s| s.node == node);
    let want = Want {
        enabled: my_caps != 0,
        my_caps,
        peer_caps: peer_caps(ctx, node),
        left,
        min_size: ctx.config.wifi_direct_min,
        busy,
    };
    let Some((method, role)) = should_try(&me, node, want) else {
        return;
    };
    let sid = uuid::Uuid::new_v4().simple().to_string();
    // A computer about to leave its Wi-Fi for a hotspot asks its user first.
    let consent = method == Method::Hotspot && role == Role::Join;
    st(ctx).sessions.lock().insert(
        sid.clone(),
        Session {
            node: node.to_string(),
            method,
            role,
            phase: if consent {
                Phase::Consent
            } else {
                Phase::Asking
            },
            asker: true,
            started: Instant::now(),
            busy_at: Instant::now(),
            platform_asked: false,
            port: 0,
        },
    );
    if consent {
        request(ctx, &sid, node, method, "consent", None, None, None);
    } else {
        send_ask(ctx, &sid, node, method, role);
    }
    watch(ctx, sid);
}

fn send_ask<P: Platform>(ctx: &NetCtx<P>, sid: &str, node: &str, method: Method, role: Role) {
    let me = me(ctx);
    let host = if role == Role::Host {
        me.clone()
    } else {
        node.to_string()
    };
    send_msg(
        ctx,
        node,
        json!({
            "type": "wifi_direct_ask", "from": me, "sid": sid,
            "method": method.as_str(), "host": host, "caps": advertised(ctx),
        }),
    );
}

fn send_msg<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: Value) {
    if supports(ctx, node) {
        transport::send_control_to(ctx, node, &msg.to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn request<P: Platform>(
    ctx: &NetCtx<P>,
    sid: &str,
    node: &str,
    method: Method,
    op: &str,
    ssid: Option<String>,
    passphrase: Option<String>,
    host_ip: Option<String>,
) {
    ctx.sink.emit(CoreEvent::WifiDirectRequest {
        sid: sid.to_string(),
        node_id: node.to_string(),
        method: method.as_str().into(),
        op: op.into(),
        ssid,
        passphrase,
        host_ip,
    });
}

/// Ask the platform to host a group (or start the hotspot).
fn ask_host<P: Platform>(ctx: &NetCtx<P>, sid: &str, node: &str, method: Method) {
    let (ssid, pass) = match method {
        Method::WifiDirect => (Some(group_name(sid)), Some(passphrase())),
        // A hotspot names itself.
        Method::Hotspot => (None, None),
    };
    request(ctx, sid, node, method, "host", ssid, pass, None);
}

/// Keeps an eye on one session: gives up a setup that takes too long, and
/// tears the group down once the transfers are over or the lane idles.
fn watch<P: Platform>(ctx: &NetCtx<P>, sid: String) {
    let ctx = ctx.clone();
    std::thread::spawn(move || loop {
        if !ctx.shutdown.wait(CHECK_EVERY) {
            return;
        }
        let Some((node, method, phase, started, busy_at)) = st(&ctx)
            .sessions
            .lock()
            .get(&sid)
            .map(|s| (s.node.clone(), s.method, s.phase, s.started, s.busy_at))
        else {
            return;
        };
        if let Some(limit) = phase_limit(phase, method, ctx.config.wifi_direct_setup) {
            if started.elapsed() >= limit {
                end(&ctx, &sid, true);
                return;
            }
            continue;
        }
        if super::files::busy_with(&ctx, &node) {
            if let Some(s) = st(&ctx).sessions.lock().get_mut(&sid) {
                s.busy_at = Instant::now();
            }
        } else if busy_at.elapsed() >= LINGER {
            end(&ctx, &sid, true);
            return;
        }
        let idle = st(&ctx)
            .lane_used
            .lock()
            .get(&node)
            .is_none_or(|at| at.elapsed() >= IDLE);
        if idle || !ctx.quic.has_lane(&node) {
            end(&ctx, &sid, true);
            return;
        }
    });
}

/// End session `sid`: the lane closes, the platform leaves the group (a
/// computer goes back to its Wi-Fi), and (`tell`) the other device hears it.
pub fn end<P: Platform>(ctx: &NetCtx<P>, sid: &str, tell: bool) {
    let Some(s) = st(ctx).sessions.lock().remove(sid) else {
        return;
    };
    let others = st(ctx).sessions.lock().values().any(|o| o.node == s.node);
    let had_lane = ctx.quic.has_lane(&s.node);
    if !others {
        ctx.quic.close_lane(&s.node);
        st(ctx).lane_used.lock().remove(&s.node);
    }
    if s.platform_asked {
        request(ctx, sid, &s.node, s.method, "leave", None, None, None);
    }
    if tell {
        send_msg(
            ctx,
            &s.node,
            json!({"type": "wifi_direct_end", "from": me(ctx), "sid": sid}),
        );
    }
    if s.phase == Phase::Active || had_lane {
        ctx.sink.emit(CoreEvent::WifiDirect {
            node_id: s.node.clone(),
            active: false,
            via: s.method.as_str().into(),
        });
        super::links_map::changed(ctx);
    }
}

// --- the platform's answers --------------------------------------------------

/// The user said whether this computer may leave its Wi-Fi for the
/// phone's hotspot.
pub fn consent<P: Platform>(ctx: &NetCtx<P>, sid: &str, yes: bool) {
    if !yes {
        return end(ctx, sid, true);
    }
    let next = {
        let mut ss = st(ctx).sessions.lock();
        let Some(s) = ss.get_mut(sid) else { return };
        if s.phase != Phase::Consent {
            return;
        }
        s.started = Instant::now();
        s.phase = if s.asker {
            Phase::Asking
        } else {
            Phase::Joining
        };
        (s.node.clone(), s.method, s.role, s.asker)
    };
    let (node, method, role, asker) = next;
    if asker {
        send_ask(ctx, sid, &node, method, role);
    } else {
        send_msg(
            ctx,
            &node,
            json!({"type": "wifi_direct_ok", "from": me(ctx), "sid": sid}),
        );
    }
}

/// The platform made the group or started the hotspot (`ssid`,
/// `passphrase`: what it really used; `host_ip`: its own address there,
/// if it knows it).
pub fn hosted<P: Platform>(
    ctx: &NetCtx<P>,
    sid: &str,
    ssid: &str,
    passphrase: &str,
    host_ip: &str,
) {
    let node = {
        let mut ss = st(ctx).sessions.lock();
        let Some(s) = ss.get_mut(sid) else { return };
        if s.role != Role::Host || s.phase != Phase::Hosting {
            return;
        }
        s.phase = Phase::Connecting;
        s.started = Instant::now();
        s.node.clone()
    };
    let mut msg = json!({
        "type": "wifi_direct_group", "from": me(ctx), "sid": sid,
        "ssid": ssid, "passphrase": passphrase, "port": ctx.quic.port(),
    });
    if host_ip.parse::<IpAddr>().is_ok() {
        msg["host_ip"] = host_ip.into();
    }
    // Reliably: a phone's own Wi-Fi may drop as its hotspot starts; this
    // goes once a link is back (mobile data, the relay).
    if supports(ctx, &node) {
        crate::engine::send_reliably(ctx, &node, &msg.to_string());
    }
}

/// The platform joined the group or hotspot; the host is at `host_ip`.
pub fn joined<P: Platform>(ctx: &NetCtx<P>, sid: &str, host_ip: &str) {
    let Ok(ip) = host_ip.trim().parse::<IpAddr>() else {
        return end(ctx, sid, true);
    };
    let port = {
        let mut ss = st(ctx).sessions.lock();
        let Some(s) = ss.get_mut(sid) else { return };
        if s.role != Role::Join || s.phase != Phase::Joining || s.port == 0 {
            return;
        }
        s.phase = Phase::Connecting;
        s.port
    };
    let (ctx, sid) = (ctx.clone(), sid.to_string());
    std::thread::spawn(move || {
        let addr = SocketAddr::new(ip, port);
        if ctx
            .quic
            .dial_lane(ctx.clone(), addr, transport::quic::LANE_WIFI_DIRECT)
            .is_err()
        {
            end(&ctx, &sid, true);
        }
    });
}

/// The platform couldn't host or join (no permission, Wi-Fi off, the user
/// said no, ...): the transfer stays on the normal link.
pub fn failed<P: Platform>(ctx: &NetCtx<P>, sid: &str) {
    end(ctx, sid, true);
}

/// A session id as it may arrive: short, letters, digits and dashes.
fn ok_sid(sid: &str) -> bool {
    !sid.is_empty() && sid.len() <= 64 && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A group's name and passphrase as they may arrive.
fn ok_group(ssid: &str, passphrase: &str) -> bool {
    !ssid.is_empty()
        && ssid.len() <= 32
        && (8..=63).contains(&passphrase.len())
        && !ssid.chars().any(char::is_control)
        && !passphrase.chars().any(char::is_control)
}

pub struct WifiDirectHandler;

impl<P: Platform> ControlHandler<P> for WifiDirectHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "wifi_direct_ask",
            "wifi_direct_ok",
            "wifi_direct_group",
            "wifi_direct_no",
            "wifi_direct_end",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let sid = msg.str("sid");
        if from.is_empty() || !ok_sid(&sid) || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        let me = me(ctx);
        let no = |ctx: &NetCtx<P>| {
            send_msg(
                ctx,
                &from,
                json!({"type": "wifi_direct_no", "from": me, "sid": sid}),
            );
        };
        match msg.kind() {
            "wifi_direct_ask" => {
                remember(ctx, &from, &msg.json("caps"));
                let my_caps = advertised(ctx);
                let chosen = choose(&me, my_caps, &from, peer_caps(ctx, &from));
                let host = msg.str("host");
                let asked = Method::parse(&msg.str("method")).unwrap_or(Method::WifiDirect);
                let agreed = match chosen {
                    Some((m, Role::Host)) => m == asked && host == me,
                    Some((m, Role::Join)) => m == asked && host == from,
                    None => false,
                };
                let busy = {
                    let ss = st(ctx).sessions.lock();
                    ss.contains_key(&sid) || ss.values().any(|s| s.node == from)
                };
                let (Some((method, role)), true, false) = (chosen, agreed, busy) else {
                    return no(ctx);
                };
                let consent = method == Method::Hotspot && role == Role::Join;
                st(ctx).sessions.lock().insert(
                    sid.clone(),
                    Session {
                        node: from.clone(),
                        method,
                        role,
                        phase: match (role, consent) {
                            (Role::Host, _) => Phase::Hosting,
                            (Role::Join, true) => Phase::Consent,
                            (Role::Join, false) => Phase::Joining,
                        },
                        asker: false,
                        started: Instant::now(),
                        busy_at: Instant::now(),
                        platform_asked: role == Role::Host,
                        port: 0,
                    },
                );
                if role == Role::Host {
                    ask_host(ctx, &sid, &from, method);
                } else if consent {
                    request(ctx, &sid, &from, method, "consent", None, None, None);
                } else {
                    send_msg(
                        ctx,
                        &from,
                        json!({"type": "wifi_direct_ok", "from": me, "sid": sid}),
                    );
                }
                watch(ctx, sid);
            }
            "wifi_direct_ok" => {
                let go = {
                    let mut ss = st(ctx).sessions.lock();
                    match ss.get_mut(&sid) {
                        Some(s)
                            if s.node == from
                                && s.role == Role::Host
                                && s.phase == Phase::Asking =>
                        {
                            s.phase = Phase::Hosting;
                            s.platform_asked = true;
                            s.started = Instant::now();
                            Some(s.method)
                        }
                        _ => None,
                    }
                };
                if let Some(method) = go {
                    ask_host(ctx, &sid, &from, method);
                }
            }
            "wifi_direct_group" => {
                let (ssid, pass) = (msg.str("ssid"), msg.str("passphrase"));
                let host_ip = msg.opt("host_ip").filter(|s| s.parse::<IpAddr>().is_ok());
                let port = msg
                    .json("port")
                    .as_u64()
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(0);
                let join = {
                    let mut ss = st(ctx).sessions.lock();
                    match ss.get_mut(&sid) {
                        Some(s)
                            if s.node == from
                                && s.role == Role::Join
                                && matches!(s.phase, Phase::Asking | Phase::Joining)
                                && port != 0
                                && ok_group(&ssid, &pass) =>
                        {
                            s.phase = Phase::Joining;
                            s.port = port;
                            s.platform_asked = true;
                            s.started = Instant::now();
                            Some(Some(s.method))
                        }
                        Some(s) if s.node == from => Some(None),
                        _ => None,
                    }
                };
                match join {
                    Some(Some(method)) => request(
                        ctx,
                        &sid,
                        &from,
                        method,
                        "join",
                        Some(ssid),
                        Some(pass),
                        host_ip,
                    ),
                    Some(None) => end(ctx, &sid, true),
                    None => {}
                }
            }
            "wifi_direct_no" | "wifi_direct_end" => {
                let ours = st(ctx)
                    .sessions
                    .lock()
                    .get(&sid)
                    .is_some_and(|s| s.node == from);
                if ours {
                    end(ctx, &sid, false);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: u64 = CAN_HOST | CAN_JOIN;
    const PHONE: u64 = BOTH | CAN_HOTSPOT;
    const MAC: u64 = CAN_JOIN_HOTSPOT;
    const WINDOWS: u64 = CAN_HOST | CAN_JOIN_HOTSPOT;

    #[test]
    fn who_hosts() {
        // Two phones: the lower node id hosts, on both ends.
        assert_eq!(choose_role("a", BOTH, "b", BOTH), Some(Role::Host));
        assert_eq!(choose_role("b", BOTH, "a", BOTH), Some(Role::Join));
        // A Windows PC can only host: it does, whatever the ids.
        assert_eq!(choose_role("a", BOTH, "b", CAN_HOST), Some(Role::Join));
        assert_eq!(choose_role("b", CAN_HOST, "a", BOTH), Some(Role::Host));
        // An older phone can only host: the newer one joins.
        assert_eq!(choose_role("a", CAN_HOST, "b", BOTH), Some(Role::Host));
        // Two that can only host, or a device that can't at all: no.
        assert_eq!(choose_role("a", CAN_HOST, "b", CAN_HOST), None);
        assert_eq!(choose_role("a", BOTH, "b", 0), None);
        assert_eq!(choose_role("a", 0, "b", BOTH), None);
        assert_eq!(choose_role("a", CAN_JOIN, "b", CAN_JOIN), None);
    }

    #[test]
    fn wifi_direct_first_then_the_hotspot() {
        // Phone and Windows: Wi-Fi Direct (the PC hosts).
        assert_eq!(
            choose("p", PHONE, "w", WINDOWS),
            Some((Method::WifiDirect, Role::Join))
        );
        // Phone and Mac: the phone's hotspot, the Mac joins, both ways.
        assert_eq!(
            choose("p", PHONE, "m", MAC),
            Some((Method::Hotspot, Role::Host))
        );
        assert_eq!(
            choose("m", MAC, "p", PHONE),
            Some((Method::Hotspot, Role::Join))
        );
        // Two phones: Wi-Fi Direct, never a hotspot.
        assert_eq!(
            choose("a", PHONE, "b", PHONE).map(|c| c.0),
            Some(Method::WifiDirect)
        );
        // Two computers, or a phone whose hotspot is off: nothing.
        assert_eq!(choose("m", MAC, "w", WINDOWS), None);
        assert_eq!(choose("p", BOTH, "m", MAC), None);
    }

    #[test]
    fn only_big_transfers_with_a_capable_device() {
        let w = Want {
            enabled: true,
            my_caps: BOTH,
            peer_caps: CAN_HOST,
            left: MIN_SIZE,
            min_size: MIN_SIZE,
            busy: false,
        };
        assert_eq!(
            should_try("a", "b", w),
            Some((Method::WifiDirect, Role::Join))
        );
        let small = Want {
            left: MIN_SIZE - 1,
            ..w
        };
        assert_eq!(should_try("a", "b", small), None);
        let off = Want {
            enabled: false,
            ..w
        };
        assert_eq!(should_try("a", "b", off), None);
        let older = Want { peer_caps: 0, ..w };
        assert_eq!(should_try("a", "b", older), None);
        let busy = Want { busy: true, ..w };
        assert_eq!(should_try("a", "b", busy), None);
    }

    #[test]
    fn how_long_each_step_may_take() {
        let s = SETUP_TIMEOUT;
        assert_eq!(phase_limit(Phase::Active, Method::Hotspot, s), None);
        assert_eq!(
            phase_limit(Phase::Consent, Method::Hotspot, s),
            Some(CONSENT_TIMEOUT)
        );
        assert_eq!(phase_limit(Phase::Joining, Method::WifiDirect, s), Some(s));
        // Leaving and rejoining Wi-Fi takes longer.
        assert_eq!(phase_limit(Phase::Joining, Method::Hotspot, s), Some(s * 2));
    }

    #[test]
    fn only_when_the_link_is_not_the_local_network() {
        let lan: SocketAddr = "192.168.1.20:4000".parse().unwrap();
        let far: SocketAddr = "203.0.113.5:4000".parse().unwrap();
        assert!(!link_worth_it(TransportKind::Quic, Some(lan), false));
        assert!(link_worth_it(TransportKind::Quic, Some(far), false));
        assert!(link_worth_it(TransportKind::Punch, Some(far), false));
        assert!(link_worth_it(TransportKind::WebRtc, None, false));
        assert!(link_worth_it(TransportKind::Relay, None, false));
        assert!(link_worth_it(TransportKind::Ble, None, false));
        assert!(link_worth_it(TransportKind::Quic, Some(lan), true));
    }

    #[test]
    fn group_names_and_what_arrives() {
        let name = group_name("0f3a-77bc-9d00-1234");
        assert!(name.starts_with("DIRECT-md-"), "{name}");
        assert!(name.len() <= 32);
        let p = passphrase();
        assert!((8..=63).contains(&p.len()));
        assert!(ok_group(&name, &p));
        assert!(!ok_group("", &p));
        assert!(!ok_group(&name, "short"));
        assert!(!ok_group(&"x".repeat(33), &p));
        assert!(!ok_group(&name, "bad\npassphrase"));
        assert!(ok_sid("abc-123"));
        assert!(!ok_sid(""));
        assert!(!ok_sid("a/b"));
        assert!(!ok_sid(&"a".repeat(65)));
        assert_eq!(Method::parse("hotspot"), Some(Method::Hotspot));
        assert_eq!(Method::parse("x"), None);
    }
}
