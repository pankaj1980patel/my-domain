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

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::send_control_to;

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
        &["status_request", "status", "ring", "open_url"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.str("from");
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
                let reply =
                    serde_json::json!({ "type": "status", "from": me, "status": status })
                        .to_string();
                send_control_to(ctx, &from, &reply);
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
            _ => {}
        }
    }
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
}
