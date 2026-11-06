//! "How your devices are connected": the map both apps draw. This device
//! sits in the middle, each of your devices around it, with a line per
//! link saying how it's connected (on the local network, directly over
//! the internet, through the relay server, over Bluetooth) or that it
//! isn't (and when it was last seen).
//!
//! The links between your OTHER devices (the phone and the laptop, say)
//! are only known to them, so each device tells the others its own links:
//!
//!   links_map { from, links: [ {node, how, rtt_ms?} ] }
//!
//! `how` is the one word the connection check uses (`Diagnosis::verdict`
//! for a live link): "lan", "direct", "punched", "webrtc", "relay", "ble".
//! The list goes over live links only (it never connects or wakes a
//! device), to every linked device when one of this device's links comes
//! up, goes down or changes how it's carried (a little after, so a burst of
//! changes is one message), and to a device as soon as it says it
//! understands it. A list only counts while its sender is linked here:
//! from a device that isn't, it would be stale.
//!
//! Only devices that said `links_map: 1` (in their status, or in the
//! `addr_update` every link starts with) are ever sent it: an older app
//! would show it as a chat message. Sending one says the same. What
//! arrives is checked: from one of your devices only, at most
//! `MAX_LINKS` entries, known words only; links to devices this one
//! doesn't know are left out.
//!
//! Why a device isn't linked (or only through the relay or Bluetooth): the
//! connect ladder notes, per path, the last attempt and what stopped it
//! (`tried`): the local network ("lan"), directly over the internet
//! ("internet"), the relay server ("relay") and Bluetooth ("ble", as the
//! platform says). A link that comes up over a path clears it. The words
//! are short codes the apps put into plain words with a hint each (see
//! `LAN_REASONS` and the others). A message from a device that couldn't
//! be read marks it "bad_key" (a different device key) for a while; a
//! device whose security key changed (`device_key`), refused until the
//! user trusts it, is marked "key_changed".
//!
//! When a link goes down, the time is kept (`links_seen:<node>` in the
//! platform's store) for "last seen" on the map.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::Source;
use crate::platform::Platform;
use crate::transport::{self, now_secs, TransportKind};

use super::diag::{self, Diagnosis};
use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device shares its link
/// list, and its version.
pub const FLAG: &str = "links_map";
const VERSION: u64 = 1;

/// The most links one list may hold (more are dropped).
pub const MAX_LINKS: usize = 64;
/// The longest node id taken from a list.
const MAX_ID: usize = 128;
/// How long after a link change the list goes (changes in that time go
/// together).
const SETTLE: Duration = Duration::from_millis(300);
const SEEN_PREFIX: &str = "links_seen:";

/// The ways a live link can be carried, in the words of the connection
/// check.
pub const HOWS: [&str; 6] = ["lan", "direct", "punched", "webrtc", "relay", "ble"];

/// The paths the ladder tries, in the order the map lists them.
pub const PATHS: [&str; 4] = ["lan", "internet", "relay", "ble"];
/// Why the local network didn't work: no address known for the device
/// ("no_address"), its app doesn't speak QUIC ("old_app"), it isn't on
/// this network and mDNS saw nothing here ("not_seen"), it's on another
/// network ("other_network"), or it's on this network and didn't answer
/// ("blocked": a firewall, the router's client isolation, or asleep).
pub const LAN_REASONS: [&str; 5] = [
    "no_address",
    "old_app",
    "not_seen",
    "other_network",
    "blocked",
];
/// Why the direct internet path didn't work (see `internet_reason`).
pub const INTERNET_REASONS: [&str; 6] = [
    "push_failed",
    "no_answer",
    "symmetric_nat",
    "punch_timeout",
    "no_port_mapping",
    "timed_out",
];
/// Why the relay didn't work: not signed in to a server, the server
/// couldn't be reached, it refused this device's sign-in, it's full, or
/// the device didn't answer through it (offline or asleep).
pub const RELAY_REASONS: [&str; 5] = [
    "signed_out",
    "unreachable",
    "auth_failed",
    "relay_full",
    "peer_offline",
];
/// Why Bluetooth didn't work (as the platform said).
pub const BLE_REASONS: [&str; 4] = ["off", "not_paired", "out_of_range", "unsupported"];
/// What can be wrong with a device as a whole (`MapDevice::problem`): its
/// security key changed and it's refused until the user trusts the new
/// one, or what it sent couldn't be read.
pub const DEVICE_PROBLEMS: [&str; 2] = ["key_changed", "bad_key"];
/// How long a message that couldn't be read keeps "bad_key" on the map.
const BAD_KEY_FOR: u64 = 15 * 60;

