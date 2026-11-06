//! QUIC transport (Phase 2, ADR-0002): one QUIC connection per peer, carrying
//! a bidirectional stream of length-prefixed frames.
//!
//! Why QUIC: it runs over UDP (so it can share a hole-punched socket later),
//! yet gives reliable, ordered, flow-controlled streams with congestion
//! control and built-in TLS 1.3. That replaces three hand-rolled things: a TCP
//! connection per message, lossy UDP chunking, and the unused `wire/` mux.
//!
//! How it plugs in: exactly like the WS, punch and WebRTC links. After the
//! `Hello` exchange the peer is registered in `ctx.ws_conns` with an outgoing
//! queue, and marked live as `TransportKind::Quic`, so chat, clipboard and
//! every feature use it with no other change.
//!
//! Wire format on the stream: `[u32 big-endian length][JSON WsFrame]`, the
//! same `Hello` / `Msg` frames the WebSocket link uses. Messages inside `Msg`
//! are still end-to-end encrypted envelopes.
//!
//! Screen and audio frames can also go as binary records beside that stream
//! (DATAGRAMs and unidirectional streams; see `transport::media`), to a peer
//! whose `Hello` says it reads them.
//!
//! Who the peer is: both ends present a certificate made from their device
//! key (`device_key`), and the key must be the one that device used before
//! (trust on first use). A device from before device keys still connects
//! with a throwaway certificate, unremembered.
//!
//! Threading: the engine's public API is still synchronous, so a small tokio
//! runtime per engine drives QUIC. Blocking engine code (`handle_incoming`)
//! is called through `block_in_place`. The engine queues outgoing frames on
//! a tokio channel (`engine::Route`), which the connection's writer awaits.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::config::KeepAlive;
use crate::device_key::DeviceKey;
use crate::engine::NetCtx;
use crate::model::{Envelope, WsFrame};
use crate::platform::Platform;

use super::media::{self, MediaKind, Sent};
use super::{drop_live, handle_incoming, mark_live, TransportKind};

/// ALPN protocol id: both ends must agree, or the TLS handshake fails. Bump
/// when the frame format changes incompatibly.
const ALPN: &[u8] = b"mydomain/1";
/// Largest frame accepted on a stream (same cap as the TCP receiver).
pub(crate) const MAX_FRAME: usize = 4 * 1024 * 1024;
/// How long a dial may take (QUIC handshake is 1 round trip).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the accepting side of a punched path waits for the dialer.
const PUNCH_ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed for the peer's `Hello` after the connection is up.
const HELLO_TIMEOUT: Duration = Duration::from_secs(3);
/// Connection ids live in their own range so they never collide with WS/punch/rtc.
static QUIC_CONN_SEQ: AtomicU64 = AtomicU64::new(3_000_000);

/// Owns the tokio runtime that drives QUIC. Held by the `Engine` (not the
/// shared context), so the runtime is dropped exactly when the engine is.
/// If background tasks held it instead, the accept loop would keep its own
/// runtime alive forever, and dropping a runtime from inside one of its own
/// tasks panics.
pub struct QuicNode {
    // Field order matters: the link (endpoint) is closed before the runtime
    // shuts down, because struct fields drop in declaration order.
    link: QuicLink,
    _runtime: tokio::runtime::Runtime,
}

/// A peer's registered QUIC connection.
struct PeerConn {
    id: u64,
    remote: SocketAddr,
    conn: Connection,
    /// We dialed it (we're the QUIC client). Only the client side of a QUIC
    /// connection may change networks; the server follows it automatically.
    dialed: bool,
    /// The boot id in the peer's `Hello`.
    boot: String,
    /// Its outgoing media lanes, when the peer's `Hello` said it reads
    /// binary media frames (and this end sends them).
    media: Option<Arc<MediaLanes>>,
}

/// A peer's outgoing unidirectional media streams, one per kind, each
/// opened on first use and fed by a bounded queue.
#[derive(Default)]
struct MediaLanes {
    /// The link is on the local network (`features::nearby`): pictures
    /// keep only the newest frame waiting.
    local: bool,
    /// Audio, screen, camera.
    lanes: Mutex<[Option<Lane>; 3]>,
}

/// A media stream's queue.
enum Lane {
    /// Up to `lane_depth` frames; a full one drops the new frame.
    Queue(tokio::sync::mpsc::Sender<Vec<u8>>),
    /// The newest frame only: a new one replaces one not yet sent (on a
    /// local link a stale picture is worth less than a fresh one).
    Latest(tokio::sync::watch::Sender<Option<Vec<u8>>>),
}

/// Frames a media lane holds before it drops new ones: a second of 20 ms
/// audio, a few snapshots (`None`: the newest only).
fn lane_depth(kind: MediaKind, local: bool) -> Option<usize> {
    match kind {
        MediaKind::Audio => Some(50),
        MediaKind::Screen | MediaKind::Camera if local => None,
        MediaKind::Screen | MediaKind::Camera => Some(8),
        // Never on a stream (see `send_media`).
        MediaKind::Kvm => Some(1),
    }
}

/// Each peer's registered connection, by node id.
type PeerConns = Arc<Mutex<HashMap<String, PeerConn>>>;

/// The `lane` a side connection says in its `Hello`: one over Wi-Fi Direct
/// that carries a file transfer (see `features::wifi_direct`).
pub const LANE_WIFI_DIRECT: &str = "wifi_direct";

