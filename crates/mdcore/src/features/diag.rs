//! "Check connection": what a device's link looks like right now, why it
//! might be slow or missing, and how fast it is.
//!
//!   diag_ping   { from, id }                    either way
//!   diag_pong   { from, id }                    answer to a ping
//!   speed_data  { from, id, off, total, data }  sender → receiver (base64)
//!   speed_ack   { from, id, got }               receiver → sender
//!   speed_get   { from, id, bytes }             "send me `bytes`" (download)
//!   speed_cancel{ from, id }                    either way
//!
//! A ping is timed here (an `Instant` per id), so the two clocks never
//! need to agree. The speed test sends zeros in `CHUNK`-sized pieces with at
//! most `WINDOW` bytes unacknowledged, as control messages over the live
//! link: the same path, framing and encryption a file transfer takes, so
//! the number is what a file would get. The receiver only counts the bytes
//! (nothing is kept) and acks how many it has. Upload is timed from the
//! first piece to the last ack; download from asking (`speed_get`) to the
//! last piece. Each direction is at most `MAX_SPEED_BYTES`, and a test can
//! be cancelled from either end at any time.
//!
//! Only devices that said `ping: 1` (in their status, or in the
//! `addr_update` every link starts with) are ever sent any of these: an
//! older app would show them as chat messages. Nothing here opens a link
//! or wakes a device: without a live link there's simply no measurement.
//!
//! The engine also notes what each connect attempt did (`note`): the
//! ladder's stages, a push the server couldn't send, a device that didn't
//! answer, links coming up and going down; a few per device, newest last,
//! for the report (`Engine::diagnose`).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::{Condvar, Mutex};
use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::{self, now_secs, TransportKind};

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device answers pings
/// and speed tests, and its version.
pub const FLAG: &str = "ping";
const VERSION: u64 = 1;

/// The most bytes a speed test sends each way.
pub const MAX_SPEED_BYTES: u32 = 32 * 1024 * 1024;
/// The fewest (anything smaller measures only the round trip).
pub const MIN_SPEED_BYTES: u32 = 256 * 1024;
/// Raw bytes per piece (as `features::files`).
const CHUNK: usize = 64 * 1024;
/// Most bytes in flight without an ack.
const WINDOW: u64 = 16 * CHUNK as u64;
/// No progress for this long: the test fails.
const STALL: Duration = Duration::from_secs(10);
/// Speed tests one device serves or receives at once.
const MAX_RUNNING: usize = 4;
/// A piece arriving for a test nobody finished in this long is dropped.
const RECV_FORGET: Duration = Duration::from_secs(60);
/// How long a ping waits for its pong.
pub const PING_TIMEOUT: Duration = Duration::from_millis(1500);
/// Attempts kept per device, and devices kept.
const HISTORY: usize = 16;
const HISTORY_DEVICES: usize = 64;
/// How often a running test reports progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// One thing that happened while connecting to a device.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Attempt {
    /// A ladder stage (`quic`, `signaled_quic`, `relay`, `punching`,
    /// `webrtc`, `nat_skip`, `connected`, `failed`) or what this module
    /// adds: `push_failed` (the server couldn't send the push),
    /// `no_answer` (the push went, the device didn't answer),
    /// `relay_no_answer`, `relay_unreachable`, `link_up` (detail: the
    /// transport), `link_down` (detail: why, see `features::reconnect`),
    /// `reconnected` (detail: `trigger:ms`), `waking`, `asleep`, `lan_try`,
    /// `key_changed` (it came with another device key: refused until the
    /// user trusts it), `key_trusted`.
    pub stage: String,
    pub detail: Option<String>,
    /// How long ago, in seconds.
    pub ago_s: u32,
}

#[derive(Clone, Debug)]
struct Noted {
    at: u64,
    stage: String,
    detail: Option<String>,
}

/// Round trips measured with pings, in milliseconds.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RttStats {
    /// Pings answered ...
    pub samples: u32,
    /// ... and not answered in time.
    pub lost: u32,
    pub min_ms: Option<f64>,
    pub avg_ms: Option<f64>,
    pub max_ms: Option<f64>,
    /// Mean difference between one round trip and the next.
    pub jitter_ms: Option<f64>,
}

