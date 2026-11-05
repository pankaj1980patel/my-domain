// This computer's own notifications on your phone: the reverse of the
// phone's notification mirror (`notif_history.rs`), over the same messages
// (mdcore `Engine::share_computer_notification`).
//
// This computer stays in charge:
//  - off until "Show this computer's notifications on my phone" is turned on
//    in Settings (`<app config dir>/prefs.json`, key `computer_notifs`);
//    nothing is read while it's off;
//  - each app that shows a notification is listed there as it's seen
//    (same file, key `computer_notif_apps`), shared until it's muted;
//  - my-domain's own notifications (the phone's, mirrored here) never go
//    back (they'd loop);
//  - a phone shows them only while its own switch for this computer is on
//    (it subscribes; this computer's status says `notif_share` so it knows
//    it may). Swiped away on the phone, a notification closes here too
//    while it's still on screen.
//
// Linux only (md-notify reads the session D-Bus). Windows: reading other
// apps' notifications needs a packaged app (UserNotificationListener and
// its manifest capability), which my-domain isn't yet. macOS has no way at
// all. Settings says so on both.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

const KEY: &str = "computer_notifs";
const APPS_KEY: &str = "computer_notif_apps";
/// The most apps listed (the oldest seen go first).
const MAX_APPS: usize = 200;
/// The most notification keys remembered as sent (for removals and
/// dismisses).
const MAX_SHARED: usize = 500;
/// Told to the window when the list of apps or the switch changes.
pub const CHANGED: &str = "computer-notifs-changed";

/// An app seen showing notifications here.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct NotifApp {
    /// What the list keys it by (its name, or GNOME's app id; "" when it
    /// gave none).
    pub id: String,
    /// Its name to show ("" when it gave none).
    pub name: String,
    /// Its notifications stay on this computer.
    pub muted: bool,
}

/// What the Settings tile shows.
#[derive(serde::Serialize, specta::Type)]
pub struct ComputerNotifsState {
    /// "linux", "macos" or "windows".
    pub os: String,
    /// This computer's notifications can be read here at all.
    pub supported: bool,
    pub on: bool,
    pub apps: Vec<NotifApp>,
    /// Why reading them didn't start (a sentence), while on.
    pub error: Option<String>,
    /// The devices showing them now (node ids).
    pub phones: Vec<String>,
}

pub struct ComputerNotifs {
    kv: Option<FileKv>,
    on: AtomicBool,
    apps: Mutex<Vec<NotifApp>>,
    watcher: Mutex<Option<md_notify::Watcher>>,
    error: Mutex<Option<String>>,
    /// Keys of the notifications sent, oldest first.
    shared: Mutex<VecDeque<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ComputerNotifs {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let on = kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref() == Some("on");
        let apps = kv
            .as_ref()
            .and_then(|kv| kv.get(APPS_KEY))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        ComputerNotifs {
            kv,
            on: AtomicBool::new(on),
            apps: Mutex::new(apps),
            watcher: Mutex::new(None),
            error: Mutex::new(None),
            shared: Mutex::new(VecDeque::new()),
        }
    }

    fn on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    fn save_apps(&self, apps: &[NotifApp]) {
        let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(apps)) else {
            return;
        };
        if let Err(e) = kv.set(APPS_KEY, &json) {
            eprintln!("[computer-notifs] couldn't save the app list: {e}");
        }
    }

    /// Add an app seen now (true when it's new); whether it's muted.
    fn seen_app(&self, id: &str, name: &str) -> (bool, bool) {
        let mut apps = lock(&self.apps);
        if let Some(a) = apps.iter().find(|a| a.id == id) {
            return (false, a.muted);
        }
        apps.push(NotifApp {
            id: id.to_string(),
            name: name.to_string(),
            muted: false,
        });
        if apps.len() > MAX_APPS {
            let extra = apps.len() - MAX_APPS;
            apps.drain(..extra);
        }
        self.save_apps(&apps);
        (true, false)
    }

    fn state(&self, engine: &Eng) -> ComputerNotifsState {
        ComputerNotifsState {
            os: std::env::consts::OS.to_string(),
            supported: md_notify::SUPPORTED,
            on: self.on(),
            apps: lock(&self.apps).clone(),
            error: lock(&self.error).clone(),
            phones: engine.computer_notification_subscribers(),
        }
    }
}

/// Start reading, if the switch is on (at startup, after the Engine).
pub fn init(app: &AppHandle) {
    let st = app.state::<ComputerNotifs>();
    if st.on() {
        start(app, &st);
    }
}

