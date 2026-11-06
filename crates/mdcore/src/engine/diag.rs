//! "Check connection": a report on one device's link, and a speed test
//! (see `features::diag`).

use super::signaling::relay_target;
use super::*;
use crate::features::diag::{self, Addresses, Diagnosis, LinkInfo, RelayInfo, MAX_SPEED_BYTES};

/// Pings a report sends.
const REPORT_PINGS: u32 = 3;
/// How long the relay gets to take a connection.
const RELAY_PROBE: Duration = Duration::from_secs(3);

impl<P: Platform> Engine<P> {
    /// Everything this device knows about its connection to `node_id`: the
    /// link and its transport, both ends' addresses and NATs, round trips
    /// measured now (only to a device that answers pings, and only over a
    /// link that's already up: this never connects or wakes it), what the
    /// last connect attempts did, and whether the relay server answers.
    /// Blocks for a few seconds at most.
    pub fn diagnose(&self, node_id: &str) -> crate::Result<Diagnosis> {
        let Some(peer) = self.ctx.peers.lock().get(node_id).cloned() else {
            return Err(CoreError::PeerNotFound);
        };
        // The relay check runs while the pings do.
        let relay = {
            let ctx = self.ctx.clone();
            let target = relay_target(&self.ctx, &self.server_url, &self.token);
            std::thread::spawn(move || relay_info(&ctx, target))
        };
        let ping_supported = diag::supports(&self.ctx, node_id);
        let rtt = diag::ping(&self.ctx, node_id, REPORT_PINGS);

        let now = transport::now_secs();
        let ago = |t: u64| u32::try_from(now.saturating_sub(t)).unwrap_or(u32::MAX);
        let conn = self.ctx.p2p_conns.lock().get(node_id).cloned();
        let link = conn.map(|c| {
            let measured = self.ctx.quic.path_stats(node_id);
            LinkInfo {
                transport: c.kind,
                direct: !c.kind.is_fallback(),
                lan: diag::is_lan(c.addr),
                remote: c.addr.map(|a| a.to_string()),
                quic_rtt_ms: measured
                    .map(|(rtt, _, _)| u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX)),
                loss_pct: measured
                    .and_then(|(_, lost, sent)| transport::quality::loss_pct(lost, sent)),
                heard_ago_s: ago(c.last_seen),
                used_ago_s: ago(c.last_used),
            }
        });
        let attempts = diag::attempts(&self.ctx, node_id);
        let (verdict, reason) = Diagnosis::judge(link.as_ref(), &attempts);
        let my_nat = *self.ctx.nat.lock();
        let this_device = {
            let id = self.ctx.identity.lock();
            Addresses {
                ip: id.ip.clone(),
                ipv6: id.ipv6.clone(),
                quic_port: id.quic_port,
                mapped_quic: id.mapped_quic.clone(),
                public: None,
                nat: my_nat,
            }
        };
        let public = peer
            .reflexive_ip
            .as_ref()
            .map(|ip| match peer.reflexive_udp_port {
                Some(port) => format!("{ip}:{port}"),
                None => ip.clone(),
            });
        let relay = relay.join().unwrap_or_default();
        Ok(Diagnosis {
            node_id: node_id.to_string(),
            name: peer.name.clone(),
            verdict,
            reason,
            last_try_ago_s: attempts.last().map(|a| a.ago_s),
            link,
            ping_supported,
            rtt,
            this_device,
            peer: Addresses {
                ip: peer.ip.clone(),
                ipv6: peer.ipv6.clone(),
                quic_port: peer.quic_port,
                mapped_quic: peer.mapped_quic.clone(),
                public,
                nat: peer.nat,
            },
            source: peer.source,
            both_symmetric: my_nat.hopeless_with(peer.nat),
            relay,
            attempts,
            reconnect: crate::features::reconnect::report(&self.ctx, node_id),
        })
    }

    /// Measure the speed of the link to `node_id`, both ways: a few pings,
    /// then `bytes` (clamped to 256 KB – 32 MB) up and the same back.
    /// Needs a live link and a device that answers pings. Returns the
    /// test's id at once; progress and the result come as `SpeedTest`
    /// events.
    pub fn speed_test(&self, node_id: &str, bytes: u32) -> crate::Result<String> {
        diag::start_test(&self.ctx, node_id, bytes.min(MAX_SPEED_BYTES))
    }

    /// Stop speed test `id` (the other device stops too). False if it
    /// isn't running.
    pub fn cancel_speed_test(&self, id: &str) -> bool {
        diag::cancel_test(&self.ctx, id)
    }

    /// "How your devices are connected": this device's links, the state of
    /// each of your devices, and the links between your other devices that
    /// they reported (`features::links_map`). Quick: measures nothing.
    pub fn links_map(&self) -> crate::features::links_map::LinksMap {
        crate::features::links_map::snapshot(&self.ctx)
    }

    /// The platform's Bluetooth can't link to anything now (`reason`:
    /// "off", "not_paired", "out_of_range", "unsupported"), or can again
    /// (`None`). The map shows it for the devices not linked by Bluetooth.
    pub fn set_ble_state(&self, reason: Option<&str>) {
        crate::features::links_map::ble_state(&self.ctx, reason);
    }

    /// `node_id` shares its own links for the map (its app is new enough).
    pub fn links_map_supported(&self, node_id: &str) -> bool {
        crate::features::links_map::supports(&self.ctx, node_id)
    }

    /// Ping `node_id` once over its live link: the round trip, or `None`
    /// (no link, no answer, or an app too old to answer).
    pub fn ping(&self, node_id: &str) -> Option<f64> {
        diag::ping(&self.ctx, node_id, 1).and_then(|s| s.avg_ms)
    }
}

fn relay_info<P: Platform>(ctx: &NetCtx<P>, target: Option<(String, String)>) -> RelayInfo {
    let connected = transport::relay::is_open(ctx);
    let Some((url, _)) = target else {
        return RelayInfo {
            signed_in: false,
            connected,
            reachable: None,
            error: None,
        };
    };
    if connected {
        return RelayInfo {
            signed_in: true,
            connected,
            reachable: Some(true),
            error: None,
        };
    }
    let probe = transport::relay::probe(&url, RELAY_PROBE);
    RelayInfo {
        signed_in: true,
        connected,
        reachable: Some(probe.is_ok()),
        error: probe.err(),
    }
}
