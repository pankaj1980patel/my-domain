//! Reacting to network changes: a new Wi-Fi network, Wi-Fi -> mobile data,
//! waking from sleep. Android reports them (`Engine::network_changed`); on
//! desktop a watcher notices the local IP changing.
//!
//! What happens:
//! 1. If our address really changed, every live link is on the old network
//!    and is dead (or soon will be), so close them now instead of waiting
//!    ~20 s for timeouts.
//! 2. Re-announce ourselves on the LAN, re-register with the registry and
//!    refresh the peer list, re-measure the NAT type.
//! 3. Re-dial every peer that was connected before, through the full ladder.

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

pub(crate) fn on_network_change<P: Platform>(h: &NetHandles<P>, new_ip: Option<&str>) {
    let ctx = &h.ctx;
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
    if moved {
        ctx.quic.close_all();
        for node in &was_live {
            ctx.ws_conns.lock().remove(node);
            transport::drop_live(ctx, node);
        }
    }

    let h = h.clone();
    std::thread::spawn(move || {
        announce(&h.ctx, &h.disco_send);
        let session = h.server_url.lock().clone().zip(h.token.lock().clone());
        if let Some((url, token)) = &session {
            let _ = peers::refresh(&h.ctx, url, token);
        }
        detect_nat_now(&h.ctx, &h.server_url, &h.token);

        std::thread::sleep(REDIAL_AFTER);
        for node in was_live {
            if h.ctx.p2p_conns.lock().contains_key(&node) || !h.ctx.peers.lock().contains_key(&node)
            {
                continue;
            }
            let (ctx, url, token) = (h.ctx.clone(), h.server_url.clone(), h.token.clone());
            std::thread::spawn(move || ladder::run_connect_ladder(ctx, url, token, node));
        }
    });
}

/// Beacon ourselves on the LAN a few times (multicast, broadcast, /24 sweep).
pub(crate) fn announce<P: Platform>(ctx: &NetCtx<P>, socket: &Arc<UdpSocket>) {
    let id = ctx.identity.lock().clone();
    let socket = socket.clone();
    let targets = discovery::lan_targets(&ctx.platform.iface_mode());
    std::thread::spawn(move || {
        for _ in 0..3 {
            discovery::send_beacon(&socket, &id, false, &targets);
            std::thread::sleep(Duration::from_millis(700));
        }
    });
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
