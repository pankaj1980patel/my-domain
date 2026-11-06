//! The engine's long-running loops, started once by `Engine::start`:
//!
//! * clipboard watcher: pushes local clipboard changes to peers (auto-sync),
//! * IP watcher (started separately, see `spawn_ip_watcher`): notices the
//!   local IP changing and runs the network-change handling,
//! * P2P heartbeat: keeps connectionless links (UDP/punch/WebRTC) alive and
//!   evicts the ones that stopped answering.
//!
//! Each loop sleeps, then checks `ctx.shutdown`, so dropping the `Engine`
//! stops them within one interval. (The socket listeners block in `accept` /
//! `recv` and can't be interrupted this way; the tokio rewrite in Phase 2
//! cancels those too.)

use super::*;

const CLIPBOARD_POLL: Duration = Duration::from_millis(1000);
const IP_POLL: Duration = Duration::from_secs(5);

/// Key in the platform's key-value store for the cached peer list.
const PEER_CACHE_KEY: &str = "peers";
/// At most this many peers are cached.
const PEER_CACHE_MAX: usize = 256;

/// Load the peers saved last time (addresses, ports, IPv6, router mapping),
/// so the engine can dial them directly after a restart without asking the
/// server: the server only introduces devices, it shouldn't be needed to
/// find them again.
pub(super) fn load_peer_cache<P: Platform>(ctx: &NetCtx<P>) {
    let Some(json) = ctx.platform.kv_get(PEER_CACHE_KEY) else {
        return;
    };
    let Ok(list) = serde_json::from_str::<Vec<Peer>>(&json) else {
        return;
    };
    let me = ctx.identity.lock().node_id.clone();
    let mut peers = ctx.peers.lock();
    // LAN-found devices aren't restored: real ones announce themselves again
    // within seconds, and stale ones (another network, a finished test run)
    // would otherwise stay listed forever.
    for p in list
        .into_iter()
        .filter(|p| p.node_id != me && p.source != Source::Lan)
        .take(PEER_CACHE_MAX)
    {
        peers.entry(p.node_id.clone()).or_insert(p);
    }
}

/// Save the peer list whenever it changes (checked every couple of seconds),
/// and once more on shutdown.
fn spawn_peer_cache<P: Platform>(ctx: NetCtx<P>) {
    std::thread::spawn(move || {
        let snapshot = |ctx: &NetCtx<P>| {
            let mut list: Vec<Peer> = ctx.peers.lock().values().cloned().collect();
            list.sort_by(|a, b| a.node_id.cmp(&b.node_id));
            list.truncate(PEER_CACHE_MAX);
            serde_json::to_string(&list).unwrap_or_default()
        };
        let mut saved = ctx.platform.kv_get(PEER_CACHE_KEY).unwrap_or_default();
        loop {
            let alive = tick(&ctx, Duration::from_secs(2));
            let now = snapshot(&ctx);
            if now != saved {
                ctx.platform.kv_set(PEER_CACHE_KEY, &now);
                saved = now;
            }
            if !alive {
                return;
            }
        }
    });
}

pub(super) fn spawn_all<P: Platform>(ctx: &NetCtx<P>) {
    spawn_peer_cache(ctx.clone());
    if ctx.platform.clipboard().is_some() {
        let ctx = ctx.clone();
        std::thread::spawn(move || clipboard_watcher(ctx));
    }
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || heartbeat(ctx));
    }
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            while tick(&ctx, Duration::from_secs(2)) {
                transport::outbox::retry_due(&ctx);
                crate::features::files::resume_paused(&ctx);
                crate::features::remote_download::on_timer(&ctx);
                crate::features::backup::on_timer(&ctx);
                crate::features::folder_sync::on_timer(&ctx);
                crate::features::insert_capture::on_timer(&ctx);
                // A device on a long-way link that's on this network now:
                // dial it locally.
                crate::features::reconnect::prefer_lan(&ctx);
            }
        });
    }
    if let Some(idle) = ctx.config.idle_close_after {
        let ctx = ctx.clone();
        std::thread::spawn(move || idle_closer(ctx, idle));
    }
}

