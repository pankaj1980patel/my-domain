// Activity: one timeline of what happened between your devices, newest
// first, for the Activity screen.
//
//  - Built from what this computer already keeps, read as copies (nothing
//    is indexed or synced for it): transfers (transfers.rs), the clipboard
//    history (clip_history.rs: a concealed or sensitive copy is never kept
//    there, so it's never shown here), saved messages (chat_history.rs),
//    the notification history (notif_history.rs), notes, links saved to read
//    later, timers, and devices whose security key changed (the core's).
//  - Plus its own small record of what nothing else keeps
//    (`activity.json` in the app config dir): links coming up and going
//    away, calls a phone told about, rings, links opened here, screens and
//    sounds shared, wake-ups sent and schedules run. "Clear activity"
//    empties only this record and hides what came before it; the
//    histories themselves are left alone (each has its own Clear).
//  - Each source is sorted once and walked lazily; a k-way merge takes the
//    newest across them until a page is full, so a page costs about its
//    length however much is kept. Paging goes on from the last row shown
//    (its time and id), so rows added meanwhile don't shift a page.
//  - Filters: one device, some kinds (files, clipboard, messages, calls,
//    notifications, connections, other), and words (every one must be in
//    the row's subject, detail or device name; ignoring case and accents,
//    as search.rs). Kinds hidden with the screen's toggles are left out
//    everywhere.
//  - Only local data: nothing here leaves the computer.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::model::{Note, ReadLaterItem, Timer};
use tauri::{AppHandle, Emitter, Manager};

use crate::chat_history::{ChatRecord, ChatStatus};
use crate::clip_history::{ClipDirection, ClipRecord};
use crate::notif_history::HistoryEntry;
use crate::search::{Fields, Query};
use crate::transfers::{TransferEntry, TransferState};

const FILE: &str = "activity.json";
/// What the window hears when the record changed.
pub const CHANGED_EVENT: &str = "activity-changed";
/// The most entries the record keeps.
pub const MAX_LOG: usize = 1000;
/// The most rows one page returns.
pub const MAX_PAGE: u32 = 200;
/// A page's length when none is asked for.
const DEFAULT_PAGE: u32 = 50;
/// The longest subject, in characters (a message's or clip's first line).
pub const SUBJECT: usize = 80;
/// The same entry again within this long is one entry.
const REPEAT_MS: f64 = 60_000.0;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// The kinds, in the order the filter shows them.
pub const KINDS: [&str; 7] = [
    "files",
    "clipboard",
    "messages",
    "calls",
    "notifications",
    "connections",
    "other",
];

/// Which kind an action is.
pub fn kind_of(action: &str) -> &'static str {
    match action {
        a if a.starts_with("file_") => "files",
        "clip_in" | "clip_out" => "clipboard",
        a if a.starts_with("msg_") => "messages",
        "call" => "calls",
        "notification" => "notifications",
        "link_up" | "link_down" | "key_changed" | "wake" => "connections",
        _ => "other",
    }
}

/// One row of the timeline. The window words it from `action`:
///
///  - files: `file_sent`, `file_received`, `file_send_failed`,
///    `file_receive_failed`, `file_sending`, `file_receiving` (`subject` the
///    file's name, `bytes` its size, `via` how it went, `path` where it is);
///  - clipboard: `clip_in`, `clip_out` (`subject`: the clip's first line);
///  - messages: `msg_in`, `msg_out`, `msg_failed` (`subject`: its first line);
///  - notifications: `notification` (`subject` its title, `detail` the app);
///  - calls: `call` (`subject` who, `detail` "ringing", "missed", "ended"…);
///  - connections: `link_up` (`detail` the link: "quic", "relay"…),
///    `link_down`, `key_changed`, `wake`;
///  - other: `note`, `read_later` (`detail` the site), `timer` (`detail`
///    its length in ms), `ring`, `open_url`, `screen_watch`, `screen_offer`,
///    `audio_offer`, `audio_listen` (`detail` "on" / "off"), `schedule`
///    (`detail` "done", "queued", "woken" or "failed").
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct ActivityItem {
    /// Unique over the timeline: its source and its id there
    /// ("transfer:…", "clip:12", "log:40"…).
    pub id: String,
    pub kind: String,
    pub action: String,
    /// The other device, if any.
    pub node: Option<String>,
    /// When, in ms since 1970.
    pub ts: f64,
    pub subject: String,
    pub detail: String,
    pub bytes: Option<f64>,
    pub via: Option<String>,
    /// Its id in its own list, to open it (a transfer's, a clip's number,
    /// a note's…; the node id for a connection).
    pub target: String,
    /// A file's place on this computer, to show it in its folder.
    pub path: Option<String>,
}