/// The last attempt over one path, and what stopped it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PathTry {
    /// "lan", "internet", "relay" or "ble".
    pub path: String,
    /// A code from that path's reasons.
    pub reason: String,
    /// How long ago, in seconds.
    pub ago_s: u32,
}

/// How this device is linked to one of your devices.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MapLink {
    /// "lan", "direct", "punched", "webrtc", "relay" or "ble".
    pub how: String,
    pub transport: TransportKind,
    /// The other end's address, when the link has one.
    pub remote: Option<String>,
    /// What that address is: "local" (on this network), "ipv6" (a global
    /// IPv6 address, no NAT), "mapped" (a port its router forwards),
    /// "public" (its public IPv4 address); none without an address.
    pub addr_kind: Option<String>,
    /// QUIC's measured round trip and loss (QUIC links only).
    pub rtt_ms: Option<u32>,
    pub loss_pct: Option<f32>,
    /// Seconds since the link came up (none: not known).
    pub since_s: Option<u32>,
    /// Seconds since anything was heard over it.
    pub heard_ago_s: u32,
}

/// One of your devices on the map.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MapDevice {
    pub node_id: String,
    pub name: String,
    /// Where this device heard of it.
    pub source: Source,
    /// The live link (none: not connected).
    pub link: Option<MapLink>,
    /// Not connected: "unreachable" (tried, no link) or "not_connected"
    /// (not tried); connected: the link's `how`.
    pub state: String,
    /// Why it's unreachable (the connection check's reasons).
    pub reason: Option<String>,
    /// Seconds since its link went down (none: connected now, or not seen
    /// since this device kept count).
    pub last_seen_ago_s: Option<u32>,
    /// It shares its own links (`links_map: 1`).
    pub shares_links: bool,
    /// What stopped each path the last time it was tried, in `PATHS`
    /// order (a path that worked since, or carries the link, isn't listed).
    pub tries: Vec<PathTry>,
    /// Something wrong with the device as a whole: "key_changed" (its
    /// security key changed: direct links are refused until the user
    /// trusts the new one), "bad_key" (what it sent couldn't be read: a
    /// different encryption key).
    pub problem: Option<String>,
}

/// A link between two of your other devices, as one of them said.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct OtherLink {
    /// The two devices, `a` < `b`.
    pub a: String,
    pub b: String,
    pub how: String,
    pub rtt_ms: Option<u32>,
    /// The device that said so, and how long ago.
    pub reported_by: String,
    pub reported_ago_s: u32,
}

/// Everything the map shows (`Engine::links_map`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LinksMap {
    /// This device.
    pub node_id: String,
    pub name: String,
    /// Your other devices, by name.
    pub devices: Vec<MapDevice>,
    /// Links between them that they reported.
    pub others: Vec<OtherLink>,
}

/// One entry of a device's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Reported {
    pub node: String,
    pub how: String,
    pub rtt_ms: Option<u32>,
}

struct Report {
    at: u64,
    links: Vec<Reported>,
}

/// path -> (what stopped it, when).
type PathTries = HashMap<String, (String, u64)>;

