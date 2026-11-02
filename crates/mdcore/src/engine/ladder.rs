//! The connect ladder: try the cheapest way to reach a peer first, fall back
//! rung by rung (LAN UDP, direct WS, signaled WS, hole punch, WebRTC).
//! See learnings/01-architecture.md.

use super::signaling::{relay_target, send_signal_via};
use super::*;

impl<P: Platform> Engine<P> {
    /// Kick off the connection ladder on a background thread (returns at once).
    /// Progress is reported via `CoreEvent::ConnectProgress`; success lands as a
    /// `PeerConnected` event when any rung establishes a live link.
    pub fn connect(&self, node_id: &str) -> crate::Result<()> {
        // Validate the peer exists up front so the button gets immediate feedback.
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        let ctx = self.net_ctx();
        let server_url = self.server_url.clone();
        let token = self.token.clone();
        let node_id = node_id.to_string();
        std::thread::spawn(move || run_connect_ladder(ctx, server_url, token, node_id));
        Ok(())
    }
}

/// Whether a peer reported it can't receive inbound connections. LAN/manual
/// peers default to `false` (unknown → attempt direct); registry peers carry the
/// real flag the device self-reported via `report_firewall`.
fn peer_inbound_blocked(peer: &Peer) -> bool {
    peer.inbound_blocked
}

// ---------------------------------------------------------------------------
// Connection ladder (runs off-thread; uses only NetCtx + the server creds, so
// it doesn't need `&Engine`). Each rung emits ConnectProgress and bails to the
// next on timeout. Rungs are ordered cheapest-first.
// ---------------------------------------------------------------------------

/// Same-LAN heuristic: LAN-discovered peer, or our IPs share a /24.
fn same_lan<P: Platform>(ctx: &NetCtx<P>, peer: &Peer) -> bool {
    if peer.source == Source::Lan {
        return true;
    }
    let my_ip = ctx.identity.lock().ip.clone();
    match (my_ip.parse::<Ipv4Addr>(), peer.ip.parse::<Ipv4Addr>()) {
        (Ok(a), Ok(b)) => a.octets()[..3] == b.octets()[..3],
        _ => false,
    }
}

fn is_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    ctx.p2p_conns.lock().contains_key(node_id)
}

/// Poll for the peer to become live, up to `ms` milliseconds.
/// Wait for a link that isn't through the relay.
fn wait_direct<P: Platform>(ctx: &NetCtx<P>, node_id: &str, ms: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(ms);
    while std::time::Instant::now() < deadline {
        if transport::is_direct(ctx, node_id) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    transport::is_direct(ctx, node_id)
}

fn wait_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str, ms: u64) -> bool {
    let steps = ms / 100;
    for _ in 0..steps {
        std::thread::sleep(Duration::from_millis(100));
        if is_live(ctx, node_id) {
            return true;
        }
    }
    is_live(ctx, node_id)
}

/// Send one UDP ping and wait briefly for the pong to flip the peer live.
fn udp_ping_connect<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    let (me, name) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.name.clone())
    };
    let ping = serde_json::json!({ "type": "p2p_ping", "from": me }).to_string();
    if transport::send_payload(
        &ctx.peers,
        &ctx.ws_conns,
        &ctx.key,
        &name,
        node_id,
        transport::SendPath::Udp,
        &ping,
    )
    .is_err()
    {
        return false;
    }
    wait_direct(ctx, node_id, 2000)
}

pub(super) fn run_connect_ladder<P: Platform>(
    ctx: NetCtx<P>,
    server_url: Arc<Mutex<Option<String>>>,
    token: Arc<Mutex<Option<String>>>,
    node_id: String,
) {
    run_ladder(ctx, server_url, token, node_id, true);
}

/// How often, and how many times, a relayed peer retries the direct rungs.
const UPGRADE_EVERY: Duration = Duration::from_secs(60);
const UPGRADE_TRIES: u32 = 10;