/// What to list: one device, some kinds (all when none), words, and where
/// to go on from (the last row shown: its `ts` and `id`).
#[derive(serde::Deserialize, specta::Type, Clone, Debug, Default)]
pub struct ActivityQuery {
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub before_ts: Option<f64>,
    #[serde(default)]
    pub before_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One page, whether there's more after it, the kinds hidden, and since
/// when the view was cleared (rows before it aren't listed).
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct ActivityPage {
    pub items: Vec<ActivityItem>,
    pub more: bool,
    pub hidden: Vec<String>,
    pub cleared_at: Option<f64>,
}

// --- the record ------------------------------------------------------------------

/// One thing the record keeps.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub id: u64,
    pub action: String,
    #[serde(default)]
    pub node: Option<String>,
    pub ts: f64,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub detail: String,
}

/// The record and the view's own settings, as saved.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Log {
    /// Newest first.
    #[serde(default)]
    pub entries: Vec<LogEntry>,
    #[serde(default)]
    pub next_id: u64,
    /// Kinds the user hid.
    #[serde(default)]
    pub hidden: Vec<String>,
    /// "Clear activity": rows before this aren't listed.
    #[serde(default)]
    pub cleared_at: Option<f64>,
    /// Devices linked right now (not saved): a link coming up twice is one
    /// entry.
    #[serde(skip)]
    up: HashSet<String>,
}

impl Log {
    /// Keep an entry, unless it's the clipboard's (never recorded here: the
    /// clipboard history decides what's kept), the same as the newest one
    /// within a minute, or a link's state it's already in. Whether it
    /// changed.
    pub fn push(
        &mut self,
        action: &str,
        node: Option<&str>,
        subject: &str,
        detail: &str,
        ts: f64,
    ) -> bool {
        if kind_of(action) == "clipboard" || action.is_empty() {
            return false;
        }
        if let Some(n) = node {
            match action {
                "link_up" if !self.up.insert(n.to_string()) => return false,
                "link_down" if !self.up.remove(n) => return false,
                _ => {}
            }
        }
        if let Some(last) = self.entries.first() {
            if last.action == action
                && last.node.as_deref() == node
                && last.subject == subject
                && last.detail == detail
                && (ts - last.ts).abs() < REPEAT_MS
            {
                return false;
            }
        }
        self.next_id += 1;
        self.entries.insert(
            0,
            LogEntry {
                id: self.next_id,
                action: action.to_string(),
                node: node.map(str::to_string),
                ts,
                subject: subject.chars().take(SUBJECT * 4).collect(),
                detail: detail.chars().take(SUBJECT).collect(),
            },
        );
        self.entries.truncate(MAX_LOG);
        true
    }

    /// "Clear activity": the record goes, and rows from before now are no
    /// longer listed (the histories keep them).
    pub fn clear(&mut self, now: f64) {
        self.entries.clear();
        self.cleared_at = Some(now);
    }

    /// Show the rows from before a clear again (the record stays empty).
    pub fn restore(&mut self) {
        self.cleared_at = None;
    }

    /// Hide these kinds (unknown ones dropped, in `KINDS` order).
    pub fn set_hidden(&mut self, kinds: &[String]) {
        self.hidden = KINDS
            .iter()
            .filter(|k| kinds.iter().any(|x| x == *k))
            .map(|k| k.to_string())
            .collect();
    }
}

fn read_file(path: &Path) -> Log {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str::<Log>(&json).ok())
        .map(|mut l| {
            l.entries.sort_by(|a, b| b.ts.total_cmp(&a.ts));
            l.entries.truncate(MAX_LOG);
            l.next_id = l
                .next_id
                .max(l.entries.iter().map(|e| e.id).max().unwrap_or(0));
            l
        })
        .unwrap_or_default()
}

