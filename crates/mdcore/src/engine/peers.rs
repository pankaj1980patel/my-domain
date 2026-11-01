//! Knowing who the peers are: registry refresh, LAN scan, network changes,
//! manual peers.

use super::*;
use crate::nat::NatKind;

/// Publish this device (endpoints, FCM token, NAT type) to the registry.
pub(crate) fn register_self<P: Platform>(
    ctx: &NetCtx<P>,
    url: &str,
    token: &str,
) -> Result<(), String> {
    let id = ctx.identity.lock().clone();
    let nat = *ctx.nat.lock();
    registry_register(
        url,
        token,
        &id,
        ctx.platform.fcm_token().as_deref(),
        ctx.platform.platform_kind(),
        false,
        nat,
    )
}

impl<P: Platform> Engine<P> {
    // --- discovery / peers ---

    pub fn refresh_from_server(&self) -> crate::Result<()> {
        let url = self
            .server_url
            .lock()
            .clone()
            .ok_or(CoreError::NotLoggedIn)?;
        let token = self.token.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        register_self(&self.ctx, &url, &token)?;
        let node_id = self.ctx.identity.lock().node_id.clone();
        let devices = registry_fetch(&url, &token, &node_id)?;
        self.apply_registry_peers(devices);
        Ok(())
    }

    fn apply_registry_peers(&self, devices: Vec<RegistryDevice>) {
        {
            let mut map = self.ctx.peers.lock();
            map.retain(|_, p| p.source != Source::Registry);
            for d in devices {
                map.insert(
                    d.node_id.clone(),
                    Peer {
                        node_id: d.node_id,
                        name: d.name,
                        ip: d.ip,
                        tcp_port: d.tcp_port,
                        udp_port: d.udp_port,
                        ws_port: d.ws_port,
                        quic_port: d.quic_port,
                        source: Source::Registry,
                        ws_open: d.ws_open,
                        inbound_blocked: d.inbound_blocked,
                        reflexive_ip: d.reflexive_ip,
                        reflexive_udp_port: d.reflexive_udp_port,
                        nat: d.nat.as_deref().map(NatKind::parse).unwrap_or_default(),
                    },
                );
            }
        }
        self.emit_peers();
    }

    pub fn scan_lan(&self) {
        let id = self.ctx.identity.lock().clone();
        let socket = self.disco_send.clone();
        let targets = discovery::lan_targets(&self.ctx.platform.iface_mode());
        std::thread::spawn(move || {
            for _ in 0..3 {
                discovery::send_beacon(&socket, &id, false, &targets);
                std::thread::sleep(Duration::from_millis(700));
            }
        });
    }

    /// Update the advertised IP after a network change (android passes the new
    /// Wi-Fi IP; desktop passes `None` to recompute). Re-registers if logged in.
    pub fn network_changed(&self, new_ip: Option<&str>) {
        let ip = match new_ip {
            Some(s) if s.parse::<Ipv4Addr>().is_ok() => s.to_string(),
            _ => self.ctx.platform.local_ip(),
        };
        self.ctx.identity.lock().ip = ip;
        if self.token.lock().is_some() {
            let _ = self.refresh_from_server();
        }
        // A new network usually means a new NAT.
        self.detect_nat();
    }

    pub fn add_manual_peer(
        &self,
        name: &str,
        ip: &str,
        tcp_port: u16,
        udp_port: u16,
        ws_port: u16,
    ) -> crate::Result<()> {
        let parsed: Ipv4Addr = ip
            .trim()
            .parse()
            .map_err(|_| format!("invalid IPv4: {ip}"))?;
        let node_id = format!("manual:{parsed}");
        let display = if name.trim().is_empty() {
            parsed.to_string()
        } else {
            name.trim().to_string()
        };
        self.ctx.peers.lock().insert(
            node_id.clone(),
            Peer {
                node_id,
                name: display,
                ip: parsed.to_string(),
                tcp_port,
                udp_port,
                ws_port,
                quic_port: 0, // unknown for a hand-entered peer
                source: Source::Manual,
                ws_open: false,
                inbound_blocked: false,
                reflexive_ip: None,
                reflexive_udp_port: None,
                nat: NatKind::Unknown,
            },
        );
        self.emit_peers();
        Ok(())
    }

    /// Insert or replace a fully-described peer, keyed by its real `node_id`.
    /// Unlike `add_manual_peer` (which only knows an IP and keys the entry
    /// `manual:<ip>`), replies such as clipboard responses and WS links are
    /// routed by node_id, so they reach a peer added this way. Used by tests
    /// today and by QR/code pairing later.
    pub fn upsert_peer(&self, peer: Peer) {
        self.ctx.peers.lock().insert(peer.node_id.clone(), peer);
        self.emit_peers();
    }

    pub fn remove_peer(&self, node_id: &str) {
        self.ctx.peers.lock().remove(node_id);
        self.emit_peers();
    }
}
