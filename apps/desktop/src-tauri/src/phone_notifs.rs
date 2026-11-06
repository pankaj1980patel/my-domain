// The phone's notifications as they are now, on this computer: grouped by
// conversation, with the sender's picture, any picture in them and the
// app's icon, and the settings that decide what comes and what shows.
//
//  - Kept in memory (newest first, at most `MAX_PER_DEVICE` per device):
//    gone on the phone, dismissed or acted on from here, they leave. The
//    same notification again (same device and key) replaces the older; an
//    update with nothing new, or one the phone says is a quiet update,
//    gets no pop-up.
//  - Grouped (`group`): one conversation (the phone's `conv`), or for a
//    messaging app without one, one sender (its title), is one group; the
//    rest stand alone.
//  - Private mode (on by default; `prefs.json` key `notif_hide_sensitive`): the
//    phone leaves the content of a sensitive notification out (a code, a
//    bank, a password manager: it says `private`), and anything that still
//    looks sensitive here (`needs_hiding`: an older phone doesn't say) has
//    its content dropped before it's shown, popped up or kept. Shown as
//    "New notification" from its app.
//  - Only while the phone isn't in use (off by default; key
//    `notif_only_locked`): the phone sends none while it's unlocked with
//    its screen on.
//  - Both go to the phone with the subscription (`subscribe`), which is
//    kept here per device (`notif_subscribed`) so a change, or a phone
//    coming back, gets it again.
//  - App icons: the phone names each app's icon; one this computer hasn't
//    got is asked for once per device and app, and kept in
//    `<app config dir>/notif-icons/` (`<pkg>.jpg`, `index.json`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine as _;
use mdcore::features::notif_rich::{pkg_ok, NotifExtra};
use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

const PRIVATE_KEY: &str = "notif_hide_sensitive";
const LOCKED_KEY: &str = "notif_only_locked";
const SUBSCRIBED_KEY: &str = "notif_subscribed";
const ICON_DIR: &str = "notif-icons";
/// The most notifications kept per device.
pub const MAX_PER_DEVICE: usize = 60;
/// What a hidden notification says.
pub const HIDDEN: &str = "New notification";

/// One of a notification's buttons: its place on the phone and its name.
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct NotifButton {
    pub index: u32,
    pub title: String,
}

/// One message of a conversation.
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct NotifLine {
    pub sender: String,
    pub text: String,
    /// When, in ms since 1970 (0: unknown).
    pub ts: f64,
}

/// A phone notification as shown here.
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct PhoneNotif {
    pub id: u32,
    /// The device it came from (node id).
    pub from: String,
    /// The app's name ("" when it didn't say).
    pub app: String,
    pub pkg: Option<String>,
    pub title: String,
    pub body: String,
    /// Its key on the phone; without one it can't be acted on.
    pub key: Option<String>,
    /// Its buttons (not the reply one).
    pub buttons: Vec<NotifButton>,
    /// The button that takes a reply, if any.
    pub reply: Option<NotifButton>,
    pub conv: Option<String>,
    pub conv_title: Option<String>,
    pub group_chat: bool,
    pub sender: Option<String>,
    pub messages: Vec<NotifLine>,
    /// The sender's picture, as a data URL.
    pub avatar: Option<String>,
    /// A picture in it, as a data URL.
    pub picture: Option<String>,
    /// Its app icon's id.
    pub icon: Option<String>,
    /// Its content was left out (private mode).
    pub private: bool,
    /// A quiet update (no pop-up).
    pub silent: bool,
    /// When it arrived here, in ms since 1970.
    pub ts: f64,
    /// A button was pressed or a reply sent from here.
    pub handled: bool,
}

/// What the device page shows: one conversation, one sender, or one
/// notification.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct NotifGroup {
    pub id: String,
    pub from: String,
    pub app: String,
    pub pkg: Option<String>,
    /// The conversation's name, else the newest one's title.
    pub title: String,
    /// The app's icon, as a data URL, when it's here.
    pub icon: Option<String>,
    /// The newest sender picture.
    pub avatar: Option<String>,
    pub group_chat: bool,
    pub private: bool,
    /// The newest one's time.
    pub ts: f64,
    /// Newest first.
    pub items: Vec<PhoneNotif>,
}

