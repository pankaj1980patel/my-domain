// Transfers: every file going to or coming from your devices, in one list.
//
//  - transfers under way are kept in memory only (a restart resumes them,
//    and their events bring them back);
//  - finished and failed ones, the newest `MAX_FINISHED`, are also kept in
//    `<app config dir>/transfer-history.json`, written a few seconds after a
//    change (`SAVE_DELAY`) through a temporary file and a rename;
//  - one entry per transfer id: each event updates it in place;
//  - a file sent with `send_file` remembers where it came from, so it can be
//    shown in its folder or sent again (folders don't: their files' ids
//    aren't known here).
//
// The hooks are called from the core's event sink (lib.rs), so they only
// take a lock and update the list; progress never writes the file.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

const FILE: &str = "transfer-history.json";
/// The most finished transfers kept.
pub const MAX_FINISHED: usize = 100;
/// How long after a change the file is written.
const SAVE_DELAY: Duration = Duration::from_secs(3);

/// Where a transfer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum TransferState {
    /// Offered or moving.
    Active,
    /// Sent (the other device confirmed it) or saved here.
    Done,
    /// Stopped for good: cancelled (either side) or an error; see `reason`.
    Failed,
}

/// One transfer, either way.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct TransferEntry {
    pub id: String,
    /// The other device.
    pub node_id: String,
    pub name: String,
    /// To this computer.
    pub incoming: bool,
    pub state: TransferState,
    /// Bytes confirmed so far, of `total` (0 while not yet known).
    pub done: f64,
    pub total: f64,
    /// It carried on from a partial file: the bytes it resumed at.
    #[serde(default)]
    pub resumed_from: Option<f64>,
    /// Received: where it was saved. Sent: the file it came from.
    #[serde(default)]
    pub path: Option<String>,
    /// Why it failed (the core's words).
    #[serde(default)]
    pub reason: Option<String>,
    /// When it was first seen and when it ended, in ms since 1970.
    pub started: f64,
    #[serde(default)]
    pub ended: Option<f64>,
    /// `path` is still there (filled in when listed).
    #[serde(default)]
    pub exists: bool,
}

impl TransferEntry {
    fn new(id: &str, node_id: &str, name: &str, incoming: bool, now: f64) -> Self {
        TransferEntry {
            id: id.to_string(),
            node_id: node_id.to_string(),
            name: name.to_string(),
            incoming,
            state: TransferState::Active,
            done: 0.0,
            total: 0.0,
            resumed_from: None,
            path: None,
            reason: None,
            started: now,
            ended: None,
            exists: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.state != TransferState::Active
    }
}

/// The list itself, without the app around it (so it can be tested).
#[derive(Default, Debug)]
pub struct Transfers {
    list: Vec<TransferEntry>,
}

impl Transfers {
    /// From saved entries: only finished ones count (a transfer under way
    /// comes back through its events).
    pub fn from_saved(mut list: Vec<TransferEntry>) -> Self {
        list.retain(TransferEntry::finished);
        let mut t = Transfers { list };
        t.trim();
        t
    }

    fn entry(
        &mut self,
        id: &str,
        node_id: &str,
        name: &str,
        incoming: bool,
        now: f64,
    ) -> &mut TransferEntry {
        let at = match self.list.iter().position(|e| e.id == id) {
            Some(i) => i,
            None => {
                self.list
                    .push(TransferEntry::new(id, node_id, name, incoming, now));
                self.list.len() - 1
            }
        };
        &mut self.list[at]
    }

    pub fn get(&self, id: &str) -> Option<&TransferEntry> {
        self.list.iter().find(|e| e.id == id)
    }

    /// A transfer moved on. Late progress for one that ended is ignored.
    #[allow(clippy::too_many_arguments)]
    pub fn progress(
        &mut self,
        id: &str,
        node_id: &str,
        name: &str,
        done: u64,
        total: u64,
        incoming: bool,
        resumed: bool,
        now: f64,
    ) {
        let e = self.entry(id, node_id, name, incoming, now);
        if e.finished() {
            return;
        }
        e.done = done as f64;
        e.total = total as f64;
        if resumed && e.resumed_from.is_none() {
            e.resumed_from = Some(done as f64);
        }
    }

