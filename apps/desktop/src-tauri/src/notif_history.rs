// Notification history: the notifications your phone shared, kept on this
// computer so they can be found again after they're gone from the phone.
//
//  - the last `MAX_ENTRIES` (all devices together), newest first, in
//    `<app config dir>/notification-history.json`, written a few seconds
//    after a change (`SAVE_DELAY`) rather than on every notification;
//  - the same notification again (same device and key: an update on the
//    phone) replaces the older entry;
//  - dismissed from here, or gone on the phone (swiped, opened, cancelled by
//    its app: the phone says so), it stays, marked dismissed;
//  - "Clear history" on the device's page forgets that device's entries.
//
// "Don't show notifications from {app}": a muted app's notifications get no
// pop-up here, don't reach the window and aren't kept. The list is in
// `<app config dir>/prefs.json`, key `muted_apps`, by app name (what the
// phone sends).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Manager, State};

const FILE: &str = "notification-history.json";
const MUTED_KEY: &str = "muted_apps";
/// The most notifications kept.
pub const MAX_ENTRIES: usize = 500;
/// The most matches one page of the list returns.
const MAX_PAGE: u32 = 500;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// One notification as kept.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct HistoryEntry {
    pub id: u32,
    /// The device it came from (node id).
    pub from: String,
    /// The app's name on the phone ("" when it didn't say).
    pub app: String,
    pub title: String,
    pub body: String,
    /// Its key on the phone, when it had one.
    #[serde(default)]
    pub key: Option<String>,
    /// When it arrived, in milliseconds since 1970.
    pub ts: f64,
    /// Dismissed from this computer.
    #[serde(default)]
    pub dismissed: bool,
}

/// What the history view shows: the first matches, how many there are, how
/// many this device has kept in all, and its apps (for the filter chips).
#[derive(serde::Serialize, specta::Type)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    pub total: u32,
    pub kept: u32,
    pub apps: Vec<String>,
}

/// Keep `entry` first in `list`, dropping an older one with the same device
/// and key, and the oldest past `max`.
pub fn record(list: &mut Vec<HistoryEntry>, entry: HistoryEntry, max: usize) {
    if let Some(key) = &entry.key {
        list.retain(|e| !(e.from == entry.from && e.key.as_ref() == Some(key)));
    }
    list.insert(0, entry);
    list.truncate(max);
}

/// Mark `from`'s notification `key` dismissed. Whether there was one.
pub fn mark_dismissed(list: &mut [HistoryEntry], from: &str, key: &str) -> bool {
    let mut found = false;
    for e in list
        .iter_mut()
        .filter(|e| e.from == from && e.key.as_deref() == Some(key))
    {
        e.dismissed = true;
        found = true;
    }
    found
}

/// Whether `app` is on the muted list (names compared ignoring case and
/// surrounding spaces). An unnamed app can't be muted.
pub fn is_muted(muted: &[String], app: &str) -> bool {
    let app = app.trim();
    !app.is_empty() && muted.iter().any(|m| m.trim().eq_ignore_ascii_case(app))
}

/// The muted list tidied: trimmed, no blanks, each app once, sorted.
pub fn tidy_muted(apps: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in apps {
        let a = a.trim();
        if !a.is_empty() && !is_muted(&out, a) {
            out.push(a.to_string());
        }
    }
    out.sort_by_key(|a| a.to_lowercase());
    out
}

/// Whether `e` matches a search: every word of `query` appears in its title,
/// text or app name, ignoring case. An empty query matches everything.
pub fn matches(e: &HistoryEntry, query: &str) -> bool {
    let hay = format!("{}\n{}\n{}", e.title, e.body, e.app).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| hay.contains(word))
}

/// `from`'s entries matching `query`, and from `app` only when given.
pub fn search<'a>(
    list: &'a [HistoryEntry],
    from: &'a str,
    query: &'a str,
    app: Option<&'a str>,
) -> impl Iterator<Item = &'a HistoryEntry> + 'a {
    list.iter()
        .filter(move |e| e.from == from)
        .filter(move |e| app.is_none_or(|a| e.app == a))
        .filter(move |e| matches(e, query))
}

struct Inner {
    list: Vec<HistoryEntry>,
    next_id: u32,
    muted: Vec<String>,
}

/// The history and the muted apps.
pub struct NotifHistory {
    /// `<app config dir>/notification-history.json`.
    path: Option<PathBuf>,
    prefs: Option<FileKv>,
    inner: Mutex<Inner>,
    /// A write is already on its way.
    save_pending: AtomicBool,
}