/// In the background, close links that carried no messages for `idle`.
/// Their keep-alive pings would otherwise wake the radio every 20 s, all
/// day; a send re-opens a link on demand, and a push reaches us without one.
fn idle_closer<P: Platform>(ctx: NetCtx<P>, idle: Duration) {
    let every = (idle / 4).clamp(Duration::from_millis(100), Duration::from_secs(15));
    while tick(&ctx, every) {
        if !ctx.background.load(Ordering::Relaxed) {
            continue;
        }
        for node in transport::idle_links(&ctx, idle.as_secs()) {
            transport::close_link(&ctx, &node, "idle");
        }
        // The relay socket too, once nothing goes through it (unless it's
        // kept open on purpose as the signal channel).
        if !ctx.config.signal_channel {
            transport::relay::close_if_unused(&ctx, idle);
        }
    }
}

/// Renew the router's port mapping this often (well inside its lease).
const MAP_RENEW: Duration = Duration::from_secs(20 * 60);
/// Retry this often while the router refuses (or there's no UPnP router).
const MAP_RETRY: Duration = Duration::from_secs(10 * 60);

/// Keep the home router forwarding our QUIC port (UPnP), if a port mapper is
/// configured: map at start, renew before the lease runs out, re-map when
/// the LAN address changes, unmap on shutdown. The public address is
/// published to the registry as `mapped_quic`.
pub(super) fn spawn_port_mapper<P: Platform>(h: netchange::NetHandles<P>) {
    use crate::nat::portmap::LEASE_SECS;
    use std::net::SocketAddrV4;
    let Some(mapper) = h.ctx.config.port_mapper.clone() else {
        return;
    };
    std::thread::spawn(move || {
        // (local address the mapping points at, public address)
        let mut mapped: Option<(SocketAddrV4, SocketAddrV4)> = None;
        let mut next_try = std::time::Instant::now();
        while !h.ctx.shutdown.load(Ordering::Relaxed) {
            let local = {
                let id = h.ctx.identity.lock();
                id.ip
                    .parse::<Ipv4Addr>()
                    .ok()
                    .filter(|ip| !ip.is_unspecified() && id.quic_port != 0)
                    .map(|ip| SocketAddrV4::new(ip, id.quic_port))
            };
            let moved = mapped.is_some_and(|(l, _)| Some(l) != local);
            if moved || std::time::Instant::now() >= next_try {
                if let (true, Some((_, public))) = (moved, mapped) {
                    mapper.unmap_udp(public.port());
                }
                mapped = local.and_then(|l| mapper.map_udp(l, LEASE_SECS).map(|p| (l, p)));
                let published = mapped.map(|(_, p)| p.to_string());
                let changed = {
                    let mut id = h.ctx.identity.lock();
                    std::mem::replace(&mut id.mapped_quic, published.clone()) != published
                };
                if changed {
                    crate::features::addr_update::announce(&h.ctx, None);
                }
                let session = h.server_url.lock().clone().zip(h.token.lock().clone());
                if let (true, Some((url, tok))) = (changed, session) {
                    let _ = peers::register_self(&h.ctx, &url, &tok);
                }
                next_try = std::time::Instant::now()
                    + if mapped.is_some() {
                        MAP_RENEW
                    } else {
                        MAP_RETRY
                    };
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        if let Some((_, public)) = mapped {
            mapper.unmap_udp(public.port());
        }
    });
}

/// While logged in, keep the server WebSocket (the relay socket) open, so the
/// server can hand us signals over it instead of an FCM push. Retries every
/// few seconds while disconnected.
pub(super) fn spawn_signal_channel<P: Platform>(ctx: NetCtx<P>) {
    if !ctx.config.signal_channel {
        return;
    }
    std::thread::spawn(move || loop {
        // Read the lock into a plain bool first: a guard taken inside an
        // `if let` scrutinee lives for the whole block, and ensure_relay
        // takes the same lock (that deadlocked this loop once).
        let connected = ctx.relay.lock().is_some();
        if !connected {
            if let Some((url, token)) = signaling::relay_target(&ctx, &ctx.server_url, &ctx.token) {
                let _ = transport::relay::ensure_relay(&ctx, &url, &token);
            }
        }
        if !tick(&ctx, Duration::from_secs(5)) {
            return;
        }
    });
}

/// Sleep for `d`; returns `false` once the engine is shutting down.
/// Sleep `d`, waking at once if the engine shuts down. False once it has:
/// the loop should exit, dropping its context (and with it the sockets the
/// context keeps alive). Sleeps the whole time: no polling wakeups.
fn tick<P: Platform>(ctx: &NetCtx<P>, d: Duration) -> bool {
    ctx.shutdown.wait(d)
}

fn clipboard_watcher<P: Platform>(ctx: NetCtx<P>) {
    while tick(&ctx, CLIPBOARD_POLL) {
        if !ctx.clip_active.load(Ordering::Relaxed) {
            continue;
        }
        let Some(cur) = ctx.platform.clipboard().and_then(|c| c.get()) else {
            continue;
        };
        if cur.is_empty() {
            continue;
        }
        let changed = {
            let mut last = ctx.clip_last.lock();
            if *last != cur {
                *last = cur.clone();
                true
            } else {
                false
            }
        };
        // A password (or a copy from an excluded app) stays here: not
        // synced, not kept. `clip_last` already holds it, so it isn't
        // looked at again until the clipboard changes.
        let private = ctx.platform.clipboard().is_some_and(|c| c.keep_private());
        if changed && !private {
            crate::features::clipboard::remember(&ctx, &cur, None);
            let me = ctx.identity.lock().node_id.clone();
            let msg =
                serde_json::json!({ "type": "clipboard", "content": cur, "from": me }).to_string();
            transport::broadcast_control(&ctx, &msg);
        }
    }
}

/// Poll the platform's local IP; when it changes, handle it as a network
/// change. Compares against the last IP the PLATFORM reported (not our
/// advertised one), so an explicit `network_changed` call doesn't make the
/// watcher fight it.
pub(super) fn spawn_ip_watcher<P: Platform>(h: netchange::NetHandles<P>) {
    std::thread::spawn(move || {
        let mut last = h.ctx.platform.local_ip();
        while tick(&h.ctx, IP_POLL) {
            let current = h.ctx.platform.local_ip();
            if current != last && current != "0.0.0.0" {
                last = current.clone();
                netchange::on_network_change(&h, Some(&current), crate::config::NetKind::Unknown);
            }
        }
    });
}

fn heartbeat<P: Platform>(ctx: NetCtx<P>) {
    use transport::TransportKind::WebRtc;
    // Same cadence as QUIC's keep-alive (CoreConfig::keepalive). Only WebRTC
    // links need it: QUIC (direct and punched) has keep-alive + idle timeout
    // built in, and the relay socket reports closure itself.
    let ka = ctx.config.keepalive;
    let every = ka.interval.unwrap_or(ka.idle_timeout / 3);
    let timeout_secs = ka.idle_timeout.as_secs().max(1);
    while tick(&ctx, every) {
        let me = ctx.identity.lock().node_id.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ping = serde_json::json!({ "type": "p2p_ping", "from": me }).to_string();
        for (node_id, kind, last_seen) in transport::live_snapshot(&ctx) {
            if kind != WebRtc {
                continue;
            }
            if now.saturating_sub(last_seen) > timeout_secs {
                transport::drop_live(&ctx, &node_id);
            } else if ka.interval.is_some() {
                transport::send_control_to(&ctx, &node_id, &ping);
            }
        }
    }
}
