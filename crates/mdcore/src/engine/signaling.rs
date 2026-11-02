//! Talking to peers through the server: reachability (firewall) checks,
//! sending signals, and handling the signals we receive (`on_signal`).

use super::*;

impl<P: Platform> Engine<P> {
    // --- signaling / connection setup (Roadmap B) ---

    /// Outbound + inbound reachability check. Outbound = the server is reachable;
    /// inbound = the server can dial back to our advertised ports. Persists the
    /// inbound result via `/devices/state`.
    pub fn firewall_check(&self) -> crate::Result<FirewallStatus> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let outbound_ok = crate::registry::health_ok(&url);
        if !outbound_ok {
            return Ok(FirewallStatus {
                outbound_ok: false,
                inbound_blocked: true,
            });
        }
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let node_id = self.ctx.identity.lock().node_id.clone();
        let probe = crate::registry::probe_inbound(&url, &token, &node_id, &["tcp", "ws"])?;
        let inbound_blocked = !(probe.tcp_reachable || probe.ws_reachable);
        // Persist so siblings see our reachability via `GET /devices` and the
        // connect ladder can use it. Best-effort — don't fail the check on this.
        let _ = crate::registry::update_device_state(
            &url,
            &token,
            &node_id,
            None,
            Some(inbound_blocked),
            None,
            None,
        );
        Ok(FirewallStatus {
            outbound_ok,
            inbound_blocked,
        })
    }

    /// Run the firewall check on a background thread and persist the result, so
    /// other devices see our reachability. Called once after login (the probe
    /// does a blocking server dial-back, so it must not run on the UI thread).
    pub fn report_firewall(&self) {
        let url = self.server_url.lock().clone();
        let token = self.token.lock().clone();
        let node_id = self.ctx.identity.lock().node_id.clone();
        let (Some(url), Some(token)) = (url, token) else {
            return;
        };
        std::thread::spawn(move || {
            if !crate::registry::health_ok(&url) {
                return;
            }
            if let Ok(probe) =
                crate::registry::probe_inbound(&url, &token, &node_id, &["tcp", "ws"])
            {
                let inbound_blocked = !(probe.tcp_reachable || probe.ws_reachable);
                let _ = crate::registry::update_device_state(
                    &url,
                    &token,
                    &node_id,
                    None,
                    Some(inbound_blocked),
                    None,
                    None,
                );
            }
        });
    }

    /// Tell the registry whether our WS server is currently reachable.
    pub fn update_ws_open(&self, open: bool) -> crate::Result<()> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let node_id = self.ctx.identity.lock().node_id.clone();
        crate::registry::update_device_state(&url, &token, &node_id, Some(open), None, None, None)
            .map_err(Into::into)
    }

    /// Relay a typed signal to a sibling device (via the server's FCM relay).
    pub fn send_signal(&self, to: &str, sig: &Signal) -> crate::Result<()> {
        send_signal_via(&self.server_url, &self.token, &self.ctx, to, sig)
    }

    /// Route signals through `signaler` instead of the registry's HTTP relay
    /// (tests use an in-memory hub; a WebSocket relay can plug in here later).
    pub fn set_signaler(&self, signaler: Arc<dyn Signaler>) {
        *self.ctx.signaler.lock() = Some(signaler);
    }

    /// Round-trip FCM self-test: ask the server to push a `ping` to our own
    /// device token. Returns the correlation id; the pong arrives via the
    /// platform's FCM receiver (which the adapter surfaces to the UI). Verifies
    /// the full push path end to end (server FCM send → device receive).
    pub fn fcm_selftest(&self) -> crate::Result<String> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        let node_id = self.ctx.identity.lock().node_id.clone();
        let sid = uuid::Uuid::new_v4().to_string();
        crate::registry::send_selfping(&url, &token, &node_id, &sid)?;
        Ok(sid)
    }

    /// Handle an inbound signal (delivered by the platform's FCM receiver as the
    /// raw JSON `payload`). Drives the responder side of the connection ladder.
    pub fn on_signal(&self, from: &str, payload: &str) {
        handle_signal(&self.ctx, from, payload);
    }
}

#[derive(Serialize)]
pub struct FirewallStatus {
    pub outbound_ok: bool,
    pub inbound_blocked: bool,
}

/// Relay a typed signal using the shared server creds (thread-friendly twin of
/// `Engine::send_signal`).
pub(super) fn send_signal_via<P: Platform>(
    server_url: &Arc<Mutex<Option<String>>>,
    token: &Arc<Mutex<Option<String>>>,
    ctx: &NetCtx<P>,
    to: &str,
    sig: &Signal,
) -> crate::Result<()> {
    let me = ctx.identity.lock().node_id.clone();
    let custom = ctx.signaler.lock().clone();
    if let Some(signaler) = custom {
        return signaler.send(&me, to, sig);
    }
    let url = server_url.lock().clone().ok_or(CoreError::NotLoggedIn)?;
    let tok = token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
    let data = serde_json::to_value(sig).map_err(|e| e.to_string())?;
    crate::registry::send_signal(&url, &tok, &me, to, "signal", data).map_err(Into::into)
}

/// Where to reach the relay and with which session token, or `None` if we
/// aren't logged in to a server (or its URL can't carry a relay).
pub(super) fn relay_target<P: Platform>(
    ctx: &NetCtx<P>,
    server_url: &Arc<Mutex<Option<String>>>,
    token: &Arc<Mutex<Option<String>>>,
) -> Option<(String, String)> {
    let token = token.lock().clone()?;
    let url = match &ctx.config.relay_url {
        Some(u) => u.clone(),
        None => transport::relay::relay_url_for(&server_url.lock().clone()?).ok()?,
    };
    Some((url, token))
}