/// The two settings.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct NotifPrefs {
    pub private_mode: bool,
    pub only_locked: bool,
}

// --- the rules (pure) ----------------------------------------------------------

/// The buttons of `actions` (`[{title, reply}]`, in the phone's order): the
/// first reply one is the reply box, other reply ones and untitled ones
/// (the phone's own suggestions) aren't shown.
pub fn buttons_from(actions: &serde_json::Value) -> (Vec<NotifButton>, Option<NotifButton>) {
    let mut buttons = Vec::new();
    let mut reply = None;
    for (i, a) in actions.as_array().into_iter().flatten().enumerate() {
        let title = a.get("title").and_then(|t| t.as_str()).unwrap_or("").trim();
        let is_reply = a.get("reply").and_then(|r| r.as_bool()) == Some(true);
        let b = NotifButton {
            index: i as u32,
            title: title.to_string(),
        };
        if is_reply {
            if reply.is_none() {
                reply = Some(b);
            }
        } else if !title.is_empty() {
            buttons.push(b);
        }
    }
    (buttons, reply)
}

/// An app that deals in money or passwords, by its name or package (the
/// phone's own rule, for a phone that doesn't say).
pub fn sensitive_app(app: &str, pkg: Option<&str>) -> bool {
    const PARTS: &[&str] = &[
        "pay",
        "upi",
        "paisa",
        "paytm",
        "phonepe",
        "otp",
        "authenticator",
        "authenticator2",
        "authy",
        "bitwarden",
        "lastpass",
        "onepassword",
        "keepass",
        "keepassdx",
        "dashlane",
        "keeper",
        "vault",
        "imobile",
        "netbanking",
        "yono",
        "lotusintouch",
    ];
    const WORDS: &[&str] = &[
        "bank",
        "banking",
        "wallet",
        "authenticator",
        "password",
        "passwords",
        "pay",
        "upi",
    ];
    let by_pkg = pkg.is_some_and(|p| {
        p.to_lowercase().split(['.', '_']).any(|part| {
            PARTS.contains(&part)
                || part.contains("bank")
                || part.contains("wallet")
                || part.starts_with("pay")
                || part.ends_with("pay")
        })
    });
    let lower = app.to_lowercase();
    let by_name = lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| WORDS.contains(&w));
    by_pkg || by_name
}

/// Text that carries a one-time code: a code word and a run of 4 to 10
/// digits.
pub fn looks_like_code(text: &str) -> bool {
    const WORDS: &[&str] = &[
        "otp",
        "code",
        "passcode",
        "password",
        "pin",
        "verification",
        "one-time",
        "one time",
    ];
    let lower = text.to_lowercase();
    if !WORDS.iter().any(|w| lower.contains(w)) {
        return false;
    }
    let mut run = 0;
    let mut best = 0;
    for c in lower.chars() {
        if c.is_ascii_digit() {
            run += 1;
            best = best.max(run);
        } else if c != ' ' && c != '-' {
            run = 0;
        }
    }
    (4..=10).contains(&best)
}