/// Everything this feature keeps.
#[derive(Default)]
pub struct LinksMapState {
    caps: Mutex<HashMap<String, bool>>,
    reports: Mutex<HashMap<String, Report>>,
    seen: Mutex<HashMap<String, Option<u64>>>,
    /// node -> path -> (reason, when).
    tries: Mutex<HashMap<String, PathTries>>,
    /// Bluetooth as the platform last said (reason, when); none: fine.
    ble: Mutex<Option<(String, u64)>>,
    /// node -> when a message from it last couldn't be read.
    bad_key: Mutex<HashMap<String, u64>>,
    /// A list is about to go (see `SETTLE`).
    pending: AtomicBool,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &LinksMapState {
    &ctx.extras.links_map
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it shares its links.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.links_map {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `links_map` value of its status or
/// `addr_update`) whether it understands the list. Newly understood, and
/// linked: it gets this device's list now.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    let was = st(ctx).caps.lock().insert(node.to_string(), on);
    if on && was != Some(true) {
        send_to(ctx, node);
    }
}

/// `node` said it shares its links (and takes this device's).
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.links_map && st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

// --- this device's links ---------------------------------------------------

/// How a live link is carried, in one word (see `HOWS`).
pub fn how(kind: TransportKind, lan: bool) -> &'static str {
    match kind {
        TransportKind::Ble => "ble",
        TransportKind::Relay => "relay",
        TransportKind::WebRtc => "webrtc",
        TransportKind::Punch => "punched",
        TransportKind::Quic if lan => "lan",
        TransportKind::Quic => "direct",
    }
}

/// What kind of address `remote` is (see `MapLink::addr_kind`); `mapped`:
/// the port the other device's router forwards, if any.
pub fn addr_kind(remote: Option<std::net::SocketAddr>, mapped: Option<&str>) -> Option<String> {
    let a = remote?;
    let k = if diag::is_local_addr(a.ip()) {
        "local"
    } else if mapped.and_then(|m| m.parse::<std::net::SocketAddr>().ok()) == Some(a) {
        "mapped"
    } else if a.is_ipv6() && a.ip().to_canonical().is_ipv6() {
        "ipv6"
    } else {
        "public"
    };
    Some(k.to_string())
}

fn rtt_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> (Option<u32>, Option<f32>) {
    let measured = ctx.quic.path_stats(node);
    (
        measured.map(|(rtt, _, _)| u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX)),
        measured.and_then(|(_, lost, sent)| transport::quality::loss_pct(lost, sent)),
    )
}

/// This device's live links, as it tells the others.
pub fn my_links<P: Platform>(ctx: &NetCtx<P>) -> Vec<Reported> {
    let live: Vec<(String, TransportKind, Option<std::net::SocketAddr>)> =
        transport::live_peers(ctx);
    let mut out: Vec<Reported> = live
        .into_iter()
        .filter(|(node, _, _)| !node.starts_with("manual:"))
        .map(|(node, kind, addr)| Reported {
            how: how(kind, diag::is_lan(addr)).to_string(),
            rtt_ms: rtt_of(ctx, &node).0,
            node,
        })
        .collect();
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out.truncate(MAX_LINKS);
    out
}

fn message<P: Platform>(ctx: &NetCtx<P>) -> String {
    let me = ctx.identity.lock().node_id.clone();
    let links: Vec<Value> = my_links(ctx)
        .into_iter()
        .map(|l| {
            let mut v = json!({ "node": l.node, "how": l.how });
            if let Some(ms) = l.rtt_ms {
                v["rtt_ms"] = ms.into();
            }
            v
        })
        .collect();
    json!({ "type": "links_map", "from": me, "links": links }).to_string()
}

/// Send this device's list to `node`, over a live link only, if it
/// understands it.
fn send_to<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !supports(ctx, node) || !transport::is_live(ctx, node) {
        return;
    }
    let from = ctx.identity.lock().name.clone();
    let _ = transport::send_payload(
        &ctx.ws_conns,
        &ctx.p2p_conns,
        &ctx.key,
        &from,
        node,
        &message(ctx),
    );
}