fn write_file(path: &Path, log: &Log) -> std::io::Result<()> {
    let json = serde_json::to_string(log)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// The record, kept in the app's state.
pub struct ActivityLog {
    path: Option<PathBuf>,
    log: Mutex<Log>,
    save_pending: AtomicBool,
}

impl ActivityLog {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_config_dir().ok().map(|d| d.join(FILE));
        let log = path.as_deref().map(read_file).unwrap_or_default();
        ActivityLog {
            path,
            log: Mutex::new(log),
            save_pending: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> Log {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }

    fn change(&self, app: &AppHandle, f: impl FnOnce(&mut Log) -> bool) {
        let changed = self.log.lock().map(|mut l| f(&mut l)).unwrap_or(false);
        if !changed {
            return;
        }
        let _ = app.emit(CHANGED_EVENT, ());
        if self.save_pending.swap(true, AtomicOrdering::AcqRel) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            if let Some(h) = app.try_state::<ActivityLog>() {
                h.save_pending.store(false, AtomicOrdering::Release);
                h.save_now();
            }
        });
    }

    fn save_now(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let log = self.snapshot();
        if let Err(e) = write_file(path, &log) {
            eprintln!("[activity] couldn't save the activity record: {e}");
        }
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Note something in the record (see `ActivityItem` for the actions).
pub fn record(app: &AppHandle, action: &str, node: Option<&str>, subject: &str, detail: &str) {
    if let Some(h) = app.try_state::<ActivityLog>() {
        let now = now_ms();
        h.change(app, |l| l.push(action, node, subject, detail, now));
    }
}

// --- the sources -------------------------------------------------------------------

/// A device whose key changed: when it first came with the new one.
#[derive(Clone, Debug, Default)]
pub struct KeyChangeRow {
    pub node: String,
    pub name: String,
    pub ts: f64,
}

/// Copies of what each list keeps, gathered for one page.
#[derive(Default)]
pub struct Sources {
    /// Device names as shown (nicknames first), for the words filter.
    pub names: BTreeMap<String, String>,
    pub transfers: Vec<TransferEntry>,
    pub clips: Vec<ClipRecord>,
    pub messages: Vec<ChatRecord>,
    pub notifications: Vec<HistoryEntry>,
    pub notes: Vec<Note>,
    pub read_later: Vec<ReadLaterItem>,
    pub timers: Vec<Timer>,
    pub key_changes: Vec<KeyChangeRow>,
    pub log: Vec<LogEntry>,
}

/// A message's or clip's first line, at most `SUBJECT` characters.
fn first_line(text: &str) -> String {
    crate::clip_history::preview(text, SUBJECT)
}

fn some_node(n: &str) -> Option<String> {
    (!n.is_empty()).then(|| n.to_string())
}

fn transfer_item(t: &TransferEntry) -> ActivityItem {
    let action = match (t.state, t.incoming) {
        (TransferState::Done, true) => "file_received",
        (TransferState::Done, false) => "file_sent",
        (TransferState::Failed, true) => "file_receive_failed",
        (TransferState::Failed, false) => "file_send_failed",
        (TransferState::Active, true) => "file_receiving",
        (TransferState::Active, false) => "file_sending",
    };
    ActivityItem {
        id: format!("transfer:{}", t.id),
        kind: "files".into(),
        action: action.into(),
        node: some_node(&t.node_id),
        ts: t.ended.unwrap_or(t.started),
        subject: t.name.clone(),
        detail: t.reason.clone().unwrap_or_default(),
        bytes: (t.total > 0.0).then_some(t.total),
        via: t.via.clone(),
        target: t.id.clone(),
        path: t.path.clone(),
    }
}

fn clip_item(c: &ClipRecord) -> ActivityItem {
    ActivityItem {
        id: format!("clip:{}", c.id),
        kind: "clipboard".into(),
        action: match c.direction {
            ClipDirection::In => "clip_in",
            ClipDirection::Out => "clip_out",
        }
        .into(),
        node: c.node().map(str::to_string),
        ts: c.at,
        subject: first_line(&c.text),
        detail: String::new(),
        bytes: None,
        via: None,
        target: c.id.to_string(),
        path: None,
    }
}

fn message_item(m: &ChatRecord) -> ActivityItem {
    let action = match m.status {
        ChatStatus::Received => "msg_in",
        ChatStatus::Failed => "msg_failed",
        _ => "msg_out",
    };
    ActivityItem {
        id: format!("msg:{}", m.id),
        kind: "messages".into(),
        action: action.into(),
        node: some_node(m.node()),
        ts: m.at,
        subject: first_line(&m.text),
        detail: String::new(),
        bytes: None,
        via: None,
        target: m.id.clone(),
        path: None,
    }
}

fn notification_item(n: &HistoryEntry) -> ActivityItem {
    let title = if n.title.trim().is_empty() {
        &n.app
    } else {
        &n.title
    };
    ActivityItem {
        id: format!("notif:{}", n.id),
        kind: "notifications".into(),
        action: "notification".into(),
        node: some_node(&n.from),
        ts: n.ts,
        subject: first_line(title),
        detail: n.app.clone(),
        bytes: None,
        via: None,
        target: n.id.to_string(),
        path: None,
    }
}

fn note_item(n: &Note) -> ActivityItem {
    ActivityItem {
        id: format!("note:{}", n.id),
        kind: "other".into(),
        action: "note".into(),
        node: some_node(&n.by),
        ts: n.updated_at as f64,
        subject: first_line(crate::search::note_title(n)),
        detail: String::new(),
        bytes: None,
        via: None,
        target: n.id.clone(),
        path: None,
    }
}

fn read_later_item(i: &ReadLaterItem) -> ActivityItem {
    let title = if i.title.trim().is_empty() {
        &i.url
    } else {
        &i.title
    };
    ActivityItem {
        id: format!("link:{}", i.id),
        kind: "other".into(),
        action: "read_later".into(),
        node: some_node(&i.by),
        ts: i.saved_at as f64,
        subject: first_line(title),
        detail: i.host.clone(),
        bytes: None,
        via: None,
        target: i.id.clone(),
        path: None,
    }
}

fn timer_item(t: &Timer) -> ActivityItem {
    ActivityItem {
        id: format!("timer:{}", t.id),
        kind: "other".into(),
        action: "timer".into(),
        node: some_node(&t.by),
        ts: t.created_at as f64,
        subject: t.label.clone(),
        detail: t.duration_ms.to_string(),
        bytes: None,
        via: None,
        target: t.id.clone(),
        path: None,
    }
}

fn key_change_item(k: &KeyChangeRow) -> ActivityItem {
    ActivityItem {
        id: format!("key:{}", k.node),
        kind: "connections".into(),
        action: "key_changed".into(),
        node: Some(k.node.clone()),
        ts: k.ts,
        subject: k.name.clone(),
        detail: String::new(),
        bytes: None,
        via: None,
        target: k.node.clone(),
        path: None,
    }
}

fn log_item(e: &LogEntry) -> ActivityItem {
    ActivityItem {
        id: format!("log:{}", e.id),
        kind: kind_of(&e.action).into(),
        action: e.action.clone(),
        node: e.node.clone(),
        ts: e.ts,
        subject: e.subject.clone(),
        detail: e.detail.clone(),
        bytes: None,
        via: None,
        target: e.node.clone().unwrap_or_default(),
        path: None,
    }
}

// --- merging -----------------------------------------------------------------------

/// Timeline order: newer first, then by id (so equal times keep one order).
pub fn newer_first(a_ts: f64, a_id: &str, b_ts: f64, b_id: &str) -> Ordering {
    b_ts.total_cmp(&a_ts).then_with(|| a_id.cmp(b_id))
}

/// One source, walked lazily: `items` sorted newest first, each made into a
/// row only when it's reached.
struct Stream<'a> {
    rows: Box<dyn Iterator<Item = ActivityItem> + 'a>,
}

/// `list` newest first by `ts`, rows made by `make` as they're taken;
/// those newer than `before` (a page's cursor) skipped without being made.
fn stream<'a, T>(
    list: &'a [T],
    before: Option<f64>,
    ts: impl Fn(&T) -> f64 + 'a,
    make: impl Fn(&T) -> ActivityItem + 'a,
) -> Stream<'a> {
    let mut order: Vec<&T> = list.iter().collect();
    order.sort_by(|a, b| ts(b).total_cmp(&ts(a)));
    if let Some(before) = before {
        let skip = order.partition_point(|x| ts(x) > before);
        order.drain(..skip);
    }
    // Equal times: the rows' ids decide, as in the merge.
    let rows = TieSorted {
        inner: order.into_iter().map(make).peekable(),
        held: Vec::new(),
    };
    Stream {
        rows: Box::new(rows),
    }
}