/// Its content has to go before it's shown here: private mode is on, the
/// phone didn't already leave it out, and it looks sensitive.
pub fn needs_hiding(n: &PhoneNotif, private_mode: bool, sensitive: bool) -> bool {
    if !private_mode || n.private {
        return false;
    }
    let said = std::iter::once(n.title.as_str())
        .chain(std::iter::once(n.body.as_str()))
        .chain(n.messages.iter().map(|m| m.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    sensitive || sensitive_app(&n.app, n.pkg.as_deref()) || looks_like_code(&said)
}

/// Its content dropped: only the app, its icon and the key stay (so it can
/// still be dismissed).
pub fn hide_content(n: &mut PhoneNotif) {
    n.title.clear();
    n.body.clear();
    n.buttons.clear();
    n.reply = None;
    n.conv_title = None;
    n.sender = None;
    n.messages.clear();
    n.avatar = None;
    n.picture = None;
    n.private = true;
}

/// What it groups under: its conversation, else for one that takes a reply
/// its sender (title), else itself.
pub fn group_key(n: &PhoneNotif) -> String {
    let app = n.pkg.as_deref().unwrap_or(&n.app);
    if let Some(conv) = n.conv.as_deref().filter(|c| !c.is_empty()) {
        return format!("{}|{app}|c:{conv}", n.from);
    }
    if n.reply.is_some() && !n.title.is_empty() && !n.private {
        return format!("{}|{app}|t:{}", n.from, n.title);
    }
    match &n.key {
        Some(k) => format!("{}|{app}|k:{k}", n.from),
        None => format!("{}|{app}|i:{}", n.from, n.id),
    }
}

/// `list` (newest first) in groups, newest group first; `icons`: package ->
/// icon data URL.
pub fn group(list: &[PhoneNotif], icons: &HashMap<String, String>) -> Vec<NotifGroup> {
    let mut groups: Vec<NotifGroup> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for n in list {
        let id = group_key(n);
        if let Some(&i) = at.get(&id) {
            groups[i].items.push(n.clone());
            if groups[i].avatar.is_none() {
                groups[i].avatar = n.avatar.clone();
            }
            continue;
        }
        at.insert(id.clone(), groups.len());
        let title = n
            .conv_title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| n.title.clone());
        groups.push(NotifGroup {
            id,
            from: n.from.clone(),
            app: n.app.clone(),
            pkg: n.pkg.clone(),
            title,
            icon: n.pkg.as_ref().and_then(|p| icons.get(p).cloned()),
            avatar: n.avatar.clone(),
            group_chat: n.group_chat,
            private: n.private,
            ts: n.ts,
            items: vec![n.clone()],
        });
    }
    groups
}

/// The same thing again: nothing a person would see changed.
pub fn same_content(a: &PhoneNotif, b: &PhoneNotif) -> bool {
    a.title == b.title
        && a.body == b.body
        && a.messages == b.messages
        && a.picture.is_some() == b.picture.is_some()
}

/// Keep `n` first in `list`, replacing the one with the same device and key
/// and dropping that device's oldest past `max`. Whether it was the same
/// thing again.
pub fn upsert(list: &mut Vec<PhoneNotif>, n: PhoneNotif, max: usize) -> bool {
    let mut repeat = false;
    if let Some(key) = &n.key {
        if let Some(i) = list
            .iter()
            .position(|e| e.from == n.from && e.key.as_ref() == Some(key))
        {
            repeat = same_content(&list[i], &n);
            list.remove(i);
        }
    }
    let from = n.from.clone();
    list.insert(0, n);
    let mut seen = 0;
    list.retain(|e| {
        if e.from != from {
            return true;
        }
        seen += 1;
        seen <= max
    });
    repeat
}

/// What arrived, as shown here (before private mode).
#[allow(clippy::too_many_arguments)]
pub fn build(
    id: u32,
    from: &str,
    app: Option<&str>,
    title: &str,
    body: &str,
    key: Option<&str>,
    actions: &serde_json::Value,
    extra: &NotifExtra,
    ts: f64,
) -> PhoneNotif {
    let data_url =
        |b64: &Option<String>| b64.as_ref().map(|b| format!("data:image/jpeg;base64,{b}"));
    let (buttons, reply) = if key.is_some_and(|k| !k.is_empty()) {
        buttons_from(actions)
    } else {
        (Vec::new(), None)
    };
    PhoneNotif {
        id,
        from: from.to_string(),
        app: app.unwrap_or("").trim().to_string(),
        pkg: extra.pkg.clone(),
        title: title.to_string(),
        body: body.to_string(),
        key: key.filter(|k| !k.is_empty()).map(str::to_string),
        buttons,
        reply,
        conv: extra.conv.clone(),
        conv_title: extra.conv_title.clone(),
        group_chat: extra.group_chat,
        sender: extra.sender.clone(),
        messages: extra
            .messages
            .iter()
            .map(|m| NotifLine {
                sender: m.sender.clone(),
                text: m.text.clone(),
                ts: m.ts as f64,
            })
            .collect(),
        avatar: data_url(&extra.avatar),
        picture: data_url(&extra.picture),
        icon: extra.icon.clone(),
        private: extra.private,
        silent: extra.silent,
        ts,
        handled: false,
    }
}

/// A package name as a file name (only what `pkg_ok` lets through).
fn icon_file(pkg: &str) -> Option<String> {
    (pkg_ok(pkg) && pkg != "*").then(|| format!("{pkg}.jpg"))
}

// --- state -----------------------------------------------------------------------

struct Inner {
    list: Vec<PhoneNotif>,
    next_id: u32,
    prefs: NotifPrefs,
    /// Device -> the apps last subscribed to there.
    subscribed: HashMap<String, Vec<String>>,
    /// Package -> (icon id, data URL).
    icons: HashMap<String, (String, String)>,
    /// (device, package) icons asked for since start.
    asked: HashSet<(String, String)>,
}

pub struct PhoneNotifs {
    kv: Option<FileKv>,
    icon_dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl PhoneNotifs {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let kv = dir.as_ref().map(|d| FileKv::new(d.join("prefs.json")));
        let icon_dir = dir.map(|d| d.join(ICON_DIR));
        let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
        let prefs = NotifPrefs {
            private_mode: get(PRIVATE_KEY).as_deref() != Some("off"),
            only_locked: get(LOCKED_KEY).as_deref() == Some("on"),
        };
        let subscribed = get(SUBSCRIBED_KEY)
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let icons = icon_dir.as_deref().map(load_icons).unwrap_or_default();
        PhoneNotifs {
            kv,
            icon_dir,
            inner: Mutex::new(Inner {
                list: Vec::new(),
                next_id: 1,
                prefs,
                subscribed,
                icons,
                asked: HashSet::new(),
            }),
        }
    }

    fn save(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                eprintln!("[phone-notifs] couldn't save {key}: {e}");
            }
        }
    }

    pub fn prefs(&self) -> NotifPrefs {
        self.inner.lock().map(|i| i.prefs).unwrap_or(NotifPrefs {
            private_mode: true,
            only_locked: false,
        })
    }
}