    /// We started sending `path` as transfer `id`.
    pub fn sending(&mut self, id: &str, node_id: &str, path: &str, now: f64) {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        let e = self.entry(id, node_id, &name, false, now);
        e.path = Some(path.to_string());
    }

    /// A file arrived and was saved at `path`. Whether it changed anything.
    pub fn received(
        &mut self,
        id: &str,
        from: &str,
        name: &str,
        path: &str,
        size: u64,
        now: f64,
    ) -> bool {
        let e = self.entry(id, from, name, true, now);
        if e.state == TransferState::Done {
            return false;
        }
        e.state = TransferState::Done;
        e.done = size as f64;
        e.total = size as f64;
        e.path = Some(path.to_string());
        e.reason = None;
        e.ended = Some(now);
        self.trim();
        true
    }

    /// The other device saved our file.
    pub fn sent(&mut self, id: &str, node_id: &str, name: &str, now: f64) -> bool {
        let e = self.entry(id, node_id, name, false, now);
        if e.state == TransferState::Done {
            return false;
        }
        e.state = TransferState::Done;
        e.done = e.total;
        e.reason = None;
        e.ended = Some(now);
        self.trim();
        true
    }

    /// A transfer stopped for good. One already done stays done.
    pub fn failed(
        &mut self,
        id: &str,
        node_id: &str,
        name: &str,
        incoming: bool,
        reason: &str,
        now: f64,
    ) -> bool {
        let e = self.entry(id, node_id, name, incoming, now);
        if e.state == TransferState::Done {
            return false;
        }
        e.state = TransferState::Failed;
        e.reason = Some(reason.to_string());
        e.ended = Some(now);
        self.trim();
        true
    }

    /// Forget every finished transfer.
    pub fn clear_finished(&mut self) {
        self.list.retain(|e| !e.finished());
    }

    /// Keep the newest `MAX_FINISHED` finished ones.
    fn trim(&mut self) {
        let mut finished: Vec<(f64, String)> = self
            .list
            .iter()
            .filter(|e| e.finished())
            .map(|e| (e.ended.unwrap_or(e.started), e.id.clone()))
            .collect();
        if finished.len() <= MAX_FINISHED {
            return;
        }
        finished.sort_by(|a, b| b.0.total_cmp(&a.0));
        let drop: std::collections::HashSet<String> =
            finished.drain(MAX_FINISHED..).map(|(_, id)| id).collect();
        self.list.retain(|e| !drop.contains(&e.id));
    }

    /// What the view shows: those under way (newest first), then the
    /// finished ones (latest ended first).
    pub fn listed(&self) -> Vec<TransferEntry> {
        let mut active: Vec<TransferEntry> = self
            .list
            .iter()
            .filter(|e| !e.finished())
            .cloned()
            .collect();
        active.sort_by(|a, b| b.started.total_cmp(&a.started));
        let mut done: Vec<TransferEntry> =
            self.list.iter().filter(|e| e.finished()).cloned().collect();
        done.sort_by(|a, b| {
            b.ended
                .unwrap_or(b.started)
                .total_cmp(&a.ended.unwrap_or(a.started))
        });
        active.extend(done);
        active
    }

    /// What's saved: the finished ones.
    pub fn saved(&self) -> Vec<TransferEntry> {
        self.listed().into_iter().filter(|e| e.finished()).collect()
    }
}

/// The list, its file and the pending write.
pub struct TransferHistory {
    path: Option<PathBuf>,
    inner: Mutex<Transfers>,
    save_pending: AtomicBool,
}

impl TransferHistory {
    pub fn load(app: &AppHandle) -> Self {
        let path = app.path().app_config_dir().ok().map(|d| d.join(FILE));
        Self::at(path)
    }

    fn at(path: Option<PathBuf>) -> Self {
        let list = path.as_deref().map(read_file).unwrap_or_default();
        TransferHistory {
            path,
            inner: Mutex::new(Transfers::from_saved(list)),
            save_pending: AtomicBool::new(false),
        }
    }