impl RttStats {
    pub fn from_samples(samples: &[f64], lost: u32) -> Self {
        let n = samples.len();
        let mut s = RttStats {
            samples: u32::try_from(n).unwrap_or(u32::MAX),
            lost,
            ..Default::default()
        };
        if n == 0 {
            return s;
        }
        let round = |x: f64| (x * 100.0).round() / 100.0;
        s.min_ms = Some(round(samples.iter().copied().fold(f64::MAX, f64::min)));
        s.max_ms = Some(round(samples.iter().copied().fold(0.0, f64::max)));
        s.avg_ms = Some(round(samples.iter().sum::<f64>() / n as f64));
        if n > 1 {
            let d: f64 = samples.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            s.jitter_ms = Some(round(d / (n - 1) as f64));
        }
        s
    }
}

/// What a speed test measured.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SpeedResult {
    /// Bytes sent each way.
    pub bytes: u32,
    /// This device to the other, and back, in Mbit/s.
    pub upload_mbps: f64,
    pub download_mbps: f64,
    pub upload_ms: u32,
    pub download_ms: u32,
    /// Pings before the transfer.
    pub rtt: RttStats,
    /// The link it ran over.
    pub transport: TransportKind,
}

/// The live link, as the report shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LinkInfo {
    pub transport: TransportKind,
    /// Peer to peer (not the relay or Bluetooth).
    pub direct: bool,
    /// The other end's address is on the local network.
    pub lan: bool,
    /// The other end's address, when the link has one (QUIC, punched).
    pub remote: Option<String>,
    /// QUIC's own round-trip and loss figures (QUIC links only).
    pub quic_rtt_ms: Option<u32>,
    pub loss_pct: Option<f32>,
    /// Seconds since the link last heard from it / last carried a message.
    pub heard_ago_s: u32,
    pub used_ago_s: u32,
}

/// Where a device says it can be reached.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Addresses {
    pub ip: String,
    pub ipv6: Option<String>,
    pub quic_port: u16,
    /// The port its router forwards to it (UPnP / NAT-PMP).
    pub mapped_quic: Option<String>,
    /// The public address the server saw it at (the other device only).
    pub public: Option<String>,
    /// What its NAT does to outgoing traffic ("easy", "symmetric", "unknown").
    pub nat: crate::nat::NatKind,
}

/// The relay server, the fallback when no direct path works.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RelayInfo {
    /// Signed in to a server (no server: no relay, no push).
    pub signed_in: bool,
    /// This device has a relay connection open now.
    pub connected: bool,
    /// The relay answered a connection just now (`None`: not checked).
    pub reachable: Option<bool>,
    pub error: Option<String>,
}

/// "Check connection" for one device (`Engine::diagnose`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Diagnosis {
    pub node_id: String,
    pub name: String,
    /// In one word, for the apps' plain-language line: "lan", "direct",
    /// "punched", "webrtc", "relay", "ble" (connected that way), or
    /// "unreachable" (tried, no link) or "not_connected" (not tried).
    pub verdict: String,
    /// Why it's unreachable, from the last attempt: "push_failed",
    /// "no_answer", "relay_no_answer", "relay_unreachable", "no_path",
    /// "dropped", "key_changed" (its device key changed: refused until
    /// the user trusts the new one).
    pub reason: Option<String>,
    /// Seconds since the last connect attempt or link change.
    pub last_try_ago_s: Option<u32>,
    pub link: Option<LinkInfo>,
    /// It answers pings (its app is new enough), and what they measured.
    pub ping_supported: bool,
    pub rtt: Option<RttStats>,
    pub this_device: Addresses,
    pub peer: Addresses,
    /// Where this device heard of it: "registry", "lan", "manual".
    pub source: crate::model::Source,
    /// Both ends behind symmetric NATs: only the relay can join them
    /// across the internet.
    pub both_symmetric: bool,
    pub relay: RelayInfo,
    /// What connecting did, oldest first.
    pub attempts: Vec<Attempt>,
    /// How fast links to it come back, how long they last, why they go,
    /// and where it stands (`features::reconnect`).
    pub reconnect: super::reconnect::ReconnectInfo,
    /// Its envelopes: the version it reads, clock skew, and the copies,
    /// stale, misaddressed, misattributed and downgraded ones refused
    /// (`transport::envelope`).
    pub envelope: crate::transport::envelope::EnvelopeInfo,
}

