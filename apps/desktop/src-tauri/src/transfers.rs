// Transfers: every file going to or coming from your devices, in one list.
//
//  - transfers under way are kept in memory only (a restart resumes them,
//    and their events bring them back);
//  - finished and failed ones, the newest `MAX_FINISHED`, are also kept in
//    `<app config dir>/transfer-history.json`, written a few seconds after a
//    change (`SAVE_DELAY`) through a temporary file and a rename;
//  - one entry per transfer id: each event updates it in place;
//  - a file sent with `send_file` remembers where it came from, so it can be
//    shown in its folder or sent again;
//  - several files sent together (a folder, or files dropped at once) are a
//    batch: one row (`kind` set, its id the batch's) with the overall
//    progress, and its files as rows with `batch` set, shown inside it;
//  - speed is smoothed (`Speed`, an exponentially weighted average of the
//    bytes a second between progress events), and time left comes from it.
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
/// The most files of one finished batch kept.
pub const MAX_BATCH_FILES: usize = 200;
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
    /// It went (at least partly) over another route than the link:
    /// "wifi_direct".
    #[serde(default)]
    pub via: Option<String>,
    /// A link downloaded from the web ("Download on <device>"): here, asked
    /// by `node_id` (`incoming`), or on `node_id`, asked from here.
    #[serde(default)]
    pub link: Option<String>,
    /// The link's host, for showing.
    #[serde(default)]
    pub host: Option<String>,
    /// A download under way: "requested", "checking", "asking", "queued"
    /// or "downloading".
    #[serde(default)]
    pub stage: Option<String>,
    /// A file of a batch: the batch's id (its row has `kind`).
    #[serde(default)]
    pub batch: Option<String>,
    /// A batch: "folder" or "files"; with how many files, how many are
    /// in and how many failed.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub files_done: u32,
    #[serde(default)]
    pub files_failed: u32,
    /// The user paused it (here or on the other device).
    #[serde(default)]
    pub paused: bool,
    /// Bytes a second, smoothed (0 while not known or not moving).
    #[serde(default)]
    pub rate: f64,
    /// Seconds left at that speed.
    #[serde(default)]
    pub eta: Option<f64>,
    #[serde(skip)]
    #[specta(skip)]
    pub(crate) speed: Speed,
}

/// Smoothed speed: each progress event's bytes a second since the last
/// one, weighed `WEIGHT` against what came before.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Speed {
    /// Bytes a second, 0 while not known.
    pub rate: f64,
    done: f64,
    /// When `done` last moved (ms); 0 before the first sample.
    at: f64,
}

impl Speed {
    /// The newest sample's weight.
    const WEIGHT: f64 = 0.3;
    /// Samples closer together than this are folded into the next.
    const MIN_GAP_MS: f64 = 200.0;
    /// No bytes for this long: the speed isn't shown.
    pub const STALE_MS: f64 = 5000.0;

    /// `done` bytes at `now` (ms).
    pub fn update(&mut self, done: f64, now: f64) {
        if self.at <= 0.0 || done < self.done {
            // First sample, or it started over (a resume from less).
            *self = Speed {
                rate: if done < self.done { 0.0 } else { self.rate },
                done,
                at: now,
            };
            return;
        }
        let gap = now - self.at;
        if done == self.done || gap < Self::MIN_GAP_MS {
            return;
        }
        let inst = (done - self.done) * 1000.0 / gap;
        self.rate = if self.rate <= 0.0 {
            inst
        } else {
            self.rate * (1.0 - Self::WEIGHT) + inst * Self::WEIGHT
        };
        self.done = done;
        self.at = now;
    }

    /// The speed now (0 when nothing moved for `STALE_MS`).
    pub fn rate_at(&self, now: f64) -> f64 {
        if self.at <= 0.0 || now - self.at > Self::STALE_MS {
            0.0
        } else {
            self.rate
        }
    }

