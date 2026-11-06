//! Link liveness for links without their own keep-alive (WebRTC): a tiny
//! encrypted ping/pong. Receiving either refreshes the link's `last_seen`; the
//! engine's heartbeat sends the pings and evicts links that go stale. Like the other control messages, the sender's
//! node_id travels in the JSON body (`"from"`) — the carrying envelope's `from`
//! is only a display name.

use crate::engine::NetCtx;
use crate::platform::Platform;
use crate::transport::{send_control_to, touch_live};

use super::{ControlHandler, ControlMsg};

pub struct P2pPingHandler;

impl<P: Platform> ControlHandler<P> for P2pPingHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["p2p_ping", "p2p_pong"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        // The link's own device (not the body's `from`): a ping can only
        // vouch for the link it came over.
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        // Either direction proves the link is alive; a pong doubles as the
        // heartbeat ack.
        touch_live(ctx, &from);
        if msg.kind() == "p2p_ping" {
            let me = ctx.identity.lock().node_id.clone();
            let pong = serde_json::json!({ "type": "p2p_pong", "from": me }).to_string();
            send_control_to(ctx, &from, &pong);
        }
    }
}