impl Diagnosis {
    /// The verdict and reason for a link (or none) and what was tried.
    pub fn judge(link: Option<&LinkInfo>, attempts: &[Attempt]) -> (String, Option<String>) {
        if let Some(l) = link {
            return (super::links_map::how(l.transport, l.lan).into(), None);
        }
        if attempts.is_empty() {
            return ("not_connected".into(), None);
        }
        // The last run: back to the last link change. A link that went
        // down and wasn't tried since dropped; otherwise the most telling
        // thing that run noted.
        let mut run = Vec::new();
        for a in attempts.iter().rev() {
            match a.stage.as_str() {
                "link_down" if run.is_empty() => {
                    return ("unreachable".into(), Some("dropped".into()))
                }
                "link_up" | "link_down" => break,
                s => run.push(s),
            }
        }
        let reason = [
            "key_changed",
            "push_failed",
            "no_answer",
            "relay_no_answer",
            "relay_unreachable",
        ]
        .into_iter()
        .find(|r| run.contains(r))
        .or_else(|| run.contains(&"failed").then_some("no_path"))
        .map(str::to_string);
        ("unreachable".into(), reason)
    }
}

/// Mbit/s for `bytes` in `d`.
pub fn mbps(bytes: u64, d: Duration) -> f64 {
    let secs = d.as_secs_f64().max(1e-6);
    ((bytes as f64 * 8.0 / secs / 1e6) * 100.0).round() / 100.0
}

/// A sender's progress: how far the receiver has acked.
struct Sending {
    node: String,
    acked: Mutex<u64>,
    cv: Condvar,
    cancel: AtomicBool,
}

/// Bytes arriving for a test.
struct Receiving {
    node: String,
    total: u64,
    got: u64,
    last: Instant,
    /// Our own download (kept until the test reads it), not a peer's upload.
    local: bool,
    cancelled: bool,
}

/// Everything this feature keeps.
#[derive(Default)]
pub struct Diag {
    caps: Mutex<HashMap<String, bool>>,
    history: Mutex<HashMap<String, VecDeque<Noted>>>,
    pings: Mutex<HashMap<String, mpsc::Sender<Instant>>>,
    sending: Mutex<HashMap<String, Arc<Sending>>>,
    receiving: Mutex<HashMap<String, Receiving>>,
    recv_cv: Condvar,
    /// Our own running tests: id -> cancelled.
    tests: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn diag<P: Platform>(ctx: &NetCtx<P>) -> &Diag {
    &ctx.extras.diag
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Send over the live link only (never connects, never wakes a device).
fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value) -> bool {
    let from = ctx.identity.lock().name.clone();
    transport::send_payload(ctx, &from, node, &msg.to_string()).is_ok()
}

fn linked<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    transport::is_live(ctx, node) && ctx.ws_conns.lock().contains_key(node)
}

// --- capability ------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it answers pings.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.diag {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `ping` value of its status or `addr_update`)
/// whether it answers pings.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    diag(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` said it answers pings and speed tests.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.diag && diag(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

// --- connect history ---------------------------------------------------------

/// Note something that happened while connecting to `node`.
pub fn note<P: Platform>(ctx: &NetCtx<P>, node: &str, stage: &str, detail: Option<&str>) {
    let mut h = diag(ctx).history.lock();
    if !h.contains_key(node) && h.len() >= HISTORY_DEVICES {
        // Forget the device heard of longest ago.
        let oldest = h
            .iter()
            .min_by_key(|(_, q)| q.back().map(|n| n.at).unwrap_or(0))
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            h.remove(&k);
        }
    }
    let q = h.entry(node.to_string()).or_default();
    if q.len() >= HISTORY {
        q.pop_front();
    }
    q.push_back(Noted {
        at: now_secs(),
        stage: stage.to_string(),
        detail: detail.map(str::to_string),
    });
}

