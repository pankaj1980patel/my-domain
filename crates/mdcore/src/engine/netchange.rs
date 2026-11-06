//! Reacting to network changes: a new Wi-Fi network, Wi-Fi -> mobile data,
//! waking from sleep. Android reports them (`Engine::network_changed`); on
//! desktop a watcher notices the local IP changing.
//!
//! What happens:
//! 1. If our address really changed: QUIC links we dialed survive (QUIC
//!    connection migration: the peer follows our new address). Every other
//!    link is on the old network and dead (or soon will be), so close it now
//!    instead of waiting ~20 s for a timeout.
//! 2. Re-announce ourselves on the LAN, re-register with the registry and
//!    refresh the peer list, re-measure the NAT type.
//! 3. Re-dial every peer whose link was closed, through the full ladder.

use super::*;

/// What the handler needs from the `Engine`, cheap to clone (all `Arc`s), so
/// a watcher thread can hold it.
pub(crate) struct NetHandles<P: Platform> {
    pub ctx: NetCtx<P>,
    pub server_url: Arc<Mutex<Option<String>>>,
    pub token: Arc<Mutex<Option<String>>>,
    pub disco_send: Arc<UdpSocket>,
}

impl<P: Platform> Clone for NetHandles<P> {
    fn clone(&self) -> Self {
        NetHandles {
            ctx: self.ctx.clone(),
            server_url: self.server_url.clone(),
            token: self.token.clone(),
            disco_send: self.disco_send.clone(),
        }
    }
}

/// Give closed links (and the peer's side of them) a moment to clean up
/// before re-dialing, so the peer doesn't reject us as a duplicate.
const REDIAL_AFTER: Duration = Duration::from_millis(500);

pub(crate) fn on_network_change<P: Platform>(
    h: &NetHandles<P>,
    new_ip: Option<&str>,
    net: crate::config::NetKind,
) {
    use crate::config::NetKind;
    use crate::features::reconnect::{self, ReconnectTrigger};
    let ctx = &h.ctx;
    let (gen, net) = reconnect::network_changed(ctx, net);
    // Links made on this network keep alive as it needs (sooner on
    // mobile data, whose NATs forget sooner).
    ctx.quic
        .set_keepalive(ctx.config.keepalive.for_network(net), &ctx.device_key);
    let ip = match new_ip {
        Some(s) if s.parse::<Ipv4Addr>().is_ok() => s.to_string(),
        _ => ctx.platform.local_ip(),
    };
    let ipv6 = ctx.platform.local_ipv6();
    let moved = {
        let mut id = ctx.identity.lock();
        let moved = id.ip != ip || id.ipv6 != ipv6;
        id.ip = ip;
        id.ipv6 = ipv6;
        moved
    };
    let was_live: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    if moved || net == NetKind::Offline {
        // QUIC links we dialed migrate to the new network by themselves;
        // everything else is on the old network and must be re-dialed.
        for node in &was_live {
            reconnect::closing(ctx, node, "network_change");
        }
        let survives = ctx.quic.close_accepted();
        for node in &survives {
            reconnect::closing(ctx, node, "");
        }
        for node in was_live.iter().filter(|n| !survives.contains(n)) {
            ctx.ws_conns.lock().remove(node);
            transport::drop_live(ctx, node);
        }
    }
    // Who to dial again: the links that went just now, and those lost
    // lately by accident (an earlier change dropped them while there was
    // no network, say: Wi-Fi gone, then mobile data up).
    let mut targets = was_live;
    for node in reconnect::wanted(ctx) {
        if !targets.contains(&node) {
            targets.push(node);
        }
    }
    targets.retain(|n| !transport::is_live(ctx, n) && ctx.peers.lock().contains_key(n));
    if net == NetKind::Offline {
        // Nothing to dial over: the next change (a network up) does it.
        return;
    }
    for node in &targets {
        reconnect::trigger(ctx, node, ReconnectTrigger::Network, Duration::ZERO);
    }

    let h = h.clone();
    std::thread::spawn(move || {
        announce(&h.ctx, &h.disco_send);
        // Redial first: refreshing the list and measuring the NAT take
        // seconds, and the peers' addresses we know are usually right.
        std::thread::sleep(REDIAL_AFTER);
        for node in targets {
            let h = h.clone();
            std::thread::spawn(move || redial(&h, &node, gen));
        }
        let session = h.server_url.lock().clone().zip(h.token.lock().clone());
        if let Some((url, token)) = &session {
            let _ = peers::refresh(&h.ctx, url, token);
        }
        detect_nat_now(&h.ctx, &h.server_url, &h.token);
        // Peers on links that survived the move learn our new addresses
        // directly (no server needed to find us again).
        crate::features::addr_update::announce(&h.ctx, None);
        // A device on a long-way link that's on this network now.
        reconnect::prefer_lan(&h.ctx);
    });
}

