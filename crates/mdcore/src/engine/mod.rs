//! The Engine: owns all networking state, spawns the receive/clipboard threads,
//! and exposes the public API that platform adapters (Tauri commands / JNI
//! shims) thin-wrap. Generic over `Platform`.
//!
//! Threading model: blocking thread-per-connection (a small fixed set of loops
//! plus short-lived per-message threads) — chosen for low cpu/battery.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde::Serialize;

use crate::config::CoreConfig;
use crate::crypto::derive_key;
use crate::discovery::{self, PeerMap, DISCOVERY_PORT};
use crate::error::CoreError;
use crate::events::{CoreEvent, EventSink};
use crate::features::ControlRegistry;
use crate::model::{Identity, Peer, Source};
use crate::platform::Platform;
use crate::registry::{
    auth_call, base, registry_fetch, registry_register, verify_password_call, RegistryDevice,
};
use crate::signal::{Signal, Signaler};
use crate::transport::{self, P2pConns};

mod background;
mod ble;
pub use ble::BleSender;
mod ladder;
mod messaging;
pub use messaging::QUEUED;
pub(crate) use messaging::{reconnect, send_reliably};
mod netchange;
mod peers;
pub(crate) use peers::{emit_peers, learn_unknown};
mod session;
mod sharing;
mod signaling;

pub(crate) use signaling::handle_signal;
pub use signaling::FirewallStatus;

pub type KeyHolder = Arc<Mutex<Option<[u8; 32]>>>;
/// node_id -> (connection id, outgoing-frame sender). One entry per peer.
pub type WsConns = Arc<Mutex<HashMap<String, (u64, Route)>>>;

/// Where a peer's frames go: its link's outgoing queue, which the link's
/// async writer (QUIC) or loop (WebRTC, relay) drains. A relayed peer shares
/// the relay socket's queue, so its frames are addressed to it on the way.
#[derive(Clone)]
pub struct Route {
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    to: Option<String>,
}

impl Route {
    /// A link of its own.
    pub fn direct(tx: tokio::sync::mpsc::UnboundedSender<String>) -> Self {
        Route { tx, to: None }
    }

    /// Through the relay socket whose queue is `tx`, to `peer`.
    pub fn relayed(tx: tokio::sync::mpsc::UnboundedSender<String>, peer: &str) -> Self {
        Route {
            tx,
            to: Some(peer.to_string()),
        }
    }

    /// Queue one frame; false once the link is gone. Never blocks.
    pub fn send(&self, frame: String) -> bool {
        let frame = match &self.to {
            Some(to) => serde_json::json!({"to": to, "data": frame}).to_string(),
            None => frame,
        };
        self.tx.send(frame).is_ok()
    }
}
/// node_id -> sender awaiting that peer's clipboard pull response.
pub type ClipPending = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
/// subscriber node_id -> the package names whose notifications they want
/// (app-notification pub/sub; held on the producer device).
pub type Subs = Arc<Mutex<HashMap<String, Vec<String>>>>;
/// In-flight punch negotiations: sid -> sender that delivers the peer's answered
/// candidates to the waiting initiator thread.
pub type PunchWaiters = Arc<Mutex<HashMap<String, mpsc::Sender<Vec<crate::signal::Candidate>>>>>;
/// In-flight signaled-QUIC negotiations: sid -> sender delivering the peer's
/// answered QUIC addresses.
pub type QuicWaiters = Arc<Mutex<HashMap<String, mpsc::Sender<Vec<String>>>>>;
/// In-flight WebRTC negotiations: sid -> sender delivering the answer SDP string.
pub type RtcWaiters = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;

/// Undecryptable-message notices: sender IP -> unix secs of the last notice we
/// surfaced for it (rate limiting; see `transport::note_undecryptable`).
pub type DecryptFailures = Arc<Mutex<HashMap<String, u64>>>;

