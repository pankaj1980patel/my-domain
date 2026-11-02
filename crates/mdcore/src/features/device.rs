//! Quick device actions between your own devices: ask for a device's status
//! (battery, charging), ring it (find my phone), and open a link on it. The
//! core only carries the messages; the host app does the device part (reads
//! the battery and pushes it with `Engine::set_device_status`, plays the ring,
//! opens the link) when it sees the event.
//!
//!   status_request { from }            asker → device
//!   status         { from, status{} }  device → asker
//!   ring           { from }            asker → device
//!   open_url       { from, url }       asker → device (http/https only)
//!   action         { from, name, args } asker → device
//!
//! `action` is the open-ended one: flashlight, volume, ringer mode, media
//! keys, ... The host app carries it out and lists the names it supports in
//! its status (`"actions": [...]`), so the asking side only offers buttons
//! that will work. Names are short lowercase words (`[a-z0-9_.]`, up to 40
//! characters); anything else is dropped.

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

pub struct DeviceHandler;

/// Only web links are opened: never `file:`, `intent:`, `javascript:` or an
/// app scheme a peer could use to trigger something on this device.
pub fn is_openable(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && url.len() <= 4096
        && !url.chars().any(char::is_control)
}

impl<P: Platform> ControlHandler<P> for DeviceHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["status_request", "status", "ring", "open_url", "action"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        match msg.kind() {
            "status_request" => {
                let me = ctx.identity.lock().node_id.clone();
                let mut status: serde_json::Value =
                    serde_json::from_str(&ctx.device_status.lock()).unwrap_or_default();
                if !status.is_object() {
                    status = serde_json::json!({});
                }
                status["platform"] = ctx.platform.platform_kind().into();
                let reply = serde_json::json!({ "type": "status", "from": me, "status": status })
                    .to_string();
                // Reliably: a reply lost on a link that just died would
                // otherwise never come (the resent request is a repeat).
                crate::engine::send_reliably(ctx, &from, &reply);
            }
            "status" => ctx.sink.emit(CoreEvent::DeviceStatus {
                from,
                status: msg.json("status"),
            }),
            "ring" => ctx.sink.emit(CoreEvent::Ring { from }),
            "open_url" => {
                let url = msg.str("url");
                if is_openable(&url) {
                    ctx.sink.emit(CoreEvent::OpenUrl {
                        from,
                        url: url.trim().to_string(),
                    });
                }
            }
            "action" => {
                let name = msg.str("name");
                if is_action_name(&name) {
                    ctx.sink.emit(CoreEvent::Action {
                        from,
                        name,
                        args: msg.json("args"),
                    });
                }
            }
            _ => {}
        }
    }
}

/// A valid action name: short, lowercase, `[a-z0-9_.]`.
pub fn is_action_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_are_opened() {
        assert!(is_openable("https://example.com/a?b=c"));
        assert!(is_openable("HTTP://example.com"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "intent://scan#Intent;end",
            "tel:123",
            "https://exa\nmple.com",
            "",
        ] {
            assert!(!is_openable(bad), "{bad}");
        }
    }

    #[test]
    fn action_names_are_short_lowercase_words() {
        assert!(is_action_name("flashlight"));
        assert!(is_action_name("media.play_pause"));
        assert!(is_action_name("volume2"));
        for bad in ["", "Flash", "rm -rf", "a/b", &"x".repeat(41)] {
            assert!(!is_action_name(bad), "{bad}");
        }
    }
}