/// This device's links changed: send the list to every linked device that
/// understands it, a little later (once for a burst of changes).
pub fn changed<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.config.links_map || st(ctx).pending.swap(true, Ordering::SeqCst) {
        return;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        st(&ctx).pending.store(false, Ordering::SeqCst);
        let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
        for node in linked {
            send_to(&ctx, &node);
        }
    });
}

/// A link to `node` came up (or got better): the path it took works, and
/// what it sends can be read.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let path = ctx
        .p2p_conns
        .lock()
        .get(node)
        .map(|c| path_of(c.kind, diag::is_lan(c.addr)));
    if let Some(path) = path {
        if let Some(m) = st(ctx).tries.lock().get_mut(node) {
            m.remove(path);
        }
    }
    st(ctx).bad_key.lock().remove(node);
    changed(ctx);
}

/// The path a live link took.
pub fn path_of(kind: TransportKind, lan: bool) -> &'static str {
    match kind {
        TransportKind::Ble => "ble",
        TransportKind::Relay => "relay",
        TransportKind::Quic if lan => "lan",
        _ => "internet",
    }
}

/// Note what stopped `path` to `node` just now (`reason`: one of that
/// path's codes).
pub fn tried<P: Platform>(ctx: &NetCtx<P>, node: &str, path: &str, reason: &str) {
    if node.is_empty() || !PATHS.contains(&path) {
        return;
    }
    st(ctx)
        .tries
        .lock()
        .entry(node.to_string())
        .or_default()
        .insert(path.to_string(), (reason.to_string(), now_secs()));
}

/// The platform's Bluetooth can't link (`reason`: one of `BLE_REASONS`),
/// or can again (`None`).
pub fn ble_state<P: Platform>(ctx: &NetCtx<P>, reason: Option<&str>) {
    *st(ctx).ble.lock() = reason
        .filter(|r| BLE_REASONS.contains(r))
        .map(|r| (r.to_string(), now_secs()));
}

/// A message from `node` couldn't be read (a different device key).
pub fn undecryptable<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !node.is_empty() {
        st(ctx).bad_key.lock().insert(node.to_string(), now_secs());
    }
}

/// Why the local network didn't reach a device whose address is `peer_ip`
/// (QUIC port `quic_port`), from this device at `my_ip`; `seen_here`: it
/// was found on this network (mDNS).
pub fn lan_reason(my_ip: &str, peer_ip: &str, quic_port: u16, seen_here: bool) -> &'static str {
    use std::net::Ipv4Addr;
    let Ok(theirs) = peer_ip.trim().parse::<Ipv4Addr>() else {
        return "no_address";
    };
    if theirs.is_unspecified() {
        return "no_address";
    }
    if quic_port == 0 {
        return "old_app";
    }
    if !diag::is_local_addr(theirs.into()) {
        return if seen_here {
            "other_network"
        } else {
            "not_seen"
        };
    }
    match my_ip.trim().parse::<Ipv4Addr>() {
        Ok(mine) if mine.octets()[..3] != theirs.octets()[..3] => "other_network",
        _ => "blocked",
    }
}

/// Why the direct internet path didn't work, the most telling first: the
/// push couldn't be sent, the device didn't answer it (asleep or offline),
/// both networks are strict (symmetric NATs), hole punching ran out of
/// time, neither router forwards a port (UPnP / NAT-PMP refused or off),
/// or it simply timed out.
pub fn internet_reason(
    push_failed: bool,
    answered: bool,
    both_symmetric: bool,
    punch_timed_out: bool,
    any_port_mapping: bool,
) -> &'static str {
    if push_failed {
        "push_failed"
    } else if !answered {
        "no_answer"
    } else if both_symmetric {
        "symmetric_nat"
    } else if punch_timed_out {
        "punch_timeout"
    } else if !any_port_mapping {
        "no_port_mapping"
    } else {
        "timed_out"
    }
}