/// A side lane to a peer: a second QUIC connection, over another network,
/// that only file transfers use. It never becomes the peer's link (that
/// stays as it was): `features::files` sends on it while it's up.
struct LaneConn {
    id: u64,
    conn: Connection,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
}

/// Each peer's side lane, by node id.
type Lanes = Arc<Mutex<HashMap<String, LaneConn>>>;

/// What the rest of the engine uses: the endpoint plus a handle to the
/// runtime. Cheap to clone (both are reference-counted inside).
#[derive(Clone)]
pub struct QuicLink {
    endpoint: Endpoint,
    rt: tokio::runtime::Handle,
    port: u16,
    /// Each peer's registered connection. Lets a peer that moved replace its
    /// stale link, and tells us which links survive OUR network change.
    conns: PeerConns,
    /// Side lanes (Wi-Fi Direct), apart from the links.
    lanes: Lanes,
    /// The client config dials use when the network changed the
    /// keep-alive (`set_keepalive`); `None`: the endpoint's default.
    dial_config: Arc<parking_lot::Mutex<Option<quinn::ClientConfig>>>,
    /// New for every start; sent in our `Hello` (see `WsFrame::Hello`).
    pub(crate) boot: String,
    /// New for every start: `media::Header::session`.
    pub(crate) media_session: u32,
}

/// One UDP socket for both IP versions: IPv6 with IPv4-mapped addresses
/// allowed (`[::]:0`, v6only off), so we can dial and accept IPv4 and IPv6
/// peers on one port. Falls back to IPv4-only where IPv6 is disabled.
///
/// Tries `preferred` first (the port from the last run, so peers' cached
/// addresses and the router mapping stay valid across restarts), then any
/// free port.
fn bind_udp(preferred: u16) -> std::io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let bind = |port: u16| -> std::io::Result<Socket> {
        let dual = (|| {
            let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
            s.set_only_v6(false)?;
            s.bind(&SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port).into())?;
            Ok::<_, std::io::Error>(s)
        })();
        match dual {
            Ok(s) => Ok(s),
            Err(_) => {
                let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                s.bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port).into())?;
                Ok(s)
            }
        }
    };
    let sock = match preferred {
        0 => bind(0)?,
        p => bind(p).or_else(|_| bind(0))?,
    };
    sock.set_nonblocking(true)?;
    Ok(sock.into())
}

/// Our QUIC endpoint's addresses to hand a peer in a signal: global IPv6,
/// the router mapping (UPnP), and the LAN IPv4, as `ip:port` strings.
pub fn my_addrs<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let id = ctx.identity.lock();
    let mut out = Vec::new();
    if let Some(v6) = id.ipv6.as_deref().and_then(|s| s.parse::<IpAddr>().ok()) {
        out.push(SocketAddr::new(v6, id.quic_port).to_string());
    }
    if let Some(m) = id.mapped_quic.clone() {
        out.push(m);
    }
    if let Ok(v4) = id.ip.parse::<Ipv4Addr>() {
        if !v4.is_unspecified() {
            out.push(SocketAddr::new(v4.into(), id.quic_port).to_string());
        }
    }
    out
}

/// Parse addresses a peer sent us (at most 6, port non-zero, deduped).
pub fn parse_addrs(addrs: &[String]) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for a in addrs.iter().take(6) {
        if let Ok(sa) = a.trim().parse::<SocketAddr>() {
            if sa.port() != 0 && !out.contains(&sa) {
                out.push(sa);
            }
        }
    }
    out
}

/// `::ffff:1.2.3.4` (how a dual-stack socket sees IPv4 peers) -> `1.2.3.4`.
fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

impl QuicNode {
    /// Create the runtime, a self-signed cert, and an endpoint bound to an
    /// ephemeral UDP port on all interfaces.
    pub fn start(
        keepalive: KeepAlive,
        preferred_port: u16,
        key: &DeviceKey,
    ) -> std::io::Result<QuicNode> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("md-quic")
            .enable_all()
            .build()?;
        let endpoint = {
            let _guard = runtime.enter(); // quinn needs a runtime context to bind
            let mut ep = Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(server_config(keepalive, key).map_err(std::io::Error::other)?),
                bind_udp(preferred_port)?,
                Arc::new(quinn::TokioRuntime),
            )?;
            ep.set_default_client_config(
                client_config(keepalive, key).map_err(std::io::Error::other)?,
            );
            ep
        };
        let port = endpoint.local_addr()?.port();
        Ok(QuicNode {
            link: QuicLink {
                endpoint,
                rt: runtime.handle().clone(),
                port,
                conns: Arc::default(),
                lanes: Arc::default(),
                dial_config: Arc::default(),
                boot: uuid::Uuid::new_v4().to_string(),
                media_session: uuid::Uuid::new_v4().as_u128() as u32,
            },
            _runtime: runtime,
        })
    }

    pub fn link(&self) -> QuicLink {
        self.link.clone()
    }
}

impl Drop for QuicNode {
    fn drop(&mut self) {
        // Tell peers we're leaving instead of letting them time out.
        self.link.endpoint.close(0u32.into(), b"bye");
    }
}

