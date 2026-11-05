// Send what you copied to your phone with a keyboard shortcut, from any app:
// Cmd+Shift+Y on a Mac, Ctrl+Shift+Y elsewhere, out of the box.
//
//  - copied text goes onto the phone's clipboard (`send_text`, "clipboard");
//  - a copied picture goes as a clipboard picture (`send_clipboard_image`);
//  - "the phone" is your Android device: the one linked right now if there
//    are several, else the one seen most recently. Phones are remembered
//    from their status (`"platform": "android"`), also across restarts;
//  - a notification says what happened ("Sent to I2405", or that it goes
//    once the phone is reachable, or that there's no phone).
//
// The phones seen are kept in `<app config dir>/prefs.json` (key
// `send_shortcut_phones`). The keys that do this (and every other keyboard
// shortcut) are `shortcuts.rs`'s; the old `send_shortcut` and
// `send_shortcut_keys` prefs are read there once, to carry them over.

use std::collections::HashMap;
use std::sync::Mutex;

use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

/// Where the phones seen so far are kept, in `prefs.json`.
const KEY_PHONES: &str = "send_shortcut_phones";

/// How long a press waits for phones to answer when none is known yet.
const ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The phones seen so far (node id -> unix seconds last seen).
pub struct ShortcutPref {
    kv: Option<FileKv>,
    phones: Mutex<HashMap<String, u64>>,
}

impl ShortcutPref {
    fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let phones = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY_PHONES))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        ShortcutPref {
            kv,
            phones: Mutex::new(phones),
        }
    }

    fn save(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                eprintln!("[shortcut] couldn't save the setting: {e}");
            }
        }
    }

    fn phones(&self) -> HashMap<String, u64> {
        self.phones.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

/// Load the phones seen so far. The keys themselves are `shortcuts.rs`'s.
pub fn init(app: &AppHandle) {
    app.manage(ShortcutPref::load(app));
}

/// A device's status arrived: remember it if it's a phone.
pub fn saw_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    if status.get("platform").and_then(|p| p.as_str()) != Some("android") {
        return;
    }
    let Some(pref) = app.try_state::<ShortcutPref>() else {
        return;
    };
    let new = {
        let Ok(mut phones) = pref.phones.lock() else {
            return;
        };
        phones.insert(from.to_string(), now()).is_none()
    };
    // Written only when a phone is new: last-seen times stay in memory.
    if new {
        let json = serde_json::to_string(&pref.phones()).unwrap_or_default();
        pref.save(KEY_PHONES, &json);
    }
}

/// The node ids of the phones seen so far (also across restarts).
pub fn known_phones(app: &AppHandle) -> Vec<String> {
    app.try_state::<ShortcutPref>()
        .map(|pref| pref.phones().into_keys().collect())
        .unwrap_or_default()
}

/// Send this computer's clipboard to the phone, and say how it went.
pub(crate) fn send_copied(app: &AppHandle) {
    let Some(engine) = app.try_state::<Eng>() else {
        return;
    };
    let Some(phone) = find_phone(app, &engine) else {
        crate::notify::post_always(
            app,
            "No phone to send to",
            "Sign in to my-domain on your Android phone with the same account.",
        );
        return;
    };
    send_clipboard_to(app, &engine, &phone);
}

/// Send this computer's clipboard to `node`: text onto its clipboard, else a
/// copied picture. `Some(true)` when it went now, `Some(false)` when it goes
/// once the device is reachable, `None` when nothing is copied.
pub fn clipboard_to(app: &AppHandle, engine: &Eng, node: &str) -> Result<Option<bool>, CoreError> {
    let live = engine.links().iter().any(|(id, _)| *id == node);
    match crate::clip_image::copied_text().filter(|t| !t.trim().is_empty()) {
        Some(text) => engine.send_text(node, &text, "clipboard").map(|how| {
            crate::clip_history::on_sent(app, node, &text);
            Some(how != "queued")
        }),
        None => match crate::clip_image::copied_to_png() {
            Ok(path) => engine
                .send_clipboard_image(node, &path.to_string_lossy())
                .map(|_| {
                    crate::clip_history::on_picture(
                        app,
                        crate::clip_history::ClipDirection::Out,
                        node,
                        &path,
                    );
                    Some(live)
                }),
            Err(_) => Ok(None),
        },
    }
}

