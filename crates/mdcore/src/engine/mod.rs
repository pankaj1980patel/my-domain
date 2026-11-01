//! The Engine: owns all networking state, spawns the receive/clipboard threads,
//! and exposes the public API that platform adapters (Tauri commands / JNI
//! shims) thin-wrap. Generic over `Platform`.
//!
//! Threading model: blocking thread-per-connection (a small fixed set of loops
//! plus short-lived per-message threads) — chosen for low cpu/battery.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};
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
use crate::transport::{self, ws, P2pConns};

mod background;
mod ladder;
mod messaging;
mod peers;
mod session;
mod sharing;
mod signaling;

pub use signaling::FirewallStatus;

pub type KeyHolder = Arc<Mutex<Option<[u8; 32]>>>;
/// node_id -> (connection id, outgoing-frame sender). One entry per peer.
pub type WsConns = Arc<Mutex<HashMap<String, (u64, mpsc::Sender<String>)>>>;
/// node_id -> sender awaiting that peer's clipboard pull response.
pub type ClipPending = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
/// subscriber node_id -> the package names whose notifications they want
/// (app-notification pub/sub; held on the producer device).
pub type Subs = Arc<Mutex<HashMap<String, Vec<String>>>>;
/// In-flight punch negotiations: sid -> sender that delivers the peer's answered
/// candidates to the waiting initiator thread.
pub type PunchWaiters = Arc<Mutex<HashMap<String, mpsc::Sender<Vec<crate::signal::Candidate>>>>>;
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
    pub sink: Arc<dyn EventSink>,
    pub clip_active: Arc<AtomicBool>,
    pub clip_last: Arc<Mutex<String>>,
    pub clip_pending: ClipPending,
    pub control: Arc<ControlRegistry<P>>,
    /// Producer's shareable app list as a JSON array string (pushed by the host).
    pub installed_apps: Arc<Mutex<String>>,
    /// App-notification subscriptions (subscriber node_id -> packages).
    pub subs: Subs,
    /// Directed transport for peers with no live persistent channel: "UDP"
    /// (default) or "TCP". A live WS/punch/WebRTC link always wins over this.
    pub directed_transport: Arc<Mutex<String>>,
    /// Rate limiter for "could not decrypt" notices, keyed by sender IP.
    pub decrypt_fail_last: DecryptFailures,
    /// Every envelope that failed to decrypt, notified or not.
    pub decrypt_fail_count: Arc<std::sync::atomic::AtomicU64>,
    /// Start-time configuration (STUN servers, enabled ladder rungs).
    pub config: Arc<CoreConfig>,
    /// Overrides how signals are delivered; `None` = the registry over HTTP.
    pub signaler: Arc<Mutex<Option<Arc<dyn Signaler>>>>,
    /// Set when the `Engine` is dropped; background loops exit on their next tick.
    pub shutdown: Arc<AtomicBool>,
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
            sink: self.sink.clone(),
            clip_active: self.clip_active.clone(),
            clip_last: self.clip_last.clone(),
            clip_pending: self.clip_pending.clone(),
            control: self.control.clone(),
            installed_apps: self.installed_apps.clone(),
            subs: self.subs.clone(),
            directed_transport: self.directed_transport.clone(),
            decrypt_fail_last: self.decrypt_fail_last.clone(),
            decrypt_fail_count: self.decrypt_fail_count.clone(),
            config: self.config.clone(),
            signaler: self.signaler.clone(),
            shutdown: self.shutdown.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub username: Option<String>,
    pub server_url: Option<String>,
    pub has_key: bool,
}