/// The icons kept: `index.json` (package -> id) and each `<pkg>.jpg`.
fn load_icons(dir: &std::path::Path) -> HashMap<String, (String, String)> {
    let index: HashMap<String, String> = std::fs::read_to_string(dir.join("index.json"))
        .ok()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    index
        .into_iter()
        .filter_map(|(pkg, hash)| {
            let bytes = std::fs::read(dir.join(icon_file(&pkg)?)).ok()?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            Some((pkg, (hash, format!("data:image/jpeg;base64,{b64}"))))
        })
        .collect()
}

fn changed(app: &AppHandle, node: &str) {
    let _ = app.emit(
        "phone-notifs-changed",
        serde_json::json!({ "node_id": node }),
    );
}

/// What a pop-up for a notification needs.
pub struct Arrived {
    pub notif: PhoneNotif,
    /// Its app icon's file, when it's here.
    pub icon_path: Option<PathBuf>,
    /// Worth a pop-up (not a quiet update or the same thing again).
    pub pop_up: bool,
}

/// A notification arrived from `from`. None for a muted app (nothing kept,
/// nothing shown); else what to pop up.
#[allow(clippy::too_many_arguments)]
pub fn on_notification(
    app: &AppHandle,
    from: &str,
    source: Option<&str>,
    title: &str,
    body: &str,
    key: Option<&str>,
    actions: &serde_json::Value,
    extra: &NotifExtra,
) -> Option<Arrived> {
    let state = app.try_state::<PhoneNotifs>()?;
    let now = crate::activity::now_ms();
    let mut n = {
        let mut i = state.inner.lock().ok()?;
        let id = i.next_id;
        i.next_id = i.next_id.wrapping_add(1);
        build(id, from, source, title, body, key, actions, extra, now)
    };
    if needs_hiding(&n, state.prefs().private_mode, extra.sensitive) {
        hide_content(&mut n);
    }
    // The history keeps what's shown, and drops a muted app's.
    let kept_title = if n.private { HIDDEN } else { n.title.as_str() };
    if !crate::notif_history::on_notification(
        app,
        from,
        Some(&n.app),
        kept_title,
        &n.body,
        n.key.as_deref(),
    ) {
        return None;
    }
    let (repeat, want_icon, icon_path) = {
        let mut i = state.inner.lock().ok()?;
        let repeat = upsert(&mut i.list, n.clone(), MAX_PER_DEVICE);
        let want_icon = match (&n.pkg, &n.icon) {
            (Some(pkg), Some(hash)) => {
                i.icons.get(pkg).map(|(h, _)| h) != Some(hash)
                    && i.asked.insert((from.to_string(), pkg.clone()))
            }
            _ => false,
        };
        let icon_path = n
            .pkg
            .as_deref()
            .filter(|p| i.icons.contains_key(*p))
            .and_then(icon_file)
            .and_then(|f| state.icon_dir.as_ref().map(|d| d.join(f)));
        (repeat, want_icon, icon_path)
    };
    if want_icon {
        if let (Some(engine), Some(pkg)) = (app.try_state::<Eng>(), &n.pkg) {
            let _ = engine.request_notification_icon(from, pkg);
        }
    }
    changed(app, from);
    let pop_up = !repeat && !n.silent;
    Some(Arrived {
        notif: n,
        icon_path,
        pop_up,
    })
}

