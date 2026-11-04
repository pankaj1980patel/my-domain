// Clipboard history: the text this computer and your other devices sent each
// other as clipboard (`send_text` with mode "clipboard", either way), kept so
// it can be copied or sent again later.
//
//  - the last `MAX_ENTRIES` (all devices together), newest first, in
//    `<app config dir>/clipboard-history.json`, written a few seconds after a
//    change (`SAVE_DELAY`) rather than on every clip;
//  - each clip's text is kept up to `MAX_TEXT` bytes; a longer one is cut
//    (on a character boundary) and marked `truncated`;
//  - "Clear" on a device's page forgets that device's clips;
//  - the window hears `clip-history-changed` after each change, and the
//    tray's "Recent clips" shows the last few received.
//
// "Keep a clipboard history" in Settings turns it off (which also forgets
// everything kept); on by default. In `<app config dir>/prefs.json`, key
// `clip_history` ("on" / "off").
//
// This is separate from the core's in-memory list of synced clipboard values
// (`clipboard_history` in lib.rs), which lasts until the app restarts.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager, State};

const FILE: &str = "clipboard-history.json";
const PREF_KEY: &str = "clip_history";
/// What the window hears when the history changed.
pub const CHANGED_EVENT: &str = "clip-history-changed";
/// The most clips kept.
pub const MAX_ENTRIES: usize = 200;
/// The most text kept of one clip, in bytes.
pub const MAX_TEXT: usize = 10 * 1024;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// Which way a clip went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ClipDirection {
    /// Another device sent it here.
    In,
    /// This computer sent it.
    Out,
}

/// One clip as kept.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ClipRecord {
    pub id: u32,
    /// Up to `MAX_TEXT` bytes of it.
    pub text: String,
    /// The text was longer and was cut.
    #[serde(default)]
    pub truncated: bool,
    /// The device it came from (node id), for one received.
    #[serde(default)]
    pub from_node: Option<String>,
    /// The device it went to (node id), for one sent.
    #[serde(default)]
    pub to_node: Option<String>,
    pub direction: ClipDirection,
    /// When, in milliseconds since 1970.
    pub at: f64,
}

impl ClipRecord {
    /// The other device: where it came from or went to.
    pub fn node(&self) -> Option<&str> {
        match self.direction {
            ClipDirection::In => self.from_node.as_deref(),
            ClipDirection::Out => self.to_node.as_deref(),
        }
    }
}

/// What a device's clipboard tile shows: the first matches, how many there
/// are, and how many this device has kept in all.
#[derive(serde::Serialize, specta::Type)]
pub struct ClipPage {
    pub entries: Vec<ClipRecord>,
    pub total: u32,
    pub kept: u32,
}

/// `text` cut to at most `max` bytes on a character boundary, and whether it
/// was cut.
pub fn cap_text(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// Keep `entry` first in `list`, dropping the oldest past `max`.
pub fn record(list: &mut Vec<ClipRecord>, entry: ClipRecord, max: usize) {
    list.insert(0, entry);
    list.truncate(max);
}

/// Whether `e` matches a search: every word of `query` appears in its text,
/// ignoring case. An empty query matches everything.
pub fn matches(e: &ClipRecord, query: &str) -> bool {
    let hay = e.text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| hay.contains(word))
}

/// `node`'s clips (either way) matching `query`, newest first.
pub fn search<'a>(
    list: &'a [ClipRecord],
    node: &'a str,
    query: &'a str,
) -> impl Iterator<Item = &'a ClipRecord> + 'a {
    list.iter()
        .filter(move |e| e.node() == Some(node))
        .filter(move |e| matches(e, query))
}

/// A clip as one short menu line: its first non-blank line, at most `max`
/// characters (an ellipsis when there's more).
pub fn preview(text: &str, max: usize) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or("");
    let more = line.chars().count() > max || text.trim().lines().nth(1).is_some();
    let mut out: String = line.chars().take(max).collect();
    if more {
        out.push('\u{2026}');
    }
    out
}

struct Inner {
    list: Vec<ClipRecord>,
    next_id: u32,
}

/// The clipboard history and its on/off switch.
pub struct ClipHistory {
    /// `<app config dir>/clipboard-history.json`.
    path: Option<PathBuf>,
    prefs: Option<FileKv>,
    on: AtomicBool,
    inner: Mutex<Inner>,
    /// A write is already on its way.
    save_pending: AtomicBool,
}