impl QuicLink {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// UDP datagrams received so far on the QUIC link to `peer`. A live
    /// peer acknowledges anything we send, so a count that doesn't move
    /// after a send means the link is dead. `None` without a QUIC link.
    pub fn rx_datagrams(&self, peer: &str) -> Option<u64> {
        Some(self.conns.lock().get(peer)?.conn.stats().udp_rx.datagrams)
    }

    /// What went out on the QUIC link to `peer` so far: (UDP bytes, all of
    /// it: our frames, QUIC's headers, acknowledgements; DATAGRAM frames).
    /// `None` without a QUIC link.
    pub fn tx_stats(&self, peer: &str) -> Option<(u64, u64)> {
        let s = self.conns.lock().get(peer)?.conn.stats();
        Some((s.udp_tx.bytes, s.frame_tx.datagram))
    }

    /// Whether `peer`'s QUIC connection `route` (its live link's id) takes
    /// binary media frames.
    pub(crate) fn reads_media(&self, peer: &str, route: u64) -> bool {
        self.conns
            .lock()
            .get(peer)
            .is_some_and(|pc| pc.id == route && pc.media.is_some())
    }

    /// Send a media record to `peer` over its QUIC connection `route`:
    /// audio as a DATAGRAM when it fits one, otherwise on the kind's
    /// stream.
    pub(crate) fn send_media(
        &self,
        peer: &str,
        route: u64,
        kind: MediaKind,
        record: Vec<u8>,
    ) -> Sent {
        let (conn, lanes) = {
            let g = self.conns.lock();
            match g.get(peer) {
                Some(PeerConn {
                    id,
                    conn,
                    media: Some(lanes),
                    ..
                }) if *id == route => (conn.clone(), lanes.clone()),
                _ => return Sent::NotBinary,
            }
        };
        if kind == MediaKind::Kvm {
            // Pointer motion only ever goes as a DATAGRAM: late motion is
            // worth nothing, and the JSON path carries it otherwise.
            let fits = conn
                .max_datagram_size()
                .is_some_and(|max| record.len() <= max);
            return match fits && conn.send_datagram(record.into()).is_ok() {
                true => Sent::Yes,
                false => Sent::NotBinary,
            };
        }
        if kind == MediaKind::Audio
            && conn
                .max_datagram_size()
                .is_some_and(|max| record.len() <= max)
        {
            return match conn.send_datagram(record.into()) {
                Ok(()) => Sent::Yes,
                Err(_) => Sent::NotBinary,
            };
        }
        let i = kind as usize - 1;
        let mut l = lanes.lanes.lock();
        let lane = l[i].get_or_insert_with(|| self.open_lane(conn, kind, lanes.local));
        let sent = match lane {
            Lane::Queue(tx) => match tx.try_send(record) {
                Ok(()) => Sent::Yes,
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => Sent::Dropped,
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => Sent::NotBinary,
            },
            Lane::Latest(tx) => match tx.send(Some(record)) {
                Ok(()) => Sent::Yes,
                Err(_) => Sent::NotBinary,
            },
        };
        if sent == Sent::NotBinary {
            // The stream failed (the connection is going): the JSON path
            // over the same link decides.
            l[i] = None;
        }
        sent
    }

    /// Open a media stream on `conn` and feed it from the returned lane.
    fn open_lane(&self, conn: Connection, kind: MediaKind, local: bool) -> Lane {
        let start = |conn: Connection| async move {
            let mut s = conn.open_uni().await.ok()?;
            s.write_all(media::STREAM_MAGIC).await.ok()?;
            Some(s)
        };
        match lane_depth(kind, local) {
            Some(depth) => {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(depth);
                self.rt.spawn(async move {
                    let Some(mut s) = start(conn).await else {
                        return;
                    };
                    while let Some(record) = rx.recv().await {
                        if write_frame(&mut s, &record).await.is_err() {
                            return;
                        }
                    }
                    let _ = s.finish();
                });
                Lane::Queue(tx)
            }
            None => {
                let (tx, mut rx) = tokio::sync::watch::channel::<Option<Vec<u8>>>(None);
                self.rt.spawn(async move {
                    let Some(mut s) = start(conn).await else {
                        return;
                    };
                    while rx.changed().await.is_ok() {
                        let record = rx.borrow_and_update().clone();
                        if let Some(record) = record {
                            if write_frame(&mut s, &record).await.is_err() {
                                return;
                            }
                        }
                    }
                    let _ = s.finish();
                });
                Lane::Latest(tx)
            }
        }
    }

    /// Measured path stats of the QUIC link to `peer`: (RTT, lost packets,
    /// sent packets). `None` when the peer has no QUIC link.
    pub fn path_stats(&self, peer: &str) -> Option<(Duration, u64, u64)> {
        let conns = self.conns.lock();
        let pc = conns.get(peer)?;
        let path = pc.conn.stats().path;
        Some((pc.conn.rtt(), path.lost_packets, path.sent_packets))
    }

    /// Our network changed. Connections we DIALED survive it: QUIC lets the
    /// client change address, and the peer follows the new one as soon as our
    /// next packet (at most a keep-alive away) arrives from it. Connections
    /// we ACCEPTED are dead (the peer keeps sending to our old address), so
    /// close them; each one's task cleans up its link. Returns the peers
    /// whose links survive, so the caller re-dials only the others.
    pub fn close_accepted(&self) -> Vec<String> {
        let mut kept = Vec::new();
        for (peer, pc) in self.conns.lock().iter() {
            if pc.dialed {
                kept.push(peer.clone());
            } else {
                pc.conn.close(0u32.into(), b"network changed");
            }
        }
        kept
    }