/// Why the relay couldn't be used, from the error connecting to it.
pub fn relay_reason(error: &str) -> &'static str {
    let e = error.to_ascii_lowercase();
    if ["401", "403", "unauthor", "forbidden", "auth"]
        .iter()
        .any(|w| e.contains(w))
    {
        "auth_failed"
    } else if ["503", "full", "capacity", "too many"]
        .iter()
        .any(|w| e.contains(w))
    {
        "relay_full"
    } else {
        "unreachable"
    }
}

/// What stopped each path to `node`, in `PATHS` order, leaving out the
/// one carrying its link (`linked_by`).
fn tries_of<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    linked_by: Option<&str>,
    now: u64,
) -> Vec<PathTry> {
    let ago = |t: u64| u32::try_from(now.saturating_sub(t)).unwrap_or(u32::MAX);
    let mine = st(ctx).tries.lock().get(node).cloned().unwrap_or_default();
    let ble = st(ctx).ble.lock().clone();
    PATHS
        .iter()
        .filter(|p| Some(**p) != linked_by)
        .filter_map(|p| {
            let (reason, at) = match mine.get(*p) {
                Some(x) => x.clone(),
                None if *p == "ble" => ble.clone()?,
                None => return None,
            };
            Some(PathTry {
                path: p.to_string(),
                reason,
                ago_s: ago(at),
            })
        })
        .collect()
}

/// The link to `node` went down: keep when, for "last seen".
pub fn link_down<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let now = now_secs();
    st(ctx).seen.lock().insert(node.to_string(), Some(now));
    ctx.platform
        .kv_set(&format!("{SEEN_PREFIX}{node}"), &now.to_string());
    changed(ctx);
}

/// When the link to `node` last went down (unix seconds).
fn last_seen<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<u64> {
    if let Some(at) = st(ctx).seen.lock().get(node) {
        return *at;
    }
    let at = ctx
        .platform
        .kv_get(&format!("{SEEN_PREFIX}{node}"))
        .and_then(|v| v.parse::<u64>().ok());
    st(ctx).seen.lock().insert(node.to_string(), at);
    at
}

// --- what arrives ------------------------------------------------------------

/// The entries of a `links` array worth keeping: known words, sane ids, no
/// duplicates, at most `MAX_LINKS`.
pub fn parse_links(v: &Value) -> Vec<Reported> {
    let mut out: Vec<Reported> = Vec::new();
    for e in v.as_array().map(Vec::as_slice).unwrap_or_default() {
        if out.len() >= MAX_LINKS {
            break;
        }
        let node = e.get("node").and_then(Value::as_str).unwrap_or_default();
        let how = e.get("how").and_then(Value::as_str).unwrap_or_default();
        if node.is_empty() || node.len() > MAX_ID || !HOWS.contains(&how) {
            continue;
        }
        if out.iter().any(|r| r.node == node) {
            continue;
        }
        let rtt_ms = e
            .get("rtt_ms")
            .and_then(Value::as_u64)
            .map(|ms| u32::try_from(ms).unwrap_or(u32::MAX));
        out.push(Reported {
            node: node.to_string(),
            how: how.to_string(),
            rtt_ms,
        });
    }
    out
}

pub struct LinksMapHandler;

impl<P: Platform> ControlHandler<P> for LinksMapHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["links_map"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return; // only your own devices
        }
        // Sending it says it understands it.
        let was = st(ctx).caps.lock().insert(from.clone(), true);
        if was != Some(true) {
            send_to(ctx, &from);
        }
        let links = parse_links(&msg.json("links"));
        st(ctx).reports.lock().insert(
            from.clone(),
            Report {
                at: now_secs(),
                links,
            },
        );
        ctx.sink.emit(CoreEvent::LinksMapChanged { from });
    }
}

// --- the map -------------------------------------------------------------------