/// The ladder. `retry_later`: if we end up on the relay, keep retrying the
/// direct rungs in the background (false for those retries themselves).
fn run_ladder<P: Platform>(
    ctx: NetCtx<P>,
    server_url: Arc<Mutex<Option<String>>>,
    token: Arc<Mutex<Option<String>>>,
    node_id: String,
    retry_later: bool,
) {
    let emit = |stage: &str, detail: Option<String>| {
        ctx.sink.emit(CoreEvent::ConnectProgress {
            node_id: node_id.clone(),
            stage: stage.into(),
            detail,
        });
    };

    // Already direct: nothing to do. (Relayed: carry on, to upgrade.)
    if transport::is_direct(&ctx, &node_id) {
        emit("connected", None);
        return;
    }
    let Some(peer) = ctx.peers.lock().get(&node_id).cloned() else {
        emit("failed", Some("peer not found".into()));
        return;
    };

    // Rung 1 — same-LAN UDP ping/pong (no FCM, no server).
    let rungs = ctx.config.rungs.clone();

    // Rung 0 — direct QUIC to the peer's advertised address. One handshake
    // round trip; works on the LAN and to any peer that accepts inbound UDP.
    if rungs.quic && peer.quic_port != 0 {
        emit("quic", None);
        if ctx
            .quic
            .connect_any(ctx.clone(), &peer.quic_targets())
            .is_ok()
            && wait_direct(&ctx, &node_id, 3000)
        {
            return; // the link emits PeerConnected
        }
    }

    if rungs.lan_udp && same_lan(&ctx, &peer) {
        emit("udp_ping", None);
        if udp_ping_connect(&ctx, &node_id) {
            emit("connected", Some("udp".into()));
            return;
        }
    }

    // Rung 2 — direct WS if the peer advertises a reachable WS server.
    if rungs.direct_ws && peer.ws_port != 0 && peer.ws_open && !peer_inbound_blocked(&peer) {
        emit("direct_ws", None);
        if ws::ws_connect(ctx.clone(), &peer.ip, peer.ws_port).is_ok()
            && wait_direct(&ctx, &node_id, 3000)
        {
            return; // WS handler emits PeerConnected
        }
    }

    // Rung 3 — signaled QUIC: swap QUIC addresses through a signal and BOTH
    // sides dial. Works when only one side can be dialed, and with addresses
    // fresher than the registry's. (1 signal round trip.)
    if rungs.signaled_quic {
        emit("signaled_quic", None);
        let sid = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = mpsc::channel::<Vec<String>>();
        ctx.quic_waiters.lock().insert(sid.clone(), tx);
        let offer = Signal::QuicOffer {
            sid: sid.clone(),
            addrs: transport::quic::my_addrs(&ctx),
        };
        if send_signal_via(&server_url, &token, &ctx, &node_id, &offer).is_ok() {
            if let Ok(addrs) = rx.recv_timeout(Duration::from_secs(8)) {
                let targets = transport::quic::parse_addrs(&addrs);
                let _ = ctx.quic.connect_any(ctx.clone(), &targets);
            }
            // The peer dials us too, so a link may exist even if ours failed.
            if wait_direct(&ctx, &node_id, 3000) {
                ctx.quic_waiters.lock().remove(&sid);
                return;
            }
        }
        ctx.quic_waiters.lock().remove(&sid);
    }

    // Rung 3b — FCM-coordinated WS: ask the peer to open its WS and reply WsReady,
    // which our on_signal dials. (1 FCM round trip.)
    if rungs.signaled_ws {
        emit("via_signal", None);
        let sid = uuid::Uuid::new_v4().to_string();
        if send_signal_via(
            &server_url,
            &token,
            &ctx,
            &node_id,
            &Signal::StartWs { sid },
        )
        .is_ok()
            && wait_direct(&ctx, &node_id, 8000)
        {
            return;
        }
    }

    // Relay through the server, BEFORE the slow rungs: if the quick rungs
    // failed, messages flow through the relay right away, while hole
    // punching and WebRTC keep trying to upgrade the link in the background
    // (a direct link replaces the relayed one when it comes up).
    if rungs.relay && !is_live(&ctx, &node_id) {
        if let Some((url, tok)) = relay_target(&ctx, &server_url, &token) {
            emit("relay", None);
            if transport::relay::ensure_relay(&ctx, &url, &tok).is_ok() {
                transport::relay::add_relay_link(&ctx, &node_id);
                let sid = uuid::Uuid::new_v4().to_string();
                let _ = send_signal_via(
                    &server_url,
                    &token,
                    &ctx,
                    &node_id,
                    &Signal::RelayOffer { sid },
                );
                transport::relay::send_hello(&ctx, &node_id);
                wait_live(&ctx, &node_id, 8000);
            }
        }
    }

    // Both behind symmetric NATs: STUN addresses are useless to the other
    // side, so punching and WebRTC's direct path can only time out (~40 s).
    // Skip straight to the relay.
    let hopeless = ctx.nat.lock().hopeless_with(peer.nat);
    if hopeless && (rungs.punch || rungs.webrtc) {
        emit(
            "nat_skip",
            Some("both devices are behind symmetric NATs; skipping hole punch and WebRTC".into()),
        );
    }

    // Rung 4 — UDP hole punch. Gather our candidates up front, offer them in one
    // signal, and punch toward the peer's answered candidates (1 FCM round trip).
    if rungs.punch && !hopeless {
        emit("punching", None);
        if let Some((socket, my_cands)) = transport::punch::gather_candidates(&ctx) {
            let sid = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = mpsc::channel::<Vec<crate::signal::Candidate>>();
            ctx.punch_waiters.lock().insert(sid.clone(), tx);
            let offer = Signal::PunchOffer {
                sid: sid.clone(),
                candidates: my_cands,
                start_in_ms: 0,
            };
            if send_signal_via(&server_url, &token, &ctx, &node_id, &offer).is_ok() {
                if let Ok(peer_cands) = rx.recv_timeout(Duration::from_secs(8)) {
                    ctx.punch_waiters.lock().remove(&sid);
                    transport::punch::punch_and_run(
                        ctx.clone(),
                        socket,
                        peer_cands,
                        node_id.clone(),
                    );
                    // hole_punch (≤8s) + bidirectional confirmation (≤3s).
                    if wait_direct(&ctx, &node_id, 12000) {
                        return;
                    }
                } else {
                    ctx.punch_waiters.lock().remove(&sid);
                }
            } else {
                ctx.punch_waiters.lock().remove(&sid);
            }
        }
    }

    // Rung 5 — WebRTC data channel. Gather candidates into one SDP offer, send
    // it, and on the single answer drive ICE/DTLS/SCTP peer-to-peer (1 FCM RTT).
    #[cfg(feature = "webrtc")]
    if rungs.webrtc && !hopeless {
        emit("webrtc", None);
        if let Some((rtc, socket, pending, offer_sdp)) = transport::rtc::create_offer(&ctx) {
            let sid = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = mpsc::channel::<String>();
            ctx.rtc_waiters.lock().insert(sid.clone(), tx);
            let offer = Signal::SdpOffer {
                sid: sid.clone(),
                sdp: offer_sdp,
            };
            if send_signal_via(&server_url, &token, &ctx, &node_id, &offer).is_ok() {
                if let Ok(answer_sdp) = rx.recv_timeout(Duration::from_secs(10)) {
                    ctx.rtc_waiters.lock().remove(&sid);
                    if transport::rtc::apply_answer_and_run(
                        ctx.clone(),
                        rtc,
                        socket,
                        pending,
                        &answer_sdp,
                        node_id.clone(),
                    ) && wait_direct(&ctx, &node_id, 12000)
                    {
                        return;
                    }
                } else {
                    ctx.rtc_waiters.lock().remove(&sid);
                }
            } else {
                ctx.rtc_waiters.lock().remove(&sid);
            }
        }
    }

    if transport::is_direct(&ctx, &node_id) {
        return;
    }
    if is_live(&ctx, &node_id) {
        // On the relay. Keep looking for a direct path in the background.
        if retry_later && !hopeless {
            let (ctx, url, tok) = (ctx.clone(), server_url.clone(), token.clone());
            std::thread::spawn(move || {
                for _ in 0..UPGRADE_TRIES {
                    std::thread::sleep(UPGRADE_EVERY);
                    let relayed = is_live(&ctx, &node_id) && !transport::is_direct(&ctx, &node_id);
                    if ctx.shutdown.load(Ordering::Relaxed) || !relayed {
                        return;
                    }
                    run_ladder(
                        ctx.clone(),
                        url.clone(),
                        tok.clone(),
                        node_id.clone(),
                        false,
                    );
                }
            });
        }
        return;
    }

    emit("failed", Some("no reachable path".into()));
}