/// `from`'s notification `key` went away there, or was dismissed here.
pub fn on_removed(app: &AppHandle, from: &str, key: &str) {
    let Some(state) = app.try_state::<PhoneNotifs>() else {
        return;
    };
    let gone = state
        .inner
        .lock()
        .map(|mut i| {
            let before = i.list.len();
            i.list
                .retain(|e| !(e.from == from && e.key.as_deref() == Some(key)));
            before != i.list.len()
        })
        .unwrap_or(false);
    crate::notif_popup::withdraw(from, key);
    if gone {
        changed(app, from);
    }
}

/// A button was pressed or a reply sent from here (`index` -1: dismissed).
pub fn on_acted(app: &AppHandle, from: &str, key: &str, index: i32) {
    if index < 0 {
        on_removed(app, from, key);
        return;
    }
    let Some(state) = app.try_state::<PhoneNotifs>() else {
        return;
    };
    if let Ok(mut i) = state.inner.lock() {
        for e in i
            .list
            .iter_mut()
            .filter(|e| e.from == from && e.key.as_deref() == Some(key))
        {
            e.handled = true;
        }
    }
    changed(app, from);
}

/// An app was muted: its notifications leave the lists.
pub fn on_muted(app: &AppHandle, muted: &[String]) {
    let Some(state) = app.try_state::<PhoneNotifs>() else {
        return;
    };
    let nodes: Vec<String> = state
        .inner
        .lock()
        .map(|mut i| {
            let gone: Vec<String> = i
                .list
                .iter()
                .filter(|e| crate::notif_history::is_muted(muted, &e.app))
                .map(|e| e.from.clone())
                .collect();
            i.list
                .retain(|e| !crate::notif_history::is_muted(muted, &e.app));
            gone
        })
        .unwrap_or_default();
    for n in nodes.iter().collect::<HashSet<_>>() {
        changed(app, n);
    }
}

/// App `pkg`'s icon arrived from `from`: kept.
pub fn on_icon(app: &AppHandle, from: &str, pkg: &str, hash: &str, data: &str) {
    let Some(state) = app.try_state::<PhoneNotifs>() else {
        return;
    };
    let Some(file) = icon_file(pkg) else {
        return;
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
        return;
    };
    let index = {
        let Ok(mut i) = state.inner.lock() else {
            return;
        };
        i.icons.insert(
            pkg.to_string(),
            (hash.to_string(), format!("data:image/jpeg;base64,{data}")),
        );
        i.icons
            .iter()
            .map(|(p, (h, _))| (p.clone(), h.clone()))
            .collect::<HashMap<_, _>>()
    };
    if let Some(dir) = &state.icon_dir {
        let written = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(dir.join(&file), &bytes))
            .and_then(|_| {
                std::fs::write(
                    dir.join("index.json"),
                    serde_json::to_string(&index).unwrap_or_default(),
                )
            });
        if let Err(e) = written {
            eprintln!("[phone-notifs] couldn't keep an app icon: {e}");
        }
    }
    changed(app, from);
}