/// Shared handles the receive loops + clipboard watcher need.
pub struct NetCtx<P: Platform> {
    pub platform: Arc<P>,
    pub identity: Arc<Mutex<Identity>>,
    pub key: KeyHolder,
    pub peers: PeerMap,
    pub ws_conns: WsConns,
    /// Unified live-connection registry (all transports). UI green dot reads this.
    pub p2p_conns: P2pConns,
    /// In-flight punch negotiations (initiator side).
    pub punch_waiters: PunchWaiters,
    /// In-flight WebRTC negotiations (initiator side).
    pub rtc_waiters: RtcWaiters,
    /// In-flight signaled-QUIC negotiations (initiator side).
    pub quic_waiters: QuicWaiters,
    pub sink: Arc<dyn EventSink>,
    pub clip_active: Arc<AtomicBool>,
    pub clip_last: Arc<Mutex<String>>,
    /// Recent clipboard values, newest first (`features::clipboard::remember`).
    pub clip_history: Arc<Mutex<std::collections::VecDeque<crate::model::ClipEntry>>>,
    pub clip_pending: ClipPending,
    pub control: Arc<ControlRegistry<P>>,
    /// Producer's shareable app list as a JSON array string (pushed by the host).
    pub installed_apps: Arc<Mutex<String>>,
    /// This device's status as last pushed by the host (JSON object).
    pub device_status: Arc<Mutex<String>>,
    /// File transfers in progress, both ways (see `features::files`).
    pub files: Arc<crate::features::files::Files>,
    /// App-notification subscriptions (subscriber node_id -> packages).
    pub subs: Subs,
    /// Peers whose QUIC dial failed recently: node_id -> when. Auto-QUIC
    /// skips them for a while (`transport::quic::ensure_link`), so an
    /// unreachable peer doesn't cost a connect timeout on every message.
    pub quic_failed: Arc<Mutex<HashMap<String, std::time::Instant>>>,
    /// Hand-entered `manual:<ip>` placeholders already re-listed under the
    /// peer's real node id: placeholder -> real id.
    pub manual_aliases: Arc<Mutex<HashMap<String, String>>>,
    /// Rate limiter for "could not decrypt" notices, keyed by sender IP.
    pub decrypt_fail_last: DecryptFailures,
    /// Every envelope that failed to decrypt, notified or not.
    pub decrypt_fail_count: Arc<std::sync::atomic::AtomicU64>,
    /// Start-time configuration (STUN servers, enabled ladder rungs).
    pub config: Arc<CoreConfig>,
    /// Overrides how signals are delivered; `None` = the registry over HTTP.
    pub signaler: Arc<Mutex<Option<Arc<dyn Signaler>>>>,
    /// Set when the `Engine` is dropped; background loops exit on their next tick.
    pub shutdown: Arc<Shutdown>,
    /// The app is in the background (phone screen off / app not visible):
    /// idle links get closed (`CoreConfig::idle_close_after`).
    pub background: Arc<AtomicBool>,
    /// Messages waiting for a peer's link (see `transport::outbox`).
    pub outbox: transport::outbox::Outbox,
    /// Ids of chat messages that arrived, per sender (drops resends).
    pub seen: transport::outbox::Seen,
    /// Dial/accept QUIC connections (the runtime itself is owned by `Engine`).
    pub quic: transport::quic::QuicLink,
    /// The relay WebSocket to the server, when open (last-resort path).
    pub relay: Arc<Mutex<Option<transport::relay::RelayConn>>>,
    /// The server session (shared with `Engine`): base URL and token. In the
    /// context so background threads (e.g. the relay reader handling a
    /// signal) can talk to the server too.
    pub server_url: Arc<Mutex<Option<String>>>,
    pub token: Arc<Mutex<Option<String>>>,
    /// Signals seen recently (drops copies that arrive by several paths).
    pub recent_signals: Arc<crate::features::signal_fwd::RecentSignals>,
    /// Our NAT's mapping behaviour, measured with STUN at start and after
    /// network changes; published to the registry for the connect ladder.
    pub nat: Arc<Mutex<crate::nat::NatKind>>,
    /// This device's own key: its QUIC certificate is made from it.
    pub device_key: Arc<crate::device_key::DeviceKey>,
    /// The keys other devices used before (see `device_key::accept`).
    pub peer_keys: crate::device_key::Pins,
    /// Open Bluetooth connections (see `ble`).
    pub(crate) ble_links: ble::BleLinks,
}