/// Rows already newest first; runs of equal times put in id order.
struct TieSorted<I: Iterator<Item = ActivityItem>> {
    inner: std::iter::Peekable<I>,
    /// A run of equal times, in reverse (popped from the end).
    held: Vec<ActivityItem>,
}

impl<I: Iterator<Item = ActivityItem>> Iterator for TieSorted<I> {
    type Item = ActivityItem;
    fn next(&mut self) -> Option<ActivityItem> {
        if let Some(x) = self.held.pop() {
            return Some(x);
        }
        let first = self.inner.next()?;
        if self.inner.peek().is_none_or(|n| n.ts != first.ts) {
            return Some(first);
        }
        let mut run = vec![first];
        while let Some(n) = self.inner.next_if(|n| n.ts == run[0].ts) {
            run.push(n);
        }
        run.sort_by(|a, b| b.id.cmp(&a.id));
        self.held = run;
        self.held.pop()
    }
}

/// A row waiting in the merge, ordered so the heap's top is the newest.
struct Head {
    item: ActivityItem,
    from: usize,
}

impl PartialEq for Head {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Head {}
impl PartialOrd for Head {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Head {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap pops the greatest: the newest is the greatest.
        newer_first(other.item.ts, &other.item.id, self.item.ts, &self.item.id)
    }
}

/// Up to `limit` rows across `streams`, newest first, that `keep` keeps;
/// and whether there are more. Each stream is read only as far as needed.
fn merge(
    mut streams: Vec<Stream<'_>>,
    limit: usize,
    mut keep: impl FnMut(&ActivityItem) -> bool,
) -> (Vec<ActivityItem>, bool) {
    let mut heap = BinaryHeap::with_capacity(streams.len());
    let pull =
        |streams: &mut Vec<Stream<'_>>, i: usize, keep: &mut dyn FnMut(&ActivityItem) -> bool| {
            streams[i]
                .rows
                .find(|x| keep(x))
                .map(|item| Head { item, from: i })
        };
    for i in 0..streams.len() {
        if let Some(h) = pull(&mut streams, i, &mut keep) {
            heap.push(h);
        }
    }
    let mut out = Vec::with_capacity(limit.min(256));
    while let Some(Head { item, from }) = heap.pop() {
        if out.len() == limit {
            return (out, true);
        }
        out.push(item);
        if let Some(h) = pull(&mut streams, from, &mut keep) {
            heap.push(h);
        }
    }
    (out, false)
}