    /// Seconds left for `total` bytes at the speed now.
    pub fn eta(&self, total: f64, now: f64) -> Option<f64> {
        let r = self.rate_at(now);
        (r > 0.0 && total > self.done).then(|| (total - self.done) / r)
    }
}

impl TransferEntry {
    pub(crate) fn new(id: &str, node_id: &str, name: &str, incoming: bool, now: f64) -> Self {
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
            via: None,
            link: None,
            host: None,
            stage: None,
            batch: None,
            kind: None,
            count: 0,
            files_done: 0,
            files_failed: 0,
            paused: false,
            rate: 0.0,
            eta: None,
            speed: Speed::default(),
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
    /// Devices whose transfers go over a side route right now, and which
    /// ("wifi_direct", "hotspot").
    wifi_direct: std::collections::HashMap<String, String>,
}

impl Transfers {
    /// From saved entries: only finished ones count (a transfer under way
    /// comes back through its events).
    pub fn from_saved(mut list: Vec<TransferEntry>) -> Self {
        list.retain(TransferEntry::finished);
        let mut t = Transfers {
            list,
            ..Transfers::default()
        };
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
        batch: Option<&str>,
        now: f64,
    ) {
        let via = self.wifi_direct.get(node_id).cloned();
        let e = self.entry(id, node_id, name, incoming, now);
        if e.finished() {
            return;
        }
        if via.is_some() {
            e.via = via;
        }
        if let Some(b) = batch {
            e.batch = Some(b.to_string());
        }
        e.done = done as f64;
        e.total = total as f64;
        e.speed.update(e.done, now);
        if resumed && e.resumed_from.is_none() {
            e.resumed_from = Some(done as f64);
        }
    }

    /// A batch moved on or changed state: its row.
    pub fn batch(&mut self, item: &mdcore::model::BatchItem, now: f64) -> bool {
        let e = self.entry(&item.id, &item.node_id, &item.name, item.incoming, now);
        e.kind = Some(item.kind.clone());
        e.count = item.count.min(u64::from(u32::MAX)) as u32;
        e.files_done = item.files_done.min(u64::from(u32::MAX)) as u32;
        e.files_failed = item.files_failed.min(u64::from(u32::MAX)) as u32;
        e.total = item.total as f64;
        e.done = item.done as f64;
        e.speed.update(e.done, now);
        if item.path.is_some() {
            e.path = item.path.clone();
        }
        e.paused = item.state == "paused";
        let state = match item.state.as_str() {
            "done" => TransferState::Done,
            "failed" | "cancelled" => TransferState::Failed,
            _ => TransferState::Active,
        };
        if state == e.state {
            return false;
        }
        // Sent again (retry): under way once more.
        e.state = state;
        e.reason = item
            .error
            .clone()
            .or_else(|| (item.state == "cancelled").then(|| "cancelled".to_string()));
        e.ended = (state != TransferState::Active).then_some(now);
        self.trim();
        true
    }

    /// The user paused or resumed transfer `id`.
    pub fn paused(&mut self, id: &str, paused: bool) {
        if let Some(e) = self.list.iter_mut().find(|e| e.id == id) {
            e.paused = paused;
        }
    }

