//! The connect ladder: try the cheapest way to reach a peer first, fall back
//! rung by rung (direct QUIC, signaled QUIC, relay, hole punch, WebRTC).
//! See learnings/01-architecture.md.

use super::signaling::{relay_target, send_signal_via};
use super::*;
use crate::features::{diag, links_map};

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

    /// Close the link to `node_id` now (the "Disconnect" button). Nothing is
    /// blocked: it connects again when either side has something to send.
    pub fn disconnect(&self, node_id: &str) -> crate::Result<()> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        transport::close_link(&self.ctx, node_id);
        Ok(())
    }

    /// Close the link to `node_id` and connect afresh (the "Reconnect"
    /// button: for a link that looks up but carries nothing).
    pub fn reconnect(&self, node_id: &str) -> crate::Result<()> {
        if !self.ctx.peers.lock().contains_key(node_id) {
            return Err(CoreError::PeerNotFound);
        }
        if self.ctx.p2p_conns.lock().contains_key(node_id) {
            super::messaging::reconnect(&self.ctx, node_id);
            Ok(())
        } else {
            self.connect(node_id)
        }
    }
}

// ---------------------------------------------------------------------------
// Connection ladder (runs off-thread; uses only NetCtx + the server creds, so
// it doesn't need `&Engine`). Each rung emits ConnectProgress and bails to the
// next on timeout. Rungs are ordered cheapest-first.
// ---------------------------------------------------------------------------

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

pub(super) fn run_connect_ladder<P: Platform>(
    ctx: NetCtx<P>,
    server_url: Arc<Mutex<Option<String>>>,
    token: Arc<Mutex<Option<String>>>,
    node_id: String,
) {
    run_ladder(ctx, server_url, token, node_id, true);
}

/// How long direct QUIC gets before signaled QUIC joins the race.
const HEAD_START: Duration = Duration::from_millis(500);
/// How long the QUIC rungs get on their own before the relay comes up too.
const QUICK_WAIT: Duration = Duration::from_millis(3000);

/// What came of signalling the peer (for the map's "why", see
/// `features::links_map`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Asked {
    PushFailed,
    NoAnswer,
    Answered,
}

/// Rung 3: offer our QUIC addresses by signal, dial the ones the peer
/// answers with, and give the link (ours, or the peer's dial to us) time to
/// come up.
fn signaled_quic<P: Platform>(
    ctx: &NetCtx<P>,
    server_url: &Arc<Mutex<Option<String>>>,
    token: &Arc<Mutex<Option<String>>>,
    node_id: &str,
) -> Asked {
    let mut asked = Asked::Answered;
    let sid = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel::<Vec<String>>();
    ctx.quic_waiters.lock().insert(sid.clone(), tx);
    let offer = Signal::QuicOffer {
        sid: sid.clone(),
        addrs: transport::quic::my_addrs(ctx),
    };
    match send_signal_via(server_url, token, ctx, node_id, &offer) {
        Ok(()) => {
            if let Ok(addrs) = rx.recv_timeout(Duration::from_secs(8)) {
                let targets = transport::quic::parse_addrs(&addrs);
                let _ = ctx.quic.connect_any(ctx.clone(), &targets);
            } else if !is_live(ctx, node_id) {
                // The push went (or a neighbour forwarded it); no answer.
                diag::note(ctx, node_id, "no_answer", None);
                asked = Asked::NoAnswer;
            }
            // The peer dials us too, so a link may exist even if ours failed.
            wait_direct(ctx, node_id, 3000);
        }
        Err(e) => {
            diag::note(ctx, node_id, "push_failed", Some(&e.to_string()));
            asked = Asked::PushFailed;
        }
    }
    ctx.quic_waiters.lock().remove(&sid);
    asked
}

/// Marks a ladder to (this device, peer) as running until dropped. A
/// second request meanwhile is recorded (`again`) instead of running.
struct Running(String);

impl Running {
    fn claim<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> Option<Running> {
        // Per engine (its QUIC `boot`), not per device id: a ladder still
        // running in an engine that was just replaced (restart in the same
        // process, or tests) mustn't stop the new engine's.
        let key = format!("{}>{node_id}", ctx.quic.boot);
        let mut running = running_map().lock();
        if let Some(again) = running.get_mut(&key) {
            *again = true;
            return None;
        }
        running.insert(key.clone(), false);
        Some(Running(key))
    }

    /// Whether another run was asked for since the last check (and reset).
    fn again(&self) -> bool {
        running_map()
            .lock()
            .get_mut(&self.0)
            .map(std::mem::take)
            .unwrap_or(false)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        running_map().lock().remove(&self.0);
    }
}

fn running_map() -> &'static Mutex<std::collections::HashMap<String, bool>> {
    static MAP: std::sync::OnceLock<Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
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
    // One ladder per peer at a time: each one signals (pushes) the peer,
    // and two at once only duplicate each other. Asked again meanwhile (the
    // first may have started before the peer's address was known), the
    // running one goes round once more if the peer still isn't live.
    let Some(running) = Running::claim(&ctx, &node_id) else {
        return;
    };
    loop {
        ladder_once(
            ctx.clone(),
            server_url.clone(),
            token.clone(),
            node_id.clone(),
            retry_later,
        );
        if !running.again() || is_live(&ctx, &node_id) {
            return;
        }
    }
}

