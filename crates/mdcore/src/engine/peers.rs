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

/// Register ourselves, then replace the registry peers with the server's list.
pub(crate) fn refresh<P: Platform>(ctx: &NetCtx<P>, url: &str, token: &str) -> Result<(), String> {
    register_self(ctx, url, token)?;
    let node_id = ctx.identity.lock().node_id.clone();
    let devices = registry_fetch(url, token, &node_id)?;
    apply_registry_peers(ctx, devices);
    Ok(())
}

pub(crate) fn emit_peers<P: Platform>(ctx: &NetCtx<P>) {
    let list: Vec<Peer> = ctx.peers.lock().values().cloned().collect();
    ctx.sink.emit(CoreEvent::PeersUpdated { peers: list });
}

fn apply_registry_peers<P: Platform>(ctx: &NetCtx<P>, devices: Vec<RegistryDevice>) {
    {
        let mut map = ctx.peers.lock();
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
    emit_peers(ctx);
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
        refresh(&self.ctx, &url, &token)?;
        Ok(())
    }

    pub fn scan_lan(&self) {
        netchange::announce(&self.ctx, &self.disco_send);
    }

    /// The network changed (Android passes the new Wi-Fi IP; `None` means
    /// "recompute"). Closes links if our address moved, re-announces,
    /// re-registers, re-measures the NAT and re-dials previously connected
    /// peers, in the background (see `netchange`).
    pub fn network_changed(&self, new_ip: Option<&str>) {
        netchange::on_network_change(&self.handles(), new_ip);
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