/// One page of the timeline from `src` for `q`, leaving out `hidden` kinds
/// and rows from before `cleared_at`.
pub fn page(
    src: &Sources,
    q: &ActivityQuery,
    hidden: &[String],
    cleared_at: Option<f64>,
) -> ActivityPage {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as usize;
    let wanted_kinds: Vec<&str> = q
        .kinds
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|k| KINDS.contains(k))
        .collect();
    let wanted = |kind: &str| {
        !hidden.iter().any(|h| h == kind)
            && (wanted_kinds.is_empty() || wanted_kinds.contains(&kind))
    };
    let before = q.before_ts;
    let mut streams = Vec::new();
    if wanted("files") {
        streams.push(stream(
            &src.transfers,
            before,
            |t| t.ended.unwrap_or(t.started),
            transfer_item,
        ));
    }
    if wanted("clipboard") {
        streams.push(stream(&src.clips, before, |c| c.at, clip_item));
    }
    if wanted("messages") {
        streams.push(stream(&src.messages, before, |m| m.at, message_item));
    }
    if wanted("notifications") {
        streams.push(stream(
            &src.notifications,
            before,
            |n| n.ts,
            notification_item,
        ));
    }
    if wanted("connections") {
        streams.push(stream(&src.key_changes, before, |k| k.ts, key_change_item));
    }
    if wanted("other") {
        streams.push(stream(
            &src.notes,
            before,
            |n| n.updated_at as f64,
            note_item,
        ));
        streams.push(stream(
            &src.read_later,
            before,
            |i| i.saved_at as f64,
            read_later_item,
        ));
        streams.push(stream(
            &src.timers,
            before,
            |t| t.created_at as f64,
            timer_item,
        ));
    }
    // The record holds several kinds: filtered row by row.
    if hidden.len() < KINDS.len() {
        streams.push(stream(&src.log, before, |e| e.ts, log_item));
    }
    let node = q.node.as_deref().filter(|n| !n.is_empty());
    let words = Query::new(q.text.as_deref().unwrap_or(""));
    let cursor = q
        .before_ts
        .map(|ts| (ts, q.before_id.clone().unwrap_or_default()));
    let keep = |x: &ActivityItem| {
        if x.ts <= 0.0 || cleared_at.is_some_and(|c| x.ts < c) {
            return false;
        }
        if let Some((ts, id)) = &cursor {
            if newer_first(x.ts, &x.id, *ts, id) != Ordering::Greater {
                return false;
            }
        }
        if !wanted(&x.kind) {
            return false;
        }
        if node.is_some() && x.node.as_deref() != node {
            return false;
        }
        if !words.is_empty() {
            let name = x
                .node
                .as_deref()
                .and_then(|n| src.names.get(n))
                .map_or("", String::as_str);
            let detail = format!("{} {name}", x.detail);
            let fields = Fields {
                title: &x.subject,
                secondary: &detail,
                body: x.path.as_deref().unwrap_or(""),
            };
            if crate::search::score(&words, &fields).is_none() {
                return false;
            }
        }
        true
    };
    let (items, more) = merge(streams, limit, keep);
    ActivityPage {
        items,
        more,
        hidden: hidden.to_vec(),
        cleared_at,
    }
}