    /// Links made from now on (dialled or accepted) keep alive with
    /// `keepalive` (the network changed: `KeepAlive::for_network`). Links
    /// already up keep theirs; the side that pings more often keeps a NAT
    /// mapping open for both.
    pub fn set_keepalive(&self, keepalive: KeepAlive, key: &DeviceKey) {
        let _guard = self.rt.enter();
        if let Ok(c) = client_config(keepalive, key) {
            *self.dial_config.lock() = Some(c);
        }
        if let Ok(c) = server_config(keepalive, key) {
            self.endpoint.set_server_config(Some(c));
        }
    }

    /// Start a dial to `addr` with the current keep-alive.
    fn dial(&self, addr: SocketAddr) -> Result<quinn::Connecting, quinn::ConnectError> {
        // Starting a connection spawns its driver on the runtime.
        let _guard = self.rt.enter();
        match self.dial_config.lock().clone() {
            Some(cfg) => self.endpoint.connect_with(cfg, addr, "mydomain"),
            None => self.endpoint.connect(addr, "mydomain"),
        }
    }

    /// Close `peer`'s QUIC connection (its task then removes the link).
    /// False when the peer has no QUIC connection.
    pub fn close_peer(&self, peer: &str, reason: &[u8]) -> bool {
        match self.conns.lock().get(peer) {
            Some(pc) => {
                pc.conn.close(0u32.into(), reason);
                true
            }
            None => false,
        }
    }

    /// Start accepting incoming connections for this engine.
    pub fn spawn_accept_loop<P: Platform>(&self, ctx: NetCtx<P>) {
        let endpoint = self.endpoint.clone();
        self.rt.spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        run_connection(conn, ctx, false, TransportKind::Quic, "").await;
                    }
                });
            }
        });
    }

    /// Dial `ip:port` and run the link in the background. Blocks only for the
    /// handshake (at most `CONNECT_TIMEOUT`), so callers learn whether it
    /// worked. Call from a normal thread, not from inside the runtime.
    /// Dial one address (see `connect_any`).
    pub fn connect<P: Platform>(&self, ctx: NetCtx<P>, ip: &str, port: u16) -> Result<(), String> {
        let ip: IpAddr = ip.trim().parse().map_err(|_| "bad peer ip")?;
        self.connect_any(ctx, &[SocketAddr::new(ip, port)])
    }

    /// Dial the peer at every address at once (e.g. its IPv6 and its IPv4)
    /// and keep the first connection that completes the handshake; the other
    /// attempts are dropped. Blocks for at most `CONNECT_TIMEOUT`.
    pub fn connect_any<P: Platform>(
        &self,
        ctx: NetCtx<P>,
        addrs: &[SocketAddr],
    ) -> Result<(), String> {
        if addrs.is_empty() {
            return Err("no address to dial".into());
        }
        // Started here (outside the runtime's tasks), awaited below.
        let dials: Vec<_> = addrs.iter().map(|a| (*a, self.dial(*a))).collect();
        let conn = self.rt.block_on(async move {
            let mut attempts = tokio::task::JoinSet::new();
            for (addr, dialed) in dials {
                attempts.spawn(async move {
                    let connecting =
                        dialed.map_err(|e| format!("QUIC connect to {addr} failed: {e}"))?;
                    tokio::time::timeout(CONNECT_TIMEOUT, connecting)
                        .await
                        .map_err(|_| format!("QUIC connect to {addr} timed out"))?
                        .map_err(|e| format!("QUIC handshake with {addr} failed: {e}"))
                });
            }
            let mut last_err = String::from("QUIC connect failed");
            while let Some(done) = attempts.join_next().await {
                match done {
                    Ok(Ok(conn)) => {
                        attempts.abort_all(); // the losers are dropped (closed)
                        return Ok(conn);
                    }
                    Ok(Err(e)) => last_err = e,
                    Err(_) => {}
                }
            }
            Err(last_err)
        })?;
        self.rt
            .spawn(run_connection(conn, ctx, true, TransportKind::Quic, ""));
        Ok(())
    }

    /// Dial `addr` for a side lane (`lane`, e.g. `LANE_WIFI_DIRECT`) to
    /// the peer there: the same handshake and device-key check as a link,
    /// but it's kept apart (see `LaneConn`). Blocks for at most
    /// `CONNECT_TIMEOUT`; the lane then reports itself to the feature.
    /// Call from a normal thread, not from inside the runtime.
    pub fn dial_lane<P: Platform>(
        &self,
        ctx: NetCtx<P>,
        addr: SocketAddr,
        lane: &'static str,
    ) -> Result<(), String> {
        let endpoint = self.endpoint.clone();
        let conn = self.rt.block_on(async move {
            let connecting = endpoint
                .connect(addr, "mydomain")
                .map_err(|e| format!("QUIC connect to {addr} failed: {e}"))?;
            tokio::time::timeout(CONNECT_TIMEOUT, connecting)
                .await
                .map_err(|_| format!("QUIC connect to {addr} timed out"))?
                .map_err(|e| format!("QUIC handshake with {addr} failed: {e}"))
        })?;
        self.rt
            .spawn(run_connection(conn, ctx, true, TransportKind::Quic, lane));
        Ok(())
    }

    /// `peer` has a side lane up.
    pub fn has_lane(&self, peer: &str) -> bool {
        self.lanes.lock().contains_key(peer)
    }

    /// Queue a frame on `peer`'s side lane; false without one.
    pub fn lane_send(&self, peer: &str, frame: String) -> bool {
        self.lanes
            .lock()
            .get(peer)
            .is_some_and(|l| l.tx.send(frame).is_ok())
    }

    /// Close `peer`'s side lane, if it has one.
    pub fn close_lane(&self, peer: &str) {
        if let Some(l) = self.lanes.lock().remove(peer) {
            l.conn.close(0u32.into(), b"done");
        }
    }

    /// The side lane's measured round trip.
    pub fn lane_rtt(&self, peer: &str) -> Option<Duration> {
        self.lanes.lock().get(peer).map(|l| l.conn.rtt())
    }

    /// Run QUIC over an already hole-punched UDP socket: a fresh endpoint on
    /// that exact socket (so the NAT mappings the punch opened are reused),
    /// then either dial `peer` or accept its connection. Exactly one side must
    /// dial; callers decide that deterministically (lower node id dials).
    /// Returns at once; the link reports itself via `PeerConnected`.
    pub fn run_over_socket<P: Platform>(
        &self,
        ctx: NetCtx<P>,
        socket: std::net::UdpSocket,
        peer: SocketAddr,
        dial: bool,
        kind: TransportKind,
    ) -> std::io::Result<()> {
        let _guard = self.rt.enter();
        socket.set_nonblocking(true)?;
        socket.set_read_timeout(None)?;
        let mut endpoint = Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(
                server_config(ctx.config.keepalive, &ctx.device_key)
                    .map_err(std::io::Error::other)?,
            ),
            socket,
            Arc::new(quinn::TokioRuntime),
        )?;
        endpoint.set_default_client_config(
            client_config(ctx.config.keepalive, &ctx.device_key).map_err(std::io::Error::other)?,
        );
        self.rt.spawn(async move {
            let conn = if dial {
                match endpoint.connect(peer, "mydomain") {
                    Ok(c) => tokio::time::timeout(CONNECT_TIMEOUT, c)
                        .await
                        .ok()
                        .and_then(Result::ok),
                    Err(_) => None,
                }
            } else {
                match tokio::time::timeout(PUNCH_ACCEPT_TIMEOUT, endpoint.accept()).await {
                    Ok(Some(incoming)) => incoming.await.ok(),
                    _ => None,
                }
            };
            if let Some(conn) = conn {
                run_connection(conn, ctx, dial, kind, "").await;
            }
            // `endpoint` lives until here: it owns the punched socket.
            endpoint.close(0u32.into(), b"bye");
        });
        Ok(())
    }
}