impl NotifHistory {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let path = dir.as_ref().map(|d| d.join(FILE));
        let prefs = dir.map(|d| FileKv::new(d.join("prefs.json")));
        let mut list: Vec<HistoryEntry> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        list.truncate(MAX_ENTRIES);
        let next_id = list.iter().map(|e| e.id).max().unwrap_or(0).wrapping_add(1);
        let muted = prefs
            .as_ref()
            .and_then(|kv| kv.get(MUTED_KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .map(tidy_muted)
            .unwrap_or_default();
        NotifHistory {
            path,
            prefs,
            inner: Mutex::new(Inner {
                list,
                next_id,
                muted,
            }),
            save_pending: AtomicBool::new(false),
        }
    }

    fn muted(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|i| i.muted.clone())
            .unwrap_or_default()
    }

    /// Write the file once `SAVE_DELAY` has passed, folding in whatever else
    /// changes meanwhile.
    fn save_soon(&self, app: &AppHandle) {
        if self.save_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            if let Some(h) = app.try_state::<NotifHistory>() {
                h.save_pending.store(false, Ordering::Release);
                h.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let json = match self.inner.lock() {
            Ok(i) => serde_json::to_string(&i.list),
            Err(_) => return,
        };
        let Ok(json) = json else {
            return;
        };
        if let Err(e) = write_atomic(path, &json) {
            eprintln!("[history] couldn't save the notification history: {e}");
        }
    }
}

/// Write to a temporary file, then rename it over `path`.
fn write_atomic(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// A notification arrived from `from`: kept, unless its app is muted.
/// Returns false for a muted app (no pop-up, nothing to the window).
pub fn on_notification(
    app: &AppHandle,
    from: &str,
    source: Option<&str>,
    title: &str,
    body: &str,
    key: Option<&str>,
) -> bool {
    let Some(h) = app.try_state::<NotifHistory>() else {
        return true;
    };
    let source = source.unwrap_or("").trim();
    {
        let Ok(mut i) = h.inner.lock() else {
            return true;
        };
        if is_muted(&i.muted, source) {
            return false;
        }
        let id = i.next_id;
        i.next_id = i.next_id.wrapping_add(1);
        let entry = HistoryEntry {
            id,
            from: from.to_string(),
            app: source.to_string(),
            title: title.to_string(),
            body: body.to_string(),
            key: key.filter(|k| !k.is_empty()).map(str::to_string),
            ts: now_ms(),
            dismissed: false,
        };
        record(&mut i.list, entry, MAX_ENTRIES);
    }
    h.save_soon(app);
    true
}

/// `from`'s notification `key` was dismissed from here, or went away on
/// `from`.
pub fn on_dismissed(app: &AppHandle, from: &str, key: &str) {
    let Some(h) = app.try_state::<NotifHistory>() else {
        return;
    };
    let changed = h
        .inner
        .lock()
        .map(|mut i| mark_dismissed(&mut i.list, from, key))
        .unwrap_or(false);
    if changed {
        h.save_soon(app);
    }
}

/// Device `node_id`'s kept notifications, newest first: the first `limit`
/// matching `query` (title, text or app; ignoring case), from `app` only when
/// given.
#[tauri::command]
#[specta::specta]
pub fn notification_history(
    node_id: String,
    query: String,
    app: Option<String>,
    limit: u32,
    history: State<NotifHistory>,
) -> HistoryPage {
    let Ok(i) = history.inner.lock() else {
        return HistoryPage {
            entries: Vec::new(),
            total: 0,
            kept: 0,
            apps: Vec::new(),
        };
    };
    let app = app.filter(|a| !a.is_empty());
    let mut entries = Vec::new();
    let mut total = 0u32;
    for e in search(&i.list, &node_id, &query, app.as_deref()) {
        if total < limit.min(MAX_PAGE) {
            entries.push(e.clone());
        }
        total += 1;
    }
    let mut apps: Vec<String> = Vec::new();
    let mut kept = 0u32;
    for e in i.list.iter().filter(|e| e.from == node_id) {
        kept += 1;
        if !e.app.is_empty() && !apps.contains(&e.app) {
            apps.push(e.app.clone());
        }
    }
    apps.sort_by_key(|a| a.to_lowercase());
    HistoryPage {
        entries,
        total,
        kept,
        apps,
    }
}

/// Forget device `node_id`'s kept notifications.
#[tauri::command]
#[specta::specta]
pub fn clear_notification_history(node_id: String, app: AppHandle, history: State<NotifHistory>) {
    if let Ok(mut i) = history.inner.lock() {
        i.list.retain(|e| e.from != node_id);
    }
    history.save_soon(&app);
}

/// The apps whose notifications aren't shown or kept here.
#[tauri::command]
#[specta::specta]
pub fn muted_apps(history: State<NotifHistory>) -> Vec<String> {
    history.muted()
}

/// Replace the muted list; returns it tidied (see `tidy_muted`).
#[tauri::command]
#[specta::specta]
pub fn set_muted_apps(apps: Vec<String>, history: State<NotifHistory>) -> Vec<String> {
    let apps = tidy_muted(apps);
    if let Ok(mut i) = history.inner.lock() {
        i.muted = apps.clone();
    }
    if let (Some(kv), Ok(json)) = (&history.prefs, serde_json::to_string(&apps)) {
        if let Err(e) = kv.set(MUTED_KEY, &json) {
            eprintln!("[history] couldn't save the muted apps: {e}");
        }
    }
    apps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, from: &str, app: &str, title: &str, key: Option<&str>) -> HistoryEntry {
        HistoryEntry {
            id,
            from: from.into(),
            app: app.into(),
            title: title.into(),
            body: format!("body {id}"),
            key: key.map(str::to_string),
            ts: f64::from(id),
            dismissed: false,
        }
    }

    #[test]
    fn keeps_the_newest_up_to_the_cap() {
        let mut list = Vec::new();
        for id in 1..=7 {
            record(&mut list, entry(id, "p", "Chat", "hi", None), 5);
        }
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [7, 6, 5, 4, 3]);
    }

