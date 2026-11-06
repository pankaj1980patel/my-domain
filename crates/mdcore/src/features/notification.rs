//! Notification sharing: mirror a peer's OS notification locally, and act
//! on one (press a button, reply, dismiss) from another device.
//!
//!   notification  { from, title, body, app?, key?, actions?, ... }
//!                 (the extra fields of `features::notif_rich`)
//!   notif_action  { from, key, index, text? }   (index -1: dismiss)
//!   notif_removed { from, key }                 (gone on the sending device)
//!
//! `notif_removed` goes only to subscribers that asked for it
//! (`subscribe_apps` with `removals: true`), so an older build never sees it.

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

pub struct NotificationHandler;

impl<P: Platform> ControlHandler<P> for NotificationHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["notification", "notif_action", "notif_removed"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        if msg.kind() == "notif_removed" {
            let key = msg.str("key");
            if !key.is_empty() {
                ctx.sink.emit(CoreEvent::NotificationRemoved {
                    from: msg.sender(),
                    key,
                });
            }
            return;
        }
        if msg.kind() == "notif_action" {
            let key = msg.str("key");
            if !key.is_empty() {
                ctx.sink.emit(CoreEvent::NotificationAction {
                    from: msg.sender(),
                    key,
                    index: msg.json("index").as_i64().unwrap_or(-1),
                    text: msg.opt("text"),
                });
            }
            return;
        }
        // The extra fields only from a build that knows them (and only
        // when this one does).
        let extra = if ctx.config.notif_rich {
            super::notif_rich::NotifExtra::from_value(msg.json_root())
        } else {
            super::notif_rich::NotifExtra::default()
        };
        if let Some(n) = ctx.platform.notifier() {
            n.post(&msg.str("title"), &msg.str("body"));
        }
        ctx.sink.emit(CoreEvent::Notification {
            from: msg.str("from"),
            title: msg.str("title"),
            body: msg.str("body"),
            app: msg.opt("app"),
            key: msg.opt("key"),
            actions: msg.json("actions"),
            extra,
        });
    }
}