/// Drive one connection: exchange `Hello`s, register the link, then pump
/// frames both ways until either side goes away.
async fn run_connection<P: Platform>(
    conn: Connection,
    ctx: NetCtx<P>,
    dialer: bool,
    kind: TransportKind,
    lane: &'static str,
) {
    // The dialer opens the stream; the acceptor waits for it.
    let stream = if dialer {
        conn.open_bi().await
    } else {
        conn.accept_bi().await
    };
    let Ok((mut send, mut recv)) = stream else {
        return;
    };

    let hello = {
        let id = ctx.identity.lock();
        serde_json::to_vec(&WsFrame::Hello {
            node_id: id.node_id.clone(),
            name: id.name.clone(),
            boot: ctx.quic.boot.clone(),
            bin_media: if ctx.config.bin_media {
                media::VERSION
            } else {
                0
            },
            lane: lane.to_string(),
        })
        .unwrap_or_default()
    };
    if write_frame(&mut send, &hello).await.is_err() {
        return;
    }
    let (peer_id, boot, bin_media, their_lane) =
        match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut recv)).await {
            Ok(Ok(bytes)) => match serde_json::from_slice::<WsFrame>(&bytes) {
                Ok(WsFrame::Hello {
                    node_id,
                    boot,
                    bin_media,
                    lane,
                    ..
                }) => (node_id, boot, bin_media, lane),
                _ => return,
            },
            _ => return,
        };
    // A side lane: the dialer said so in its `Hello`.
    let is_lane = if dialer {
        !lane.is_empty()
    } else {
        their_lane == LANE_WIFI_DIRECT
    };

    // The handshake proved which key the peer holds: it must be the one this
    // node id used before (or a first meeting).
    let key = peer_key(&conn);
    match crate::device_key::check(&ctx.peer_keys, &*ctx.platform, &peer_id, key.as_deref()) {
        crate::device_key::Verdict::Accept => {}
        crate::device_key::Verdict::Refuse => {
            conn.close(0u32.into(), b"not the key this device had");
            return;
        }
        crate::device_key::Verdict::Changed { first } => {
            conn.close(0u32.into(), b"not the key this device had");
            crate::engine::key_refused(&ctx, &peer_id, first);
            return;
        }
    }

    if is_lane {
        return run_lane(conn, ctx, peer_id, send, recv).await;
    }

    // A hand-entered peer is listed as `manual:<ip>` until we learn its real
    // node id: now. Swap the placeholder for the real id, keeping its name.
    adopt_manual_peer(&ctx, &peer_id, canonical(conn.remote_address()));

    // Register as this peer's live link. One link per peer. If the peer
    // already has one:
    // - a relay link (or one that never went live): replace it, direct wins;
    // - another direct non-QUIC link (WebRTC): keep it;
    // - a QUIC link from before the peer restarted (another boot id in its
    //   `Hello`): that connection is dead, replace it (it would otherwise
    //   block the peer until it times out);
    // - a QUIC link in the SAME direction from a different address: the peer
    //   moved networks, so the new one replaces the stale one;
    // - a QUIC link in the OPPOSITE direction (both sides dialed at once):
    //   keep the connection whose client has the lower node id. Both ends
    //   apply the same rule, so they keep the same connection.
    let my_conn_id = QUIC_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let remote = canonical(conn.remote_address());
    let me = ctx.identity.lock().node_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    // This link replaced another one, and whether the peer restarted since
    // (a new `boot`).
    let (mut replaced, mut restarted) = (false, false);
    // Nearby media mode (`features::nearby`): a link on the local network
    // takes over from one that goes the long way round, whichever side
    // dialed. Only between devices that both know this rule (both ends
    // must agree which link stays).
    let local = crate::features::diag::is_local_addr(remote.ip());
    let nearby_wins = local && crate::features::nearby::supports(&ctx, &peer_id);
    {
        let mut g = ctx.ws_conns.lock();
        if let Some((existing_id, _)) = g.get(&peer_id) {
            let existing = ctx
                .quic
                .conns
                .lock()
                .get(&peer_id)
                .filter(|pc| pc.id == *existing_id)
                .map(|pc| (pc.dialed, pc.remote, pc.conn.clone(), pc.boot.clone()));
            let replace = match &existing {
                // Not QUIC: replace the relay (or a link that never went
                // live); keep a working direct link (WebRTC), unless this
                // one is local and it isn't.
                None => {
                    !super::is_direct(&ctx, &peer_id)
                        || (nearby_wins
                            && !ctx
                                .p2p_conns
                                .lock()
                                .get(&peer_id)
                                .is_some_and(|c| crate::features::diag::is_lan(c.addr)))
                }
                Some((_, _, _, was_boot)) if !boot.is_empty() && *was_boot != boot => true,
                Some((_, was_remote, _, _))
                    if nearby_wins && !crate::features::diag::is_local_addr(was_remote.ip()) =>
                {
                    true
                }
                Some((was_dialed, was_remote, _, _)) if *was_dialed == dialer => {
                    *was_remote != remote
                }
                Some(_) => dialer == (me < peer_id),
            };
            if !replace {
                conn.close(0u32.into(), b"duplicate");
                return;
            }
            replaced = existing.is_some();
            restarted = matches!(&existing, Some((_, _, _, was_boot)) if !boot.is_empty() && *was_boot != boot);
            g.remove(&peer_id);
            if let Some((_, _, old, _)) = existing {
                old.close(0u32.into(), b"replaced");
            }
        }
        g.insert(
            peer_id.clone(),
            (my_conn_id, crate::engine::Route::direct(tx)),
        );
        // Record the QUIC side under the SAME lock: a second connection
        // registering in between would otherwise find a link with no QUIC
        // record, mistake it for a dead non-QUIC link, and orphan it.
        ctx.quic.conns.lock().insert(
            peer_id.clone(),
            PeerConn {
                id: my_conn_id,
                remote,
                conn: conn.clone(),
                dialed: dialer,
                boot,
                media: (ctx.config.bin_media && bin_media >= media::VERSION).then(|| {
                    Arc::new(MediaLanes {
                        local,
                        ..Default::default()
                    })
                }),
            },
        );
    }
    let was_live = super::is_live(&ctx, &peer_id);
    mark_live(&ctx, &peer_id, kind, Some(remote));
    if was_live {
        // It took over from another direct link: say how it's carried now.
        super::relabel(&ctx, &peer_id, kind, Some(remote));
    }
    if was_live && restarted {
        super::peer_came_up(&ctx, &peer_id);
    } else if was_live && replaced {
        // A message may have gone out on the link just closed: resend what
        // isn't acknowledged (the receiver drops repeats).
        super::outbox::flush(&ctx, &peer_id);
    }

    // Writer: the frames the engine queued for this peer (the queue closes
    // when the link is removed).
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write_frame(&mut send, frame.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    // Binary media frames from the peer, if this end reads them (it said so
    // in its `Hello`): DATAGRAMs, and the peer's media streams.
    let media_reader = ctx
        .config
        .bin_media
        .then(|| tokio::spawn(read_media(conn.clone(), ctx.clone(), peer_id.clone())));

    // Reader: every `Msg` frame goes through the normal incoming path.
    let ip = remote.ip().to_string();
    while let Ok(bytes) = read_frame(&mut recv).await {
        if let Ok(WsFrame::Msg { nonce, ciphertext }) = serde_json::from_slice::<WsFrame>(&bytes) {
            let env = Envelope { nonce, ciphertext };
            let (ctx, ip, from) = (ctx.clone(), ip.clone(), peer_id.clone());
            // handle_incoming is synchronous (locks, sink, feature handlers).
            tokio::task::block_in_place(move || {
                handle_incoming(&ctx, &env, Some(&from), ip, "QUIC")
            });
        }
    }

    // Cleanup: only remove our own entry (a newer link may have replaced it).
    writer.abort();
    if let Some(r) = media_reader {
        r.abort();
    }
    // The PEER closed this as the loser of a simultaneous dial (or as stale):
    // the winning connection is on its way. Give it a moment to take over
    // before reporting the peer disconnected, so the UI doesn't flap.
    if let Some(quinn::ConnectionError::ApplicationClosed(c)) = conn.close_reason() {
        if c.reason.as_ref() == b"replaced" || c.reason.as_ref() == b"duplicate" {
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }
    {
        let mut qc = ctx.quic.conns.lock();
        if qc.get(&peer_id).is_some_and(|pc| pc.id == my_conn_id) {
            qc.remove(&peer_id);
        }
    }
    let ours = {
        let mut g = ctx.ws_conns.lock();
        let ours = g
            .get(&peer_id)
            .map(|(id, _)| *id == my_conn_id)
            .unwrap_or(false);
        if ours {
            g.remove(&peer_id);
        }
        ours
    };
    // A local link that went mid-stream falls back to the relay at once
    // (`features::nearby`); otherwise the peer is gone.
    if ours {
        let fell_back = tokio::task::block_in_place(|| {
            crate::features::nearby::link_lost(&ctx, &peer_id, remote)
        });
        if !fell_back {
            // Why it went (closed here: the closer already said why).
            if let Some(why) = conn.close_reason().as_ref().and_then(drop_reason) {
                crate::features::reconnect::closing(&ctx, &peer_id, why);
            }
            drop_live(&ctx, &peer_id);
        }
    }
}

/// Why a QUIC link went, for `features::reconnect`: `None` when this end
/// closed it (whoever did said why).
fn drop_reason(e: &quinn::ConnectionError) -> Option<&'static str> {
    use quinn::ConnectionError as E;
    Some(match e {
        E::LocallyClosed => return None,
        E::TimedOut => "timeout",
        E::ApplicationClosed(c) => match c.reason.as_ref() {
            // It closed an unused link to save battery: asleep.
            b"idle" => "peer_idle",
            b"bye" => "peer_quit",
            b"network changed" => "network_change",
            _ => "closed",
        },
        _ => "lost",
    })
}

