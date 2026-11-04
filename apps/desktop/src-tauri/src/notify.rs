// OS notifications, through the official notification plugin (it names the
// app properly, unlike the old `osascript` / `notify-send` shell-outs, and
// works on Windows too). Posted from Rust; the UI only flips the switch.
//
// What gets one: a ring (always, the window comes forward too), and while the
// window isn't focused, a chat message, a saved file and a notification
// shared from the phone. All of it can be turned off in Settings ("Show
// notifications"), kept in `<app config dir>/prefs.json`, on by default.
//
// "Stay quiet while my phone is on Do Not Disturb" (off by default, same
// file, key `follow_dnd`): while one of your phones is on Do Not Disturb (see
// `quiet`: a live link and a fresh word from it), pop-ups are held instead
// of shown, and when it ends one pop-up says how many came. What they're
// about still lands in its history (notifications, chat, clipboard).
//
// Two ways to post, and every caller picks one:
//  - `post` / `post_if_away`: everyday notices (mirrored phone
//    notifications, messages, clipboard and text, files, battery, automation,
//    schedule and webhook runs) are held while quiet;
//  - `post_always`: safety and consent, and what you just did yourself (a
//    ring, find-my, lost mode, location shared, someone watching this
//    screen, a lock because the phone went away, a permission this needs,
//    the answer to a tray or shortcut action) always show.
// Only "Show notifications" turned off stops those. The OS's own Do Not
// Disturb is never touched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use mdcore::kv::FileKv;
use tauri::plugin::PermissionState;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::Eng;

const KEY: &str = "notifications";
const DND_KEY: &str = "follow_dnd";
/// Most pop-ups held during Do Not Disturb (only the count is shown).
const MAX_HELD: usize = 200;

/// The "Show notifications" and "Stay quiet while my phone is on Do Not Disturb"
/// preferences, read once at startup, and the pop-ups held meanwhile.
pub struct Prefs {
    kv: Option<FileKv>,
    enabled: AtomicBool,
    follow_dnd: AtomicBool,
    /// Titles of the pop-ups held during Do Not Disturb.
    held: Mutex<Vec<String>>,
}

impl Prefs {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let enabled = kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref() != Some("off");
        let follow_dnd = kv.as_ref().and_then(|kv| kv.get(DND_KEY)).as_deref() == Some("on");
        Prefs {
            kv,
            enabled: AtomicBool::new(enabled),
            follow_dnd: AtomicBool::new(follow_dnd),
            held: Mutex::new(Vec::new()),
        }
    }

    pub fn follow_dnd(&self) -> bool {
        self.follow_dnd.load(Ordering::Relaxed)
    }

    pub fn set_follow_dnd(&self, on: bool) {
        self.follow_dnd.store(on, Ordering::Relaxed);
        self.save(DND_KEY, on);
    }

    fn save(&self, key: &str, on: bool) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, if on { "on" } else { "off" }) {
                eprintln!("[notify] couldn't save the setting: {e}");
            }
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        self.save(KEY, on);
    }
}

/// How a pop-up takes "quiet".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Held while a phone of yours is on Do Not Disturb (switch on).
    Everyday,
    /// Safety and consent: always shown.
    Always,
}

/// Hold this one instead of showing it?
pub fn holds(kind: Kind, follow_dnd: bool, phone_quiet: bool) -> bool {
    kind == Kind::Everyday && follow_dnd && phone_quiet
}

/// Post an everyday OS notification, unless they're turned off; held
/// instead while a phone of yours is on Do Not Disturb and this computer
/// stays quiet with it.
pub fn post(app: &AppHandle, title: &str, body: &str) {
    post_as(app, Kind::Everyday, title, body);
}

/// Post one even while quiet (safety, consent, a permission this needs, the
/// answer to something you just did): only "Show notifications" turned off
/// stops it.
pub fn post_always(app: &AppHandle, title: &str, body: &str) {
    post_as(app, Kind::Always, title, body);
}

fn post_as(app: &AppHandle, kind: Kind, title: &str, body: &str) {
    let Some(prefs) = app.try_state::<Prefs>() else {
        return;
    };
    // The phones are looked at only for a pop-up that could be held.
    let quiet = kind == Kind::Everyday && prefs.follow_dnd() && dnd_now(app);
    if holds(kind, prefs.follow_dnd(), quiet) {
        if let Ok(mut held) = prefs.held.lock() {
            if held.len() < MAX_HELD {
                held.push(title.to_string());
            }
        }
        return;
    }
    if !prefs.enabled() {
        return;
    }
    if !permitted(app) {
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("[notify] couldn't show a notification: {e}");
    }
}

/// One of your phones is on Do Not Disturb now (see `quiet`).
fn dnd_now(app: &AppHandle) -> bool {
    !crate::quiet::phones(app).is_empty()
}

/// A phone went into or out of Do Not Disturb (or the setting changed):
/// once none is in it any more, say how many pop-ups were held.
pub fn dnd_changed(app: &AppHandle) {
    let Some(prefs) = app.try_state::<Prefs>() else {
        return;
    };
    if prefs.follow_dnd() && dnd_now(app) {
        return;
    }
    let held = match prefs.held.lock() {
        Ok(mut h) => std::mem::take(&mut *h),
        Err(_) => return,
    };
    if let Some(body) = held_summary(&held) {
        post_always(app, "While Do Not Disturb was on", &body);
    }
}

/// "3 notifications: Asha, Files, Mom" (the first few titles).
fn held_summary(titles: &[String]) -> Option<String> {
    if titles.is_empty() {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    for t in titles {
        if !names.contains(&t.as_str()) {
            names.push(t);
        }
    }
    let shown = names.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    let more = if names.len() > 3 { ", \u{2026}" } else { "" };
    let what = if titles.len() == 1 {
        "1 notification".to_string()
    } else {
        format!("{} notifications", titles.len())
    };
    Some(format!("{what}: {shown}{more}"))
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

/// A peer's display name: the nickname you gave it here (see
/// `device_labels`), else its own name tidied ("Pankajs-MacBook-Pro.local"
/// reads as "Pankajs MacBook Pro"), the same as the UI (`peerName`).
pub fn device_name(app: &AppHandle, node_id: &str) -> String {
    let own = app.try_state::<Eng>().and_then(|e| {
        e.get_peers()
            .into_iter()
            .find(|p| p.node_id == node_id)
            .map(|p| p.name)
    });
    match own {
        Some(name) => crate::device_labels::display_name(app, node_id, &name),
        None => crate::device_labels::nickname(app, node_id)
            .unwrap_or_else(|| "One of your devices".to_string()),
    }
}

pub(crate) fn nice_name(name: &str) -> String {
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
    use super::{held_summary, holds, nice_name, Kind};

    #[test]
    fn only_everyday_pop_ups_are_held_and_only_while_quiet() {
        // Quiet, switch on: everyday held, safety shown.
        assert!(holds(Kind::Everyday, true, true));
        assert!(!holds(Kind::Always, true, true));
        // Switch off, or no phone quiet: nothing held.
        assert!(!holds(Kind::Everyday, false, true));
        assert!(!holds(Kind::Everyday, true, false));
        assert!(!holds(Kind::Always, false, false));
    }

    #[test]
    fn held_pop_ups_are_summed_up() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(held_summary(&[]), None);
        assert_eq!(
            held_summary(&t(&["Asha"])).as_deref(),
            Some("1 notification: Asha")
        );
        assert_eq!(
            held_summary(&t(&["Asha", "Asha", "Files", "Mom", "Bank"])).as_deref(),
            Some("5 notifications: Asha, Files, Mom, \u{2026}")
        );
    }

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