/// What happened while connecting to `node`, oldest first.
pub fn attempts<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Vec<Attempt> {
    let now = now_secs();
    diag(ctx)
        .history
        .lock()
        .get(node)
        .map(|q| {
            q.iter()
                .map(|n| Attempt {
                    stage: n.stage.clone(),
                    detail: n.detail.clone(),
                    ago_s: u32::try_from(now.saturating_sub(n.at)).unwrap_or(u32::MAX),
                })
                .collect()
        })
        .unwrap_or_default()
}

// --- addresses ---------------------------------------------------------------

/// An address on this network: private IPv4, link-local, IPv6 unique-local,
/// or loopback. (A link to one never leaves the building.)
pub fn is_local_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_addr(IpAddr::V4(v4));
            }
            let s0 = v6.segments()[0];
            v6.is_loopback() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80
        }
    }
}

/// A link to `addr` stays on the local network.
pub fn is_lan(addr: Option<SocketAddr>) -> bool {
    addr.is_some_and(|a| is_local_addr(a.ip()))
}

// --- ping ----------------------------------------------------------------------

/// Ping `node` `count` times over its live link, one after another. `None`
/// when it doesn't answer pings or there's no link.
pub fn ping<P: Platform>(ctx: &NetCtx<P>, node: &str, count: u32) -> Option<RttStats> {
    if !supports(ctx, node) || !linked(ctx, node) {
        return None;
    }
    let me = me(ctx);
    let mut samples = Vec::new();
    let mut lost = 0;
    for _ in 0..count {
        let id = transport::outbox::new_id();
        let (tx, rx) = mpsc::channel();
        diag(ctx).pings.lock().insert(id.clone(), tx);
        let start = Instant::now();
        let sent = send(
            ctx,
            node,
            &json!({ "type": "diag_ping", "from": me, "id": id }),
        );
        let got = if sent {
            rx.recv_timeout(PING_TIMEOUT).ok()
        } else {
            None
        };
        diag(ctx).pings.lock().remove(&id);
        match got {
            Some(at) => samples.push(at.duration_since(start).as_secs_f64() * 1000.0),
            None => lost += 1,
        }
        if !sent {
            break;
        }
    }
    Some(RttStats::from_samples(&samples, lost))
}

// --- speed test ------------------------------------------------------------------

/// The piece of zeros a sender sends, in base64 (made once per size).
fn zeros_b64(n: usize) -> String {
    base64::engine::general_purpose::STANDARD.encode(vec![0u8; n])
}

/// Send `total` bytes to `node` as test `id`. Returns how long it took
/// from the first piece to the last ack. `progress` gets the acked bytes.
fn run_send<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    id: &str,
    total: u64,
    cancel: Option<&AtomicBool>,
    progress: &dyn Fn(u64),
) -> Result<Duration, String> {
    let s = new_sending(node);
    diag(ctx).sending.lock().insert(id.to_string(), s.clone());
    run_reserved(ctx, &s, id, total, cancel, progress)
}

fn new_sending(node: &str) -> Arc<Sending> {
    Arc::new(Sending {
        node: node.to_string(),
        acked: Mutex::new(0),
        cv: Condvar::new(),
        cancel: AtomicBool::new(false),
    })
}

/// Send with `s` already registered under `id`; unregisters it after.
fn run_reserved<P: Platform>(
    ctx: &NetCtx<P>,
    s: &Arc<Sending>,
    id: &str,
    total: u64,
    cancel: Option<&AtomicBool>,
    progress: &dyn Fn(u64),
) -> Result<Duration, String> {
    let out = send_loop(ctx, s, id, total, cancel, progress);
    diag(ctx).sending.lock().remove(id);
    out
}