/// Copies of the lists `q` and `hidden` need, from the app's state.
fn gather(app: &AppHandle, q: &ActivityQuery, hidden: &[String], log: Vec<LogEntry>) -> Sources {
    let asked: Vec<&str> = q.kinds.iter().flatten().map(String::as_str).collect();
    let wanted =
        |k: &str| !hidden.iter().any(|h| h == k) && (asked.is_empty() || asked.contains(&k));
    let mut src = Sources {
        log,
        ..Sources::default()
    };
    let engine = app.try_state::<crate::Eng>();
    if let Some(engine) = &engine {
        let labels = app
            .try_state::<crate::device_labels::DeviceLabels>()
            .map(crate::device_labels::get_device_labels)
            .unwrap_or_default();
        for p in engine.get_peers() {
            let shown = crate::device_labels::name_for(&labels, &p.node_id, &p.name);
            src.names.insert(p.node_id, shown);
        }
    }
    if wanted("files") {
        if let Some(h) = app.try_state::<crate::transfers::TransferHistory>() {
            src.transfers = h.snapshot();
        }
    }
    if wanted("clipboard") {
        if let Some(h) = app.try_state::<crate::clip_history::ClipHistory>() {
            src.clips = h.snapshot();
        }
    }
    if wanted("messages") {
        if let Some(h) = app.try_state::<crate::chat_history::ChatHistory>() {
            src.messages = h.snapshot();
        }
    }
    if wanted("notifications") {
        if let Some(h) = app.try_state::<crate::notif_history::NotifHistory>() {
            src.notifications = h.snapshot();
        }
    }
    if let Some(engine) = &engine {
        if wanted("other") {
            src.notes = engine.notes();
            src.read_later = engine.read_later();
            src.timers = engine.timers();
        }
        if wanted("connections") {
            let now = now_ms();
            src.key_changes = engine
                .key_changes()
                .into_iter()
                .map(|k| KeyChangeRow {
                    ts: now - f64::from(k.first_ago_s) * 1000.0,
                    node: k.node_id,
                    name: k.name,
                })
                .collect();
        }
    }
    src
}

/// One page of the timeline (see `ActivityQuery`). Only local data is read.
#[tauri::command]
#[specta::specta]
pub async fn activity_feed(query: ActivityQuery, app: AppHandle) -> ActivityPage {
    let log = app
        .try_state::<ActivityLog>()
        .map(|h| h.snapshot())
        .unwrap_or_default();
    let src = gather(&app, &query, &log.hidden, log.entries);
    page(&src, &query, &log.hidden, log.cleared_at)
}

/// Hide these kinds from the timeline (the others show); the kinds hidden.
#[tauri::command]
#[specta::specta]
pub fn set_activity_hidden(kinds: Vec<String>, app: AppHandle) -> Vec<String> {
    let Some(h) = app.try_state::<ActivityLog>() else {
        return Vec::new();
    };
    h.change(&app, |l| {
        let was = l.hidden.clone();
        l.set_hidden(&kinds);
        l.hidden != was
    });
    h.snapshot().hidden
}

/// "Clear activity": empties the timeline's own record and hides what came
/// before now. The transfer, clipboard, message and notification histories
/// are not touched.
#[tauri::command]
#[specta::specta]
pub fn clear_activity(app: AppHandle) {
    if let Some(h) = app.try_state::<ActivityLog>() {
        let now = now_ms();
        h.change(&app, |l| {
            l.clear(now);
            true
        });
    }
}

