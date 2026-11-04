// Send what you copied to your phone with a keyboard shortcut, from any app:
// Cmd+Shift+Y on a Mac, Ctrl+Shift+Y elsewhere (changeable in Settings).
//
//  - copied text goes onto the phone's clipboard (`send_text`, "clipboard");
//  - a copied picture goes as a clipboard picture (`send_clipboard_image`);
//  - "the phone" is your Android device: the one linked right now if there
//    are several, else the one seen most recently. Phones are remembered
//    from their status (`"platform": "android"`), also across restarts;
//  - a notification says what happened ("Sent to I2405", or that it goes
//    once the phone is reachable, or that there's no phone).
//
// The switch and the keys live in `<app config dir>/prefs.json` (keys
// `send_shortcut` "on"/"off", `send_shortcut_keys`, `send_shortcut_phones`).
// On by default. The shortcut is registered with the OS through Tauri's
// global-shortcut plugin, from here (the UI never registers anything).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{
    GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::Eng;

const KEY_ON: &str = "send_shortcut";
const KEY_KEYS: &str = "send_shortcut_keys";
const KEY_PHONES: &str = "send_shortcut_phones";

/// The shortcut out of the box.
pub const DEFAULT_KEYS: &str = "CmdOrCtrl+Shift+Y";

/// How long a press waits for phones to answer when none is known yet.
const ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The switch, the keys, and the phones seen so far (node id -> unix
/// seconds last seen).
pub struct ShortcutPref {
    kv: Option<FileKv>,
    on: AtomicBool,
    keys: Mutex<String>,
    /// Why the shortcut couldn't be registered (taken by another app...).
    problem: Mutex<Option<String>>,
    phones: Mutex<HashMap<String, u64>>,
}

/// What Settings shows.
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct ShortcutSettings {
    pub on: bool,
    /// As stored, e.g. "CmdOrCtrl+Shift+Y".
    pub keys: String,
    /// For people, e.g. "Cmd+Shift+Y" on a Mac.
    pub label: String,
    /// Set when the OS wouldn't take the shortcut.
    pub problem: Option<String>,
}

impl ShortcutPref {
    fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
        let on = get(KEY_ON).as_deref() != Some("off");
        let keys = get(KEY_KEYS)
            .filter(|k| parse(k).is_ok())
            .unwrap_or_else(|| DEFAULT_KEYS.to_string());
        let phones = get(KEY_PHONES)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        ShortcutPref {
            kv,
            on: AtomicBool::new(on),
            keys: Mutex::new(keys),
            problem: Mutex::new(None),
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

    fn keys(&self) -> String {
        self.keys
            .lock()
            .map(|k| k.clone())
            .unwrap_or_else(|_| DEFAULT_KEYS.to_string())
    }

    fn settings(&self) -> ShortcutSettings {
        let keys = self.keys();
        ShortcutSettings {
            on: self.on.load(Ordering::Relaxed),
            label: label(&keys),
            keys,
            problem: self.problem.lock().ok().and_then(|p| p.clone()),
        }
    }

    fn set_problem(&self, why: Option<String>) {
        if let Ok(mut p) = self.problem.lock() {
            *p = why;
        }
    }

    fn phones(&self) -> HashMap<String, u64> {
        self.phones.lock().map(|p| p.clone()).unwrap_or_default()
    }
}

/// Load the setting, add the plugin, and register the shortcut if it's on.
pub fn init(app: &AppHandle) {
    app.manage(ShortcutPref::load(app));
    let plugin = tauri_plugin_global_shortcut::Builder::new()
        .with_handler(on_event)
        .build();
    if let Err(e) = app.plugin(plugin) {
        eprintln!("[shortcut] no global shortcuts here: {e}");
        if let Some(pref) = app.try_state::<ShortcutPref>() {
            pref.set_problem(Some("Shortcuts aren't available on this computer.".into()));
        }
        return;
    }
    apply(app);
}

/// Register the stored keys if the switch is on, unregister otherwise.
fn apply(app: &AppHandle) {
    let Some(pref) = app.try_state::<ShortcutPref>() else {
        return;
    };
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        eprintln!("[shortcut] couldn't clear the shortcut: {e}");
    }
    pref.set_problem(None);
    if !pref.on.load(Ordering::Relaxed) {
        return;
    }
    let keys = pref.keys();
    let result = parse(&keys).and_then(|s| gs.register(s).map_err(|e| e.to_string()));
    if let Err(e) = result {
        eprintln!("[shortcut] couldn't register {keys}: {e}");
        pref.set_problem(Some(format!(
            "{} couldn't be used: another app may have it. Try other keys.",
            label(&keys)
        )));
    }
}

/// A press of the shortcut (from the OS's event thread).
fn on_event(app: &AppHandle, _shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || send_copied(&app));
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
fn send_copied(app: &AppHandle) {
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
                .map(|_| Some(live)),
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

/// Keys in the plugin's format ("CmdOrCtrl+Shift+Y"), checked: one key and
/// at least one of Cmd, Ctrl or Alt (Shift alone would steal typing).
pub fn parse(keys: &str) -> Result<Shortcut, String> {
    let s = Shortcut::from_str(keys.trim()).map_err(|_| {
        format!("\u{201c}{keys}\u{201d} isn't a shortcut. Write it like CmdOrCtrl+Shift+Y.")
    })?;
    if !s
        .mods
        .intersects(Modifiers::SUPER | Modifiers::CONTROL | Modifiers::ALT)
    {
        return Err("Add Cmd, Ctrl or Alt, like CmdOrCtrl+Shift+Y.".into());
    }
    Ok(s)
}

/// Keys as people read them on this computer ("Cmd+Shift+Y" on a Mac).
pub fn label(keys: &str) -> String {
    let cmd_or_ctrl = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    keys.split('+')
        .map(|k| {
            let k = k.trim();
            match k.to_uppercase().as_str() {
                "COMMANDORCONTROL" | "COMMANDORCTRL" | "CMDORCTRL" | "CMDORCONTROL" => {
                    cmd_or_ctrl.to_string()
                }
                "COMMAND" | "CMD" | "SUPER" => "Cmd".into(),
                "CONTROL" | "CTRL" => "Ctrl".into(),
                "OPTION" | "ALT" => "Alt".into(),
                "SHIFT" => "Shift".into(),
                _ if k.len() == 1 => k.to_uppercase(),
                _ => k.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// --- commands ---

/// The "send what I copy to my phone" shortcut: on or off, and its keys.
#[tauri::command]
#[specta::specta]
pub fn send_shortcut(pref: State<ShortcutPref>) -> ShortcutSettings {
    pref.settings()
}

/// Turn the shortcut on or off.
#[tauri::command]
#[specta::specta]
pub fn set_send_shortcut(on: bool, app: AppHandle, pref: State<ShortcutPref>) -> ShortcutSettings {
    pref.on.store(on, Ordering::Relaxed);
    pref.save(KEY_ON, if on { "on" } else { "off" });
    apply(&app);
    pref.settings()
}

/// Use other keys ("CmdOrCtrl+Shift+Y"); refused if they aren't a shortcut.
#[tauri::command]
#[specta::specta]
pub fn set_send_shortcut_keys(
    keys: String,
    app: AppHandle,
    pref: State<ShortcutPref>,
) -> Result<ShortcutSettings, CoreError> {
    parse(&keys).map_err(CoreError::InvalidInput)?;
    let keys = keys.trim().to_string();
    if let Ok(mut k) = pref.keys.lock() {
        k.clone_from(&keys);
    }
    pref.save(KEY_KEYS, &keys);
    apply(&app);
    Ok(pref.settings())
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

    #[test]
    fn shortcut_keys_are_checked() {
        assert!(parse(DEFAULT_KEYS).is_ok());
        assert!(parse("Ctrl+Alt+P").is_ok());
        assert!(parse("Shift+Y").is_err());
        assert!(parse("Y").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+Shift+Nope").is_err());
        let mac = cfg!(target_os = "macos");
        assert_eq!(
            label("CmdOrCtrl+Shift+y"),
            if mac { "Cmd+Shift+Y" } else { "Ctrl+Shift+Y" }
        );
    }
}