/// Handle an inbound signal, from the platform's FCM receiver or from the
/// server's WebSocket (see transport/relay.rs). Drives the responder side of
/// the connection ladder.
pub(crate) fn handle_signal<P: Platform>(ctx: &NetCtx<P>, from: &str, payload: &str) {
    let Ok(sig) = serde_json::from_str::<Signal>(payload) else {
        return;
    };
    match sig {
        // Initiator wants us reachable: our WS server is already listening,
        // so mark it open and tell them where to dial.
        Signal::StartWs { sid } => {
            let id = ctx.identity.lock().clone();
            let _ = update_ws_open_via(ctx, true);
            let _ = send_signal_via(
                &ctx.server_url,
                &ctx.token,
                ctx,
                from,
                &Signal::WsReady {
                    sid,
                    ip: id.ip,
                    ws_port: id.ws_port,
                },
            );
        }
        // Responder is reachable: dial it.
        Signal::WsReady { ip, ws_port, .. } => {
            let _ = ws::ws_connect(ctx.clone(), &ip, ws_port);
        }
        // Initiator wants to hole-punch: gather our candidates, answer with
        // them, and punch toward theirs. Off-thread (STUN + HTTP block).
        Signal::PunchOffer {
            sid, candidates, ..
        } => {
            let ctx = ctx.clone();
            let from = from.to_string();
            let server_url = ctx.server_url.clone();
            let token = ctx.token.clone();
            std::thread::spawn(move || {
                if let Some((socket, my_cands)) = transport::punch::gather_candidates(&ctx) {
                    let _ = send_signal_via(
                        &server_url,
                        &token,
                        &ctx,
                        &from,
                        &Signal::PunchAnswer {
                            sid,
                            candidates: my_cands,
                            start_in_ms: 0,
                        },
                    );
                    transport::punch::punch_and_run(ctx, socket, candidates, from);
                }
            });
        }
        // Our offer was answered: hand the peer's candidates to the waiting
        // ladder thread so it can punch.
        Signal::PunchAnswer {
            sid, candidates, ..
        } => {
            let waiter = ctx.punch_waiters.lock().remove(&sid);
            if let Some(tx) = waiter {
                let _ = tx.send(candidates);
            }
        }
        // WebRTC: responder accepts the offer, spawns the driver, answers.
        #[cfg(feature = "webrtc")]
        Signal::SdpOffer { sid, sdp } => {
            let ctx = ctx.clone();
            let from = from.to_string();
            let server_url = ctx.server_url.clone();
            let token = ctx.token.clone();
            std::thread::spawn(move || {
                if let Some(answer) =
                    transport::rtc::accept_offer_and_run(ctx.clone(), &sdp, from.clone())
                {
                    let _ = send_signal_via(
                        &server_url,
                        &token,
                        &ctx,
                        &from,
                        &Signal::SdpAnswer { sid, sdp: answer },
                    );
                }
            });
        }
        // WebRTC: hand the answer SDP to the waiting initiator thread.
        #[cfg(feature = "webrtc")]
        Signal::SdpAnswer { sid, sdp } => {
            let waiter = ctx.rtc_waiters.lock().remove(&sid);
            if let Some(tx) = waiter {
                let _ = tx.send(sdp);
            }
        }
        // Signaled QUIC: answer with our addresses, then dial theirs.
        Signal::QuicOffer { sid, addrs } => {
            let ctx = ctx.clone();
            let from = from.to_string();
            let server_url = ctx.server_url.clone();
            let token = ctx.token.clone();
            std::thread::spawn(move || {
                let answer = Signal::QuicAnswer {
                    sid,
                    addrs: transport::quic::my_addrs(&ctx),
                };
                let _ = send_signal_via(&server_url, &token, &ctx, &from, &answer);
                // The initiator dials our answered addresses right away.
                // We dial back only if that hasn't worked within a second
                // (it can't reach us). Dialing at once from both sides
                // would make two connections, and a message sent on the
                // one that loses the tie-break would be lost.
                std::thread::sleep(Duration::from_secs(1));
                if !transport::is_direct(&ctx, &from) {
                    let targets = transport::quic::parse_addrs(&addrs);
                    let _ = ctx.quic.connect_any(ctx.clone(), &targets);
                }
            });
        }
        // Our QUIC offer was answered: hand the addresses to the ladder.
        Signal::QuicAnswer { sid, addrs } => {
            let waiter = ctx.quic_waiters.lock().remove(&sid);
            if let Some(tx) = waiter {
                let _ = tx.send(addrs);
            }
        }
        // Peer can't reach us directly: meet it on the server relay.
        Signal::RelayOffer { .. } => {
            let ctx = ctx.clone();
            let from = from.to_string();
            let target = relay_target(&ctx, &ctx.server_url, &ctx.token);
            std::thread::spawn(move || {
                if let Some((url, token)) = target {
                    if transport::relay::ensure_relay(&ctx, &url, &token).is_ok() {
                        transport::relay::add_relay_link(&ctx, &from);
                        transport::relay::send_hello(&ctx, &from);
                    }
                }
            });
        }
        // Teardown.
        Signal::Bye { .. } => {
            transport::drop_live(&ctx.clone(), from);
        }
        // RelayOffer/RelayAnswer (TURN) remain a future hook.
        _ => {}
    }
}

/// Tell the registry whether our WS server is currently reachable.
fn update_ws_open_via<P: Platform>(ctx: &NetCtx<P>, open: bool) -> crate::Result<()> {
    let url = ctx
        .server_url
        .lock()
        .clone()
        .ok_or(CoreError::NotLoggedIn)?;
    let token = ctx.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
    let node_id = ctx.identity.lock().node_id.clone();
    crate::registry::update_device_state(&url, &token, &node_id, Some(open), None, None, None)
        .map_err(Into::into)
}