/// Read `peer`'s binary media frames on `conn` until it closes.
/// Drive a side lane (see `LaneConn`): only to one of your devices that
/// this one expects a lane from right now; frames go through the normal
/// incoming path, so the same encryption applies.
async fn run_lane<P: Platform>(
    conn: Connection,
    ctx: NetCtx<P>,
    peer_id: String,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let wanted = ctx.peers.lock().contains_key(&peer_id)
        && crate::features::wifi_direct::lane_wanted(&ctx, &peer_id);
    if !wanted {
        conn.close(0u32.into(), b"no lane expected");
        return;
    }
    let my_id = QUIC_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(old) = ctx.quic.lanes.lock().insert(
        peer_id.clone(),
        LaneConn {
            id: my_id,
            conn: conn.clone(),
            tx,
        },
    ) {
        old.conn.close(0u32.into(), b"replaced");
    }
    {
        let (ctx, peer) = (ctx.clone(), peer_id.clone());
        tokio::task::block_in_place(move || crate::features::wifi_direct::lane_up(&ctx, &peer));
    }
    let writer = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write_frame(&mut send, frame.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let ip = canonical(conn.remote_address()).ip().to_string();
    while let Ok(bytes) = read_frame(&mut recv).await {
        if let Ok(WsFrame::Msg { nonce, ciphertext }) = serde_json::from_slice::<WsFrame>(&bytes) {
            let env = Envelope { nonce, ciphertext };
            let (ctx, ip, from) = (ctx.clone(), ip.clone(), peer_id.clone());
            tokio::task::block_in_place(move || {
                handle_incoming(&ctx, &env, Some(&from), ip, "Wi-Fi Direct")
            });
        }
    }
    writer.abort();
    let ours = {
        let mut l = ctx.quic.lanes.lock();
        let ours = l.get(&peer_id).is_some_and(|x| x.id == my_id);
        if ours {
            l.remove(&peer_id);
        }
        ours
    };
    if ours {
        let ctx2 = ctx.clone();
        tokio::task::block_in_place(move || {
            crate::features::wifi_direct::lane_down(&ctx2, &peer_id)
        });
    }
}

async fn read_media<P: Platform>(conn: Connection, ctx: NetCtx<P>, peer: String) {
    let datagrams = {
        let (conn, ctx, peer) = (conn.clone(), ctx.clone(), peer.clone());
        async move {
            while let Ok(d) = conn.read_datagram().await {
                tokio::task::block_in_place(|| media::on_record(&ctx, &peer, &d));
            }
        }
    };
    let streams = async move {
        while let Ok(s) = conn.accept_uni().await {
            tokio::spawn(read_media_stream(s, ctx.clone(), peer.clone()));
        }
    };
    tokio::join!(datagrams, streams);
}

/// One media stream: the magic, then length-prefixed records.
async fn read_media_stream<P: Platform>(mut recv: RecvStream, ctx: NetCtx<P>, peer: String) {
    let mut magic = [0u8; 4];
    if recv.read_exact(&mut magic).await.is_err() || &magic != media::STREAM_MAGIC {
        return;
    }
    while let Ok(record) = read_frame(&mut recv).await {
        tokio::task::block_in_place(|| media::on_record(&ctx, &peer, &record));
    }
}

async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame too large",
        ));
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    send.write_all(bytes).await?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len)
        .await
        .map_err(std::io::Error::other)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(std::io::Error::other)?;
    Ok(buf)
}

