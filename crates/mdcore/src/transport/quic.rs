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
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

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
/// QUIC keep-alive. NAT routers forget idle UDP mappings (often within
/// 30 s), which would silently close a punched path; a ping every few
/// seconds keeps it open. Also lets each side notice a dead peer.
const KEEP_ALIVE: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);
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

/// Each peer's registered connection: node_id -> (conn id, remote address, handle).
type PeerConns = Arc<Mutex<HashMap<String, (u64, SocketAddr, Connection)>>>;

/// What the rest of the engine uses: the endpoint plus a handle to the
/// runtime. Cheap to clone (both are reference-counted inside).
#[derive(Clone)]
pub struct QuicLink {
    endpoint: Endpoint,
    rt: tokio::runtime::Handle,
    port: u16,
    /// Each peer's registered connection: (conn id, remote address, handle).
    /// Lets a peer that moved replace its stale link, and lets us close ours
    /// when OUR network changes.
    conns: PeerConns,
}

impl QuicNode {
    /// Create the runtime, a self-signed cert, and an endpoint bound to an
    /// ephemeral UDP port on all interfaces.
    pub fn start() -> std::io::Result<QuicNode> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("md-quic")
            .enable_all()
            .build()?;
        let endpoint = {
            let _guard = runtime.enter(); // quinn needs a runtime context to bind
            let mut ep = Endpoint::server(
                server_config().map_err(std::io::Error::other)?,
                SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0),
            )?;
            ep.set_default_client_config(client_config().map_err(std::io::Error::other)?);
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

    /// Close every QUIC connection (our network changed: their paths are
    /// probably dead). Each connection's task then cleans up its link and
    /// reports the peer disconnected, so the caller can re-dial.
    pub fn close_all(&self) {
        let conns: Vec<Connection> = self
            .conns
            .lock()
            .values()
            .map(|(_, _, c)| c.clone())
            .collect();
        for c in conns {
            c.close(0u32.into(), b"network changed");
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
    pub fn connect<P: Platform>(&self, ctx: NetCtx<P>, ip: &str, port: u16) -> Result<(), String> {
        let ip: Ipv4Addr = ip.parse().map_err(|_| "bad peer ip")?;
        let addr = SocketAddr::new(ip.into(), port);
        let endpoint = self.endpoint.clone();
        let conn = self.rt.block_on(async move {
            let connecting = endpoint
                .connect(addr, "mydomain")
                .map_err(|e| format!("QUIC connect failed: {e}"))?;
            tokio::time::timeout(CONNECT_TIMEOUT, connecting)
                .await
                .map_err(|_| "QUIC connect timed out".to_string())?
                .map_err(|e| format!("QUIC handshake failed: {e}"))
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
            Some(server_config().map_err(std::io::Error::other)?),
            socket,
            Arc::new(quinn::TokioRuntime),
        )?;
        endpoint.set_default_client_config(client_config().map_err(std::io::Error::other)?);
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

    // Register as this peer's live link. One link per peer: if one exists,
    // keep it (both sides dialed at once), UNLESS it is a QUIC link from a
    // different address: then the peer moved networks and the old link is
    // stale, so the new one replaces it.
    let my_conn_id = QUIC_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let remote = conn.remote_address();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    {
        let mut g = ctx.ws_conns.lock();
        if let Some((existing_id, _)) = g.get(&peer_id) {
            let stale = ctx
                .quic
                .conns
                .lock()
                .get(&peer_id)
                .filter(|(id, addr, _)| id == existing_id && *addr != remote)
                .map(|(_, _, c)| c.clone());
            match stale {
                Some(old) => {
                    g.remove(&peer_id);
                    old.close(0u32.into(), b"peer moved");
                }
                None => {
                    conn.close(0u32.into(), b"duplicate");
                    return;
                }
            }
        }
        g.insert(peer_id.clone(), (my_conn_id, tx));
    }
    ctx.quic
        .conns
        .lock()
        .insert(peer_id.clone(), (my_conn_id, remote, conn.clone()));
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
            let (ctx, ip) = (ctx.clone(), ip.clone());
            // handle_incoming is synchronous (locks, sink, feature handlers).
            tokio::task::block_in_place(move || handle_incoming(&ctx, &env, ip, "QUIC"));
        }
    }

    // Cleanup: only remove our own entry (a newer link may have replaced it).
    writer.abort();
    {
        let mut qc = ctx.quic.conns.lock();
        if qc.get(&peer_id).is_some_and(|(id, _, _)| *id == my_conn_id) {
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

fn server_config() -> Result<quinn::ServerConfig, Box<dyn std::error::Error + Send + Sync>> {
    let cert = rcgen::generate_simple_self_signed(vec!["mydomain".into()])?;
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.der().clone()], key.into())?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
    cfg.transport_config(transport_config()?);
    Ok(cfg)
}

fn transport_config(
) -> Result<Arc<quinn::TransportConfig>, Box<dyn std::error::Error + Send + Sync>> {
    let mut t = quinn::TransportConfig::default();
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t.max_idle_timeout(Some(IDLE_TIMEOUT.try_into()?));
    Ok(Arc::new(t))
}

fn client_config() -> Result<quinn::ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let mut tls = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert(provider())))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let mut cfg = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
    cfg.transport_config(transport_config()?);
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
