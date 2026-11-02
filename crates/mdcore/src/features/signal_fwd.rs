//! Signals between devices without the server. A signal for a peer we have a
//! live link to goes straight over that link. Otherwise it floods through the
//! mesh of live links: a device linked to the target hands it over, any other
//! device passes it to its own neighbours, for at most `MAX_HOPS` relays. Each
//! forwarder passes a given signal on only once, which stops loops. The
//! server is still tried too, so a signal arrives whether or not the server
//! is reachable. Duplicates (forwarded copies plus the server copy) are
//! dropped by the receiver.
//!
//! Every hop is end-to-end encrypted with the shared key, like any message.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};

use crate::engine::NetCtx;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

pub struct SignalFwdHandler;

/// How many recent signals we remember for de-duplication.
const RECENT: usize = 256;

/// How many devices a signal may pass through on its way to the target.
/// A family's devices are a small mesh; three relays reach any of them
/// while keeping the flood bounded.
pub const MAX_HOPS: u64 = 3;

fn frame(from: &str, to: &str, payload: &str, ttl: u64) -> String {
    serde_json::json!({"type": "signal_fwd", "from": from, "to": to, "payload": payload, "ttl": ttl})
        .to_string()
}

fn live_links<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    ctx.p2p_conns.lock().keys().cloned().collect()
}

/// Send `payload` (a serialized `Signal`) to `to` over the mesh of live links.
/// Returns `(direct, forwarded)`: whether it went straight to `to`, and
/// whether at least one neighbour was asked to forward it.
pub fn send<P: Platform>(ctx: &NetCtx<P>, to: &str, payload: &str) -> (bool, bool) {
    let me = ctx.identity.lock().node_id.clone();
    let live = live_links(ctx);
    if live.iter().any(|p| p == to) {
        send_control_to(ctx, to, &frame(&me, to, payload, 0));
        return (true, false);
    }
    // We never forward our own signal if it floods back to us.
    seen(ctx, ("fwd", me.as_str(), to, payload));
    let neighbours: Vec<&String> = live.iter().filter(|p| p.as_str() != to).collect();
    for n in &neighbours {
        send_control_to(ctx, n, &frame(&me, to, payload, MAX_HOPS));
    }
    (false, !neighbours.is_empty())
}

/// `true` the first time a (sender, signal) pair is seen.
pub fn first_time<P: Platform>(ctx: &NetCtx<P>, from: &str, payload: &str) -> bool {
    seen(ctx, (from, payload))
}

/// `true` the first time `key` is seen in the recent window.
fn seen<P: Platform>(ctx: &NetCtx<P>, key: impl Hash) -> bool {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    let key = h.finish();
    let mut recent = ctx.recent_signals.lock();
    if recent.contains(&key) {
        return false;
    }
    if recent.len() >= RECENT {
        recent.pop_front();
    }
    recent.push_back(key);
    true
}

/// Recently seen signals (de-duplication window).
pub type RecentSignals = parking_lot::Mutex<VecDeque<u64>>;

impl<P: Platform> ControlHandler<P> for SignalFwdHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["signal_fwd"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let (from, to, payload) = (msg.str("from"), msg.str("to"), msg.str("payload"));
        if from.is_empty() || payload.is_empty() {
            return;
        }
        let me = ctx.identity.lock().node_id.clone();
        if to == me {
            // handle_signal drops duplicates itself (one check, one place).
            let ctx = ctx.clone();
            std::thread::spawn(move || crate::engine::handle_signal(&ctx, &from, &payload));
            return;
        }
        // Hops left. Older builds sent `hops: 1` for "forward once".
        let ttl = msg
            .json("ttl")
            .as_u64()
            .or_else(|| msg.json("hops").as_u64())
            .unwrap_or(0)
            .min(MAX_HOPS);
        if ttl == 0 || !seen(ctx, ("fwd", from.as_str(), to.as_str(), payload.as_str())) {
            return;
        }
        let live = live_links(ctx);
        if live.contains(&to) {
            send_control_to(ctx, &to, &frame(&from, &to, &payload, 0));
        } else if ttl > 1 {
            for n in live.iter().filter(|n| **n != from) {
                send_control_to(ctx, n, &frame(&from, &to, &payload, ttl - 1));
            }
        }
    }
}
