//! Signals between devices without the server. A signal for a peer we have a
//! live link to goes straight over that link. Otherwise each connected
//! device that has a link to the target forwards it once (one hop). The
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

/// Send `payload` (a serialized `Signal`) to `to` over the mesh of live links.
/// Returns `(direct, forwarded)`: whether it went straight to `to`, and
/// whether at least one neighbour was asked to forward it.
pub fn send<P: Platform>(ctx: &NetCtx<P>, to: &str, payload: &str) -> (bool, bool) {
    let me = ctx.identity.lock().node_id.clone();
    let live: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    let frame = |hops: u8| {
        serde_json::json!({"type": "signal_fwd", "from": me, "to": to, "payload": payload, "hops": hops})
            .to_string()
    };
    if live.iter().any(|p| p == to) {
        send_control_to(ctx, to, &frame(0));
        return (true, false);
    }
    let neighbours: Vec<&String> = live.iter().filter(|p| p.as_str() != to).collect();
    for n in &neighbours {
        send_control_to(ctx, n, &frame(1));
    }
    (false, !neighbours.is_empty())
}

/// `true` the first time a (sender, signal) pair is seen.
pub fn first_time<P: Platform>(ctx: &NetCtx<P>, from: &str, payload: &str) -> bool {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (from, payload).hash(&mut h);
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
        // One hop only, and only to a target we're linked to.
        let hops = msg.json("hops").as_u64().unwrap_or(0);
        if hops >= 1 && ctx.p2p_conns.lock().contains_key(&to) {
            let fwd = serde_json::json!({"type": "signal_fwd", "from": from, "to": to, "payload": payload, "hops": 0})
                .to_string();
            send_control_to(ctx, &to, &fwd);
        }
    }
}
