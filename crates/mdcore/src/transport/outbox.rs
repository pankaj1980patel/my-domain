//! Messages sent while a peer had no link: held here and delivered the moment
//! a link comes up (the connect ladder was started by the send). Like any
//! messenger, so "not connected yet, try again" never reaches the user.
//!
//! Bounded: at most `MAX_PER_PEER` messages per peer (the oldest are dropped),
//! and a message older than `MAX_AGE_SECS` is discarded instead of sent late.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;

use super::{now_secs, send_payload};
use crate::engine::NetCtx;
use crate::platform::Platform;

const MAX_PER_PEER: usize = 100;
const MAX_AGE_SECS: u64 = 10 * 60;

/// One held message.
pub struct Queued {
    at: u64,
    from: String,
    text: String,
}

/// node_id -> messages waiting for its link.
pub type Outbox = Arc<Mutex<HashMap<String, VecDeque<Queued>>>>;

/// Hold `text` for `node_id` until it has a link.
pub fn hold<P: Platform>(ctx: &NetCtx<P>, node_id: &str, from: &str, text: &str) {
    let mut out = ctx.outbox.lock();
    let q = out.entry(node_id.to_string()).or_default();
    if q.len() >= MAX_PER_PEER {
        q.pop_front();
    }
    q.push_back(Queued {
        at: now_secs(),
        from: from.to_string(),
        text: text.to_string(),
    });
}

/// `node_id` just got a link: send what was waiting for it, in order. Runs
/// on its own thread (we're called while a link is being registered).
pub fn flush<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    let Some(waiting) = ctx.outbox.lock().remove(node_id) else {
        return;
    };
    let (ctx, node) = (ctx.clone(), node_id.to_string());
    std::thread::spawn(move || {
        let now = now_secs();
        let mut left = waiting
            .into_iter()
            .filter(|m| now.saturating_sub(m.at) <= MAX_AGE_SECS);
        while let Some(m) = left.next() {
            let sent = send_payload(
                &ctx.ws_conns,
                &ctx.p2p_conns,
                &ctx.key,
                &m.from,
                &node,
                &m.text,
            );
            if sent.is_err() {
                // The link went away again: keep the rest for the next one.
                let mut out = ctx.outbox.lock();
                let q = out.entry(node.clone()).or_default();
                for (i, m) in std::iter::once(m).chain(left).enumerate() {
                    q.insert(i, m);
                }
                return;
            }
        }
    });
}

/// How many messages are waiting for `node_id`.
pub fn waiting<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> usize {
    ctx.outbox.lock().get(node_id).map_or(0, VecDeque::len)
}
