// Chat history: the messages between this computer and each of your devices,
// kept so a chat is still there after a restart.
//
//  - per device, the newest `MAX_PER_DEVICE` messages, and the newest
//    `MAX_TOTAL` over all devices together, in
//    `<app config dir>/chat-history.json`, written a few seconds after a
//    change (`SAVE_DELAY`, a temporary file renamed over it) rather than on
//    every message;
//  - each message's text is kept up to `MAX_TEXT` bytes; a longer one is cut
//    (on a character boundary) and marked `truncated`;
//  - recorded where messages already pass: `MessageReceived` and
//    `MessageDelivered` in lib.rs's event bridge, and the `send_message`
//    command (queued, sent, or failed);
//  - the window pages through it (`chat_history`: the newest 100 first, older
//    ones as the chat is scrolled up); search.rs reads a copy (`snapshot`);
//  - "Clear" on a device's chat forgets that device's messages, on the disk
//    too.
//
// "Keep message history" in Settings turns it off (which also forgets
// everything kept, and stops saving); on by default. In
// `<app config dir>/prefs.json`, key `chat_history` ("on" / "off").
//
// Local only: nothing here is synced to your other devices or the server.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Manager, State};

use crate::clip_history::cap_text;

const FILE: &str = "chat-history.json";
const PREF_KEY: &str = "chat_history";
/// The most messages kept per device.
pub const MAX_PER_DEVICE: usize = 2_000;
/// The most messages kept over all devices.
pub const MAX_TOTAL: usize = 20_000;
/// The most text kept of one message, in bytes.
pub const MAX_TEXT: usize = 64 * 1024;
/// The most messages one page returns.
const MAX_PAGE: u32 = 500;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// Where a message stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ChatStatus {
    /// Sent from here; handed to a link, not confirmed yet.
    Sent,
    /// Sent from here with no link yet: goes out once there is one.
    Queued,
    /// Sent from here, and the device confirmed it arrived.
    Delivered,
    /// Sent from here, and the send failed.
    Failed,
    /// Another device sent it here.
    Received,
}

/// One message as kept.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ChatRecord {
    /// The message's id (shared with the core and the device).
    pub id: String,
    /// The device it came from (node id), for one received.
    #[serde(default)]
    pub from_node: Option<String>,
    /// The device it went to (node id), for one sent.
    #[serde(default)]
    pub to_node: Option<String>,
    /// Up to `MAX_TEXT` bytes of it.
    pub text: String,
    /// The text was longer and was cut.
    #[serde(default)]
    pub truncated: bool,
    /// When, in milliseconds since 1970.
    pub at: f64,
    pub status: ChatStatus,
}

impl ChatRecord {
    /// The other device: where it came from or went to.
    pub fn node(&self) -> &str {
        match self.status {
            ChatStatus::Received => self.from_node.as_deref().unwrap_or(""),
            _ => self.to_node.as_deref().unwrap_or(""),
        }
    }

    /// Whether this computer received it.
    pub fn incoming(&self) -> bool {
        self.status == ChatStatus::Received
    }
}

/// One page of a device's chat: oldest first, whether older ones are kept,
/// and how many this device has kept in all.
#[derive(Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct ChatPage {
    pub entries: Vec<ChatRecord>,
    pub more: bool,
    pub kept: u32,
}

/// The messages per device, oldest first, with the caps applied.
#[derive(Default, Debug)]
pub struct Store {
    by_node: BTreeMap<String, Vec<ChatRecord>>,
    total: usize,
}

impl Store {
    /// A store of `list` (any order), keeping the newest within the caps.
    pub fn from_list(mut list: Vec<ChatRecord>) -> Self {
        list.sort_by(|a, b| a.at.total_cmp(&b.at));
        let mut s = Store::default();
        for r in list {
            s.add(r);
        }
        s
    }

    /// Every message, oldest first per device (what the file holds).
    pub fn to_list(&self) -> Vec<ChatRecord> {
        self.by_node.values().flatten().cloned().collect()
    }