fn send_loop<P: Platform>(
    ctx: &NetCtx<P>,
    s: &Sending,
    id: &str,
    total: u64,
    cancel: Option<&AtomicBool>,
    progress: &dyn Fn(u64),
) -> Result<Duration, String> {
    let me = me(ctx);
    let full = zeros_b64(CHUNK);
    let start = Instant::now();
    let mut pos = 0u64;
    let mut moved = Instant::now();
    let mut seen = 0u64;
    let mut reported = Instant::now();
    loop {
        let stopped = || {
            s.cancel.load(Ordering::Relaxed)
                || cancel.is_some_and(|c| c.load(Ordering::Relaxed))
                || ctx.shutdown.load(Ordering::Relaxed)
        };
        if stopped() {
            return Err("cancelled".into());
        }
        if !linked(ctx, &s.node) {
            return Err("the link went down".into());
        }
        let acked = {
            let mut acked = s.acked.lock();
            if *acked < total && (pos >= total || pos.saturating_sub(*acked) >= WINDOW) {
                s.cv.wait_for(&mut acked, Duration::from_millis(250));
            }
            *acked
        };
        if acked >= total {
            progress(total);
            return Ok(start.elapsed());
        }
        if acked > seen {
            seen = acked;
            moved = Instant::now();
        } else if moved.elapsed() > STALL {
            return Err("the other device stopped answering".into());
        }
        if reported.elapsed() >= PROGRESS_EVERY {
            reported = Instant::now();
            progress(acked);
        }
        if pos < total && pos.saturating_sub(acked) < WINDOW {
            let n = (total - pos).min(CHUNK as u64);
            let data = if n == CHUNK as u64 {
                full.clone()
            } else {
                zeros_b64(n as usize)
            };
            let m = json!({
                "type": "speed_data", "from": me, "id": id,
                "off": pos, "total": total, "data": data,
            });
            if !send(ctx, &s.node, &m) {
                return Err("the link went down".into());
            }
            pos += n;
        }
    }
}

/// Ask `node` for `total` bytes as test `id` and wait for them all.
/// Returns how long from asking to the last piece.
fn run_receive<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    id: &str,
    total: u64,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<Duration, String> {
    let d = diag(ctx);
    d.receiving.lock().insert(
        id.to_string(),
        Receiving {
            node: node.to_string(),
            total,
            got: 0,
            last: Instant::now(),
            local: true,
            cancelled: false,
        },
    );
    let start = Instant::now();
    let asked = send(
        ctx,
        node,
        &json!({ "type": "speed_get", "from": me(ctx), "id": id, "bytes": total }),
    );
    let out = (|| {
        if !asked {
            return Err("the link went down".to_string());
        }
        let mut reported = Instant::now();
        let mut r = d.receiving.lock();
        loop {
            let Some(e) = r.get(id) else {
                return Err("cancelled".into());
            };
            if e.got >= total {
                return Ok(start.elapsed());
            }
            if e.cancelled || cancel.load(Ordering::Relaxed) || ctx.shutdown.load(Ordering::Relaxed)
            {
                return Err("cancelled".into());
            }
            if e.last.elapsed() > STALL {
                return Err("the other device stopped sending".into());
            }
            let got = e.got;
            if reported.elapsed() >= PROGRESS_EVERY {
                reported = Instant::now();
                parking_lot::MutexGuard::unlocked(&mut r, || progress(got));
            }
            if !linked(ctx, node) {
                return Err("the link went down".into());
            }
            d.recv_cv.wait_for(&mut r, Duration::from_millis(250));
        }
    })();
    d.receiving.lock().remove(id);
    if out.is_ok() {
        progress(total);
    }
    out
}

/// A running test of ours: start it (`Engine::speed_test`).
pub fn start_test<P: Platform>(ctx: &NetCtx<P>, node: &str, bytes: u32) -> crate::Result<String> {
    use crate::error::CoreError;
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if !linked(ctx, node) {
        return Err(CoreError::NoLink("not connected: connect first".into()));
    }
    if !supports(ctx, node) {
        return Err(CoreError::Unsupported(
            "the other device's app is too old for a speed test: update it",
        ));
    }
    let total = u64::from(bytes.clamp(MIN_SPEED_BYTES, MAX_SPEED_BYTES));
    let id = transport::outbox::new_id();
    let cancel = Arc::new(AtomicBool::new(false));
    diag(ctx).tests.lock().insert(id.clone(), cancel.clone());
    let (ctx, node, tid) = (ctx.clone(), node.to_string(), id.clone());
    std::thread::spawn(move || {
        run_test(&ctx, &node, &tid, total, &cancel);
        diag(&ctx).tests.lock().remove(&tid);
    });
    Ok(id)
}