/// Links between your other devices from what they reported: only from
/// devices linked here now, only between devices this one knows, one per
/// pair (the newest report wins).
pub fn others(
    me: &str,
    known: &[String],
    reports: &[(String, u64, Vec<Reported>)],
    now: u64,
) -> Vec<OtherLink> {
    let mut best: HashMap<(String, String), OtherLink> = HashMap::new();
    for (by, at, links) in reports {
        if by == me || !known.contains(by) {
            continue;
        }
        for l in links {
            if l.node == *me || l.node == *by || !known.contains(&l.node) {
                continue;
            }
            let (a, b) = if *by < l.node {
                (by.clone(), l.node.clone())
            } else {
                (l.node.clone(), by.clone())
            };
            let ago = u32::try_from(now.saturating_sub(*at)).unwrap_or(u32::MAX);
            let link = OtherLink {
                a: a.clone(),
                b: b.clone(),
                how: l.how.clone(),
                rtt_ms: l.rtt_ms,
                reported_by: by.clone(),
                reported_ago_s: ago,
            };
            match best.get(&(a.clone(), b.clone())) {
                Some(old) if old.reported_ago_s <= ago => {}
                _ => {
                    best.insert((a, b), link);
                }
            }
        }
    }
    let mut out: Vec<OtherLink> = best.into_values().collect();
    out.sort_by(|x, y| (&x.a, &x.b).cmp(&(&y.a, &y.b)));
    out
}

