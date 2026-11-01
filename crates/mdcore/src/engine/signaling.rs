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
        let Ok(sig) = serde_json::from_str::<Signal>(payload) else {
            return;
        };
        match sig {
            // Initiator wants us reachable: our WS server is already listening,
            // so mark it open and tell them where to dial.
            Signal::StartWs { sid } => {
                let id = self.ctx.identity.lock().clone();
                let _ = self.update_ws_open(true);
                let _ = self.send_signal(
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
                let _ = ws::ws_connect(self.net_ctx(), &ip, ws_port);
            }
            // Initiator wants to hole-punch: gather our candidates, answer with
            // them, and punch toward theirs. Off-thread (STUN + HTTP block).
            Signal::PunchOffer {
                sid, candidates, ..
            } => {
                let ctx = self.net_ctx();
                let from = from.to_string();
                let server_url = self.server_url.clone();
                let token = self.token.clone();
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
                let waiter = self.ctx.punch_waiters.lock().remove(&sid);
                if let Some(tx) = waiter {
                    let _ = tx.send(candidates);
                }
            }
            // WebRTC: responder accepts the offer, spawns the driver, answers.
            #[cfg(feature = "webrtc")]
            Signal::SdpOffer { sid, sdp } => {
                let ctx = self.net_ctx();
                let from = from.to_string();
                let server_url = self.server_url.clone();
                let token = self.token.clone();
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
                let waiter = self.ctx.rtc_waiters.lock().remove(&sid);
                if let Some(tx) = waiter {
                    let _ = tx.send(sdp);
                }
            }
            // Teardown.
            Signal::Bye { .. } => {
                transport::drop_live(&self.net_ctx(), from);
            }
            // RelayOffer/RelayAnswer (TURN) remain a future hook.
            _ => {}
        }
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