    fn save_soon(&self, app: &AppHandle) {
        if self.save_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SAVE_DELAY);
            if let Some(h) = app.try_state::<TransferHistory>() {
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
            Ok(t) => serde_json::to_string(&t.saved()),
            Err(_) => return,
        };
        let Ok(json) = json else {
            return;
        };
        if let Err(e) = write_atomic(path, &json) {
            eprintln!("[transfers] couldn't save the transfer history: {e}");
        }
    }
}

fn read_file(path: &Path) -> Vec<TransferEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Write to a temporary file, then rename it over `path`.
fn write_atomic(path: &Path, data: &str) -> std::io::Result<()> {
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

/// Run `f` on the list; save soon when it says something finished changed.
fn update(app: &AppHandle, f: impl FnOnce(&mut Transfers, f64) -> bool) {
    let Some(h) = app.try_state::<TransferHistory>() else {
        return;
    };
    let changed = h
        .inner
        .lock()
        .map(|mut t| f(&mut t, now_ms()))
        .unwrap_or(false);
    if changed {
        h.save_soon(app);
    }
}

// --- hooks (from the event sink and send_file) -------------------------------

#[allow(clippy::too_many_arguments)]
pub fn on_progress(
    app: &AppHandle,
    id: &str,
    node_id: &str,
    name: &str,
    done: u64,
    total: u64,
    incoming: bool,
    resumed: bool,
) {
    update(app, |t, now| {
        t.progress(id, node_id, name, done, total, incoming, resumed, now);
        false
    });
}

pub fn on_received(app: &AppHandle, id: &str, from: &str, name: &str, path: &str, size: u64) {
    update(app, |t, now| t.received(id, from, name, path, size, now));
}

pub fn on_sent(app: &AppHandle, id: &str, node_id: &str, name: &str) {
    update(app, |t, now| t.sent(id, node_id, name, now));
}

pub fn on_failed(
    app: &AppHandle,
    id: &str,
    node_id: &str,
    name: &str,
    incoming: bool,
    reason: &str,
) {
    update(app, |t, now| {
        t.failed(id, node_id, name, incoming, reason, now)
    });
}

/// `send_file` started sending `path`: remembered for "Send again".
pub fn on_sending(app: &AppHandle, id: &str, node_id: &str, path: &str) {
    update(app, |t, now| {
        t.sending(id, node_id, path, now);
        // An already-finished tiny file now has its path too.
        t.get(id).is_some_and(TransferEntry::finished)
    });
}

// --- commands -----------------------------------------------------------------

/// Every transfer: those under way first, then the newest finished ones.
#[tauri::command]
#[specta::specta]
pub fn transfer_list(history: State<TransferHistory>) -> Vec<TransferEntry> {
    let mut list = history.inner.lock().map(|t| t.listed()).unwrap_or_default();
    for e in &mut list {
        e.exists = e.path.as_deref().is_some_and(|p| Path::new(p).is_file());
    }
    list
}

/// Forget the finished and failed transfers.
#[tauri::command]
#[specta::specta]
pub fn transfer_clear_finished(app: AppHandle, history: State<TransferHistory>) {
    if let Ok(mut t) = history.inner.lock() {
        t.clear_finished();
    }
    history.save_soon(&app);
}

/// Show a transfer's file in Finder / Explorer: where a received one was
/// saved, or the file a sent one came from. Only paths this list recorded.
#[tauri::command]
#[specta::specta]
pub fn transfer_reveal(
    id: String,
    app: AppHandle,
    history: State<TransferHistory>,
) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let path = history
        .inner
        .lock()
        .ok()
        .and_then(|t| t.get(&id).and_then(|e| e.path.clone()))
        .filter(|p| Path::new(p).exists())
        .ok_or_else(|| CoreError::InvalidInput("that file isn't there any more".into()))?;
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| CoreError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(t: &mut Transfers, i: u32) {
        let id = format!("t{i}");
        t.progress(&id, "p", "a.txt", 1, 10, false, false, f64::from(i));
        assert!(t.sent(&id, "p", "a.txt", f64::from(i)));
    }

    #[test]
    fn keeps_the_newest_finished_up_to_the_cap() {
        let mut t = Transfers::default();
        t.progress("live", "p", "big.mov", 5, 100, true, false, 0.0);
        for i in 1..=(MAX_FINISHED as u32 + 20) {
            finish(&mut t, i);
        }
        let list = t.listed();
        // The one under way is never trimmed and comes first.
        assert_eq!(list[0].id, "live");
        assert_eq!(list.len(), MAX_FINISHED + 1);
        assert_eq!(list[1].id, format!("t{}", MAX_FINISHED + 20));
        assert_eq!(list.last().unwrap().id, "t21");
        assert_eq!(t.saved().len(), MAX_FINISHED);
    }

    #[test]
    fn events_update_one_entry_by_id() {
        let mut t = Transfers::default();
        t.sending("x", "p", "/tmp/dir/photo.jpg", 1.0);
        t.progress("x", "p", "photo.jpg", 40, 100, false, true, 2.0);
        t.progress("x", "p", "photo.jpg", 70, 100, false, true, 3.0);
        assert_eq!(t.listed().len(), 1);
        let e = t.get("x").unwrap();
        assert_eq!(e.name, "photo.jpg");
        assert_eq!(e.path.as_deref(), Some("/tmp/dir/photo.jpg"));
        assert_eq!(e.resumed_from, Some(40.0));
        assert_eq!(e.done, 70.0);
        assert_eq!(e.started, 1.0);
        assert!(t.sent("x", "p", "photo.jpg", 4.0));
        // Done stays done: a late failure or progress changes nothing.
        assert!(!t.failed("x", "p", "photo.jpg", false, "cancelled", 5.0));
        t.progress("x", "p", "photo.jpg", 1, 100, false, false, 6.0);
        let e = t.get("x").unwrap();
        assert_eq!(e.state, TransferState::Done);
        assert_eq!(e.done, 100.0);
        assert_eq!(e.ended, Some(4.0));

        assert!(t.received("y", "q", "in.pdf", "/dl/in.pdf", 9, 7.0));
        assert!(t.failed("z", "q", "c.bin", true, "cancelled", 8.0));
        let ids: Vec<String> = t.listed().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["z", "y", "x"]);
        assert_eq!(t.get("y").unwrap().path.as_deref(), Some("/dl/in.pdf"));
        assert_eq!(t.get("z").unwrap().reason.as_deref(), Some("cancelled"));
    }

    #[test]
    fn clear_finished_keeps_those_under_way() {
        let mut t = Transfers::default();
        finish(&mut t, 1);
        t.progress("live", "p", "b", 1, 2, true, false, 2.0);
        t.clear_finished();
        let ids: Vec<String> = t.listed().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["live"]);
    }

    #[test]
    fn saved_and_loaded_again() {
        let dir =
            std::env::temp_dir().join(format!("md-transfers-{}-{}", std::process::id(), now_ms()));
        let path = dir.join(FILE);
        let h = TransferHistory::at(Some(path.clone()));
        {
            let mut t = h.inner.lock().unwrap();
            t.progress("a", "p", "a.txt", 3, 10, false, true, 1.0);
            t.sending("a", "p", "/src/a.txt", 1.0);
            t.sent("a", "p", "a.txt", 2.0);
            t.received("b", "q", "b.txt", "/dl/b.txt", 5, 3.0);
            t.progress("live", "p", "c", 1, 2, true, false, 4.0);
        }
        h.save_now();
        assert!(path.is_file());
        assert!(!path.with_extension("json.tmp").exists());
        let again = TransferHistory::at(Some(path));
        let list = again.inner.lock().unwrap().listed();
        // Only finished ones come back.
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(list[1].path.as_deref(), Some("/src/a.txt"));
        assert_eq!(list[1].resumed_from, Some(3.0));
        assert_eq!(list[1].done, 10.0);
        assert_eq!(list[0].state, TransferState::Done);
        let _ = std::fs::remove_dir_all(dir);
    }
}
