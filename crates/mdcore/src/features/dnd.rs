//! Do Not Disturb follows the phone: when the phone goes into or out of
//! Do Not Disturb, your computers hear about it, and one that has the option
//! on holds my-domain's own notification pop-ups meanwhile. The OS's own Do
//! Not Disturb is never changed, on either side.
//!
//!   dnd { from, on, at }   phone → all your devices (`at`: unix ms)
//!
//! The host reads the state (Android: the interruption filter, readable
//! without any permission) and reports it with `Engine::set_dnd`. Only a
//! change is sent, the reliable way (held until each device is reachable,
//! a sleeping one woken by push); the last state sent is kept (`kv`), so a
//! restart doesn't send it again. A resend can overtake a newer message,
//! so a device keeps only the newest `at` from each sender.
//!
//! A receiving device keeps who is in Do Not Disturb (`Engine::dnd_active`)
//! and gets `CoreEvent::Dnd`. A state it hasn't heard again for `FORGET_AFTER`
//! is dropped, so a phone that went away mid-"on" doesn't hold pop-ups for
//! good.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// An "on" not heard again for this long is forgotten.
pub const FORGET_AFTER: Duration = Duration::from_secs(12 * 60 * 60);

const SENT_KEY: &str = "dnd_sent";

/// This device's last reported state, and which other devices are in Do
/// Not Disturb (since when).
#[derive(Default)]
pub struct Dnd {
    /// What was last sent: `None` until read from `kv`.
    sent: Mutex<Option<bool>>,
    others: Mutex<HashMap<String, Instant>>,
    /// The newest `at` heard from each device.
    newest: Mutex<HashMap<String, u64>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The host's Do Not Disturb state; sent to your other devices if it
/// changed.
pub fn set<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    {
        let mut sent = ctx.extras.dnd.sent.lock();
        let last =
            *sent.get_or_insert_with(|| ctx.platform.kv_get(SENT_KEY).as_deref() == Some("on"));
        if last == on {
            return;
        }
        *sent = Some(on);
        ctx.platform.kv_set(SENT_KEY, if on { "on" } else { "off" });
    }
    let me = ctx.identity.lock().node_id.clone();
    let msg = json!({ "type": "dnd", "from": me, "on": on, "at": now_ms() }).to_string();
    let peers: Vec<String> = ctx.peers.lock().keys().cloned().collect();
    for node in peers.iter().filter(|n| !n.starts_with("manual:")) {
        crate::engine::send_reliably(ctx, node, &msg);
    }
}

/// The devices in Do Not Disturb right now.
pub fn active<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let mut o = ctx.extras.dnd.others.lock();
    o.retain(|_, at| at.elapsed() < FORGET_AFTER);
    let mut nodes: Vec<String> = o.keys().cloned().collect();
    nodes.sort();
    nodes
}

pub struct DndHandler;

impl<P: Platform> ControlHandler<P> for DndHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["dnd"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let Some(on) = msg.json("on").as_bool() else {
            return;
        };
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        let at = msg.json("at").as_u64().unwrap_or(0);
        {
            let mut newest = ctx.extras.dnd.newest.lock();
            if newest.get(&from).is_some_and(|n| at < *n) {
                return; // an older state, resent late
            }
            newest.insert(from.clone(), at);
        }
        {
            let mut o = ctx.extras.dnd.others.lock();
            if on {
                o.insert(from.clone(), Instant::now());
            } else {
                o.remove(&from);
            }
        }
        ctx.sink.emit(CoreEvent::Dnd { from, on });
    }
}