/// Show the rows from before "Clear activity" again (those still kept in
/// their histories).
#[tauri::command]
#[specta::specta]
pub fn restore_activity(app: AppHandle) {
    if let Some(h) = app.try_state::<ActivityLog>() {
        h.change(&app, |l| {
            let was = l.cleared_at.is_some();
            l.restore();
            was
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_700_000_000_000.0;

    fn transfer(id: &str, node: &str, name: &str, incoming: bool, ts: f64) -> TransferEntry {
        TransferEntry {
            id: id.into(),
            node_id: node.into(),
            name: name.into(),
            incoming,
            state: TransferState::Done,
            done: 12e6,
            total: 12e6,
            resumed_from: None,
            path: Some(format!("/Users/me/Downloads/{name}")),
            reason: None,
            started: ts - 1000.0,
            ended: Some(ts),
            exists: true,
            via: Some("wifi_direct".into()),
        }
    }

    fn clip(id: u32, node: &str, text: &str, at: f64) -> ClipRecord {
        ClipRecord {
            id,
            text: text.into(),
            truncated: false,
            from_node: Some(node.into()),
            to_node: None,
            direction: ClipDirection::In,
            at,
        }
    }

    fn message(id: &str, node: &str, text: &str, at: f64) -> ChatRecord {
        ChatRecord {
            id: id.into(),
            from_node: None,
            to_node: Some(node.into()),
            text: text.into(),
            truncated: false,
            at,
            status: ChatStatus::Delivered,
        }
    }

    fn notification(id: u32, node: &str, title: &str, ts: f64) -> HistoryEntry {
        HistoryEntry {
            id,
            from: node.into(),
            app: "Mail".into(),
            title: title.into(),
            body: "body".into(),
            key: None,
            ts,
            dismissed: false,
        }
    }

    fn q() -> ActivityQuery {
        ActivityQuery::default()
    }

    fn seeded() -> Sources {
        let mut names = BTreeMap::new();
        names.insert("p".to_string(), "Pixel".to_string());
        names.insert("m".to_string(), "MacBook".to_string());
        let mut log = Log::default();
        log.push("link_up", Some("p"), "", "quic", NOW - 500.0);
        Sources {
            names,
            transfers: vec![transfer("t1", "p", "report.pdf", false, NOW - 1000.0)],
            clips: vec![clip(1, "p", "Café on Rue Cler\nsecond line", NOW - 2000.0)],
            messages: vec![message("m1", "m", "see you at 8", NOW - 3000.0)],
            notifications: vec![notification(1, "p", "New mail", NOW - 4000.0)],
            log: log.entries,
            ..Sources::default()
        }
    }

    #[test]
    fn kinds_of_actions() {
        assert_eq!(kind_of("file_sent"), "files");
        assert_eq!(kind_of("clip_in"), "clipboard");
        assert_eq!(kind_of("msg_failed"), "messages");
        assert_eq!(kind_of("call"), "calls");
        assert_eq!(kind_of("notification"), "notifications");
        assert_eq!(kind_of("link_down"), "connections");
        assert_eq!(kind_of("key_changed"), "connections");
        assert_eq!(kind_of("screen_offer"), "other");
    }

    #[test]
    fn merges_every_source_newest_first() {
        let p = page(&seeded(), &q(), &[], None);
        let actions: Vec<&str> = p.items.iter().map(|i| i.action.as_str()).collect();
        assert_eq!(
            actions,
            ["link_up", "file_sent", "clip_in", "msg_out", "notification"]
        );
        assert!(!p.more);
        let file = &p.items[1];
        assert_eq!(file.subject, "report.pdf");
        assert_eq!(file.bytes, Some(12e6));
        assert_eq!(file.via.as_deref(), Some("wifi_direct"));
        assert_eq!(file.node.as_deref(), Some("p"));
        // A clip shows its first line only.
        assert_eq!(p.items[2].subject, "Café on Rue Cler\u{2026}");
    }

    #[test]
    fn filters_by_device_kind_and_words() {
        let src = seeded();
        let by_node = page(
            &src,
            &ActivityQuery {
                node: Some("m".into()),
                ..q()
            },
            &[],
            None,
        );
        assert_eq!(by_node.items.len(), 1);
        assert_eq!(by_node.items[0].action, "msg_out");

        let files = page(
            &src,
            &ActivityQuery {
                kinds: Some(vec!["files".into(), "clipboard".into()]),
                ..q()
            },
            &[],
            None,
        );
        let kinds: Vec<&str> = files.items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["files", "clipboard"]);

        // Words: in the subject (accents ignored), or the device's name.
        let cafe = page(
            &src,
            &ActivityQuery {
                text: Some("CAFE".into()),
                ..q()
            },
            &[],
            None,
        );
        assert_eq!(cafe.items.len(), 1);
        let pixel = page(
            &src,
            &ActivityQuery {
                text: Some("pixel".into()),
                ..q()
            },
            &[],
            None,
        );
        assert_eq!(pixel.items.len(), 4);
    }

    #[test]
    fn hidden_kinds_and_clear() {
        let src = seeded();
        let hidden = vec!["connections".to_string(), "files".to_string()];
        let p = page(&src, &q(), &hidden, None);
        assert!(p
            .items
            .iter()
            .all(|i| i.kind != "connections" && i.kind != "files"));
        assert_eq!(p.items.len(), 3);
        // A hidden kind stays hidden even when asked for.
        let asked = page(
            &src,
            &ActivityQuery {
                kinds: Some(vec!["files".into()]),
                ..q()
            },
            &hidden,
            None,
        );
        assert!(asked.items.is_empty());
        // Cleared: only what came after.
        let p = page(&src, &q(), &[], Some(NOW - 2500.0));
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.cleared_at, Some(NOW - 2500.0));
    }

    #[test]
    fn the_record_and_clear_leave_histories_alone() {
        let mut log = Log::default();
        assert!(log.push("link_up", Some("p"), "", "quic", NOW));
        // Up again: already up.
        assert!(!log.push("link_up", Some("p"), "", "relay", NOW + 1.0));
        assert!(log.push("link_down", Some("p"), "", "", NOW + 2.0));
        assert!(!log.push("link_down", Some("p"), "", "", NOW + 3.0));
        // The same call twice within a minute is one entry.
        assert!(log.push("call", Some("p"), "Mum", "ringing", NOW + 4.0));
        assert!(!log.push("call", Some("p"), "Mum", "ringing", NOW + 5.0));
        assert!(log.push("call", Some("p"), "Mum", "missed", NOW + 6.0));
        assert_eq!(log.entries.len(), 4);
        // The clipboard's never recorded here.
        assert!(!log.push("clip_in", Some("p"), "hunter2", "", NOW + 7.0));
        assert_eq!(log.entries.len(), 4);
        // Capped.
        for i in 0..(MAX_LOG + 10) {
            log.push("ring", Some("p"), &i.to_string(), "", NOW + 10.0 + i as f64);
        }
        assert_eq!(log.entries.len(), MAX_LOG);
        log.clear(NOW + 99_999.0);
        assert!(log.entries.is_empty());
        let mut src = seeded();
        src.log = log.entries.clone();
        // The histories still have their rows; the view hides them until
        // shown again.
        assert!(page(&src, &q(), &[], log.cleared_at).items.is_empty());
        log.restore();
        assert_eq!(page(&src, &q(), &[], log.cleared_at).items.len(), 4);
        assert_eq!(src.transfers.len(), 1);
        assert_eq!(src.clips.len(), 1);
    }

    #[test]
    fn hidden_kinds_saved_in_order() {
        let mut log = Log::default();
        log.set_hidden(&["other".into(), "bogus".into(), "files".into()]);
        assert_eq!(log.hidden, ["files", "other"]);
        let json = serde_json::to_string(&log).unwrap();
        let back: Log = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hidden, log.hidden);
    }

    #[test]
    fn privacy_only_kept_clips_show_and_only_their_first_line() {
        // A concealed or sensitive copy never reaches the clipboard history
        // (clip_privacy.rs, lib.rs), and the record refuses clips, so the
        // only clips listed are the history's.
        let mut log = Log::default();
        assert!(!log.push("clip_out", Some("p"), "s3cret!", "", NOW));
        let src = Sources {
            clips: vec![clip(1, "p", "first line\nsecond: hunter2", NOW)],
            log: log.entries,
            ..Sources::default()
        };
        let p = page(&src, &q(), &[], None);
        assert_eq!(p.items.len(), 1);
        assert!(!p.items[0].subject.contains("hunter2"));
        // And the words filter doesn't look past what's shown.
        let found = page(
            &src,
            &ActivityQuery {
                text: Some("hunter2".into()),
                ..q()
            },
            &[],
            None,
        );
        assert!(found.items.is_empty());
    }

    /// 10 000 rows in each of several sources: pages walk them all in order,
    /// with no row twice or missed, fast.
    #[test]
    fn pages_through_ten_thousand() {
        let n = 10_000;
        let mut src = Sources::default();
        for i in 0..n {
            // Equal times across sources, to check the tie order.
            let ts = NOW - (i / 3) as f64 * 1000.0;
            src.transfers
                .push(transfer(&format!("t{i}"), "p", "f.bin", i % 2 == 0, ts));
            src.clips.push(clip(i as u32, "p", "clip", ts));
            src.messages.push(message(&format!("m{i}"), "m", "hi", ts));
            src.notifications.push(notification(i as u32, "p", "n", ts));
        }
        // Not sorted as kept.
        src.clips.reverse();
        let started = std::time::Instant::now();
        let first = page(&src, &q(), &[], None);
        assert_eq!(first.items.len(), DEFAULT_PAGE as usize);
        assert!(first.more);
        let one_page = started.elapsed();
        assert!(one_page.as_millis() < 2000, "a page took {one_page:?}");

        let mut seen = HashSet::new();
        let mut all = Vec::new();
        let mut query = ActivityQuery {
            limit: Some(MAX_PAGE),
            ..q()
        };
        loop {
            let p = page(&src, &query, &[], None);
            for i in &p.items {
                assert!(seen.insert(i.id.clone()), "{} twice", i.id);
            }
            let last = p.items.last().cloned();
            all.extend(p.items);
            if !p.more {
                break;
            }
            let last = last.unwrap();
            query.before_ts = Some(last.ts);
            query.before_id = Some(last.id);
        }
        assert_eq!(all.len(), 4 * n);
        assert!(all
            .windows(2)
            .all(|w| newer_first(w[0].ts, &w[0].id, w[1].ts, &w[1].id) == Ordering::Less));

        // Filtered paging too: one device's, one kind.
        let p = page(
            &src,
            &ActivityQuery {
                node: Some("m".into()),
                limit: Some(10),
                ..q()
            },
            &[],
            None,
        );
        assert!(p.items.iter().all(|i| i.action == "msg_out"));
        assert!(p.more);
    }

    #[test]
    fn saved_record_reads_back() {
        let dir = std::env::temp_dir().join(format!("md-activity-{}", std::process::id()));
        let path = dir.join(FILE);
        let mut log = Log::default();
        log.push("wake", Some("m"), "MacBook", "", NOW);
        log.push("ring", Some("p"), "", "", NOW + 1.0);
        log.set_hidden(&["calls".into()]);
        write_file(&path, &log).unwrap();
        let back = read_file(&path);
        assert_eq!(back.entries, log.entries);
        assert_eq!(back.hidden, ["calls"]);
        assert_eq!(back.next_id, 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