impl<P: Platform> Clone for NetCtx<P> {
    fn clone(&self) -> Self {
        NetCtx {
            platform: self.platform.clone(),
            identity: self.identity.clone(),
            key: self.key.clone(),
            peers: self.peers.clone(),
            ws_conns: self.ws_conns.clone(),
            p2p_conns: self.p2p_conns.clone(),
            punch_waiters: self.punch_waiters.clone(),
            rtc_waiters: self.rtc_waiters.clone(),
            quic_waiters: self.quic_waiters.clone(),
            sink: self.sink.clone(),
            clip_active: self.clip_active.clone(),
            clip_last: self.clip_last.clone(),
            clip_history: self.clip_history.clone(),
            clip_pending: self.clip_pending.clone(),
            control: self.control.clone(),
            installed_apps: self.installed_apps.clone(),
            device_status: self.device_status.clone(),
            files: self.files.clone(),
            subs: self.subs.clone(),
            quic_failed: self.quic_failed.clone(),
            manual_aliases: self.manual_aliases.clone(),
            decrypt_fail_last: self.decrypt_fail_last.clone(),
            decrypt_fail_count: self.decrypt_fail_count.clone(),
            config: self.config.clone(),
            signaler: self.signaler.clone(),
            shutdown: self.shutdown.clone(),
            background: self.background.clone(),
            outbox: self.outbox.clone(),
            device_key: self.device_key.clone(),
            peer_keys: self.peer_keys.clone(),
            ble_links: self.ble_links.clone(),
            seen: self.seen.clone(),
            quic: self.quic.clone(),
            relay: self.relay.clone(),
            nat: self.nat.clone(),
            server_url: self.server_url.clone(),
            token: self.token.clone(),
            recent_signals: self.recent_signals.clone(),
        }
    }
}

#[derive(Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SessionInfo {
    pub username: Option<String>,
    pub server_url: Option<String>,
    pub has_key: bool,
}

#[derive(Serialize, Default)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SavedSession {
    pub server_url: String,
    pub username: String,
}

/// The public facade. Everything shared with the background threads lives in
/// `ctx` (one copy, cheap to clone: every field is an `Arc`); only state the
/// threads never touch (the server session, the discovery send socket) is here.
pub struct Engine<P: Platform> {
    ctx: NetCtx<P>,
    server_url: Arc<Mutex<Option<String>>>,
    token: Arc<Mutex<Option<String>>>,
    username: Arc<Mutex<Option<String>>>,
    disco_send: Arc<UdpSocket>,
    /// Owns the QUIC runtime; dropped (after `Drop::drop` runs) with the engine.
    _quic: transport::quic::QuicNode,
}

/// kv key holding the QUIC port of the last run.
const QUIC_PORT_KEY: &str = "quic_port";

impl<P: Platform> Engine<P> {
    /// Bind sockets, build identity, and spawn the discovery / tcp / udp / ws /
    /// clipboard / network-change threads.
    pub fn start(platform: P, sink: Arc<dyn EventSink>) -> crate::Result<Engine<P>> {
        Self::start_with_config(platform, sink, CoreConfig::default())
    }

    /// Like `start`, with explicit configuration (STUN servers, ladder rungs).
    pub fn start_with_config(
        platform: P,
        sink: Arc<dyn EventSink>,
        config: CoreConfig,
    ) -> crate::Result<Engine<P>> {
        let platform = Arc::new(platform);

        let raw_id = platform.device_id();
        let node_id = if raw_id.trim().is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            raw_id
        };
        let name = platform.device_name();

        // Same QUIC port as last run when it's free: peers' cached addresses
        // and the router mapping then survive a restart.
        let last_port = platform
            .kv_get(QUIC_PORT_KEY)
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        let device_key = Arc::new(crate::device_key::DeviceKey::load_or_create(&*platform));
        let quic = transport::quic::QuicNode::start(config.keepalive, last_port, &device_key)?;
        platform.kv_set(QUIC_PORT_KEY, &quic.link().port().to_string());