fn run_test<P: Platform>(ctx: &NetCtx<P>, node: &str, id: &str, total: u64, cancel: &AtomicBool) {
    let emit = |phase: &str, done: u64, result: Option<SpeedResult>, error: Option<String>| {
        ctx.sink.emit(CoreEvent::SpeedTest {
            node_id: node.to_string(),
            id: id.to_string(),
            phase: phase.to_string(),
            done,
            total,
            result,
            error,
        });
    };
    let fail = |why: String| {
        let cancelled = why == "cancelled" || cancel.load(Ordering::Relaxed);
        if cancelled {
            send(
                ctx,
                node,
                &json!({ "type": "speed_cancel", "from": me(ctx), "id": id }),
            );
            emit("cancelled", 0, None, None);
        } else {
            emit("failed", 0, None, Some(why));
        }
    };
    emit("ping", 0, None, None);
    let rtt = ping(ctx, node, 5).unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return fail("cancelled".into());
    }
    let transport = ctx.p2p_conns.lock().get(node).map(|c| c.kind);
    let Some(transport) = transport else {
        return fail("the link went down".into());
    };
    emit("upload", 0, None, None);
    let up = match run_send(ctx, node, id, total, Some(cancel), &|d| {
        emit("upload", d, None, None)
    }) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    emit("download", 0, None, None);
    let down = match run_receive(ctx, node, id, total, cancel, &|d| {
        emit("download", d, None, None)
    }) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let ms = |d: Duration| u32::try_from(d.as_millis()).unwrap_or(u32::MAX);
    let result = SpeedResult {
        bytes: u32::try_from(total).unwrap_or(u32::MAX),
        upload_mbps: mbps(total, up),
        download_mbps: mbps(total, down),
        upload_ms: ms(up),
        download_ms: ms(down),
        rtt,
        transport,
    };
    emit("done", total, Some(result), None);
}

/// Stop our test `id` (both directions; the other device is told).
pub fn cancel_test<P: Platform>(ctx: &NetCtx<P>, id: &str) -> bool {
    let d = diag(ctx);
    let Some(flag) = d.tests.lock().get(id).cloned() else {
        return false;
    };
    flag.store(true, Ordering::Relaxed);
    if let Some(s) = d.sending.lock().get(id) {
        s.cancel.store(true, Ordering::Relaxed);
        s.cv.notify_all();
    }
    d.recv_cv.notify_all();
    true
}

// --- incoming --------------------------------------------------------------------

fn on_data<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let off = msg.json("off").as_u64().unwrap_or(u64::MAX);
    let total = msg.json("total").as_u64().unwrap_or(0);
    let n = base64::engine::general_purpose::STANDARD
        .decode(msg.str("data"))
        .map(|b| b.len() as u64)
        .unwrap_or(0);
    if id.is_empty() || n == 0 || total == 0 || total > u64::from(MAX_SPEED_BYTES) {
        return;
    }
    let d = diag(ctx);
    let got = {
        let mut r = d.receiving.lock();
        r.retain(|_, e| e.local || e.last.elapsed() < RECV_FORGET);
        if !r.contains_key(&id) {
            if off != 0 || r.len() >= MAX_RUNNING {
                return;
            }
            r.insert(
                id.clone(),
                Receiving {
                    node: from.to_string(),
                    total,
                    got: 0,
                    last: Instant::now(),
                    local: false,
                    cancelled: false,
                },
            );
        }
        let Some(e) = r.get_mut(&id) else {
            return;
        };
        if e.node != from || e.cancelled {
            return;
        }
        // In order over one link; a piece that doesn't continue is
        // answered with what we have.
        if off == e.got {
            e.got = (e.got + n).min(e.total);
        }
        e.last = Instant::now();
        let got = e.got;
        if got >= e.total && !e.local {
            r.remove(&id);
        }
        got
    };
    d.recv_cv.notify_all();
    send(
        ctx,
        from,
        &json!({ "type": "speed_ack", "from": me(ctx), "id": id, "got": got }),
    );
}

fn on_ack<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let got = msg.json("got").as_u64().unwrap_or(0);
    let s = diag(ctx).sending.lock().get(&msg.str("id")).cloned();
    if let Some(s) = s.filter(|s| s.node == from) {
        let mut acked = s.acked.lock();
        if got > *acked {
            *acked = got;
        }
        s.cv.notify_all();
    }
}