    #[test]
    fn an_update_replaces_the_same_notification() {
        let mut list = Vec::new();
        record(&mut list, entry(1, "p", "Chat", "1 message", Some("k")), 10);
        record(&mut list, entry(2, "p", "Mail", "news", Some("m")), 10);
        // Same key from another device is a different notification.
        record(&mut list, entry(3, "q", "Chat", "other", Some("k")), 10);
        record(
            &mut list,
            entry(4, "p", "Chat", "2 messages", Some("k")),
            10,
        );
        // Without a key, nothing is replaced.
        record(&mut list, entry(5, "p", "Chat", "plain", None), 10);
        record(&mut list, entry(6, "p", "Chat", "plain", None), 10);
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [6, 5, 4, 3, 2]);
    }

    #[test]
    fn dismissed_ones_stay_marked() {
        let mut list = vec![
            entry(2, "p", "Chat", "a", Some("k")),
            entry(1, "q", "Chat", "b", Some("k")),
        ];
        assert!(mark_dismissed(&mut list, "p", "k"));
        assert!(!mark_dismissed(&mut list, "p", "gone"));
        assert_eq!(list.len(), 2);
        assert!(list[0].dismissed);
        assert!(!list[1].dismissed);
    }

    #[test]
    fn muting_ignores_case_and_spaces() {
        let muted = vec!["WhatsApp".to_string(), " Gmail ".to_string()];
        assert!(is_muted(&muted, "whatsapp"));
        assert!(is_muted(&muted, "Gmail"));
        assert!(!is_muted(&muted, "Messages"));
        assert!(!is_muted(&muted, ""));
        assert!(!is_muted(&["".to_string()], "  "));
    }

    #[test]
    fn the_muted_list_is_tidied() {
        let apps = vec![
            "  whatsapp ".to_string(),
            "Gmail".to_string(),
            "".to_string(),
            "WhatsApp".to_string(),
            "bank".to_string(),
        ];
        assert_eq!(tidy_muted(apps), ["bank", "Gmail", "whatsapp"]);
    }

    #[test]
    fn search_matches_title_text_and_app_ignoring_case() {
        let mut e = entry(1, "p", "WhatsApp", "Asha", None);
        e.body = "Dinner at 8?".into();
        assert!(matches(&e, ""));
        assert!(matches(&e, "  "));
        assert!(matches(&e, "asha"));
        assert!(matches(&e, "DINNER"));
        assert!(matches(&e, "whatsapp"));
        assert!(matches(&e, "asha dinner"));
        assert!(!matches(&e, "asha lunch"));
        assert!(!matches(&e, "gmail"));
    }

    #[test]
    fn search_keeps_to_the_device_and_app() {
        let list = vec![
            entry(4, "p", "Chat", "hello there", None),
            entry(3, "p", "Mail", "hello mail", None),
            entry(2, "q", "Chat", "hello other", None),
            entry(1, "p", "Chat", "bye", None),
        ];
        let ids = |it: Vec<&HistoryEntry>| it.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(search(&list, "p", "", None).collect()), [4, 3, 1]);
        assert_eq!(ids(search(&list, "p", "hello", None).collect()), [4, 3]);
        assert_eq!(ids(search(&list, "p", "", Some("Chat")).collect()), [4, 1]);
        assert_eq!(
            ids(search(&list, "p", "hello", Some("Chat")).collect()),
            [4]
        );
        assert_eq!(
            ids(search(&list, "x", "", None).collect()),
            Vec::<u32>::new()
        );
    }
}
