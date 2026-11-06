//! Quick device actions between your own devices: ask for a device's status
//! (battery, charging), ring it (find my phone), and open a link on it. The
//! core only carries the messages; the host app does the device part (reads
//! the battery and pushes it with `Engine::set_device_status`, plays the ring,
//! opens the link) when it sees the event.
//!
//!   status_request { from }            asker → device
//!   status         { from, status{} }  device → asker
//!   ring           { from }            asker → device
//!   open_url       { from, url, app? } asker → device (http/https only;
//!                                      `app`: open it in that app if installed)
//!   action         { from, name, args } asker → device
//!
//! `action` is the open-ended one: flashlight, volume, ringer mode, media
//! keys, ... The host app carries it out and lists the names it supports in
//! its status (`"actions": [...]`), so the asking side only offers buttons
//! that will work. Names are short lowercase words (`[a-z0-9_.]`, up to 40
//! characters); anything else is dropped.
//!
//! The core adds to every status reply: `platform`, `now_playing` (if
//! something plays) and `shared_folders`, how many folders this device lets
//! the others browse (`features::browse`; absent from older versions), and
//! `sync: 1` when it syncs folders (`features::folder_sync`), and
//! `labels: 1` when it shares device labels (`features::labels`), and
//! `notes: 1` when it shares notes (`features::notes`),
//! `ping: 1` when it answers pings and speed tests (`features::diag`), and
//! `wol` when it sends Wake-on-LAN packets for the others
//! (`features::wake`), and `print: 1` when it prints for your phones
//! (`features::print`).
//! A host that pushes `boot_at` (unix seconds it started) has it turned
//! into `uptime_s` at reply time, so the uptime is right however long ago
//! the status was pushed (it's pushed only when something changes).

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
                status["shared_folders"] = ctx.files.shared.lock().len().into();
                if let Some(boot) = status.as_object_mut().and_then(|o| o.remove("boot_at")) {
                    if let Some(up) = uptime(boot.as_u64(), crate::transport::now_secs()) {
                        status["uptime_s"] = up.into();
                    }
                }
                // Whether it plays or sends a phone's audio.
                super::audio_share::extend_status(ctx, &mut status);
                // Whether it syncs folders.
                super::folder_sync::extend_status(ctx, &mut status);
                // Whether it shares device labels.
                super::labels::extend_status(ctx, &mut status);
                // Whether it shares notes.
                super::notes::extend_status(ctx, &mut status);
                // Whether it shares the read-later list.
                super::read_later::extend_status(ctx, &mut status);
                // Whether it answers pings and speed tests.
                super::diag::extend_status(ctx, &mut status);
                // Whether it shares its link list (the map).
                super::links_map::extend_status(ctx, &mut status);
                // Whether it knows nearby mode and the smaller audio.
                super::nearby::extend_status(ctx, &mut status);
                super::audio_share::extend_codec(ctx, &mut status);
                // Whether it knows the camera messages.
                super::camera::extend_status(ctx, &mut status);
                // Whether it knows insert from phone.
                super::insert_capture::extend_status(ctx, &mut status);
                // Whether it shares timers.
                super::timers::extend_status(ctx, &mut status);
                // What it can do with Wi-Fi Direct.
                super::wifi_direct::extend_status(ctx, &mut status);
                // Whether it shares mouse and keyboard (and its OS).
                super::kvm::extend_status(ctx, &mut status);
                // Whether it prints for your phones.
                super::print::extend_status(ctx, &mut status);
                // Whether it downloads links for the others.
                super::remote_download::extend_status(ctx, &mut status);
                // Whether it knows rich notifications.
                super::notif_rich::extend_status(ctx, &mut status);
                // Whether it groups, pauses and re-checks transfers.
                super::batches::extend_status(ctx, &mut status);
                // Whether it takes media commands (and sends the fuller
                // now-playing).
                super::media::extend_status(ctx, &mut status);
                // Whether it sends wake-ups for the others (and may be woken).
                super::wake::extend_status(ctx, &mut status);
                // It's looking at this device: tell it when the song changes.
                super::media::watch(ctx, &from);
                let playing = super::media::current_for(ctx, &from);
                if !playing.is_null() {
                    status["now_playing"] = playing;
                }
                let reply = serde_json::json!({ "type": "status", "from": me, "status": status })
                    .to_string();
                // Reliably: a reply lost on a link that just died would
                // otherwise never come (the resent request is a repeat).
                crate::engine::send_reliably(ctx, &from, &reply);
            }
            "status" => {
                let status = msg.json("status");
                // How to wake it later, while it's asleep and says nothing.
                super::wake::remember(ctx, &from, &status);
                // Whether it plays or sends a phone's audio.
                super::audio_share::remember(ctx, &from, &status);
                super::folder_sync::remember(ctx, &from, &status);
                super::labels::remember(ctx, &from, &status[super::labels::FLAG]);
                super::notes::remember(ctx, &from, &status[super::notes::FLAG]);
                super::read_later::remember(ctx, &from, &status[super::read_later::FLAG]);
                super::diag::remember(ctx, &from, &status[super::diag::FLAG]);
                super::links_map::remember(ctx, &from, &status[super::links_map::FLAG]);
                super::nearby::remember(ctx, &from, &status[super::nearby::FLAG]);
                super::audio_share::remember_codec(
                    ctx,
                    &from,
                    &status[super::audio_share::CODEC_FLAG],
                );
                super::camera::remember(ctx, &from, &status[super::camera::FLAG]);
                super::insert_capture::remember(ctx, &from, &status[super::insert_capture::FLAG]);
                super::notif_rich::remember(ctx, &from, &status[super::notif_rich::FLAG]);
                super::timers::remember(ctx, &from, &status[super::timers::FLAG]);
                super::wifi_direct::remember(ctx, &from, &status[super::wifi_direct::FLAG]);
                super::kvm::remember(
                    ctx,
                    &from,
                    &status[super::kvm::FLAG],
                    &status[super::kvm::OS_FLAG],
                );
                super::print::remember(ctx, &from, &status[super::print::FLAG]);
                super::remote_download::remember(ctx, &from, &status[super::remote_download::FLAG]);
                super::batches::remember(ctx, &from, &status[super::batches::FLAG]);
                super::media::remember(ctx, &from, &status[super::media::FLAG]);
                super::media::saw(ctx, &from, &status["now_playing"]);
                ctx.sink.emit(CoreEvent::DeviceStatus { from, status });
            }
            "ring" => ctx.sink.emit(CoreEvent::Ring { from }),
            "open_url" => {
                let url = msg.str("url");
                if is_openable(&url) {
                    ctx.sink.emit(CoreEvent::OpenUrl {
                        from,
                        url: url.trim().to_string(),
                        app: msg.opt("app").filter(|a| is_package(a)),
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

/// Seconds since `boot_at` (unix seconds), or `None` if it's missing or in
/// the future (a clock that moved back).
fn uptime(boot_at: Option<u64>, now: u64) -> Option<u64> {
    now.checked_sub(boot_at.filter(|b| *b > 0)?)
}

/// An Android package name (`com.google.android.youtube`): what `open_url`
/// may name as the app to open a link in.
pub fn is_package(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    name.len() <= 100
        && parts.len() >= 2
        && parts.iter().all(|p| {
            p.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// Apps a `search` action can search in (the device lists the ones it has
/// in its status as `search_apps`).
pub const SEARCH_APPS: &[&str] = &["youtube", "spotify", "web"];

/// Longest search text.
pub const MAX_QUERY: usize = 500;

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
    fn uptime_counts_from_boot() {
        assert_eq!(uptime(Some(1_000), 4_600), Some(3_600));
        assert_eq!(uptime(Some(5_000), 4_600), None);
        assert_eq!(uptime(Some(0), 4_600), None);
        assert_eq!(uptime(None, 4_600), None);
    }

    #[test]
    fn package_names_look_like_packages() {
        assert!(is_package("com.google.android.youtube"));
        assert!(is_package("com.spotify.music"));
        for bad in [
            "",
            "youtube",
            "com..x",
            "com.1x",
            "com.x/y",
            "com.x;rm",
            &"a.b".repeat(50),
        ] {
            assert!(!is_package(bad), "{bad}");
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
