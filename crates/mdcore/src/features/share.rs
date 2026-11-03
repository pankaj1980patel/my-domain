//! Sending a piece of text (a note, an address, a link) to one of your
//! devices: onto its clipboard, or shown there as a notification ("share
//! to my laptop").
//!
//!   share_text { from, text, mode }   sender → device
//!
//! `mode` is `"clipboard"` or `"notify"`. With `"clipboard"`, a device that
//! has a clipboard in the core (the desktop) puts it there and in the
//! clipboard history; either way the host app hears `TextShared` and does
//! the rest (the phone writes its own clipboard, and shows the notice).
//! Sent the reliable way, like a chat message.

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// Longest text accepted, in bytes.
pub const MAX_TEXT: usize = 256 * 1024;

/// The two ways shared text lands.
pub fn is_mode(mode: &str) -> bool {
    matches!(mode, "clipboard" | "notify")
}

pub struct ShareHandler;

impl<P: Platform> ControlHandler<P> for ShareHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["share_text"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let (text, mode) = (msg.str("text"), msg.str("mode"));
        if from.is_empty() || text.trim().is_empty() || text.len() > MAX_TEXT || !is_mode(&mode) {
            return;
        }
        if mode == "clipboard" {
            if let Some(clip) = ctx.platform.clipboard() {
                // Not sent on by the clipboard sync: it came from that device.
                *ctx.clip_last.lock() = text.clone();
                clip.set(&text);
                super::clipboard::remember(ctx, &text, Some(&from));
            }
        }
        ctx.sink.emit(CoreEvent::TextShared { from, text, mode });
    }
}