/// Dial `node` again after network change `gen`: at once, then with
/// jittered backoff (`reconnect::backoff`), until it's up, a newer change
/// takes over, or `REDIAL_WINDOW` is over. Automatic: a device that
/// didn't answer a push isn't pushed again (`reconnect::cooldown`).
fn redial<P: Platform>(h: &NetHandles<P>, node: &str, gen: u64) {
    use crate::features::reconnect::{self, REDIAL_BASE, REDIAL_CAP, REDIAL_WINDOW};
    let ctx = &h.ctx;
    let current = || reconnect::generation(ctx) == gen && !ctx.shutdown.load(Ordering::Relaxed);
    let start = std::time::Instant::now();
    let mut attempt = 0u32;
    while current() && !transport::is_live(ctx, node) {
        ladder::run_auto_ladder(
            ctx.clone(),
            h.server_url.clone(),
            h.token.clone(),
            node.to_string(),
        );
        if transport::is_live(ctx, node) || start.elapsed() >= REDIAL_WINDOW {
            break;
        }
        let wait = reconnect::backoff(attempt, REDIAL_BASE, REDIAL_CAP, reconnect::jitter());
        attempt += 1;
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            if !current() || transport::is_live(ctx, node) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Beacon ourselves on the LAN a few times (multicast and broadcasts). The
/// /24 unicast sweep (254 packets per interface) is added only if, after the
/// first round, we still know no device on the LAN: mDNS or a beacon reply
/// normally finds them first, and the sweep is for networks that drop both.
pub(crate) fn announce<P: Platform>(ctx: &NetCtx<P>, socket: &Arc<UdpSocket>) {
    // No key yet: nobody would accept our beacon (see `crypto::lan_tag`).
    let Some(tag) = crate::crypto::lan_tag_of(&ctx.key) else {
        return;
    };
    let id = ctx.identity.lock().clone();
    let socket = socket.clone();
    let mode = ctx.platform.iface_mode();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut targets = discovery::lan_targets(&mode, false);
        for round in 0..3 {
            if round == 1 && !knows_a_lan_device(&ctx) {
                targets = discovery::lan_targets(&mode, true);
            }
            discovery::send_beacon(&socket, &id, &tag, false, &targets);
            std::thread::sleep(ROUND_GAP);
        }
    });
}

/// Pause between announce rounds; long enough for mDNS and beacon replies.
const ROUND_GAP: Duration = Duration::from_millis(700);

fn knows_a_lan_device<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.peers.lock().values().any(|p| p.source == Source::Lan)
}

/// Measure our NAT type now (blocks for the STUN round trips) and re-register
/// if it changed. No-op without STUN servers.
pub(crate) fn detect_nat_now<P: Platform>(
    ctx: &NetCtx<P>,
    server_url: &Arc<Mutex<Option<String>>>,
    token: &Arc<Mutex<Option<String>>>,
) {
    if ctx.config.stun_servers.is_empty() {
        return;
    }
    let kind = crate::nat::classify(&ctx.config.stun_addrs());
    if ctx.shutdown.load(Ordering::Relaxed) {
        return;
    }
    let changed = std::mem::replace(&mut *ctx.nat.lock(), kind) != kind;
    let session = server_url.lock().clone().zip(token.lock().clone());
    if let (true, Some((url, tok))) = (changed, session) {
        let _ = peers::register_self(ctx, &url, &tok);
    }
}
