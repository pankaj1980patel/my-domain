//! Chat messages on their way to a peer: each one stays here until the peer
//! acknowledges it. A message sent with no link waits for one (the connect
//! ladder was started by the send); a message sent on a link that then died,
//! or was replaced (relay to direct), is sent again on the next link. The
//! receiver drops repeats by id, so resending is always safe. Like any
//! messenger, "not connected yet, try again" never reaches the user, and a
//! message isn't lost because a link changed under it.
//!
//! Bounded: at most `MAX_PER_PEER` messages per peer (the oldest are dropped),
//! and a message older than `MAX_AGE_SECS` is discarded instead of sent late.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;

use super::{now_secs, send_plaintext};
use crate::engine::NetCtx;
use crate::model::Plaintext;
use crate::platform::Platform;

const MAX_PER_PEER: usize = 100;
const MAX_AGE_SECS: u64 = 10 * 60;
/// Sent on a live link but not acknowledged this long after: send it again.
const RESEND_SECS: u64 = 10;
/// Ids remembered per sender, to drop repeats.
const SEEN_PER_PEER: usize = 256;

/// One message waiting for its acknowledgement.
pub struct Queued {
    at: u64,
    /// When it last went out on a link (0 = not yet).
    sent_at: u64,
    id: String,
    from: String,
    text: String,
}

/// node_id -> messages not yet acknowledged, oldest first.
pub type Outbox = Arc<Mutex<HashMap<String, VecDeque<Queued>>>>;

/// sender node_id -> ids of the messages that arrived from it, newest last.
pub type Seen = Arc<Mutex<HashMap<String, VecDeque<String>>>>;

/// A fresh message id.
pub fn new_id() -> String {
    use rand_core::{OsRng, RngCore};
    let mut b = [0u8; 8];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Keep `text` for `node_id` until it's acknowledged.
pub fn hold<P: Platform>(ctx: &NetCtx<P>, node_id: &str, id: &str, from: &str, text: &str) {
    let mut out = ctx.outbox.lock();
    let q = out.entry(node_id.to_string()).or_default();
    if q.len() >= MAX_PER_PEER {
        q.pop_front();
    }
    q.push_back(Queued {
        at: now_secs(),
        sent_at: 0,
        id: id.to_string(),
        from: from.to_string(),
        text: text.to_string(),
    });
}

/// Send what's due for `node_id`, in order: messages not sent yet, ones sent
/// over `RESEND_SECS` ago, or (`all`, on a new link) everything unacknowledged.
/// Stops at the first failure (no link). Returns whether message `id` went out.
pub fn pump<P: Platform>(ctx: &NetCtx<P>, node_id: &str, all: bool, id: Option<&str>) -> bool {
    // Held for the whole pass, so two pumps can't interleave and reorder.
    let mut out = ctx.outbox.lock();
    let Some(q) = out.get_mut(node_id) else {
        return false;
    };
    let now = now_secs();
    q.retain(|m| now.saturating_sub(m.at) <= MAX_AGE_SECS);
    let mut sent_it = false;
    for m in q.iter_mut() {
        let due = all || m.sent_at == 0 || now.saturating_sub(m.sent_at) >= RESEND_SECS;
        if !due {
            continue;
        }
        let pt = Plaintext {
            from: m.from.clone(),
            text: m.text.clone(),
            id: Some(m.id.clone()),
            ack: None,
        };
        if send_plaintext(&ctx.ws_conns, &ctx.p2p_conns, &ctx.key, node_id, &pt).is_err() {
            break;
        }
        m.sent_at = now;
        sent_it |= id == Some(m.id.as_str());
    }
    if q.is_empty() {
        out.remove(node_id);
    }
    sent_it
}

/// `node_id` just got a link (new, or upgraded): send everything it hasn't
/// acknowledged. Runs on its own thread (we're called while a link is being
/// registered).
pub fn flush<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    if !ctx.outbox.lock().contains_key(node_id) {
        return;
    }
    let (ctx, node) = (ctx.clone(), node_id.to_string());
    std::thread::spawn(move || {
        pump(&ctx, &node, true, None);
    });
}

/// Resend overdue messages to every peer that has a link. Called on a timer.
///
/// A message sent on a link and still unacknowledged `RESEND_SECS` later
/// means the link is dead even if it looks alive: a phone whose CPU went to
/// sleep with the screen off neither answers nor closes it, and QUIC only
/// notices after its idle timeout. So instead of resending into it, close
/// it and reconnect; the connect ladder wakes the device by push, and the
/// new link gets everything unacknowledged.
pub fn retry_due<P: Platform>(ctx: &NetCtx<P>) {
    let nodes: Vec<String> = ctx.outbox.lock().keys().cloned().collect();
    for node in nodes {
        if !ctx.ws_conns.lock().contains_key(&node) {
            continue;
        }
        if unanswered(ctx, &node) {
            crate::engine::reconnect(ctx, &node);
        } else {
            pump(ctx, &node, false, None);
        }
    }
}

/// Whether a message went out on a link `RESEND_SECS` ago or more without an
/// ack. Marks them all unsent, so this fires once per dead link.
fn unanswered<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    let mut out = ctx.outbox.lock();
    let Some(q) = out.get_mut(node_id) else {
        return false;
    };
    let now = now_secs();
    let dead = q
        .iter()
        .any(|m| m.sent_at > 0 && now.saturating_sub(m.sent_at) >= RESEND_SECS);
    if dead {
        for m in q.iter_mut() {
            m.sent_at = 0;
        }
    }
    dead
}

/// `node_id` acknowledged message `id`. Returns whether it was waiting.
pub fn acked<P: Platform>(ctx: &NetCtx<P>, node_id: &str, id: &str) -> bool {
    let mut out = ctx.outbox.lock();
    let Some(q) = out.get_mut(node_id) else {
        return false;
    };
    let before = q.len();
    q.retain(|m| m.id != id);
    let found = q.len() != before;
    if q.is_empty() {
        out.remove(node_id);
    }
    found
}

/// Record that message `id` arrived from `node_id`. Returns false if it had
/// already arrived (a resend): don't show it twice.
pub fn first_arrival(seen: &Seen, node_id: &str, id: &str) -> bool {
    let mut seen = seen.lock();
    let ids = seen.entry(node_id.to_string()).or_default();
    if ids.iter().any(|x| x == id) {
        return false;
    }
    if ids.len() >= SEEN_PER_PEER {
        ids.pop_front();
    }
    ids.push_back(id.to_string());
    true
}

/// How many messages to `node_id` aren't acknowledged yet.
pub fn waiting<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> usize {
    ctx.outbox.lock().get(node_id).map_or(0, VecDeque::len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeat_is_recognised_and_old_ids_are_forgotten() {
        let seen = Seen::default();
        assert!(first_arrival(&seen, "a", "1"));
        assert!(!first_arrival(&seen, "a", "1"));
        assert!(first_arrival(&seen, "b", "1"), "per sender");
        for i in 0..SEEN_PER_PEER {
            first_arrival(&seen, "a", &format!("x{i}"));
        }
        assert!(first_arrival(&seen, "a", "1"), "bounded memory");
    }

    #[test]
    fn ids_are_unique_hex() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