/// The names this app's own notifications go by here.
fn own_names(app: &AppHandle) -> Vec<String> {
    let mut names = vec!["my-domain".to_string(), app.config().identifier.clone()];
    if let Some(p) = &app.config().product_name {
        names.push(p.clone());
    }
    if let Some(stem) = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
    {
        names.push(stem);
    }
    names
}

fn start(app: &AppHandle, st: &ComputerNotifs) {
    if !md_notify::SUPPORTED {
        return;
    }
    let mut watcher = lock(&st.watcher);
    if watcher.is_some() {
        return;
    }
    let handle = app.clone();
    match md_notify::Watcher::start(own_names(app), move |c| on_change(&handle, c)) {
        Ok(w) => {
            *watcher = Some(w);
            *lock(&st.error) = None;
        }
        Err(e) => {
            eprintln!("[computer-notifs] {e}");
            *lock(&st.error) = Some(e);
        }
    }
}

fn stop(st: &ComputerNotifs) {
    // Dropping the watcher stops it (Linux).
    *lock(&st.watcher) = None;
    lock(&st.shared).clear();
    *lock(&st.error) = None;
}

/// A notification came or went here (the watcher's thread).
fn on_change(app: &AppHandle, change: md_notify::Change) {
    let st = app.state::<ComputerNotifs>();
    if !st.on() {
        return;
    }
    let Some(engine) = app.try_state::<Eng>() else {
        return;
    };
    match change {
        md_notify::Change::Posted(seen) => {
            let (new, muted) = st.seen_app(&seen.app_id, &seen.app);
            if new {
                let _ = app.emit(CHANGED, ());
            }
            if muted {
                return;
            }
            {
                let mut shared = lock(&st.shared);
                if !shared.contains(&seen.key) {
                    shared.push_back(seen.key.clone());
                    while shared.len() > MAX_SHARED {
                        shared.pop_front();
                    }
                }
            }
            engine.share_computer_notification(&seen.app, &seen.title, &seen.body, &seen.key);
        }
        md_notify::Change::Closed(key) => {
            let was_sent = {
                let mut shared = lock(&st.shared);
                let before = shared.len();
                shared.retain(|k| *k != key);
                shared.len() != before
            };
            if was_sent {
                engine.share_computer_notification_removed(&key);
            }
        }
    }
}

/// A device dismissed notification `key` (`notif_action`, index -1): close
/// it here if it's one this computer sent and still on screen.
pub fn on_dismiss(app: &AppHandle, key: &str) {
    let st = app.state::<ComputerNotifs>();
    if !st.on() || !lock(&st.shared).iter().any(|k| k == key) {
        return;
    }
    let (app, key) = (app.clone(), key.to_string());
    std::thread::spawn(move || {
        let st = app.state::<ComputerNotifs>();
        let watcher = lock(&st.watcher);
        if let Some(w) = watcher.as_ref() {
            w.dismiss(&key);
        }
    });
}

/// Put `notif_share` (the switch) in this computer's status where its
/// notifications can be read, so a phone knows it may ask for them.
pub fn extend(app: &AppHandle, status: &mut serde_json::Value) {
    if md_notify::SUPPORTED {
        if let Some(st) = app.try_state::<ComputerNotifs>() {
            status["notif_share"] = st.on().into();
        }
    }
}

/// The Settings tile: the switch, the apps seen, the phones showing them.
#[tauri::command]
#[specta::specta]
pub fn computer_notifs_state(st: State<ComputerNotifs>, engine: State<Eng>) -> ComputerNotifsState {
    st.state(&engine)
}

/// Turn "Show this computer's notifications on my phone" on or off.
#[tauri::command]
#[specta::specta]
pub fn computer_notifs_set_enabled(
    on: bool,
    app: AppHandle,
    st: State<ComputerNotifs>,
    engine: State<Eng>,
) -> ComputerNotifsState {
    st.on.store(on, Ordering::Relaxed);
    if let Some(kv) = &st.kv {
        if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
            eprintln!("[computer-notifs] couldn't save the setting: {e}");
        }
    }
    if on {
        start(&app, &st);
    } else {
        stop(&st);
    }
    crate::device::refresh_status(&app);
    st.state(&engine)
}

/// Mute an app (its notifications stay here) or share it again.
#[tauri::command]
#[specta::specta]
pub fn computer_notifs_set_muted(
    id: String,
    muted: bool,
    st: State<ComputerNotifs>,
    engine: State<Eng>,
) -> ComputerNotifsState {
    {
        let mut apps = lock(&st.apps);
        if let Some(a) = apps.iter_mut().find(|a| a.id == id) {
            a.muted = muted;
        }
        st.save_apps(&apps);
    }
    st.state(&engine)
}