    /// Transfer `id` is one file of a batch.
    pub fn in_batch(&self, id: &str) -> bool {
        self.get(id).is_some_and(|e| e.batch.is_some())
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

    /// Transfers with `node_id` go over a side route now (`active`, `via`
    /// "wifi_direct" or "hotspot"), or no longer do. Those under way say
    /// so from now on (and keep saying it once finished).
    pub fn wifi_direct(&mut self, node_id: &str, active: bool, via: &str) {
        if !active {
            self.wifi_direct.remove(node_id);
            return;
        }
        self.wifi_direct
            .insert(node_id.to_string(), via.to_string());
        for e in self
            .list
            .iter_mut()
            .filter(|e| e.node_id == node_id && !e.finished())
        {
            e.via = Some(via.to_string());
        }
    }

    /// A link downloaded here or on `item.node_id` changed state: the row
    /// learns the link, the file's name and size, and the stage. Its end
    /// comes with the transfer events.
    pub fn remote_download(&mut self, item: &mdcore::model::RemoteDownloadItem, now: f64) {
        let e = self.entry(&item.id, &item.node_id, &item.name, item.here, now);
        e.link = Some(item.url.clone());
        e.host = Some(item.host.clone()).filter(|h| !h.is_empty());
        if !item.name.is_empty() {
            e.name = item.name.clone();
        }
        if item.size > 0 && !e.finished() {
            e.total = item.size as f64;
        }
        e.stage = (!item.finished()).then(|| item.state.clone());
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
            .filter(|e| e.finished() && e.batch.is_none())
            .map(|e| (e.ended.unwrap_or(e.started), e.id.clone()))
            .collect();
        if finished.len() > MAX_FINISHED {
            finished.sort_by(|a, b| b.0.total_cmp(&a.0));
            let drop: std::collections::HashSet<String> =
                finished.drain(MAX_FINISHED..).map(|(_, id)| id).collect();
            self.list.retain(|e| {
                !drop.contains(&e.id) && !e.batch.as_ref().is_some_and(|b| drop.contains(b))
            });
        }
        // A finished batch keeps its failed files and the newest of the
        // rest, up to `MAX_BATCH_FILES`.
        let mut per_batch: std::collections::HashMap<String, Vec<(bool, f64, String)>> =
            std::collections::HashMap::new();
        for e in self.list.iter().filter(|e| e.finished()) {
            if let Some(b) = &e.batch {
                per_batch.entry(b.clone()).or_default().push((
                    e.state != TransferState::Failed,
                    -e.ended.unwrap_or(e.started),
                    e.id.clone(),
                ));
            }
        }
        let mut drop = std::collections::HashSet::new();
        for (_, mut files) in per_batch {
            if files.len() > MAX_BATCH_FILES {
                files.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
                drop.extend(files.drain(MAX_BATCH_FILES..).map(|(_, _, id)| id));
            }
        }
        if !drop.is_empty() {
            self.list.retain(|e| !drop.contains(&e.id));
        }
    }

    /// What the view shows: those under way (newest first), then the
    /// finished ones (latest ended first).
    pub fn listed(&self) -> Vec<TransferEntry> {
        self.listed_at(now_ms())
    }

    /// `listed`, with speed and time left as of `now`.
    pub fn listed_at(&self, now: f64) -> Vec<TransferEntry> {
        let mut active: Vec<TransferEntry> = self
            .list
            .iter()
            .filter(|e| !e.finished())
            .cloned()
            .map(|mut e| {
                if e.paused {
                    (e.rate, e.eta) = (0.0, None);
                } else {
                    e.rate = e.speed.rate_at(now);
                    e.eta = e.speed.eta(e.total, now);
                }
                e
            })
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

    /// A copy of the list as the view shows it (for search.rs).
    pub fn snapshot(&self) -> Vec<TransferEntry> {
        self.inner.lock().map(|t| t.listed()).unwrap_or_default()
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
    batch: Option<&str>,
) {
    update(app, |t, now| {
        t.progress(
            id, node_id, name, done, total, incoming, resumed, batch, now,
        );
        false
    });
}

/// A batch moved on or changed state.
pub fn on_batch(app: &AppHandle, item: &mdcore::model::BatchItem) {
    update(app, |t, now| t.batch(item, now));
}

pub fn on_paused(app: &AppHandle, id: &str, paused: bool) {
    update(app, |t, _| {
        t.paused(id, paused);
        false
    });
}

/// Where transfer `id` is ("active", "done", "failed"), if it's listed.
pub fn get_state(app: &AppHandle, id: &str) -> Option<String> {
    let h = app.try_state::<TransferHistory>()?;
    let t = h.inner.lock().ok()?;
    let e = t.get(id)?;
    serde_json::to_value(e.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

/// Transfer `id` is one file of a batch (its notice comes with the batch).
pub fn in_batch(app: &AppHandle, id: &str) -> bool {
    app.try_state::<TransferHistory>()
        .and_then(|h| h.inner.lock().ok().map(|t| t.in_batch(id)))
        .unwrap_or(false)
}

pub fn on_wifi_direct(app: &AppHandle, node_id: &str, active: bool, via: &str) {
    update(app, |t, _| {
        t.wifi_direct(node_id, active, via);
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

/// A link downloaded here or on another device changed state.
pub fn on_remote_download(app: &AppHandle, item: &mdcore::model::RemoteDownloadItem) {
    update(app, |t, now| {
        t.remote_download(item, now);
        false
    });
}

/// Transfer `id` is a link downloaded from the web.
pub fn is_link(app: &AppHandle, id: &str) -> bool {
    app.try_state::<TransferHistory>()
        .and_then(|h| {
            h.inner
                .lock()
                .ok()
                .map(|t| t.get(id).is_some_and(|e| e.link.is_some()))
        })
        .unwrap_or(false)
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
        e.exists = e.path.as_deref().is_some_and(|p| Path::new(p).exists());
    }
    list
}

/// Pause (`paused`) or resume a transfer or a batch, either way.
#[tauri::command]
#[specta::specta]
pub fn transfer_pause(
    id: String,
    paused: bool,
    app: AppHandle,
    engine: State<crate::Eng>,
) -> Result<(), CoreError> {
    engine.pause_file(&id, paused)?;
    // A batch says so with its next event; a file here and now.
    on_paused(&app, &id, paused);
    Ok(())
}

/// Send a failed transfer again (a batch's files that didn't go, a file
/// sent from here, a link asked of a device). Returns the id to follow.
#[tauri::command]
#[specta::specta]
pub fn transfer_retry(
    id: String,
    app: AppHandle,
    engine: State<crate::Eng>,
    history: State<TransferHistory>,
) -> Result<String, CoreError> {
    let entry = history.inner.lock().ok().and_then(|t| t.get(&id).cloned());
    match engine.retry_file(&id) {
        Ok(new) => Ok(new),
        // A file sent before this run (the core forgot it): from its path.
        Err(e) => match entry {
            Some(x) if !x.incoming && x.kind.is_none() && x.link.is_none() => {
                let path = x.path.filter(|p| Path::new(p).is_file()).ok_or(e)?;
                let new = engine.send_file(&x.node_id, &path)?;
                on_sending(&app, &new, &x.node_id, &path);
                Ok(new)
            }
            _ => Err(e),
        },
    }
}

/// Send several files and folders to a device as one batch. Returns the
/// batch's id and how many files it holds.
#[tauri::command]
#[specta::specta]
pub fn send_batch(
    node_id: String,
    paths: Vec<String>,
    engine: State<crate::Eng>,
) -> Result<BatchStarted, CoreError> {
    let b = engine.send_batch(&node_id, &paths)?;
    Ok(BatchStarted {
        id: b.id,
        count: b.count.min(u64::from(u32::MAX)) as u32,
    })
}

/// What `send_batch` started.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct BatchStarted {
    pub id: String,
    pub count: u32,
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
        t.progress(&id, "p", "a.txt", 1, 10, false, false, None, f64::from(i));
        assert!(t.sent(&id, "p", "a.txt", f64::from(i)));
    }

    #[test]
    fn keeps_the_newest_finished_up_to_the_cap() {
        let mut t = Transfers::default();
        t.progress("live", "p", "big.mov", 5, 100, true, false, None, 0.0);
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
    fn transfers_over_wifi_direct_say_so() {
        let mut t = Transfers::default();
        t.progress("going", "phone", "big.mov", 5, 100, false, false, None, 0.0);
        t.progress("other", "laptop", "x.zip", 5, 100, false, false, None, 0.0);
        finish(&mut t, 1);
        t.wifi_direct("phone", true, "wifi_direct");
        // One that starts while it's up says so too.
        t.progress("next", "phone", "b.mov", 1, 100, true, false, None, 1.0);
        let via = |t: &Transfers, id: &str| t.get(id).unwrap().via.clone();
        assert_eq!(via(&t, "going").as_deref(), Some("wifi_direct"));
        assert_eq!(via(&t, "next").as_deref(), Some("wifi_direct"));
        assert_eq!(via(&t, "other"), None, "another device's transfer");
        assert_eq!(via(&t, "t1"), None, "one that ended before");
        // Once it's down: new ones don't, the ones that used it still do.
        t.wifi_direct("phone", false, "wifi_direct");
        // Over the phone's hotspot, it says that.
        t.wifi_direct("mac-phone", true, "hotspot");
        t.progress("hs", "mac-phone", "d.mov", 1, 100, true, false, None, 2.0);
        assert_eq!(via(&t, "hs").as_deref(), Some("hotspot"));
        t.progress("later", "phone", "c.mov", 1, 100, true, false, None, 2.0);
        assert_eq!(via(&t, "later"), None);
        assert!(t.sent("going", "phone", "big.mov", 3.0));
        assert_eq!(via(&t, "going").as_deref(), Some("wifi_direct"));
        // Kept in the saved history; older files without it still read.
        let saved = serde_json::to_string(&t.saved()).unwrap();
        assert!(saved.contains("wifi_direct"), "{saved}");
        let old: TransferEntry = serde_json::from_str(
            r#"{"id":"o","node_id":"p","name":"a","incoming":false,"state":"done",
                "done":1,"total":1,"started":0}"#,
        )
        .unwrap();
        assert_eq!(old.via, None);
    }

    #[test]
    fn events_update_one_entry_by_id() {
        let mut t = Transfers::default();
        t.sending("x", "p", "/tmp/dir/photo.jpg", 1.0);
        t.progress("x", "p", "photo.jpg", 40, 100, false, true, None, 2.0);
        t.progress("x", "p", "photo.jpg", 70, 100, false, true, None, 3.0);
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
        t.progress("x", "p", "photo.jpg", 1, 100, false, false, None, 6.0);
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
    fn speed_is_smoothed_and_gives_time_left() {
        let mut s = Speed::default();
        s.update(0.0, 1000.0);
        assert_eq!(s.rate, 0.0);
        s.update(1000.0, 2000.0); // 1000 B/s
        assert_eq!(s.rate, 1000.0);
        s.update(4000.0, 3000.0); // 3000 B/s, weighed 0.3
        assert!((s.rate - 1600.0).abs() < 1e-9, "{}", s.rate);
        // Too soon after the last: folded into the next.
        s.update(4100.0, 3050.0);
        assert!((s.rate - 1600.0).abs() < 1e-9);
        // 6000 bytes left at 1600 B/s.
        let eta = s.eta(10_000.0, 3100.0).unwrap();
        assert!((eta - 3.75).abs() < 1e-9, "{eta}");
        // Nothing moved for a while: no speed, no time left.
        assert_eq!(s.rate_at(3000.0 + Speed::STALE_MS + 1.0), 0.0);
        assert_eq!(s.eta(10_000.0, 3000.0 + Speed::STALE_MS + 1.0), None);
        // Started over from less: the old speed is dropped.
        s.update(100.0, 9000.0);
        assert_eq!(s.rate, 0.0);
        // Done: nothing left.
        let mut d = Speed::default();
        d.update(0.0, 0.0);
        d.update(500.0, 1000.0);
        assert_eq!(d.eta(500.0, 1000.0), None);
    }

    #[test]
    fn the_list_says_speed_and_time_left() {
        let mut t = Transfers::default();
        t.progress("x", "p", "a", 0, 10_000, false, false, None, 1000.0);
        t.progress("x", "p", "a", 2000, 10_000, false, false, None, 2000.0);
        let e = &t.listed_at(2100.0)[0];
        assert_eq!(e.rate, 2000.0);
        assert_eq!(e.eta, Some(4.0));
        t.paused("x", true);
        let e = &t.listed_at(2100.0)[0];
        assert!(e.paused);
        assert_eq!((e.rate, e.eta), (0.0, None));
    }

    fn batch_item(state: &str, done: u64) -> mdcore::model::BatchItem {
        mdcore::model::BatchItem {
            id: "b1".into(),
            node_id: "p".into(),
            incoming: true,
            name: "Trip".into(),
            kind: "folder".into(),
            count: 3,
            files_done: 1,
            files_failed: 0,
            done,
            total: 300,
            state: state.into(),
            error: None,
            path: Some("/dl/Trip".into()),
            started_at: 0,
        }
    }

    #[test]
    fn a_batch_is_one_row_with_its_files() {
        let mut t = Transfers::default();
        assert!(!t.batch(&batch_item("active", 0), 1.0));
        t.progress("f1", "p", "a.jpg", 50, 100, true, false, Some("b1"), 2.0);
        assert!(t.in_batch("f1"));
        assert!(!t.in_batch("b1"));
        let b = t.get("b1").unwrap();
        assert_eq!(
            (b.kind.as_deref(), b.count, b.total),
            (Some("folder"), 3, 300.0)
        );
        assert!(!t.batch(&batch_item("paused", 100), 3.0));
        assert!(t.get("b1").unwrap().paused);
        assert!(t.batch(&batch_item("done", 300), 4.0));
        let b = t.get("b1").unwrap();
        assert_eq!(b.state, TransferState::Done);
        assert_eq!(b.path.as_deref(), Some("/dl/Trip"));
        // Sent again: under way once more.
        let mut again = batch_item("failed", 200);
        again.error = Some("the device wasn't reachable".into());
        assert!(t.batch(&again, 5.0));
        assert_eq!(
            t.get("b1").unwrap().reason.as_deref(),
            Some("the device wasn't reachable")
        );
        assert!(t.batch(&batch_item("active", 200), 6.0));
        assert_eq!(t.get("b1").unwrap().state, TransferState::Active);
    }

    #[test]
    fn a_big_batch_does_not_push_out_other_transfers() {
        let mut t = Transfers::default();
        finish(&mut t, 1);
        let mut item = batch_item("active", 0);
        item.count = 500;
        t.batch(&item, 2.0);
        for i in 0..500 {
            let id = format!("f{i}");
            t.progress(&id, "p", "x", 1, 1, true, false, Some("b1"), 3.0);
            t.received(&id, "p", "x", "/dl/x", 1, 3.0 + f64::from(i));
        }
        item.state = "done".into();
        t.batch(&item, 600.0);
        assert!(t.get("t1").is_some(), "the other transfer is kept");
        let files = t.listed().iter().filter(|e| e.batch.is_some()).count();
        assert_eq!(files, MAX_BATCH_FILES);
    }

    #[test]
    fn clear_finished_keeps_those_under_way() {
        let mut t = Transfers::default();
        finish(&mut t, 1);
        t.progress("live", "p", "b", 1, 2, true, false, None, 2.0);
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
            t.progress("a", "p", "a.txt", 3, 10, false, true, None, 1.0);
            t.sending("a", "p", "/src/a.txt", 1.0);
            t.sent("a", "p", "a.txt", 2.0);
            t.received("b", "q", "b.txt", "/dl/b.txt", 5, 3.0);
            t.progress("live", "p", "c", 1, 2, true, false, None, 4.0);
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