// --- TLS ---------------------------------------------------------------------

fn server_config(
    keepalive: KeepAlive,
    key: &DeviceKey,
) -> Result<quinn::ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let (cert, private) = key.certificate()?;
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(AnyCert(provider())))
        .with_single_cert(vec![cert], private)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    cfg.transport_config(transport_config(keepalive)?);
    // Follow a client that changes address (Wi-Fi -> mobile data). On by
    // default in quinn; explicit because the network-change logic relies on it.
    cfg.migration(true);
    Ok(cfg)
}

fn transport_config(
    keepalive: KeepAlive,
) -> Result<Arc<quinn::TransportConfig>, Box<dyn std::error::Error + Send + Sync>> {
    // Keep-alive holds NAT mappings open (a punched path dies when its
    // mapping expires) and lets each side notice a dead peer; see KeepAlive.
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(keepalive.interval);
    t.max_idle_timeout(Some(keepalive.idle_timeout.try_into()?));
    Ok(Arc::new(t))
}

fn client_config(
    keepalive: KeepAlive,
    key: &DeviceKey,
) -> Result<quinn::ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let (cert, private) = key.certificate()?;
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert(provider())))
        .with_client_auth_cert(vec![cert], private)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    cfg.transport_config(transport_config(keepalive)?);
    Ok(cfg)
}