        let identity = Arc::new(Mutex::new(Identity {
            node_id,
            name,
            ip: platform.local_ip(),
            // Retired transports; the registry still expects the fields.
            tcp_port: 0,
            udp_port: 0,
            ws_port: 0,
            quic_port: quic.link().port(),
            ipv6: platform.local_ipv6(),
            mapped_quic: None,
        }));

        let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));
        let ws_conns: WsConns = Arc::new(Mutex::new(HashMap::new()));
        let p2p_conns: P2pConns = Arc::new(Mutex::new(HashMap::new()));
        let punch_waiters: PunchWaiters = Arc::new(Mutex::new(HashMap::new()));
        let rtc_waiters: RtcWaiters = Arc::new(Mutex::new(HashMap::new()));
        let key: KeyHolder = Arc::new(Mutex::new(None));
        let clip_active = Arc::new(AtomicBool::new(false));
        let clip_last = Arc::new(Mutex::new(String::new()));
        let clip_pending: ClipPending = Arc::new(Mutex::new(HashMap::new()));
        let control = Arc::new(ControlRegistry::with_defaults(
            platform.clipboard().is_some(),
        ));
        let installed_apps = Arc::new(Mutex::new(String::from("[]")));
        let subs: Subs = Arc::new(Mutex::new(HashMap::new()));

        // Discovery socket. If multicast bind fails (some networks/permissions),
        // fall back to a plain UDP socket for sending — messaging still works.
        let iface_mode = platform.iface_mode();
        let disco_send: Arc<UdpSocket> = match discovery::bind_multicast(&iface_mode) {
            Ok(recv) => {
                let send = Arc::new(recv.try_clone().map_err(|e| e.to_string())?);
                let peers_c = peers.clone();
                let identity_c = identity.clone();
                let sink_c = sink.clone();
                let send_c = send.clone();
                let key_c = key.clone();
                std::thread::spawn(move || {
                    discovery::discovery_recv_loop(recv, send_c, peers_c, identity_c, sink_c, key_c)
                });
                send
            }
            Err(_) => Arc::new(UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?),
        };

        let ctx = NetCtx {
            platform: platform.clone(),
            identity: identity.clone(),
            key: key.clone(),
            peers: peers.clone(),
            ws_conns: ws_conns.clone(),
            p2p_conns: p2p_conns.clone(),
            punch_waiters: punch_waiters.clone(),
            rtc_waiters: rtc_waiters.clone(),
            quic_waiters: Arc::default(),
            sink: sink.clone(),
            clip_active: clip_active.clone(),
            clip_last: clip_last.clone(),
            clip_history: Arc::default(),
            clip_pending: clip_pending.clone(),
            control: control.clone(),
            installed_apps: installed_apps.clone(),
            device_status: Arc::new(Mutex::new(String::from("{}"))),
            files: Arc::default(),
            device_key,
            peer_keys: Arc::default(),
            ble_links: Arc::default(),
            subs: subs.clone(),
            quic_failed: Arc::default(),
            manual_aliases: Arc::default(),
            decrypt_fail_last: Arc::default(),
            decrypt_fail_count: Arc::default(),
            config: Arc::new(config),
            signaler: Arc::default(),
            shutdown: Arc::default(),
            background: Arc::default(),
            outbox: Arc::default(),
            seen: Arc::default(),
            quic: quic.link(),
            relay: Arc::default(),
            nat: Arc::default(),
            server_url: Arc::new(Mutex::new(None)),
            token: Arc::new(Mutex::new(None)),
            recent_signals: Arc::default(),
        };

        ctx.quic.spawn_accept_loop(ctx.clone());
        if ctx.config.mdns {
            crate::mdns::start(ctx.clone(), ctx.config.mdns_loopback);
        }

        // Long-running loops: clipboard watcher, IP watcher, P2P heartbeat.
        // Peers remembered from last time: reconnect directly, server or not.
        background::load_peer_cache(&ctx);
        background::spawn_all(&ctx);

        let engine = Engine {
            server_url: ctx.server_url.clone(),
            token: ctx.token.clone(),
            ctx,
            username: Arc::new(Mutex::new(None)),
            disco_send,
            _quic: quic,
        };
        // Signed in last time: carry on, and re-register in the background
        // (the FCM token, our addresses) so a push can reach us again.
        if engine.restore_session() {
            if engine.ctx.config.mdns {
                netchange::announce(&engine.ctx, &engine.disco_send);
            }
            let h = engine.handles();
            std::thread::spawn(move || {
                let session = h.server_url.lock().clone().zip(h.token.lock().clone());
                if let Some((url, token)) = session {
                    let _ = peers::refresh(&h.ctx, &url, &token);
                }
            });
        }
        engine.detect_nat();
        background::spawn_ip_watcher(engine.handles());
        background::spawn_port_mapper(engine.handles());
        background::spawn_signal_channel(engine.ctx.clone());
        Ok(engine)
    }

    /// Clonable handles for threads that act on the engine's behalf.
    pub(crate) fn handles(&self) -> netchange::NetHandles<P> {
        netchange::NetHandles {
            ctx: self.ctx.clone(),
            server_url: self.server_url.clone(),
            token: self.token.clone(),
            disco_send: self.disco_send.clone(),
        }
    }

    /// Our NAT type as last measured (`Unknown` until the first STUN check).
    pub fn nat_kind(&self) -> crate::nat::NatKind {
        *self.ctx.nat.lock()
    }

    /// Measure our NAT type in the background (two STUN servers, a few
    /// seconds), then re-register so peers see the result. No-op without STUN
    /// servers (loopback tests).
    pub(crate) fn detect_nat(&self) {
        let h = self.handles();
        std::thread::spawn(move || netchange::detect_nat_now(&h.ctx, &h.server_url, &h.token));
    }

    /// Whether this engine is shutting down (its `Engine` was dropped).
    /// The app went to the background (`true`) or came back (`false`). In
    /// the background, links idle for `CoreConfig::idle_close_after` are
    /// closed to save battery; they re-open when a message needs them.
    pub fn set_background(&self, background: bool) {
        self.ctx.background.store(background, Ordering::Relaxed);
    }

    pub fn is_shut_down(&self) -> bool {
        self.ctx.shutdown.load(Ordering::Relaxed)
    }

    /// Make sure there is a live link to `node_id` if QUIC can provide one
    /// (see `transport::quic::ensure_link`).
    ///
    /// Only call from API threads (`send`, `get_clipboard`): it blocks on the
    /// QUIC runtime, which would panic if called from inside that runtime.
    fn ensure_link(&self, node_id: &str) -> bool {
        transport::quic::ensure_link(&self.ctx, node_id)
    }

    fn net_ctx(&self) -> NetCtx<P> {
        self.ctx.clone()
    }

    /// The transport a message to `node_id` should use right now: the active
    /// persistent link (WS/punch/WebRTC) if any, else the directed transport.
    /// The transport label to report for a send: the live link's kind.
    fn display_transport(&self, node_id: &str) -> String {
        use transport::TransportKind::*;
        match self.ctx.p2p_conns.lock().get(node_id).map(|c| c.kind) {
            Some(Punch) => "punch",
            Some(WebRtc) => "webrtc",
            Some(Quic) => "QUIC",
            Some(Relay) => "relay",
            Some(Ble) => "BLE",
            None => "QUIC",
        }
        .to_string()
    }

    /// Send an already-built control-message JSON to one peer over its link.
    fn send_control(&self, node_id: &str, json: &str) -> crate::Result<()> {
        let from = self.ctx.identity.lock().name.clone();
        transport::send_payload(
            &self.ctx.ws_conns,
            &self.ctx.p2p_conns,
            &self.ctx.key,
            &from,
            node_id,
            json,
        )
    }

    fn emit_peers(&self) {
        peers::emit_peers(&self.ctx);
    }

    // --- read-only state ---

    /// How many incoming envelopes failed to decrypt since start (wrong key,
    /// corruption, or junk from another host). Only the first per sender IP per
    /// minute is shown to the UI; this counts all of them.
    pub fn decrypt_failures(&self) -> u64 {
        self.ctx.decrypt_fail_count.load(Ordering::Relaxed)
    }

    pub fn identity(&self) -> Identity {
        self.ctx.identity.lock().clone()
    }

    pub fn get_peers(&self) -> Vec<Peer> {
        self.ctx.peers.lock().values().cloned().collect()
    }

    /// Every live link with its measured quality (RTT and loss come from
    /// QUIC; other link kinds report them as unknown).
    pub fn link_stats(&self) -> Vec<transport::quality::LinkStats> {
        use transport::quality::{loss_pct, rate, LinkStats};
        let live: Vec<(String, transport::TransportKind)> = transport::live_peers(&self.ctx)
            .into_iter()
            .map(|(id, kind, _)| (id, kind))
            .collect();
        live.into_iter()
            .map(|(node_id, transport)| {
                let measured = self.ctx.quic.path_stats(&node_id);
                let rtt_ms =
                    measured.map(|(rtt, _, _)| rtt.as_millis().min(u32::MAX as u128) as u32);
                let loss = measured.and_then(|(_, lost, sent)| loss_pct(lost, sent));
                let quality = rate(rtt_ms, loss, transport);
                LinkStats {
                    direct: !transport.is_fallback(),
                    rtt_ms,
                    loss_pct: loss,
                    quality,
                    label: quality.label(),
                    node_id,
                    transport,
                }
            })
            .collect()
    }

    pub fn connected_peers(&self) -> Vec<String> {
        self.ctx.p2p_conns.lock().keys().cloned().collect()
    }

    /// Every live link and the transport it rides, by node id.
    pub fn links(&self) -> Vec<(String, transport::TransportKind)> {
        self.ctx
            .p2p_conns
            .lock()
            .iter()
            .map(|(k, c)| (k.clone(), c.kind))
            .collect()
    }

    pub fn is_ready(&self) -> bool {
        self.token.lock().is_some() && self.ctx.key.lock().is_some()
    }

    pub fn session_info(&self) -> SessionInfo {
        SessionInfo {
            username: self.username.lock().clone(),
            server_url: self.server_url.lock().clone(),
            has_key: self.ctx.key.lock().is_some(),
        }
    }

    pub fn saved_session(&self) -> SavedSession {
        SavedSession {
            server_url: self.ctx.platform.kv_get("server_url").unwrap_or_default(),
            username: self.ctx.platform.kv_get("username").unwrap_or_default(),
        }
    }
}