fn on_get<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let bytes = msg.json("bytes").as_u64().unwrap_or(0);
    if id.is_empty() || bytes == 0 || bytes > u64::from(MAX_SPEED_BYTES) {
        return;
    }
    // Checked and taken under one lock: a burst of requests can't all
    // pass the check before any of them is counted.
    let sending = {
        let mut s = diag(ctx).sending.lock();
        if s.len() >= MAX_RUNNING || s.contains_key(&id) {
            return;
        }
        let sending = new_sending(from);
        s.insert(id.clone(), sending.clone());
        sending
    };
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let _ = run_reserved(&ctx, &sending, &id, bytes, None, &|_| {});
    });
}

fn on_cancel<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let d = diag(ctx);
    if let Some(s) = d.sending.lock().get(&id).filter(|s| s.node == from) {
        s.cancel.store(true, Ordering::Relaxed);
        s.cv.notify_all();
    }
    {
        let mut r = d.receiving.lock();
        if let Some(e) = r.get_mut(&id).filter(|e| e.node == from) {
            e.cancelled = true;
            if !e.local {
                r.remove(&id);
            }
        }
    }
    d.recv_cv.notify_all();
}

pub struct DiagHandler;

impl<P: Platform> ControlHandler<P> for DiagHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "diag_ping",
            "diag_pong",
            "speed_data",
            "speed_ack",
            "speed_get",
            "speed_cancel",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own devices only (a speed test sends up to 32 MB back).
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        match msg.kind() {
            "diag_ping" => {
                // It pings, so it knows pongs.
                diag(ctx).caps.lock().insert(from.clone(), true);
                send(
                    ctx,
                    &from,
                    &json!({ "type": "diag_pong", "from": me(ctx), "id": msg.str("id") }),
                );
            }
            "diag_pong" => {
                if let Some(tx) = diag(ctx).pings.lock().remove(&msg.str("id")) {
                    let _ = tx.send(Instant::now());
                }
            }
            "speed_data" => on_data(ctx, msg, &from),
            "speed_ack" => on_ack(ctx, msg, &from),
            "speed_get" => on_get(ctx, msg, &from),
            "speed_cancel" => on_cancel(ctx, msg, &from),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_key_is_the_reason_first() {
        let a = |stage: &str| Attempt {
            stage: stage.into(),
            detail: None,
            ago_s: 1,
        };
        let run = [a("quic"), a("key_changed"), a("no_answer"), a("failed")];
        assert_eq!(
            Diagnosis::judge(None, &run),
            ("unreachable".into(), Some("key_changed".into()))
        );
        // From before the link came up: not this run.
        let run = [a("key_changed"), a("link_up"), a("quic"), a("failed")];
        assert_eq!(Diagnosis::judge(None, &run).1.as_deref(), Some("no_path"));
    }

    #[test]
    fn rtt_stats_add_up() {
        let s = RttStats::from_samples(&[10.0, 14.0, 12.0], 1);
        assert_eq!(s.samples, 3);
        assert_eq!(s.lost, 1);
        assert_eq!(s.min_ms, Some(10.0));
        assert_eq!(s.max_ms, Some(14.0));
        assert_eq!(s.avg_ms, Some(12.0));
        assert_eq!(s.jitter_ms, Some(3.0));
        let none = RttStats::from_samples(&[], 3);
        assert_eq!((none.samples, none.lost, none.avg_ms), (0, 3, None));
    }

    #[test]
    fn local_addresses_are_told_apart() {
        let ip = |a: &str| a.parse::<IpAddr>().ok();
        for a in [
            "192.168.1.4",
            "10.0.0.2",
            "172.20.1.1",
            "169.254.3.3",
            "127.0.0.1",
            "fe80::1",
            "fd12::3",
            "::ffff:192.168.0.9",
        ] {
            assert!(ip(a).is_some_and(is_local_addr), "{a}");
        }
        for a in ["8.8.8.8", "100.64.1.1", "2001:db8::1", "::ffff:1.1.1.1"] {
            assert!(ip(a).is_some_and(|i| !is_local_addr(i)), "{a}");
        }
        assert!(!is_lan(None));
    }

    #[test]
    fn mbps_is_bits_per_second() {
        assert_eq!(mbps(1_000_000, Duration::from_secs(1)), 8.0);
        assert_eq!(mbps(4_000_000, Duration::from_millis(500)), 64.0);
    }
}