fn ladder_once<P: Platform>(
    ctx: NetCtx<P>,
    server_url: Arc<Mutex<Option<String>>>,
    token: Arc<Mutex<Option<String>>>,
    node_id: String,
    retry_later: bool,
) {
    let emit = |stage: &str, detail: Option<String>| {
        diag::note(&ctx, &node_id, stage, detail.as_deref());
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

    let rungs = ctx.config.rungs.clone();

    // The two QUIC rungs race (they share one QUIC endpoint, and a second
    // link to the same peer is merged), and the relay comes up as soon as
    // neither has a direct link after `QUICK_WAIT`: messages flow through
    // it while signaled QUIC carries on and may still upgrade the link.
    // Hole punching and WebRTC start only once both QUIC rungs are done, so
    // two kinds of direct link never race each other.
    // What signalling the peer came to (for the map's "why").
    let asked: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
    let direct = std::thread::scope(|s| {
        let mut quick = Vec::new();
        // Rung 0 — direct QUIC to the peer's advertised address. One
        // handshake round trip; works on the LAN and to any peer that
        // accepts inbound UDP.
        if rungs.quic && peer.quic_port != 0 {
            emit("quic", None);
            let (ctx, node_id, targets) = (&ctx, &node_id, peer.quic_targets());
            quick.push(s.spawn(move || {
                // Done = failed, or the link is registered.
                if ctx.quic.connect_any(ctx.clone(), &targets).is_ok() {
                    wait_direct(ctx, node_id, 1000);
                }
            }));
        }
        // Rung 3 — signaled QUIC: swap QUIC addresses through a signal and
        // BOTH sides dial. Works when only one side can be dialed, and with
        // addresses fresher than the registry's. (1 signal round trip.)
        // Started once rung 0 has had `HEAD_START` (a LAN handshake takes
        // milliseconds), so a reachable peer costs no signal.
        let start = std::time::Instant::now();
        let deadline = start + QUICK_WAIT;
        let mut signaled = !rungs.signaled_quic;
        while std::time::Instant::now() < deadline {
            if transport::is_direct(&ctx, &node_id) {
                return true;
            }
            let rung0_done = quick.iter().all(|h| h.is_finished());
            if !signaled && (rung0_done || start.elapsed() >= HEAD_START) {
                signaled = true;
                emit("signaled_quic", None);
                let (ctx, server_url, token, node_id, asked) =
                    (&ctx, &server_url, &token, &node_id, &asked);
                quick.push(s.spawn(move || {
                    let a = signaled_quic(ctx, server_url, token, node_id);
                    asked.lock().push(a);
                }));
            }
            if signaled && quick.iter().all(|h| h.is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // A link that just came up still has to register.
        if !quick.is_empty() && wait_direct(&ctx, &node_id, 300) {
            return true;
        }

        // Relay through the server, BEFORE the slow rungs: if the quick
        // rungs failed, messages flow through the relay right away, while
        // signaled QUIC, hole punching and WebRTC keep trying to upgrade the
        // link in the background (a direct link replaces the relayed one
        // when it comes up).
        if rungs.relay && !is_live(&ctx, &node_id) {
            let target = relay_target(&ctx, &server_url, &token);
            if target.is_none() {
                links_map::tried(&ctx, &node_id, "relay", "signed_out");
            }
            if let Some((url, tok)) = target {
                emit("relay", None);
                let opened = transport::relay::ensure_relay(&ctx, &url, &tok);
                if let Err(e) = &opened {
                    links_map::tried(&ctx, &node_id, "relay", links_map::relay_reason(e));
                }
                if opened.is_ok() {
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
                    if !wait_live(&ctx, &node_id, 8000) {
                        diag::note(&ctx, &node_id, "relay_no_answer", None);
                        links_map::tried(&ctx, &node_id, "relay", "peer_offline");
                    }
                } else {
                    diag::note(&ctx, &node_id, "relay_unreachable", None);
                }
            }
        }
        for h in quick {
            let _ = h.join();
        }
        transport::is_direct(&ctx, &node_id)
    });
    if direct {
        return; // the link emits PeerConnected
    }
    // The local network didn't do it: say why.
    if rungs.quic {
        let mine = ctx.identity.lock().ip.clone();
        let why = links_map::lan_reason(
            &mine,
            &peer.ip,
            peer.quic_port,
            peer.source == crate::model::Source::Lan,
        );
        links_map::tried(&ctx, &node_id, "lan", why);
    }
    let mut punch_answered = false;
    let mut punch_timed_out = false;

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
                    punch_answered = true;
                    // hole_punch (≤8s) + bidirectional confirmation (≤3s).
                    if wait_direct(&ctx, &node_id, 12000) {
                        return;
                    }
                    punch_timed_out = true;
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
    // Nor directly over the internet: say why, the most telling first.
    {
        let asked = asked.into_inner();
        let push_failed = !asked.is_empty() && asked.iter().all(|a| *a == Asked::PushFailed);
        let answered = punch_answered || asked.contains(&Asked::Answered);
        let mapped = ctx.identity.lock().mapped_quic.is_some() || peer.mapped_quic.is_some();
        let why =
            links_map::internet_reason(push_failed, answered, hopeless, punch_timed_out, mapped);
        links_map::tried(&ctx, &node_id, "internet", why);
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
                    // Each try signals the peer, which wakes a sleeping
                    // phone by push. Only worth it while the link is used.
                    if !transport::used_within(&ctx, &node_id, UPGRADE_EVERY.as_secs()) {
                        continue;
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