/// The device key in the certificate the peer presented, if it has one.
fn peer_key(conn: &Connection) -> Option<String> {
    let certs = conn
        .peer_identity()?
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .ok()?;
    crate::device_key::key_in_cert(certs.first()?.as_ref())
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts any certificate in the handshake, but checks that the handshake
/// was signed by the key in it, so the key is proven. Whether that key is
/// the right one for the device is decided after the `Hello`, when we know
/// which device it claims to be (`device_key::accept`).
#[derive(Debug)]
struct AnyCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The accepting side asks for the dialer's certificate too. Optional: a
/// device from before device keys has none to give.
impl rustls::server::danger::ClientCertVerifier for AnyCert {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// If `remote` is the address of a hand-entered `manual:<ip>` peer, replace
/// that placeholder with an entry under the peer's real node id.
fn adopt_manual_peer<P: Platform>(ctx: &NetCtx<P>, peer_id: &str, remote: SocketAddr) {
    let manual_id = format!("manual:{}", remote.ip());
    let adopted = {
        let mut peers = ctx.peers.lock();
        if peers.contains_key(&manual_id) {
            ctx.manual_aliases
                .lock()
                .insert(manual_id.clone(), peer_id.to_string());
        }
        match peers.remove(&manual_id) {
            Some(mut p) if !peers.contains_key(peer_id) => {
                p.node_id = peer_id.to_string();
                p.quic_port = remote.port();
                peers.insert(peer_id.to_string(), p);
                true
            }
            Some(p) => {
                // Already known under its real id: just drop the placeholder.
                drop(p);
                true
            }
            None => false,
        }
    };
    if adopted {
        crate::engine::emit_peers(ctx);
    }
}

/// How long to wait before auto-dialing a peer again after a failed attempt.
const QUIC_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Make sure there is a live link to `node_id` if QUIC can provide one:
/// true at once if a link exists; otherwise dials the peer's QUIC addresses
/// (at most ~3 s) unless auto-QUIC is off, the peer has no QUIC port, or a
/// dial failed in the last `QUIC_RETRY_AFTER`. True if a link is up after.
///
/// Blocks on the QUIC runtime: never call from inside it.
pub fn ensure_link<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    if ctx.ws_conns.lock().contains_key(node_id) {
        return true;
    }
    if !ctx.config.auto_quic {
        return false;
    }
    let Some(peer) = ctx.peers.lock().get(node_id).cloned() else {
        return false;
    };
    if peer.quic_port == 0 {
        return false;
    }
    if let Some(t) = ctx.quic_failed.lock().get(node_id) {
        if t.elapsed() < QUIC_RETRY_AFTER {
            return false;
        }
    }
    let ok = ctx
        .quic
        .connect_any(ctx.clone(), &peer.quic_targets())
        .is_ok()
        && wait_for_link(ctx, node_id, Duration::from_secs(2));
    if ok {
        ctx.quic_failed.lock().remove(node_id);
    } else {
        ctx.quic_failed
            .lock()
            .insert(node_id.to_string(), std::time::Instant::now());
    }
    ok
}

/// Wait until `node_id` has an entry in `ws_conns` (a link registered its
/// outgoing queue). The QUIC dial returns after the handshake; registration
/// happens a moment later, after the `Hello` exchange.
fn wait_for_link<P: Platform>(ctx: &NetCtx<P>, node_id: &str, max: Duration) -> bool {
    let deadline = std::time::Instant::now() + max;
    while std::time::Instant::now() < deadline {
        if ctx.ws_conns.lock().contains_key(node_id) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}