    /// Keep `r` (its text capped), dropping the oldest past the caps. False
    /// (nothing kept) for blank text, no device, or an id already kept for
    /// that device.
    pub fn add(&mut self, mut r: ChatRecord) -> bool {
        if r.text.trim().is_empty() || r.node().is_empty() {
            return false;
        }
        let (text, cut) = cap_text(&r.text, MAX_TEXT);
        r.text = text;
        r.truncated |= cut;
        let list = self.by_node.entry(r.node().to_string()).or_default();
        if list.iter().rev().any(|e| e.id == r.id) {
            return false;
        }
        // Mostly in order; one stamped earlier (a clock change) goes in its place.
        let at = list.partition_point(|e| e.at <= r.at);
        list.insert(at, r);
        self.total += 1;
        if list.len() > MAX_PER_DEVICE {
            list.remove(0);
            self.total -= 1;
        }
        while self.total > MAX_TOTAL {
            self.drop_oldest();
        }
        true
    }

    /// Forget the oldest message of all.
    fn drop_oldest(&mut self) {
        let oldest = self
            .by_node
            .iter()
            .filter_map(|(node, list)| list.first().map(|e| (node, e.at)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(node, _)| node.clone());
        let Some(node) = oldest else {
            self.total = 0;
            return;
        };
        if let Some(list) = self.by_node.get_mut(&node) {
            list.remove(0);
            self.total -= 1;
            if list.is_empty() {
                self.by_node.remove(&node);
            }
        }
    }

    /// Message `id` (sent from here) now stands at `status`. A delivered one
    /// stays delivered and a received one never changes. Whether it changed.
    pub fn set_status(&mut self, id: &str, status: ChatStatus) -> bool {
        // Newest first: a receipt is nearly always for a recent message.
        let found = self
            .by_node
            .values_mut()
            .find_map(|list| list.iter_mut().rev().find(|e| e.id == id));
        match found {
            Some(e)
                if e.status != status
                    && e.status != ChatStatus::Delivered
                    && e.status != ChatStatus::Received
                    && status != ChatStatus::Received =>
            {
                e.status = status;
                true
            }
            _ => false,
        }
    }

    /// `node`'s messages before message `before` (or the newest, without
    /// one): the last `limit` of them, oldest first. A `before` that isn't
    /// kept (cleared since) gives nothing.
    pub fn page(&self, node: &str, before: Option<&str>, limit: u32) -> ChatPage {
        let empty = |kept| ChatPage {
            entries: Vec::new(),
            more: false,
            kept,
        };
        let Some(list) = self.by_node.get(node) else {
            return empty(0);
        };
        let kept = list.len() as u32;
        let end = match before {
            Some(id) => match list.iter().rposition(|e| e.id == id) {
                Some(i) => i,
                None => return empty(kept),
            },
            None => list.len(),
        };
        let limit = limit.min(MAX_PAGE) as usize;
        let start = end.saturating_sub(limit);
        ChatPage {
            entries: list[start..end].to_vec(),
            more: start > 0,
            kept,
        }
    }

    /// Forget `node`'s messages. Whether there were any.
    pub fn clear(&mut self, node: &str) -> bool {
        match self.by_node.remove(node) {
            Some(list) => {
                self.total -= list.len();
                true
            }
            None => false,
        }
    }

    pub fn clear_all(&mut self) {
        self.by_node.clear();
        self.total = 0;
    }

    /// After a restart: what was waiting for a link went with the app (the
    /// core keeps unsent messages in memory only), so it's marked failed.
    pub fn settle_after_restart(&mut self) {
        for e in self.by_node.values_mut().flatten() {
            if e.status == ChatStatus::Queued {
                e.status = ChatStatus::Failed;
            }
        }
    }
}

/// The history kept in `path`'s file (none when it's missing or unreadable).
pub fn read_file(path: &Path) -> Store {
    let list: Vec<ChatRecord> = std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let mut store = Store::from_list(list);
    store.settle_after_restart();
    store
}

/// Write `store` to `path`: a temporary file, then renamed over it.
pub fn write_file(path: &Path, store: &Store) -> std::io::Result<()> {
    let json = serde_json::to_string(&store.to_list())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// The chat history and its on/off switch.
pub struct ChatHistory {
    /// `<app config dir>/chat-history.json`.
    path: Option<PathBuf>,
    prefs: Option<FileKv>,
    on: AtomicBool,
    store: Mutex<Store>,
    /// A write is already on its way.
    save_pending: AtomicBool,
}

impl ChatHistory {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app.path().app_config_dir().ok();
        let path = dir.as_ref().map(|d| d.join(FILE));
        let prefs = dir.map(|d| FileKv::new(d.join("prefs.json")));
        let on = prefs.as_ref().and_then(|kv| kv.get(PREF_KEY)).as_deref() != Some("off");
        let store = match (&path, on) {
            (Some(p), true) => read_file(p),
            _ => Store::default(),
        };
        ChatHistory {
            path,
            prefs,
            on: AtomicBool::new(on),
            store: Mutex::new(store),
            save_pending: AtomicBool::new(false),
        }
    }

    pub fn enabled(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// A copy of every kept message, oldest first per device (for
    /// search.rs). Read only.
    pub fn snapshot(&self) -> Vec<ChatRecord> {
        self.store.lock().map(|s| s.to_list()).unwrap_or_default()
    }

    /// Change the store (when the history is on) and save soon if `f` says
    /// it changed.
    fn change(&self, app: &AppHandle, f: impl FnOnce(&mut Store) -> bool) {
        if !self.enabled() {
            return;
        }
        let changed = self.store.lock().map(|mut s| f(&mut s)).unwrap_or(false);
        if changed {
            self.save_soon(app);
        }
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
            if let Some(h) = app.try_state::<ChatHistory>() {
                h.save_pending.store(false, Ordering::Release);
                h.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        // Off: nothing on the disk (a write already on its way finds it so).
        if !self.enabled() {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(path) {
                    eprintln!("[chat] couldn't delete the message history: {e}");
                }
            }
            return;
        }
        let result = match self.store.lock() {
            Ok(s) => write_file(path, &s),
            Err(_) => return,
        };
        if let Err(e) = result {
            eprintln!("[chat] couldn't save the message history: {e}");
        }
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// A message arrived from device `from` (`id`: the sender's, when it gave
/// one).
pub fn on_received(app: &AppHandle, from: &str, id: Option<&str>, text: &str) {
    let Some(h) = app.try_state::<ChatHistory>() else {
        return;
    };
    let id = match id.filter(|i| !i.is_empty()) {
        Some(i) => i.to_string(),
        None => uuid::Uuid::new_v4().to_string(),
    };
    let r = ChatRecord {
        id,
        from_node: Some(from.to_string()),
        to_node: None,
        text: text.to_string(),
        truncated: false,
        at: now_ms(),
        status: ChatStatus::Received,
    };
    h.change(app, |s| s.add(r));
}

/// This computer is sending `text` to `to` as message `id`: kept as queued
/// until the send says how it went (`on_send_result`).
pub fn on_sending(app: &AppHandle, to: &str, id: &str, text: &str) {
    let Some(h) = app.try_state::<ChatHistory>() else {
        return;
    };
    let r = ChatRecord {
        id: id.to_string(),
        from_node: None,
        to_node: Some(to.to_string()),
        text: text.to_string(),
        truncated: false,
        at: now_ms(),
        status: ChatStatus::Queued,
    };
    h.change(app, |s| s.add(r));
}

/// How sending message `id` went: `Ok(via)` ("queued", or the link used),
/// or an error. A receipt that came first isn't undone.
pub fn on_send_result<E>(app: &AppHandle, id: &str, result: &Result<String, E>) {
    let status = match result {
        Ok(via) if via == "queued" => ChatStatus::Queued,
        Ok(_) => ChatStatus::Sent,
        Err(_) => ChatStatus::Failed,
    };
    if let Some(h) = app.try_state::<ChatHistory>() {
        h.change(app, |s| s.set_status(id, status));
    }
}

/// The device confirmed message `id` arrived.
pub fn on_delivered(app: &AppHandle, id: &str) {
    if let Some(h) = app.try_state::<ChatHistory>() {
        h.change(app, |s| s.set_status(id, ChatStatus::Delivered));
    }
}

/// Device `node_id`'s kept messages before message `before` (the newest
/// without it): the last `limit`, oldest first.
#[tauri::command]
#[specta::specta]
pub fn chat_history(
    node_id: String,
    before: Option<String>,
    limit: u32,
    history: State<ChatHistory>,
) -> ChatPage {
    history
        .store
        .lock()
        .map(|s| s.page(&node_id, before.as_deref(), limit))
        .unwrap_or(ChatPage {
            entries: Vec::new(),
            more: false,
            kept: 0,
        })
}

/// Forget device `node_id`'s kept messages (on the disk too, in a moment).
#[tauri::command]
#[specta::specta]
pub fn clear_chat_history(node_id: String, app: AppHandle, history: State<ChatHistory>) {
    let changed = history
        .store
        .lock()
        .map(|mut s| s.clear(&node_id))
        .unwrap_or(false);
    if changed {
        history.save_soon(&app);
    }
}

/// Whether messages are kept ("Keep message history" in Settings).
#[tauri::command]
#[specta::specta]
pub fn chat_history_enabled(history: State<ChatHistory>) -> bool {
    history.enabled()
}

/// Turn the history on or off; off also forgets everything kept, on the
/// disk too, now.
#[tauri::command]
#[specta::specta]
pub fn set_chat_history_enabled(on: bool, history: State<ChatHistory>) {
    history.on.store(on, Ordering::Relaxed);
    if let Some(kv) = &history.prefs {
        if let Err(e) = kv.set(PREF_KEY, if on { "on" } else { "off" }) {
            eprintln!("[chat] couldn't save the setting: {e}");
        }
    }
    if !on {
        if let Ok(mut s) = history.store.lock() {
            s.clear_all();
        }
        history.save_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(id: &str, to: &str, text: &str, at: f64) -> ChatRecord {
        ChatRecord {
            id: id.into(),
            from_node: None,
            to_node: Some(to.into()),
            text: text.into(),
            truncated: false,
            at,
            status: ChatStatus::Sent,
        }
    }

    fn got(id: &str, from: &str, text: &str, at: f64) -> ChatRecord {
        ChatRecord {
            id: id.into(),
            from_node: Some(from.into()),
            to_node: None,
            text: text.into(),
            truncated: false,
            at,
            status: ChatStatus::Received,
        }
    }

    fn ids(p: &ChatPage) -> Vec<&str> {
        p.entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn keeps_the_newest_per_device() {
        let mut s = Store::default();
        for i in 0..MAX_PER_DEVICE + 5 {
            assert!(s.add(sent(&i.to_string(), "p", "hi", i as f64)));
        }
        s.add(got("q1", "q", "hello", 1.0));
        assert_eq!(s.total, MAX_PER_DEVICE + 1);
        let p = s.page("p", None, 1);
        assert_eq!(p.kept as usize, MAX_PER_DEVICE);
        assert_eq!(ids(&p), [(MAX_PER_DEVICE + 4).to_string()]);
        // The oldest five went.
        let first = &s.by_node["p"][0];
        assert_eq!(first.id, "5");
        assert_eq!(s.page("q", None, 10).kept, 1);
    }

    #[test]
    fn keeps_the_newest_over_all_devices() {
        let mut s = Store::default();
        let devices = MAX_TOTAL / MAX_PER_DEVICE + 1;
        let mut at = 0.0;
        for d in 0..devices {
            for i in 0..MAX_PER_DEVICE {
                at += 1.0;
                s.add(sent(&format!("{d}-{i}"), &format!("n{d}"), "x", at));
            }
        }
        assert_eq!(s.total, MAX_TOTAL);
        // The first device's (the oldest) went entirely.
        assert_eq!(s.page("n0", None, 10).kept, 0);
        assert_eq!(s.page("n1", None, 10).kept as usize, MAX_PER_DEVICE);
    }

    #[test]
    fn long_text_is_cut_and_blank_or_repeated_ones_are_not_kept() {
        let mut s = Store::default();
        let long = "é".repeat(MAX_TEXT); // two bytes each
        assert!(s.add(got("a", "p", &long, 1.0)));
        let e = &s.page("p", None, 1).entries[0];
        assert!(e.truncated);
        assert!(e.text.len() <= MAX_TEXT);
        assert_eq!(e.text.chars().count(), MAX_TEXT / 2);
        assert!(!s.add(got("b", "p", "   ", 2.0)));
        assert!(!s.add(got("a", "p", "again", 3.0)));
        assert!(!s.add(got("c", "", "no device", 4.0)));
        assert_eq!(s.total, 1);
    }

    #[test]
    fn pages_back_from_the_newest() {
        let mut s = Store::default();
        for i in 0..250 {
            s.add(sent(&format!("m{i}"), "p", "hi", f64::from(i)));
        }
        let p = s.page("p", None, 100);
        assert_eq!(p.entries.len(), 100);
        assert_eq!(p.entries[0].id, "m150");
        assert_eq!(p.entries[99].id, "m249");
        assert!(p.more);
        let p = s.page("p", Some("m150"), 100);
        assert_eq!(
            (p.entries[0].id.as_str(), p.entries[99].id.as_str()),
            ("m50", "m149")
        );
        assert!(p.more);
        let p = s.page("p", Some("m50"), 100);
        assert_eq!(p.entries.len(), 50);
        assert!(!p.more);
        assert_eq!(p.kept, 250);
        // Gone since: nothing, rather than a repeat of the newest.
        assert!(s.page("p", Some("gone"), 100).entries.is_empty());
        assert!(s.page("nobody", None, 100).entries.is_empty());
    }

    #[test]
    fn a_message_stamped_earlier_goes_in_its_place() {
        let mut s = Store::default();
        s.add(sent("a", "p", "1", 10.0));
        s.add(sent("c", "p", "3", 30.0));
        s.add(got("b", "p", "2", 20.0));
        assert_eq!(ids(&s.page("p", None, 10)), ["a", "b", "c"]);
    }

    #[test]
    fn statuses_change_by_id() {
        let mut s = Store::default();
        let mut q = sent("a", "p", "hi", 1.0);
        q.status = ChatStatus::Queued;
        s.add(q);
        s.add(got("b", "p", "yo", 2.0));
        assert!(s.set_status("a", ChatStatus::Sent));
        assert!(s.set_status("a", ChatStatus::Delivered));
        // Delivered stays delivered (a late send result doesn't undo it).
        assert!(!s.set_status("a", ChatStatus::Sent));
        assert!(!s.set_status("a", ChatStatus::Failed));
        // A received one never changes; an unknown id is nothing.
        assert!(!s.set_status("b", ChatStatus::Delivered));
        assert!(!s.set_status("x", ChatStatus::Delivered));
        let p = s.page("p", None, 10);
        assert_eq!(p.entries[0].status, ChatStatus::Delivered);
        assert_eq!(p.entries[1].status, ChatStatus::Received);
    }

    #[test]
    fn clearing_forgets_one_device() {
        let mut s = Store::default();
        s.add(sent("a", "p", "hi", 1.0));
        s.add(got("b", "q", "yo", 2.0));
        assert!(s.clear("p"));
        assert!(!s.clear("p"));
        assert_eq!(s.total, 1);
        assert_eq!(s.page("q", None, 10).kept, 1);
        s.clear_all();
        assert_eq!(s.total, 0);
        assert!(s.to_list().is_empty());
    }

    #[test]
    fn saved_and_read_back() {
        let dir = std::env::temp_dir().join(format!("md-chat-{}", uuid::Uuid::new_v4()));
        let path = dir.join(FILE);
        let mut s = Store::default();
        s.add(sent("a", "p", "hello", 1.0));
        s.add(got("b", "p", "Café ☕", 2.0));
        let mut q = sent("c", "q", "later", 3.0);
        q.status = ChatStatus::Queued;
        s.add(q);
        s.set_status("a", ChatStatus::Delivered);
        write_file(&path, &s).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        let back = read_file(&path);
        assert_eq!(back.total, 3);
        let p = back.page("p", None, 10);
        assert_eq!(ids(&p), ["a", "b"]);
        assert_eq!(p.entries[0].status, ChatStatus::Delivered);
        assert_eq!(p.entries[1].text, "Café ☕");
        assert_eq!(p.entries[1].node(), "p");
        assert!(p.entries[1].incoming());
        // Still waiting for a link when the app closed: it never went.
        assert_eq!(
            back.page("q", None, 10).entries[0].status,
            ChatStatus::Failed
        );
        // A missing or broken file is an empty history.
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read_file(&path).total, 0);
        assert_eq!(read_file(&dir.join("none.json")).total, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_snapshot_holds_every_device() {
        let mut s = Store::default();
        s.add(sent("a", "p", "hello", 1.0));
        s.add(got("b", "q", "there", 2.0));
        let all = s.to_list();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|e| e.node() == "p" && !e.incoming()));
        assert!(all.iter().any(|e| e.node() == "q" && e.incoming()));
    }
}