/// Subscribe to `apps_json` on `node` with this computer's settings, and
/// remember them for next time.
pub fn subscribe(app: &AppHandle, node: &str, apps_json: &str) -> Result<(), mdcore::CoreError> {
    let engine = app
        .try_state::<Eng>()
        .ok_or_else(|| mdcore::CoreError::Other("not started".into()))?;
    let state = app.state::<PhoneNotifs>();
    let apps: Vec<String> = serde_json::from_str(apps_json).unwrap_or_default();
    let (prefs, all) = {
        let mut i = state
            .inner
            .lock()
            .map_err(|_| mdcore::CoreError::Other("busy".into()))?;
        i.subscribed.insert(node.to_string(), apps);
        (
            i.prefs,
            serde_json::to_string(&i.subscribed).unwrap_or_default(),
        )
    };
    state.save(SUBSCRIBED_KEY, &all);
    engine.subscribe_apps_with(node, apps_json, prefs.only_locked, prefs.private_mode)
}

/// Say the subscription to `node` again (a link came up, or the settings
/// changed), if this computer ever made one there.
pub fn resubscribe(app: &AppHandle, node: &str) {
    let Some(state) = app.try_state::<PhoneNotifs>() else {
        return;
    };
    let (apps, prefs) = match state.inner.lock() {
        Ok(i) => (i.subscribed.get(node).cloned(), i.prefs),
        Err(_) => return,
    };
    let (Some(apps), Some(engine)) = (apps, app.try_state::<Eng>()) else {
        return;
    };
    let json = serde_json::to_string(&apps).unwrap_or_else(|_| "[]".into());
    let _ = engine.subscribe_apps_with(node, &json, prefs.only_locked, prefs.private_mode);
}

// --- commands ---------------------------------------------------------------------

/// Device `node_id`'s notifications now, in groups (newest first).
#[tauri::command]
#[specta::specta]
pub fn phone_notifications(node_id: String, state: State<PhoneNotifs>) -> Vec<NotifGroup> {
    let Ok(i) = state.inner.lock() else {
        return Vec::new();
    };
    let mine: Vec<PhoneNotif> = i
        .list
        .iter()
        .filter(|n| n.from == node_id)
        .cloned()
        .collect();
    let icons: HashMap<String, String> = i
        .icons
        .iter()
        .map(|(p, (_, url))| (p.clone(), url.clone()))
        .collect();
    group(&mine, &icons)
}

/// Forget notification `id` here only (one without a key: there's nothing
/// to tell the phone).
#[tauri::command]
#[specta::specta]
pub fn forget_phone_notification(id: u32, app: AppHandle, state: State<PhoneNotifs>) {
    let from = state.inner.lock().ok().and_then(|mut i| {
        let at = i.list.iter().position(|e| e.id == id)?;
        Some(i.list.remove(at).from)
    });
    if let Some(from) = from {
        changed(&app, &from);
    }
}

#[tauri::command]
#[specta::specta]
pub fn phone_notif_prefs(state: State<PhoneNotifs>) -> NotifPrefs {
    state.prefs()
}

