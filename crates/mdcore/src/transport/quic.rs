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
//! SECURITY NOTE (until Phase 4): each engine uses a throwaway self-signed
//! certificate and the dialer accepts ANY certificate. The QUIC connection is
//! encrypted, but the peer is not authenticated (same as today's WS `Hello`).
//! Phase 4 derives certificates from per-device identity keys and pins them,
//! which makes the handshake itself prove who the peer is.
//!
//! Threading: the engine's public API is still synchronous, so a small tokio
//! runtime per engine drives QUIC. Blocking engine code (`handle_incoming`)
//! is called through `block_in_place`, and a tiny bridge thread forwards the
//! engine's synchronous outgoing queue (`std::sync::mpsc`) into an async one.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::config::KeepAlive;
use crate::engine::NetCtx;
use crate::model::{Envelope, WsFrame};
use crate::platform::Platform;

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
}

/// Each peer's registered connection, by node id.
type PeerConns = Arc<Mutex<HashMap<String, PeerConn>>>;

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
    pub fn start(keepalive: KeepAlive, preferred_port: u16) -> std::io::Result<QuicNode> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("md-quic")
            .enable_all()
            .build()?;
        let endpoint = {
            let _guard = runtime.enter(); // quinn needs a runtime context to bind
            let mut ep = Endpoint::new(
                quinn::EndpointConfig::default(),
                Some(server_config(keepalive).map_err(std::io::Error::other)?),
                bind_udp(preferred_port)?,
                Arc::new(quinn::TokioRuntime),
            )?;
            ep.set_default_client_config(client_config(keepalive).map_err(std::io::Error::other)?);
            ep
        };
        let port = endpoint.local_addr()?.port();
        Ok(QuicNode {
            link: QuicLink {
                endpoint,
                rt: runtime.handle().clone(),
                port,
                conns: Arc::default(),
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
                        run_connection(conn, ctx, false, TransportKind::Quic).await;
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
        let addrs = addrs.to_vec();
        let endpoint = self.endpoint.clone();
        let conn = self.rt.block_on(async move {
            let mut attempts = tokio::task::JoinSet::new();
            for addr in addrs {
                let ep = endpoint.clone();
                attempts.spawn(async move {
                    let connecting = ep
                        .connect(addr, "mydomain")
                        .map_err(|e| format!("QUIC connect to {addr} failed: {e}"))?;
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
            .spawn(run_connection(conn, ctx, true, TransportKind::Quic));
        Ok(())
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
            Some(server_config(ctx.config.keepalive).map_err(std::io::Error::other)?),
            socket,
            Arc::new(quinn::TokioRuntime),
        )?;
        endpoint.set_default_client_config(
            client_config(ctx.config.keepalive).map_err(std::io::Error::other)?,
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
                run_connection(conn, ctx, dial, kind).await;
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
        })
        .unwrap_or_default()
    };
    if write_frame(&mut send, &hello).await.is_err() {
        return;
    }
    let peer_id = match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut recv)).await {
        Ok(Ok(bytes)) => match serde_json::from_slice::<WsFrame>(&bytes) {
            Ok(WsFrame::Hello { node_id, .. }) => node_id,
            _ => return,
        },
        _ => return,
    };

    // A hand-entered peer is listed as `manual:<ip>` until we learn its real
    // node id: now. Swap the placeholder for the real id, keeping its name.
    adopt_manual_peer(&ctx, &peer_id, canonical(conn.remote_address()));

    // Register as this peer's live link. One link per peer. If the peer
    // already has one:
    // - a relay link (or one that never went live): replace it, direct wins;
    // - another direct non-QUIC link (WebRTC): keep it;
    // - a QUIC link in the SAME direction from a different address: the peer
    //   moved networks, so the new one replaces the stale one;
    // - a QUIC link in the OPPOSITE direction (both sides dialed at once):
    //   keep the connection whose client has the lower node id. Both ends
    //   apply the same rule, so they keep the same connection.
    let my_conn_id = QUIC_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let remote = canonical(conn.remote_address());
    let me = ctx.identity.lock().node_id.clone();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    {
        let mut g = ctx.ws_conns.lock();
        if let Some((existing_id, _)) = g.get(&peer_id) {
            let existing = ctx
                .quic
                .conns
                .lock()
                .get(&peer_id)
                .filter(|pc| pc.id == *existing_id)
                .map(|pc| (pc.dialed, pc.remote, pc.conn.clone()));
            let replace = match &existing {
                // Not QUIC: replace the relay (or a link that never went
                // live); keep a working direct link (WebRTC).
                None => !super::is_direct(&ctx, &peer_id),
                Some((was_dialed, was_remote, _)) if *was_dialed == dialer => *was_remote != remote,
                Some(_) => dialer == (me < peer_id),
            };
            if !replace {
                conn.close(0u32.into(), b"duplicate");
                return;
            }
            g.remove(&peer_id);
            if let Some((_, _, old)) = existing {
                old.close(0u32.into(), b"replaced");
            }
        }
        g.insert(peer_id.clone(), (my_conn_id, tx));
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
            },
        );
    }
    mark_live(&ctx, &peer_id, kind, Some(remote));

    // Bridge: the engine queues outgoing frames on a std channel; forward them
    // into an async channel the writer task awaits. The thread ends when the
    // engine drops the std sender (link removed) or the writer is gone.
    let (atx, mut arx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        while let Ok(frame) = rx.recv() {
            if atx.send(frame).is_err() {
                break;
            }
        }
    });
    let writer = tokio::spawn(async move {
        while let Some(frame) = arx.recv().await {
            if write_frame(&mut send, frame.as_bytes()).await.is_err() {
                break;
            }
        }
    });

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
    if ours {
        drop_live(&ctx, &peer_id);
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
) -> Result<quinn::ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let cert = rcgen::generate_simple_self_signed(vec!["mydomain".into()])?;
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())?;
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
) -> Result<quinn::ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider())))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    cfg.transport_config(transport_config(keepalive)?);
    Ok(cfg)
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts any server certificate, but still checks that the handshake was
/// signed by the key in that certificate (so the TLS session is sound; only
/// the *identity* is unchecked). Replaced by key pinning in Phase 4.
#[derive(Debug)]
struct AcceptAnyCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
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