/// The engine is going away. Background loops sleep on it (`wait`) instead
/// of waking every few milliseconds to check, and are woken at once by
/// `store(true)`.
#[derive(Default)]
pub struct Shutdown {
    flag: AtomicBool,
    lock: parking_lot::Mutex<()>,
    woken: parking_lot::Condvar,
}

impl Shutdown {
    pub fn load(&self, order: Ordering) -> bool {
        self.flag.load(order)
    }

    pub fn store(&self, value: bool, order: Ordering) {
        let _guard = self.lock.lock();
        self.flag.store(value, order);
        self.woken.notify_all();
    }

    /// Sleep for `d`, or until shut down. False once shut down.
    pub fn wait(&self, d: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + d;
        let mut guard = self.lock.lock();
        while !self.flag.load(Ordering::Relaxed) {
            if self.woken.wait_until(&mut guard, deadline).timed_out() {
                return !self.flag.load(Ordering::Relaxed);
            }
        }
        false
    }
}

impl<P: Platform> Drop for Engine<P> {
    /// Stop the background loops. (Node analogy: clearing your setIntervals in
    /// a `close()` method; Rust calls `drop` automatically when the value goes
    /// out of scope.)
    fn drop(&mut self) {
        self.ctx.shutdown.store(true, Ordering::Relaxed);
    }
}

// Keep DISCOVERY_PORT reachable for adapters that want it.
pub use crate::discovery::MCAST_GROUP;
pub const DISCO_PORT: u16 = DISCOVERY_PORT;