#[derive(Serialize, Default)]
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
}

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

        let tcp_listener = TcpListener::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let tcp_port = tcp_listener.local_addr().map_err(|e| e.to_string())?.port();
        let udp_msg_socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let udp_port = udp_msg_socket
            .local_addr()
            .map_err(|e| e.to_string())?
            .port();
        let ws_listener = TcpListener::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let ws_port = ws_listener.local_addr().map_err(|e| e.to_string())?.port();

        let identity = Arc::new(Mutex::new(Identity {
            node_id,
            name,
            ip: platform.local_ip(),
            tcp_port,
            udp_port,
            ws_port,
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
        let directed_transport = Arc::new(Mutex::new(String::from("UDP")));

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
                std::thread::spawn(move || {
                    discovery::discovery_recv_loop(recv, send_c, peers_c, identity_c, sink_c)
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
            sink: sink.clone(),
            clip_active: clip_active.clone(),
            clip_last: clip_last.clone(),
            clip_pending: clip_pending.clone(),
            control: control.clone(),
            installed_apps: installed_apps.clone(),
            subs: subs.clone(),
            directed_transport: directed_transport.clone(),
            decrypt_fail_last: Arc::default(),
            decrypt_fail_count: Arc::default(),
            config: Arc::new(config),
            signaler: Arc::default(),
            shutdown: Arc::default(),
        };

        // Direct messaging receivers.
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || transport::tcp_recv_loop(tcp_listener, ctx));
        }
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || transport::udp_recv_loop(udp_msg_socket, ctx));
        }
        {
            let ctx = ctx.clone();
            std::thread::spawn(move || ws::ws_server_loop(ws_listener, ctx));
        }

        // Long-running loops: clipboard watcher, IP watcher, P2P heartbeat.
        background::spawn_all(&ctx);

        Ok(Engine {
            ctx,
            server_url: Arc::new(Mutex::new(None)),
            token: Arc::new(Mutex::new(None)),
            username: Arc::new(Mutex::new(None)),
            disco_send,
        })
    }

    /// Whether this engine is shutting down (its `Engine` was dropped).
    pub fn is_shut_down(&self) -> bool {
        self.ctx.shutdown.load(Ordering::Relaxed)
    }

    fn net_ctx(&self) -> NetCtx<P> {
        self.ctx.clone()
    }

    /// The transport a message to `node_id` should use right now: the active
    /// persistent link (WS/punch/WebRTC) if any, else the directed transport.
    fn proto_for(&self, node_id: &str) -> &'static str {
        let directed = self.ctx.directed_transport.lock().clone();
        transport::best_proto(&self.ctx.ws_conns, &directed, node_id)
    }

    /// The transport label to *report* for a send. Punch and WebRTC links reuse
    /// the WS send path (registered in `ws_conns`), so `proto_for` returns "WS"
    /// for them; resolve the real kind from the live-connection registry so the
    /// UI shows "punch"/"webrtc" rather than the internal send-path name.
    fn display_transport(&self, node_id: &str, proto: &'static str) -> String {
        use transport::TransportKind::*;
        if let Some(c) = self.ctx.p2p_conns.lock().get(node_id) {
            return match c.kind {
                Ws => "WS",
                Udp => "UDP",
                Punch => "punch",
                WebRtc => "webrtc",
            }
            .to_string();
        }
        proto.to_string()
    }

    /// Send an already-built control-message JSON to one peer (active connection).
    fn send_control(&self, node_id: &str, json: &str) -> crate::Result<()> {
        let from = self.ctx.identity.lock().name.clone();
        let proto = self.proto_for(node_id);
        transport::send_payload(
            &self.ctx.peers,
            &self.ctx.ws_conns,
            &self.ctx.key,
            &from,
            node_id,
            proto,
            json,
        )
    }

    /// Directed transport setting ("UDP" | "TCP") for peers with no live link.
    pub fn directed_transport(&self) -> String {
        self.ctx.directed_transport.lock().clone()
    }

    /// Set the directed transport ("UDP" default, or "TCP"); anything else maps
    /// to UDP. Only affects peers without a live persistent connection.
    pub fn set_directed_transport(&self, t: &str) {
        let v = if t.trim().eq_ignore_ascii_case("tcp") {
            "TCP"
        } else {
            "UDP"
        };
        *self.ctx.directed_transport.lock() = v.to_string();
    }

    fn emit_peers(&self) {
        let list: Vec<Peer> = self.ctx.peers.lock().values().cloned().collect();
        self.ctx.sink.emit(CoreEvent::PeersUpdated { peers: list });
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

    pub fn connected_peers(&self) -> Vec<String> {
        self.ctx.p2p_conns.lock().keys().cloned().collect()
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
