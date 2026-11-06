//! Sending a piece of text (a note, an address, a link) to one of your
//! devices: onto its clipboard, or shown there as a notification ("share
//! to my laptop").
//!
//!   share_text { from, text, mode, sensitive? }   sender → device
//!
//! `sensitive: true` (only sent when set, so older devices see the same
//! message and ignore it) marks a password copied from a password manager:
//! the receiver keeps it out of its clipboard history and notices, and the
//! phone puts it on its clipboard marked sensitive.
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

/// The `share_text` message. `sensitive` is only written when set.
pub fn message(from: &str, text: &str, mode: &str, sensitive: bool) -> String {
    let mut m =
        serde_json::json!({ "type": "share_text", "from": from, "text": text, "mode": mode });
    if sensitive {
        m["sensitive"] = serde_json::Value::Bool(true);
    }
    m.to_string()
}

pub struct ShareHandler;

impl<P: Platform> ControlHandler<P> for ShareHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["share_text"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let (text, mode) = (msg.str("text"), msg.str("mode"));
        if !super::known(ctx, &from)
            || text.trim().is_empty()
            || text.len() > MAX_TEXT
            || !is_mode(&mode)
        {
            return;
        }
        let sensitive = msg.flag("sensitive");
        if mode == "clipboard" {
            if let Some(clip) = ctx.platform.clipboard() {
                // Not sent on by the clipboard sync: it came from that device.
                *ctx.clip_last.lock() = text.clone();
                clip.set(&text);
                if !sensitive {
                    super::clipboard::remember(ctx, &text, Some(&from));
                }
            }
        }
        ctx.sink.emit(CoreEvent::TextShared {
            from,
            text,
            mode,
            sensitive,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::message;

    #[test]
    fn the_sensitive_flag_is_only_written_when_set() {
        let plain: serde_json::Value =
            serde_json::from_str(&message("a", "hi", "clipboard", false)).unwrap();
        // What an older device sends and expects: no new field.
        assert_eq!(
            plain,
            serde_json::json!({ "type": "share_text", "from": "a", "text": "hi", "mode": "clipboard" })
        );
        let secret: serde_json::Value =
            serde_json::from_str(&message("a", "pw", "clipboard", true)).unwrap();
        assert_eq!(secret["sensitive"], true);
        assert_eq!(secret["text"], "pw");
    }
}
