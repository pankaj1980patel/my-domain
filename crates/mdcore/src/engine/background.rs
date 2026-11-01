//! The engine's long-running loops, started once by `Engine::start`:
//!
//! * clipboard watcher: pushes local clipboard changes to peers (auto-sync),
//! * IP watcher: keeps the advertised local IP current across network changes,
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
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
/// A connectionless link with no traffic for this long is considered dead.
const LINK_TIMEOUT_SECS: u64 = 15;

pub(super) fn spawn_all<P: Platform>(ctx: &NetCtx<P>) {
    if ctx.platform.clipboard().is_some() {
        let ctx = ctx.clone();
        std::thread::spawn(move || clipboard_watcher(ctx));
    }
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || ip_watcher(ctx));
    }
    {
        let ctx = ctx.clone();
        std::thread::spawn(move || heartbeat(ctx));
    }
}

/// Sleep for `d`; returns `false` once the engine is shutting down.
fn tick<P: Platform>(ctx: &NetCtx<P>, d: Duration) -> bool {
    std::thread::sleep(d);
    !ctx.shutdown.load(Ordering::Relaxed)
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
        if changed {
            let me = ctx.identity.lock().node_id.clone();
            let msg =
                serde_json::json!({ "type": "clipboard", "content": cur, "from": me }).to_string();
            transport::broadcast_control(&ctx, &msg);
        }
    }
}

fn ip_watcher<P: Platform>(ctx: NetCtx<P>) {
    while tick(&ctx, IP_POLL) {
        let current = ctx.platform.local_ip();
        let mut id = ctx.identity.lock();
        if id.ip != current && current != "0.0.0.0" {
            id.ip = current;
        }
    }
}

fn heartbeat<P: Platform>(ctx: NetCtx<P>) {
    while tick(&ctx, HEARTBEAT_EVERY) {
        let me = ctx.identity.lock().node_id.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ping = serde_json::json!({ "type": "p2p_ping", "from": me }).to_string();
        for (node_id, kind, last_seen) in transport::live_snapshot(&ctx) {
            // WS links are event-driven (the socket reports closure itself).
            if kind == transport::TransportKind::Ws {
                continue;
            }
            if now.saturating_sub(last_seen) > LINK_TIMEOUT_SECS {
                transport::drop_live(&ctx, &node_id);
            } else {
                transport::send_control_to(&ctx, &node_id, &ping);
            }
        }
    }
}