/// What the map shows now.
pub fn snapshot<P: Platform>(ctx: &NetCtx<P>) -> LinksMap {
    let (me, my_name) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.name.clone())
    };
    let mut peers: Vec<crate::model::Peer> = ctx.peers.lock().values().cloned().collect();
    peers.retain(|p| p.node_id != me);
    peers.sort_by(|a, b| {
        (a.name.to_lowercase(), &a.node_id).cmp(&(b.name.to_lowercase(), &b.node_id))
    });
    let conns = ctx.p2p_conns.lock().clone();
    let now = now_secs();
    let ago = |t: u64| u32::try_from(now.saturating_sub(t)).unwrap_or(u32::MAX);

    let devices = peers
        .iter()
        .map(|p| {
            let attempts = diag::attempts(ctx, &p.node_id);
            let link = conns.get(&p.node_id).map(|c| {
                let (rtt_ms, loss_pct) = rtt_of(ctx, &p.node_id);
                MapLink {
                    how: how(c.kind, diag::is_lan(c.addr)).to_string(),
                    transport: c.kind,
                    remote: c.addr.map(|a| a.to_string()),
                    addr_kind: addr_kind(c.addr, p.mapped_quic.as_deref()),
                    rtt_ms,
                    loss_pct,
                    since_s: attempts
                        .iter()
                        .rev()
                        .find(|a| a.stage == "link_up")
                        .map(|a| a.ago_s),
                    heard_ago_s: ago(c.last_seen),
                }
            });
            let linked_by = conns
                .get(&p.node_id)
                .map(|c| path_of(c.kind, diag::is_lan(c.addr)));
            let tries = tries_of(ctx, &p.node_id, linked_by, now);
            let bad_key = st(ctx)
                .bad_key
                .lock()
                .get(&p.node_id)
                .is_some_and(|at| now.saturating_sub(*at) < BAD_KEY_FOR);
            let problem = if crate::engine::key_changed(ctx, &p.node_id) {
                Some("key_changed".to_string())
            } else {
                bad_key.then(|| "bad_key".to_string())
            };
            let (state, reason) = match &link {
                Some(l) => (l.how.clone(), None),
                None => Diagnosis::judge(None, &attempts),
            };
            MapDevice {
                node_id: p.node_id.clone(),
                name: p.name.clone(),
                source: p.source,
                last_seen_ago_s: if link.is_some() {
                    None
                } else {
                    last_seen(ctx, &p.node_id).map(ago)
                },
                link,
                state,
                reason,
                shares_links: supports(ctx, &p.node_id),
                tries,
                problem,
            }
        })
        .collect();

    let known: Vec<String> = peers.iter().map(|p| p.node_id.clone()).collect();
    let reports: Vec<(String, u64, Vec<Reported>)> = st(ctx)
        .reports
        .lock()
        .iter()
        .filter(|(by, _)| conns.contains_key(*by))
        .map(|(by, r)| (by.clone(), r.at, r.links.clone()))
        .collect();
    LinksMap {
        node_id: me.clone(),
        name: my_name,
        devices,
        others: others(&me, &known, &reports, now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(node: &str, how: &str) -> Reported {
        Reported {
            node: node.into(),
            how: how.into(),
            rtt_ms: None,
        }
    }

    #[test]
    fn hows_match_the_connection_check() {
        assert_eq!(how(TransportKind::Quic, true), "lan");
        assert_eq!(how(TransportKind::Quic, false), "direct");
        assert_eq!(how(TransportKind::Punch, false), "punched");
        assert_eq!(how(TransportKind::Relay, false), "relay");
        assert_eq!(how(TransportKind::Ble, true), "ble");
        assert_eq!(how(TransportKind::WebRtc, false), "webrtc");
        for k in [
            TransportKind::Quic,
            TransportKind::Punch,
            TransportKind::Relay,
            TransportKind::Ble,
            TransportKind::WebRtc,
        ] {
            assert!(HOWS.contains(&how(k, false)));
        }
    }

    #[test]
    fn why_the_local_network_failed() {
        assert_eq!(lan_reason("192.168.1.2", "", 9000, false), "no_address");
        assert_eq!(
            lan_reason("192.168.1.2", "0.0.0.0", 9000, false),
            "no_address"
        );
        assert_eq!(lan_reason("192.168.1.2", "192.168.1.9", 0, true), "old_app");
        assert_eq!(
            lan_reason("192.168.1.2", "8.8.8.8", 9000, false),
            "not_seen"
        );
        assert_eq!(
            lan_reason("192.168.1.2", "8.8.8.8", 9000, true),
            "other_network"
        );
        assert_eq!(
            lan_reason("192.168.1.2", "10.0.0.5", 9000, false),
            "other_network"
        );
        assert_eq!(
            lan_reason("192.168.1.2", "192.168.1.9", 9000, true),
            "blocked"
        );
        assert_eq!(lan_reason("", "192.168.1.9", 9000, false), "blocked");
        for r in [
            lan_reason("1.2.3.4", "", 1, false),
            lan_reason("1.2.3.4", "10.0.0.1", 0, false),
            lan_reason("1.2.3.4", "8.8.8.8", 1, false),
            lan_reason("10.0.0.2", "10.0.0.1", 1, false),
        ] {
            assert!(LAN_REASONS.contains(&r), "{r}");
        }
    }

    #[test]
    fn why_the_internet_path_failed() {
        let r = internet_reason;
        assert_eq!(r(true, false, true, true, false), "push_failed");
        assert_eq!(r(false, false, true, true, true), "no_answer");
        assert_eq!(r(false, true, true, true, true), "symmetric_nat");
        assert_eq!(r(false, true, false, true, true), "punch_timeout");
        assert_eq!(r(false, true, false, false, false), "no_port_mapping");
        assert_eq!(r(false, true, false, false, true), "timed_out");
        for code in [
            r(true, true, true, true, true),
            r(false, false, false, false, false),
            r(false, true, false, false, true),
        ] {
            assert!(INTERNET_REASONS.contains(&code));
        }
    }

    #[test]
    fn why_the_relay_failed() {
        assert_eq!(
            relay_reason("relay handshake failed: HTTP error: 401 Unauthorized"),
            "auth_failed"
        );
        assert_eq!(relay_reason("relay handshake failed: 403"), "auth_failed");
        assert_eq!(
            relay_reason("relay handshake failed: 503 Service Unavailable"),
            "relay_full"
        );
        assert_eq!(relay_reason("relay is full"), "relay_full");
        assert_eq!(
            relay_reason("relay connect failed: connection refused"),
            "unreachable"
        );
        assert_eq!(relay_reason("relay host did not resolve"), "unreachable");
        for code in ["auth_failed", "relay_full", "unreachable"] {
            assert!(RELAY_REASONS.contains(&code));
        }
    }

    #[test]
    fn paths_of_links() {
        assert_eq!(path_of(TransportKind::Quic, true), "lan");
        assert_eq!(path_of(TransportKind::Quic, false), "internet");
        assert_eq!(path_of(TransportKind::Punch, false), "internet");
        assert_eq!(path_of(TransportKind::WebRtc, false), "internet");
        assert_eq!(path_of(TransportKind::Relay, false), "relay");
        assert_eq!(path_of(TransportKind::Ble, false), "ble");
    }

    #[test]
    fn address_kinds() {
        let a = |s: &str| s.parse::<std::net::SocketAddr>().ok();
        assert_eq!(addr_kind(None, None), None);
        assert_eq!(
            addr_kind(a("192.168.1.4:9000"), None).as_deref(),
            Some("local")
        );
        assert_eq!(
            addr_kind(a("[fe80::1]:9000"), None).as_deref(),
            Some("local")
        );
        assert_eq!(
            addr_kind(a("[2001:db8::1]:9000"), None).as_deref(),
            Some("ipv6")
        );
        assert_eq!(
            addr_kind(a("8.8.4.4:9000"), None).as_deref(),
            Some("public")
        );
        assert_eq!(
            addr_kind(a("[::ffff:8.8.4.4]:9000"), None).as_deref(),
            Some("public")
        );
        assert_eq!(
            addr_kind(a("8.8.4.4:9000"), Some("8.8.4.4:9000")).as_deref(),
            Some("mapped")
        );
    }

    #[test]
    fn lists_are_checked_as_they_arrive() {
        let mut many: Vec<Value> = (0..100)
            .map(|i| json!({ "node": format!("n{i}"), "how": "lan" }))
            .collect();
        many.insert(0, json!({ "node": "a", "how": "teleport" }));
        many.insert(0, json!({ "node": "", "how": "lan" }));
        many.insert(0, json!({ "node": "x".repeat(200), "how": "lan" }));
        many.insert(0, json!({ "node": "dup", "how": "relay", "rtt_ms": 42 }));
        many.insert(1, json!({ "node": "dup", "how": "lan" }));
        many.insert(2, json!("junk"));
        let got = parse_links(&Value::Array(many));
        assert_eq!(got.len(), MAX_LINKS);
        assert_eq!(
            got[0],
            Reported {
                node: "dup".into(),
                how: "relay".into(),
                rtt_ms: Some(42)
            }
        );
        assert!(got.iter().all(|l| HOWS.contains(&l.how.as_str())));
        assert_eq!(got.iter().filter(|l| l.node == "dup").count(), 1);
        assert!(parse_links(&json!(null)).is_empty());
        assert!(parse_links(&json!({ "node": "a" })).is_empty());
    }

    #[test]
    fn other_links_are_one_per_pair_and_only_between_known_devices() {
        let known: Vec<String> = ["phone", "laptop", "tablet"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let reports = vec![
            // The phone: to me (left out), to the laptop, to a stranger.
            (
                "phone".to_string(),
                90,
                vec![r("me", "lan"), r("laptop", "relay"), r("stranger", "lan")],
            ),
            // The laptop, more recently: the same pair, now direct.
            (
                "laptop".to_string(),
                95,
                vec![r("phone", "direct"), r("laptop", "lan")],
            ),
            // Someone who isn't one of yours.
            ("stranger".to_string(), 99, vec![r("tablet", "lan")]),
            // A report claiming to be from me.
            ("me".to_string(), 99, vec![r("tablet", "lan")]),
        ];
        let got = others("me", &known, &reports, 100);
        assert_eq!(got.len(), 1, "{got:?}");
        let l = &got[0];
        assert_eq!((l.a.as_str(), l.b.as_str()), ("laptop", "phone"));
        assert_eq!(l.how, "direct", "the newest report wins");
        assert_eq!(l.reported_by, "laptop");
        assert_eq!(l.reported_ago_s, 5);
    }
}