/// Send this computer's clipboard to `phone` (text onto its clipboard, else
/// a copied picture), and say how it went. Also the tray's "Send clipboard".
pub fn send_clipboard_to(app: &AppHandle, engine: &Eng, phone: &str) {
    let name = crate::notify::device_name(app, phone);
    match clipboard_to(app, engine, phone) {
        Ok(Some(true)) => {
            crate::notify::post_always(app, &format!("Sent to {name}"), "It's on its clipboard.")
        }
        Ok(Some(false)) => crate::notify::post_always(
            app,
            &format!("Will send when {name} is reachable"),
            "It goes as soon as your phone is back.",
        ),
        Ok(None) => {
            crate::notify::post_always(app, "Nothing to send", "Copy some text or a picture first.")
        }
        Err(e) => {
            crate::notify::post_always(app, &format!("Couldn't send to {name}"), &e.to_string())
        }
    }
}

/// Send this computer's clipboard to a device (the command palette).
/// Returns "sent", or "queued" when it goes once the device is reachable.
#[tauri::command]
#[specta::specta]
pub fn send_clipboard(
    node_id: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    match clipboard_to(&app, &engine, &node_id)? {
        Some(true) => Ok("sent".into()),
        Some(false) => Ok("queued".into()),
        None => Err(CoreError::InvalidInput(
            "Nothing to send. Copy some text or a picture first.".into(),
        )),
    }
}

/// The phone to send to; when none is known yet, asks your devices for
/// their status and waits a moment for a phone to answer.
fn find_phone(app: &AppHandle, engine: &Eng) -> Option<String> {
    let pref = app.try_state::<ShortcutPref>()?;
    let pick = || {
        let peers: Vec<String> = engine.get_peers().into_iter().map(|p| p.node_id).collect();
        let live: Vec<String> = engine.links().into_iter().map(|(id, _)| id).collect();
        pick_phone(&pref.phones(), &peers, &live)
    };
    if let Some(p) = pick() {
        return Some(p);
    }
    for p in engine.get_peers() {
        let _ = engine.request_status(&p.node_id);
    }
    let start = std::time::Instant::now();
    while start.elapsed() < ASK_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Some(p) = pick() {
            return Some(p);
        }
    }
    None
}

/// Which of your devices is "the phone": among the known phones (node id
/// -> last seen) that are still your devices, the one linked right now,
/// else the one seen most recently. Ties go to the smaller node id, so the
/// choice is stable.
pub fn pick_phone(
    phones: &HashMap<String, u64>,
    peers: &[String],
    live: &[String],
) -> Option<String> {
    phones
        .iter()
        .filter(|(id, _)| peers.contains(id))
        .max_by(|(a, a_seen), (b, b_seen)| {
            let a_live = live.contains(a);
            let b_live = live.contains(b);
            a_live.cmp(&b_live).then(a_seen.cmp(b_seen)).then(b.cmp(a))
        })
        .map(|(id, _)| id.clone())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn phones(v: &[(&str, u64)]) -> HashMap<String, u64> {
        v.iter().map(|(id, t)| (id.to_string(), *t)).collect()
    }

    #[test]
    fn shortcut_no_phone_no_pick() {
        assert_eq!(
            pick_phone(&phones(&[]), &ids(&["mac"]), &ids(&["mac"])),
            None
        );
        // A phone that's no longer one of your devices doesn't count.
        assert_eq!(
            pick_phone(&phones(&[("old", 9)]), &ids(&["mac"]), &ids(&[])),
            None
        );
    }

    #[test]
    fn shortcut_linked_phone_wins() {
        let known = phones(&[("a", 100), ("b", 50)]);
        let peers = ids(&["a", "b"]);
        assert_eq!(
            pick_phone(&known, &peers, &ids(&["b"])).as_deref(),
            Some("b")
        );
    }

    #[test]
    fn shortcut_else_most_recently_seen() {
        let known = phones(&[("a", 100), ("b", 50), ("c", 200)]);
        let peers = ids(&["a", "b"]);
        assert_eq!(pick_phone(&known, &peers, &ids(&[])).as_deref(), Some("a"));
        // Ties are stable.
        let tie = phones(&[("x", 5), ("y", 5)]);
        assert_eq!(
            pick_phone(&tie, &ids(&["x", "y"]), &ids(&[])).as_deref(),
            Some("x")
        );
    }
}
