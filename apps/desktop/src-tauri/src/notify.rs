// OS notifications, through the official notification plugin (it names the
// app properly, unlike the old `osascript` / `notify-send` shell-outs, and
// works on Windows too). Posted from Rust; the UI only flips the switch.
//
// What gets one: a ring (always, the window comes forward too), and while the
// window isn't focused, a chat message, a saved file and a notification
// shared from the phone. All of it can be turned off in Settings ("Show
// notifications"), kept in `<app config dir>/prefs.json`, on by default.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use mdcore::kv::FileKv;
use tauri::plugin::PermissionState;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::Eng;

const KEY: &str = "notifications";

/// The "Show notifications" preference, read once at startup.
pub struct Prefs {
    kv: Option<FileKv>,
    enabled: AtomicBool,
}

impl Prefs {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let enabled = kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref() != Some("off");
        Prefs {
            kv,
            enabled: AtomicBool::new(enabled),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
                eprintln!("[notify] couldn't save the setting: {e}");
            }
        }
    }
}

/// Post an OS notification, unless they're turned off.
pub fn post(app: &AppHandle, title: &str, body: &str) {
    if !app.try_state::<Prefs>().is_some_and(|p| p.enabled()) {
        return;
    }
    if !permitted(app) {
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("[notify] couldn't show a notification: {e}");
    }
}

/// Post one only while the main window isn't focused: when it is, the UI
/// already shows the same thing (activity, toast, the chat itself).
pub fn post_if_away(app: &AppHandle, title: &str, body: &str) {
    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if !focused {
        post(app, title, body);
    }
}

/// Asked once, on the first notification. Desktop builds of the plugin always
/// answer "granted" (macOS asks the user itself the first time one is shown),
/// but this keeps working if that changes.
fn permitted(app: &AppHandle) -> bool {
    static ASKED: Once = Once::new();
    static GRANTED: AtomicBool = AtomicBool::new(false);
    ASKED.call_once(|| {
        let n = app.notification();
        let state = match n.permission_state() {
            Ok(PermissionState::Granted) => PermissionState::Granted,
            _ => n.request_permission().unwrap_or(PermissionState::Denied),
        };
        GRANTED.store(state == PermissionState::Granted, Ordering::Relaxed);
    });
    GRANTED.load(Ordering::Relaxed)
}

/// A peer's display name ("Pankajs-MacBook-Pro.local" reads as "Pankajs
/// MacBook Pro"), the same tidy-up the UI does (`niceName`).
pub fn device_name(app: &AppHandle, node_id: &str) -> String {
    app.try_state::<Eng>()
        .and_then(|e| {
            e.get_peers()
                .into_iter()
                .find(|p| p.node_id == node_id)
                .map(|p| p.name)
        })
        .map(|name| nice_name(&name))
        .unwrap_or_else(|| "One of your devices".to_string())
}

fn nice_name(name: &str) -> String {
    let base = name.strip_suffix(".local").unwrap_or(name);
    let words: Vec<&str> = base.split(['-', '_']).filter(|w| !w.is_empty()).collect();
    let nice = words.join(" ");
    if nice.trim().is_empty() {
        name.to_string()
    } else {
        nice.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::nice_name;

    #[test]
    fn tidies_device_names() {
        assert_eq!(
            nice_name("Pankajs-MacBook-Pro.local"),
            "Pankajs MacBook Pro"
        );
        assert_eq!(nice_name("pixel_8__pro"), "pixel 8 pro");
        assert_eq!(nice_name("laptop"), "laptop");
        assert_eq!(nice_name("--"), "--");
    }
}