impl ClipHistory {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let path = dir.as_ref().map(|d| d.join(FILE));
        let prefs = dir.map(|d| FileKv::new(d.join("prefs.json")));
        let on = prefs.as_ref().and_then(|kv| kv.get(PREF_KEY)).as_deref() != Some("off");
        let mut list: Vec<ClipRecord> = if on {
            path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        list.truncate(MAX_ENTRIES);
        let next_id = list.iter().map(|e| e.id).max().unwrap_or(0).wrapping_add(1);
        ClipHistory {
            path,
            prefs,
            on: AtomicBool::new(on),
            inner: Mutex::new(Inner { list, next_id }),
            save_pending: AtomicBool::new(false),
        }
    }

    pub fn enabled(&self) -> bool {
        self.on.load(Ordering::Relaxed)
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
            if let Some(h) = app.try_state::<ClipHistory>() {
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
            eprintln!("[clips] couldn't save the clipboard history: {e}");
        }
    }

    /// Keep a clip, if the history is on. Whether it was kept.
    fn add(&self, direction: ClipDirection, node: &str, text: &str) -> bool {
        if !self.enabled() || text.trim().is_empty() {
            return false;
        }
        let Ok(mut i) = self.inner.lock() else {
            return false;
        };
        let (text, truncated) = cap_text(text, MAX_TEXT);
        let id = i.next_id;
        i.next_id = i.next_id.wrapping_add(1);
        let node = Some(node.to_string());
        let (from_node, to_node) = match direction {
            ClipDirection::In => (node, None),
            ClipDirection::Out => (None, node),
        };
        let entry = ClipRecord {
            id,
            text,
            truncated,
            from_node,
            to_node,
            direction,
            at: now_ms(),
        };
        record(&mut i.list, entry, MAX_ENTRIES);
        true
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

/// Saved soon, the window told, and the tray's recent clips rebuilt.
fn changed(app: &AppHandle, h: &ClipHistory) {
    h.save_soon(app);
    let _ = app.emit(CHANGED_EVENT, ());
    crate::tray::refresh(app);
}

/// Text arrived from `from` as clipboard.
pub fn on_received(app: &AppHandle, from: &str, text: &str) {
    if let Some(h) = app.try_state::<ClipHistory>() {
        if h.add(ClipDirection::In, from, text) {
            changed(app, &h);
        }
    }
}

/// This computer sent `text` to `to` as clipboard.
pub fn on_sent(app: &AppHandle, to: &str, text: &str) {
    if let Some(h) = app.try_state::<ClipHistory>() {
        if h.add(ClipDirection::Out, to, text) {
            changed(app, &h);
        }
    }
}

/// The last `n` clips received (from any device), newest first.
pub fn recent_received(app: &AppHandle, n: usize) -> Vec<ClipRecord> {
    let Some(h) = app.try_state::<ClipHistory>() else {
        return Vec::new();
    };
    h.inner
        .lock()
        .map(|i| {
            i.list
                .iter()
                .filter(|e| e.direction == ClipDirection::In)
                .take(n)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The kept clip `id`'s text, if it's still kept.
pub fn text_of(app: &AppHandle, id: u32) -> Option<String> {
    let h = app.try_state::<ClipHistory>()?;
    let i = h.inner.lock().ok()?;
    i.list.iter().find(|e| e.id == id).map(|e| e.text.clone())
}

/// Device `node_id`'s kept clips (sent and received), newest first: the
/// first `limit` matching `query` (every word in the text, ignoring case).
#[tauri::command]
#[specta::specta]
pub fn clip_history(
    node_id: String,
    query: String,
    limit: u32,
    history: State<ClipHistory>,
) -> ClipPage {
    let Ok(i) = history.inner.lock() else {
        return ClipPage {
            entries: Vec::new(),
            total: 0,
            kept: 0,
        };
    };
    let mut entries = Vec::new();
    let mut total = 0u32;
    for e in search(&i.list, &node_id, &query) {
        if total < limit.min(MAX_ENTRIES as u32) {
            entries.push(e.clone());
        }
        total += 1;
    }
    let kept = i
        .list
        .iter()
        .filter(|e| e.node() == Some(node_id.as_str()))
        .count() as u32;
    ClipPage {
        entries,
        total,
        kept,
    }
}

/// Forget device `node_id`'s kept clips.
#[tauri::command]
#[specta::specta]
pub fn clear_clip_history(node_id: String, app: AppHandle, history: State<ClipHistory>) {
    if let Ok(mut i) = history.inner.lock() {
        i.list.retain(|e| e.node() != Some(node_id.as_str()));
    }
    changed(&app, &history);
}

/// Whether clips are kept ("Keep a clipboard history" in Settings).
#[tauri::command]
#[specta::specta]
pub fn clip_history_enabled(history: State<ClipHistory>) -> bool {
    history.enabled()
}

/// Turn the history on or off; off also forgets everything kept.
#[tauri::command]
#[specta::specta]
pub fn set_clip_history_enabled(on: bool, app: AppHandle, history: State<ClipHistory>) {
    history.on.store(on, Ordering::Relaxed);
    if let Some(kv) = &history.prefs {
        if let Err(e) = kv.set(PREF_KEY, if on { "on" } else { "off" }) {
            eprintln!("[clips] couldn't save the setting: {e}");
        }
    }
    if !on {
        if let Ok(mut i) = history.inner.lock() {
            i.list.clear();
        }
        // Gone from the disk now, not in a few seconds.
        history.save_now();
        let _ = app.emit(CHANGED_EVENT, ());
        crate::tray::refresh(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: u32, dir: ClipDirection, node: &str, text: &str) -> ClipRecord {
        let node = Some(node.to_string());
        let (from_node, to_node) = match dir {
            ClipDirection::In => (node, None),
            ClipDirection::Out => (None, node),
        };
        ClipRecord {
            id,
            text: text.into(),
            truncated: false,
            from_node,
            to_node,
            direction: dir,
            at: f64::from(id),
        }
    }

    #[test]
    fn keeps_the_newest_up_to_the_cap() {
        let mut list = Vec::new();
        for id in 1..=7 {
            record(&mut list, clip(id, ClipDirection::In, "p", "hi"), 5);
        }
        let ids: Vec<u32> = list.iter().map(|e| e.id).collect();
        assert_eq!(ids, [7, 6, 5, 4, 3]);
    }

    #[test]
    fn the_real_cap_is_200() {
        let mut list = Vec::new();
        for id in 1..=250 {
            record(
                &mut list,
                clip(id, ClipDirection::Out, "p", "x"),
                MAX_ENTRIES,
            );
        }
        assert_eq!(list.len(), 200);
        assert_eq!(list[0].id, 250);
        assert_eq!(list[199].id, 51);
    }

    #[test]
    fn long_text_is_cut_and_marked() {
        assert_eq!(cap_text("short", 10), ("short".to_string(), false));
        assert_eq!(
            cap_text("exactly10!", 10),
            ("exactly10!".to_string(), false)
        );
        assert_eq!(
            cap_text("0123456789abc", 10),
            ("0123456789".to_string(), true)
        );
        let big = "a".repeat(MAX_TEXT + 1);
        let (kept, cut) = cap_text(&big, MAX_TEXT);
        assert!(cut);
        assert_eq!(kept.len(), 10 * 1024);
    }

    #[test]
    fn cutting_keeps_whole_characters() {
        // "é" is two bytes: a cut through it backs off to before it.
        let (kept, cut) = cap_text("aé", 2);
        assert_eq!((kept.as_str(), cut), ("a", true));
        // "🙂" is four bytes.
        let (kept, cut) = cap_text("🙂🙂", 6);
        assert_eq!((kept.as_str(), cut), ("🙂", true));
    }

    #[test]
    fn search_matches_every_word_ignoring_case() {
        let e = clip(1, ClipDirection::In, "p", "Meet at the Blue Door, 8pm");
        assert!(matches(&e, ""));
        assert!(matches(&e, "   "));
        assert!(matches(&e, "blue"));
        assert!(matches(&e, "DOOR blue"));
        assert!(!matches(&e, "blue window"));
    }

    #[test]
    fn search_keeps_to_the_device_either_way() {
        let list = vec![
            clip(5, ClipDirection::Out, "p", "hello sent"),
            clip(4, ClipDirection::In, "q", "hello other"),
            clip(3, ClipDirection::In, "p", "hello received"),
            clip(2, ClipDirection::In, "p", "bye"),
            // A sent clip names its device in `to_node` only.
            ClipRecord {
                from_node: Some("p".into()),
                ..clip(1, ClipDirection::Out, "q", "to q")
            },
        ];
        let ids = |it: Vec<&ClipRecord>| it.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(search(&list, "p", "").collect()), [5, 3, 2]);
        assert_eq!(ids(search(&list, "p", "hello").collect()), [5, 3]);
        assert_eq!(ids(search(&list, "q", "").collect()), [4, 1]);
        assert_eq!(ids(search(&list, "x", "").collect()), Vec::<u32>::new());
    }

    #[test]
    fn menu_preview_is_one_short_line() {
        assert_eq!(preview("hello", 10), "hello");
        assert_eq!(
            preview("  \n  first line \nsecond", 20),
            "first line\u{2026}"
        );
        assert_eq!(preview("abcdefghijkl", 5), "abcde\u{2026}");
        assert_eq!(preview("", 5), "");
    }

    #[test]
    fn the_file_round_trips() {
        let list = vec![
            clip(2, ClipDirection::Out, "p", "sent"),
            ClipRecord {
                truncated: true,
                ..clip(1, ClipDirection::In, "q", "got")
            },
        ];
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.contains("\"direction\":\"out\""));
        let back: Vec<ClipRecord> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, list);
    }
}
