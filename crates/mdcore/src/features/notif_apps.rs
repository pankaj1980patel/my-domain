//! App-notification pub/sub. A consumer asks a producer for its shareable app
//! list, then subscribes to specific packages; the subscription is stored on the
//! producer (in `NetCtx::subs`). When the producer captures a notification from
//! a subscribed app it forwards it as a normal `notification` (see
//! `Engine::share_app_notification`).
//!
//!   apps_request   { from }                          consumer → producer
//!   apps_list      { from, apps[], subscribed[] }    producer → consumer
//!   subscribe_apps { from, apps[], removals? }       consumer → producer
//!
//! `removals: true` asks the producer to also say when a shared notification
//! goes away there (`notif_removed`, see `features::notification`). Builds
//! that don't send it never get one: to them it would be an unknown message.
//!
//! A computer shares its own notifications the same way, under one package,
//! [`ALL_APPS`]: which of its apps go out is decided on the computer (its
//! own switch and per-app list), so a phone subscribes to that one name
//! (`Engine::subscribe_computer_notifications`). A phone only does that for
//! a computer whose status says `notif_share`; older computers never say
//! it, so they never get the subscription (and an older phone never sends
//! one). The name matches no Android package, so a phone producer never
//! sends anything for it.

use serde_json::Value;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The package a computer shares its own notifications under (see the
/// module docs).
pub const ALL_APPS: &str = "*";

pub struct NotifAppsHandler;

impl<P: Platform> ControlHandler<P> for NotifAppsHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["apps_request", "subscribe_apps", "apps_list"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        match msg.kind() {
            // Producer: reply with our app list + the requester's subscription.
            "apps_request" => {
                let from = msg.sender();
                if from.is_empty() {
                    return;
                }
                let me = ctx.identity.lock().node_id.clone();
                let apps: Value = serde_json::from_str(&ctx.installed_apps.lock())
                    .unwrap_or_else(|_| Value::Array(vec![]));
                let subscribed = ctx.subs.lock().get(&from).cloned().unwrap_or_default();
                let reply = serde_json::json!({
                    "type": "apps_list", "from": me, "apps": apps, "subscribed": subscribed,
                })
                .to_string();
                // Reliably: a reply lost on a link that just died would
                // otherwise never come (the resent request is a repeat).
                crate::engine::send_reliably(ctx, &from, &reply);
            }
            // Producer: store this peer's subscription (full enabled set).
            "subscribe_apps" => {
                let from = msg.sender();
                if from.is_empty() {
                    return;
                }
                let apps: Vec<String> = msg
                    .json("apps")
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut removal_subs = ctx.removal_subs.lock();
                if msg.json("removals").as_bool() == Some(true) {
                    removal_subs.insert(from.clone());
                } else {
                    removal_subs.remove(&from);
                }
                drop(removal_subs);
                ctx.subs.lock().insert(from, apps);
            }
            // Consumer: surface the producer's app list to the UI.
            "apps_list" => {
                ctx.sink.emit(CoreEvent::AppsList {
                    from: msg.sender(),
                    apps: msg.json("apps"),
                    subscribed: msg.json("subscribed"),
                });
            }
            _ => {}
        }
    }
}
