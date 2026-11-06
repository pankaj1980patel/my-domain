//! Clipboard sharing: active push (`clipboard`) plus passive pull
//! (`clipboard_request` / `clipboard_response`).

use std::sync::atomic::Ordering;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// Values kept in the clipboard history (newest first).
pub const HISTORY_LEN: usize = 20;
/// Longer values are kept cut to this many characters.
const HISTORY_MAX_CHARS: usize = 100_000;

/// Put `text` at the top of the clipboard history (moving it there if it's
/// already in it) and tell the UI. Kept in memory only: clipboards hold
/// passwords and codes, which shouldn't end up on disk.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, text: &str, from: Option<&str>) {
    if text.trim().is_empty() {
        return;
    }
    let text: String = text.chars().take(HISTORY_MAX_CHARS).collect();
    let entries = {
        let mut h = ctx.clip_history.lock();
        if h.front().is_some_and(|e| e.text == text) {
            return;
        }
        h.retain(|e| e.text != text);
        h.push_front(crate::model::ClipEntry {
            text,
            from: from.map(str::to_string),
            ts: crate::transport::now_secs(),
        });
        h.truncate(HISTORY_LEN);
        h.iter().cloned().collect()
    };
    ctx.sink.emit(CoreEvent::ClipboardHistory { entries });
}

pub struct ClipboardHandler;

impl<P: Platform> ControlHandler<P> for ClipboardHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["clipboard", "clipboard_request", "clipboard_response"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        match msg.kind() {
            // Unsolicited push from a peer in active mode: only apply if we too
            // have auto-sync enabled and a clipboard.
            "clipboard" => {
                if ctx.clip_active.load(Ordering::Relaxed) {
                    if let Some(clip) = ctx.platform.clipboard() {
                        let c = msg.str("content");
                        *ctx.clip_last.lock() = c.clone();
                        clip.set(&c);
                        remember(ctx, &c, msg.opt("from").as_deref());
                        ctx.sink.emit(CoreEvent::Clipboard {
                            from: msg.from.to_string(),
                            ip: msg.ip.to_string(),
                            protocol: msg.protocol.to_string(),
                            action: "synced".into(),
                        });
                    }
                }
            }
            // A peer is pulling our clipboard: reply with the current value.
            "clipboard_request" => {
                let requester = msg.sender();
                if !requester.is_empty() {
                    let me = ctx.identity.lock().node_id.clone();
                    // A password on the clipboard isn't handed out.
                    let current = ctx
                        .platform
                        .clipboard()
                        .filter(|c| !c.keep_private())
                        .and_then(|c| c.get())
                        .unwrap_or_default();
                    let resp = serde_json::json!({
                        "type": "clipboard_response",
                        "content": current,
                        "from": me,
                    })
                    .to_string();
                    send_control_to(ctx, &requester, &resp);
                }
            }
            // Reply to a pull we initiated: hand it to the waiting command.
            "clipboard_response" => {
                if let Some(tx) = ctx.clip_pending.lock().remove(&msg.sender()) {
                    let _ = tx.send(msg.str("content"));
                }
            }
            _ => {}
        }
    }
}