/// Change the two settings; the phones hear at once.
#[tauri::command]
#[specta::specta]
pub fn set_phone_notif_prefs(prefs: NotifPrefs, app: AppHandle, state: State<PhoneNotifs>) {
    let nodes: Vec<String> = match state.inner.lock() {
        Ok(mut i) => {
            i.prefs = prefs;
            i.subscribed.keys().cloned().collect()
        }
        Err(_) => return,
    };
    state.save(PRIVATE_KEY, if prefs.private_mode { "on" } else { "off" });
    state.save(LOCKED_KEY, if prefs.only_locked { "on" } else { "off" });
    for node in nodes {
        resubscribe(&app, &node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdcore::features::notif_rich::NotifMessage;

    fn n(id: u32, from: &str, title: &str, key: Option<&str>) -> PhoneNotif {
        build(
            id,
            from,
            Some("Chat"),
            title,
            "hi",
            key,
            &serde_json::json!([]),
            &NotifExtra::default(),
            f64::from(id),
        )
    }

    #[test]
    fn buttons_keep_their_place_and_one_reply_box() {
        let actions = serde_json::json!([
            {"title": "Reply", "reply": true},
            {"title": "", "reply": false},
            {"title": "Mark as read", "reply": false},
            {"title": "Quick reply", "reply": true},
            {"title": "Mute", "reply": false},
        ]);
        let (buttons, reply) = buttons_from(&actions);
        assert_eq!(reply.map(|r| r.index), Some(0));
        let names: Vec<(u32, &str)> = buttons
            .iter()
            .map(|b| (b.index, b.title.as_str()))
            .collect();
        assert_eq!(names, [(2, "Mark as read"), (4, "Mute")]);
        assert_eq!(buttons_from(&serde_json::Value::Null), (vec![], None));
    }

    #[test]
    fn a_conversation_groups_and_a_sender_groups_and_the_rest_stand_alone() {
        let chat = |id: u32, conv: Option<&str>, title: &str, key: &str| {
            let extra = NotifExtra {
                pkg: Some("com.chat".into()),
                conv: conv.map(str::to_string),
                conv_title: conv.map(|_| "Family".to_string()),
                group_chat: conv.is_some(),
                messages: vec![NotifMessage {
                    sender: title.into(),
                    text: format!("m{id}"),
                    ts: 1,
                }],
                ..Default::default()
            };
            build(
                id,
                "p",
                Some("Chat"),
                title,
                "x",
                Some(key),
                &serde_json::json!([{"title": "Reply", "reply": true}]),
                &extra,
                f64::from(id),
            )
        };
        // Newest first.
        let list = vec![
            chat(6, Some("fam"), "Ravi", "k6"),
            n(5, "p", "Update ready", Some("k5")),
            chat(4, None, "Asha", "k4"),
            chat(3, Some("fam"), "Asha", "k3"),
            chat(2, None, "Asha", "k2"),
            n(1, "p", "Update ready", Some("k1")),
        ];
        let icons = HashMap::from([("com.chat".to_string(), "data:icon".to_string())]);
        let groups = group(&list, &icons);
        let shape: Vec<(String, Vec<u32>)> = groups
            .iter()
            .map(|g| (g.title.clone(), g.items.iter().map(|i| i.id).collect()))
            .collect();
        assert_eq!(
            shape,
            [
                ("Family".to_string(), vec![6, 3]),
                ("Update ready".to_string(), vec![5]),
                ("Asha".to_string(), vec![4, 2]),
                ("Update ready".to_string(), vec![1]),
            ]
        );
        assert!(groups[0].group_chat);
        assert_eq!(groups[0].icon.as_deref(), Some("data:icon"));
        assert_eq!(groups[1].icon, None);
        assert_eq!(groups[0].ts, 6.0);
        // Another device's conversation of the same name is another group.
        let mut other = chat(7, Some("fam"), "Ravi", "k7");
        other.from = "q".into();
        assert_eq!(
            group(&[other, chat(6, Some("fam"), "Ravi", "k6")], &icons).len(),
            2
        );
    }

    #[test]
    fn private_mode_hides_what_looks_sensitive() {
        let mut otp = n(1, "p", "AX-HDFCBK", None);
        otp.body = "482913 is your OTP. Do not share it.".into();
        assert!(needs_hiding(&otp, true, false));
        assert!(!needs_hiding(&otp, false, false));
        let plain = n(2, "p", "Asha", None);
        assert!(!needs_hiding(&plain, true, false));
        // The phone says so.
        assert!(needs_hiding(&plain, true, true));
        // A bank or password manager, by name or package.
        let mut bank = n(3, "p", "Credited", None);
        bank.app = "HDFC Bank".into();
        assert!(needs_hiding(&bank, true, false));
        let mut vault = n(4, "p", "Login", None);
        vault.app = "Vault".into();
        vault.pkg = Some("com.x8bit.bitwarden".into());
        assert!(needs_hiding(&vault, true, false));
        // Already left out on the phone: nothing more to do.
        let mut gone = n(5, "p", "", None);
        gone.private = true;
        assert!(!needs_hiding(&gone, true, true));

        let mut x = build(
            6,
            "p",
            Some("Bank"),
            "Your code",
            "123456",
            Some("k"),
            &serde_json::json!([{"title": "Copy 123456", "reply": false}, {"title": "Reply", "reply": true}]),
            &NotifExtra {
                sender: Some("Bank".into()),
                avatar: Some("AAAA".into()),
                messages: vec![NotifMessage {
                    sender: "Bank".into(),
                    text: "123456".into(),
                    ts: 0,
                }],
                ..Default::default()
            },
            1.0,
        );
        hide_content(&mut x);
        assert!(x.private);
        assert!(x.title.is_empty() && x.body.is_empty());
        assert!(x.buttons.is_empty() && x.reply.is_none());
        assert!(x.messages.is_empty() && x.avatar.is_none() && x.sender.is_none());
        assert_eq!(x.key.as_deref(), Some("k"));
        assert!(!serde_json::to_string(&x).unwrap().contains("123456"));
    }

    #[test]
    fn sensitive_apps_and_codes() {
        assert!(sensitive_app("Google Pay", None));
        assert!(sensitive_app("PhonePe", Some("com.phonepe.app")));
        assert!(sensitive_app("Authenticator", None));
        assert!(sensitive_app("X", Some("com.example.mybankapp")));
        assert!(!sensitive_app("WhatsApp", Some("com.whatsapp")));
        assert!(!sensitive_app("Spotify", Some("com.spotify.music")));
        assert!(!sensitive_app("Display", Some("com.android.display")));
        assert!(looks_like_code("Your verification code is 123-456"));
        assert!(looks_like_code("OTP 4821 for login"));
        assert!(!looks_like_code("Your order 123456 has shipped"));
        assert!(!looks_like_code("Use code SAVE10"));
        assert!(!looks_like_code("pin 12"));
    }

    #[test]
    fn the_same_one_again_replaces_and_is_known_as_a_repeat() {
        let mut list = Vec::new();
        assert!(!upsert(&mut list, n(1, "p", "1 message", Some("k")), 10));
        assert!(!upsert(&mut list, n(2, "p", "2 messages", Some("k")), 10));
        assert!(upsert(&mut list, n(3, "p", "2 messages", Some("k")), 10));
        // Another device's same key is another notification.
        assert!(!upsert(&mut list, n(4, "q", "2 messages", Some("k")), 10));
        // Without a key nothing is replaced.
        upsert(&mut list, n(5, "p", "plain", None), 10);
        upsert(&mut list, n(6, "p", "plain", None), 10);
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [6, 5, 4, 3]);
        // A device's oldest go past the cap; others are left alone.
        upsert(&mut list, n(7, "p", "more", None), 3);
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [7, 6, 5, 4]);
    }

    #[test]
    fn what_arrived_is_read_in() {
        let extra = NotifExtra {
            pkg: Some("com.chat".into()),
            avatar: Some("QUJD".into()),
            silent: true,
            ..Default::default()
        };
        let x = build(
            1,
            "p",
            Some(" Chat "),
            "Asha",
            "hi",
            Some(""),
            &serde_json::json!([{"title": "Reply", "reply": true}]),
            &extra,
            9.0,
        );
        assert_eq!(x.app, "Chat");
        assert_eq!(x.key, None);
        // No key: nothing to press.
        assert!(x.reply.is_none() && x.buttons.is_empty());
        assert_eq!(x.avatar.as_deref(), Some("data:image/jpeg;base64,QUJD"));
        assert!(x.silent);
        assert_eq!(icon_file("com.chat").as_deref(), Some("com.chat.jpg"));
        assert_eq!(icon_file("../x"), None);
        assert_eq!(icon_file("*"), None);
    }
}
