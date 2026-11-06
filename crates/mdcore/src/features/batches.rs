//! Several files sent together, shown and handled as one: a folder with
//! everything in it (`features::folders`), or files picked at once.
//!
//! Each file still goes as its own transfer (`features::files`, so it
//! resumes, is checked with SHA-256 and is asked for again if it arrived
//! damaged); its offer says which batch it belongs to:
//!
//!   file_offer { ..., folder?: "Trip/day 1",
//!                batch: { id, name, kind: "folder"|"files", count, bytes } }
//!   batch_end  { from, id, state, sent, failed }          sender → receiver
//!   file_pause { from, id, paused }                       either way
//!
//! (`file_pause`'s id is a file's or a batch's.) Both sides keep the batch
//! (`BatchItem`, the `TransferBatch` event about four times a second while
//! it moves) and its files' events say `batch`, so the hosts show one row
//! with the overall progress and the files inside it.
//!
//! - At most `MAX_PARALLEL` files go at once to a device over a direct link
//!   or a side lane (QUIC carries them side by side; each keeps its own
//!   window, so a message still gets between their chunks); one at a time
//!   over the relay or Bluetooth, which other traffic shares.
//! - A file that couldn't be read or kept arriving damaged is skipped; any
//!   other failure (cancelled on either side, the device unreachable, its
//!   storage full) stops the batch. Its files that didn't go can be sent
//!   again (`retry`), under the same batch.
//! - The sender keeps its batches in the platform's store (what's left to
//!   send, and the files going now, which `features::files` keeps and
//!   offers again), so a restart carries on; so does the receiver, so its
//!   row comes back.
//! - Pausing a batch pauses its files and starts no new one.
//!
//! Only devices that said `xfer: 1` (status and `addr_update`) are sent
//! `batch_end` or `file_pause`; an older receiver ignores `batch` and just
//! saves each file (into its folder), an older sender never sends it.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{BatchItem, BatchStart};
use crate::platform::Platform;

use super::files::{safe_folder, safe_name, start_with_extra};
use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag.
pub const FLAG: &str = "xfer";
const VERSION: u64 = 1;
const CAPS_PREFIX: &str = "xfer_caps:";
const SAVED_KEY: &str = "files_batches";
/// Files of batches going to one device at once over a direct link.
pub const MAX_PARALLEL: usize = 3;
/// Most files in one batch.
pub const MAX_FILES: usize = 10_000;
/// Ended batches kept (to show, and to send again what failed).
const KEEP_ENDED: usize = 20;
const EVENT_EVERY: Duration = Duration::from_millis(250);

/// One file of a batch being sent.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Entry {
    path: PathBuf,
    /// Its folder on the receiver (`Trip/day 1`), if it's in one.
    #[serde(default)]
    folder: Option<String>,
    size: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Out {
    item: BatchItem,
    queue: VecDeque<Entry>,
    /// File id -> its entry, for the files going now.
    going: HashMap<String, Entry>,
    /// Bytes of the files that arrived.
    sent_bytes: u64,
    /// Files that didn't go, with why (sent again by `retry`).
    failed: Vec<(Entry, String)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct In {
    item: BatchItem,
    /// File id -> (size, bytes in, ended: ok?).
    files: HashMap<String, (u64, u64, Option<bool>)>,
    /// The sender said it's over (`batch_end`).
    #[serde(default)]
    ended_by_sender: Option<String>,
}

/// Batches both ways.
#[derive(Default)]
pub struct Batches {
    out: Mutex<HashMap<String, Out>>,
    inc: Mutex<HashMap<String, In>>,
    /// File id -> batch id (both ways).
    by_file: Mutex<HashMap<String, String>>,
    /// Batch id -> when its event last went out.
    shown: Mutex<HashMap<String, Instant>>,
    restored: std::sync::atomic::AtomicBool,
    caps: Mutex<HashMap<String, u64>>,
    saved: Mutex<String>,
    /// Files the last run had going, to check once `features::files` took
    /// back its transfers.
    check_after_restore: Mutex<Vec<String>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &Batches {
    &ctx.files.batches
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

fn now_ms() -> u64 {
    crate::transport::media::now_ms()
}

// --- who knows batches ------------------------------------------------------

pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.transfer_batches {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`) whether it knows batches. Kept across restarts.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let level = flag.as_u64().unwrap_or(0);
    if node.is_empty() || caps(ctx, node) == level {
        return;
    }
    st(ctx).caps.lock().insert(node.to_string(), level);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), &level.to_string());
}

fn caps<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    if let Some(level) = st(ctx).caps.lock().get(node) {
        return *level;
    }
    let level = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    st(ctx).caps.lock().insert(node.to_string(), level);
    level
}

/// `node` knows `file_pause` and `batch_end` (and this end does).
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.transfer_batches && caps(ctx, node) >= 1
}

// --- keeping them -----------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Saved {
    #[serde(default)]
    out: Vec<Out>,
    #[serde(default)]
    inc: Vec<In>,
}

fn save<P: Platform>(ctx: &NetCtx<P>) {
    restore(ctx);
    let b = st(ctx);
    let json = {
        let out = b.out.lock();
        let inc = b.inc.lock();
        let mut o: Vec<&Out> = out.values().collect();
        o.sort_by(|a, b| a.item.id.cmp(&b.item.id));
        let mut i: Vec<&In> = inc.values().collect();
        i.sort_by(|a, b| a.item.id.cmp(&b.item.id));
        serde_json::to_string(&json!({ "out": o, "inc": i })).unwrap_or_default()
    };
    let mut last = b.saved.lock();
    if *last != json {
        ctx.platform.kv_set(SAVED_KEY, &json);
        *last = json;
    }
}

/// Once per run: take back the batches of the last run (before
/// `features::files` takes back its transfers, which may end some files).
pub(crate) fn restore<P: Platform>(ctx: &NetCtx<P>) {
    use std::sync::atomic::Ordering;
    let b = st(ctx);
    if b.restored.swap(true, Ordering::Relaxed) {
        return;
    }
    let saved: Saved = ctx
        .platform
        .kv_get(SAVED_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut items = Vec::new();
    let mut links = Vec::new();
    {
        let mut out = b.out.lock();
        for o in saved.out {
            for fid in o.going.keys() {
                links.push((fid.clone(), o.item.id.clone()));
            }
            items.push(o.item.clone());
            out.insert(o.item.id.clone(), o);
        }
    }
    {
        let mut inc = b.inc.lock();
        for i in saved.inc {
            for fid in i.files.keys() {
                links.push((fid.clone(), i.item.id.clone()));
            }
            items.push(i.item.clone());
            inc.insert(i.item.id.clone(), i);
        }
    }
    {
        let out = b.out.lock();
        b.check_after_restore.lock().extend(
            links
                .iter()
                .filter(|(_, bid)| out.contains_key(bid))
                .map(|(fid, _)| fid.clone()),
        );
    }
    b.by_file.lock().extend(links);
    for item in items.into_iter().filter(|i| !i.finished()) {
        ctx.sink.emit(CoreEvent::TransferBatch { item });
    }
}

/// After `features::files` took back its transfers: a file that was
/// going but isn't one of them (the app stopped before it was offered)
/// goes back in the queue.
pub(crate) fn restored_files<P: Platform>(ctx: &NetCtx<P>) {
    // Only the files the last run had going (this run's may still be
    // hashing, not yet transfers).
    let check: Vec<String> = std::mem::take(&mut *st(ctx).check_after_restore.lock());
    let lost: Vec<String> = check
        .into_iter()
        .filter(|fid| super::files::outgoing_progress(ctx, fid).is_none())
        .collect();
    if lost.is_empty() {
        return;
    }
    {
        let mut out = st(ctx).out.lock();
        for o in out.values_mut() {
            for fid in &lost {
                if let Some(e) = o.going.remove(fid) {
                    o.queue.push_front(e);
                }
            }
        }
    }
    {
        let mut by_file = st(ctx).by_file.lock();
        for fid in &lost {
            by_file.remove(fid);
        }
    }
    save(ctx);
}

// --- the item -----------------------------------------------------------------

fn out_item<P: Platform>(ctx: &NetCtx<P>, o: &Out) -> BatchItem {
    let mut item = o.item.clone();
    let going: u64 = o
        .going
        .keys()
        .filter_map(|fid| super::files::outgoing_progress(ctx, fid))
        .map(|(done, _)| done)
        .sum();
    item.done = (o.sent_bytes + going).min(item.total);
    item.files_failed = o.failed.len() as u64;
    item
}

fn in_item(i: &In) -> BatchItem {
    let mut item = i.item.clone();
    item.done = i
        .files
        .values()
        .map(|(_, d, _)| *d)
        .sum::<u64>()
        .min(item.total);
    item.files_done = i.files.values().filter(|f| f.2 == Some(true)).count() as u64;
    item.files_failed = i.files.values().filter(|f| f.2 == Some(false)).count() as u64;
    item
}

/// The batch's item as it is now, either way.
fn item_of<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<BatchItem> {
    if let Some(o) = st(ctx).out.lock().get(id) {
        return Some(out_item(ctx, o));
    }
    st(ctx).inc.lock().get(id).map(in_item)
}

fn emit<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    if let Some(item) = item_of(ctx, id) {
        st(ctx).shown.lock().insert(id.to_string(), Instant::now());
        ctx.sink.emit(CoreEvent::TransferBatch { item });
    }
}

/// Every batch this device knows (going, and the last ones that ended),
/// the newest first.
pub fn list<P: Platform>(ctx: &NetCtx<P>) -> Vec<BatchItem> {
    restore(ctx);
    let mut v: Vec<BatchItem> = {
        let out = st(ctx).out.lock();
        out.values().map(|o| out_item(ctx, o)).collect()
    };
    v.extend(st(ctx).inc.lock().values().map(in_item));
    v.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(a.id.cmp(&b.id)));
    v
}

/// The batch file `id` belongs to, if any.
pub fn batch_of<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<String> {
    st(ctx).by_file.lock().get(id).cloned()
}

/// `id` is a batch this device knows.
pub fn is_batch<P: Platform>(ctx: &NetCtx<P>, id: &str) -> bool {
    restore(ctx);
    st(ctx).out.lock().contains_key(id) || st(ctx).inc.lock().contains_key(id)
}

/// Forget the oldest ended batches past `KEEP_ENDED`.
fn trim<P: Platform>(ctx: &NetCtx<P>) {
    let b = st(ctx);
    let mut ended: Vec<(u64, String, bool)> = b
        .out
        .lock()
        .values()
        .filter(|o| o.item.finished())
        .map(|o| (o.item.started_at, o.item.id.clone(), true))
        .collect();
    ended.extend(
        b.inc
            .lock()
            .values()
            .filter(|i| i.item.finished())
            .map(|i| (i.item.started_at, i.item.id.clone(), false)),
    );
    if ended.len() <= KEEP_ENDED {
        return;
    }
    ended.sort();
    let drop = ended.len() - KEEP_ENDED;
    for (_, id, out) in ended.into_iter().take(drop) {
        let files: Vec<String> = if out {
            b.out
                .lock()
                .remove(&id)
                .map(|o| o.going.into_keys().collect())
        } else {
            b.inc
                .lock()
                .remove(&id)
                .map(|i| i.files.into_keys().collect())
        }
        .unwrap_or_default();
        let mut by_file = b.by_file.lock();
        for f in files {
            by_file.remove(&f);
        }
        b.shown.lock().remove(&id);
    }
}

// --- sending --------------------------------------------------------------------

/// The files under `dir` (sorted by name, folder by folder), each with the
/// folder it goes into on the receiver (`rel`, then its subfolders).
/// Symlinks and hidden entries are skipped.
pub(crate) fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<(PathBuf, String)>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() > MAX_FILES {
            break; // too many anyway; `start` says so
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // `file_type` doesn't follow symlinks: a link is skipped whatever
        // it points at.
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_file() {
            out.push((e.path(), rel.to_string()));
        } else if kind.is_dir() && depth < 32 {
            let sub = format!("{rel}/{}", safe_name(&name));
            // A subfolder we may not read is left out, not the whole send.
            let _ = walk(&e.path(), &sub, depth + 1, out);
        }
    }
    Ok(())
}

/// Send `paths` (files and folders) to `node` as one batch.
pub fn start<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    paths: &[PathBuf],
) -> crate::Result<BatchStart> {
    restore(ctx);
    let mut entries: Vec<Entry> = Vec::new();
    let mut folders = 0;
    let mut first_name = String::new();
    for p in paths {
        let meta = std::fs::metadata(p).map_err(|e| format!("can't read {}: {e}", p.display()))?;
        if meta.is_dir() {
            // The real path, so `.` or a trailing `/` still has a name.
            let root =
                std::fs::canonicalize(p).map_err(|e| format!("can't read the folder: {e}"))?;
            let top = safe_name(&root.file_name().unwrap_or_default().to_string_lossy());
            if first_name.is_empty() {
                first_name = top.clone();
            }
            folders += 1;
            let mut found = Vec::new();
            walk(&root, &top, 0, &mut found).map_err(|e| format!("can't read the folder: {e}"))?;
            for (path, rel) in found {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                entries.push(Entry {
                    path,
                    folder: Some(rel),
                    size,
                });
            }
        } else if meta.is_file() {
            if first_name.is_empty() {
                first_name = safe_name(&p.file_name().unwrap_or_default().to_string_lossy());
            }
            entries.push(Entry {
                path: p.clone(),
                folder: None,
                size: meta.len(),
            });
        }
        if entries.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} files at once").into());
        }
    }
    if entries.is_empty() {
        return Err(if folders > 0 {
            "the folder has no files to send"
        } else {
            "there are no files to send"
        }
        .to_string()
        .into());
    }
    let kind = if folders == 1 && paths.len() == 1 {
        "folder"
    } else {
        "files"
    };
    let id = format!("b{}", crate::transport::outbox::new_id());
    let count = entries.len() as u64;
    let item = BatchItem {
        id: id.clone(),
        node_id: node.to_string(),
        incoming: false,
        name: first_name,
        kind: kind.into(),
        count,
        total: entries.iter().map(|e| e.size).sum(),
        state: "active".into(),
        started_at: now_ms(),
        ..BatchItem::default()
    };
    st(ctx).out.lock().insert(
        id.clone(),
        Out {
            item,
            queue: entries.into(),
            going: HashMap::new(),
            sent_bytes: 0,
            failed: Vec::new(),
        },
    );
    save(ctx);
    emit(ctx, &id);
    pump(ctx);
    Ok(BatchStart { id, count })
}

/// How many files may go at once to `node` now.
fn parallel<P: Platform>(ctx: &NetCtx<P>, node: &str) -> usize {
    if crate::transport::is_direct(ctx, node) || ctx.quic.has_lane(node) {
        MAX_PARALLEL
    } else {
        1
    }
}

/// Start files of every batch, up to `parallel` per device; batches that
/// have nothing left end.
pub(crate) fn pump<P: Platform>(ctx: &NetCtx<P>) {
    loop {
        let next = {
            let mut out = st(ctx).out.lock();
            let mut going: HashMap<String, usize> = HashMap::new();
            for o in out.values() {
                *going.entry(o.item.node_id.clone()).or_default() += o.going.len();
            }
            let mut ids: Vec<String> = out
                .values()
                .filter(|o| o.item.state == "active" && !o.queue.is_empty())
                .map(|o| o.item.id.clone())
                .collect();
            ids.sort();
            let mut pick = None;
            for bid in ids {
                let Some(o) = out.get_mut(&bid) else { continue };
                let node = o.item.node_id.clone();
                if going.get(&node).copied().unwrap_or(0) >= parallel(ctx, &node) {
                    continue;
                }
                let Some(e) = o.queue.pop_front() else {
                    continue;
                };
                let fid = crate::transport::outbox::new_id();
                // Booked before it starts: it may end before `start` returns.
                o.going.insert(fid.clone(), e.clone());
                st(ctx).by_file.lock().insert(fid.clone(), bid.clone());
                let mut extra = json!({ "batch": {
                    "id": bid, "name": o.item.name, "kind": o.item.kind,
                    "count": o.item.count, "bytes": o.item.total,
                }});
                if let Some(f) = &e.folder {
                    extra["folder"] = f.clone().into();
                }
                pick = Some((node, e, fid, extra));
                break;
            }
            pick
        };
        let Some((node, e, fid, extra)) = next else {
            break;
        };
        if let Err(err) = start_with_extra(ctx, &node, &e.path, &fid, Some(extra)) {
            // Gone or changed since: skip it.
            outgoing_ended(ctx, &fid, Some(&format!("can't read: {err}")));
        }
    }
    save(ctx);
}

/// A failure that's about this one file: the rest still goes.
fn only_this_file(why: &str) -> bool {
    why.starts_with("can't read")
        || why == "the file couldn't be read"
        || why == "the file arrived damaged"
        || why == "the file changed since it was offered"
        || why == "the file changed or is gone"
}

/// Outgoing transfer `id` ended (`why` = the failure, if it failed): if it
/// belongs to a batch, start the next files, or stop the batch.
pub fn outgoing_ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>) {
    let Some(bid) = st(ctx).by_file.lock().remove(id) else {
        return;
    };
    let mut stop = Vec::new();
    let mut ended = None;
    {
        let mut out = st(ctx).out.lock();
        let Some(o) = out.get_mut(&bid) else { return };
        let Some(e) = o.going.remove(id) else { return };
        match why {
            None => {
                o.sent_bytes += e.size;
                o.item.files_done += 1;
            }
            Some(why) if only_this_file(why) => o.failed.push((e, why.to_string())),
            Some(why) => {
                // The others would fail the same way, or the user wants
                // it stopped: what's left can be sent again later.
                o.failed.push((e, why.to_string()));
                for (fid, e) in o.going.drain() {
                    stop.push(fid);
                    o.failed.push((e, why.to_string()));
                }
                let left: Vec<Entry> = o.queue.drain(..).collect();
                o.failed
                    .extend(left.into_iter().map(|e| (e, why.to_string())));
                o.item.state = if why.contains("cancelled") {
                    "cancelled".into()
                } else {
                    "failed".into()
                };
                o.item.error = Some(why.to_string());
            }
        }
        if o.going.is_empty() && o.queue.is_empty() {
            if !o.item.finished() {
                o.item.state = if o.failed.is_empty() {
                    "done".into()
                } else {
                    "failed".into()
                };
                if o.item.error.is_none() {
                    o.item.error = o.failed.first().map(|(_, w)| w.clone());
                }
            }
            ended = Some((
                o.item.node_id.clone(),
                o.item.state.clone(),
                o.item.files_done,
                o.failed.len(),
            ));
        }
    }
    {
        let mut by_file = st(ctx).by_file.lock();
        for fid in &stop {
            by_file.remove(fid);
        }
    }
    for fid in stop {
        super::files::cancel(ctx, &fid);
    }
    if let Some((node, state, sent, failed)) = ended {
        if supports(ctx, &node) {
            let msg = json!({
                "type": "batch_end", "from": me(ctx), "id": bid,
                "state": state, "sent": sent, "failed": failed,
            });
            crate::engine::send_reliably(ctx, &node, &msg.to_string());
        }
        trim(ctx);
    }
    save(ctx);
    emit(ctx, &bid);
    pump(ctx);
}

/// How many batches are still being sent (files going or still to start).
pub fn sending<P: Platform>(ctx: &NetCtx<P>) -> usize {
    st(ctx)
        .out
        .lock()
        .values()
        .filter(|o| !o.item.finished())
        .count()
}

// --- receiving ------------------------------------------------------------------

/// A file offer says it's of batch `batch` (`{id, name, kind, count,
/// bytes}`): note it. Returns the batch id if it's a good one.
pub(crate) fn incoming_offer<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    fid: &str,
    size: u64,
    batch: &Value,
    dir: &Path,
) -> Option<String> {
    if !ctx.config.transfer_batches {
        return None;
    }
    restore(ctx);
    let bid = batch.get("id")?.as_str()?.to_string();
    if !super::files::is_transfer_id(&bid) {
        return None;
    }
    let s = |k: &str| {
        batch
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let n = |k: &str| batch.get(k).and_then(Value::as_u64).unwrap_or(0);
    let kind = if s("kind") == "folder" {
        "folder"
    } else {
        "files"
    };
    let name = safe_name(&s("name"));
    let fresh = {
        let mut inc = st(ctx).inc.lock();
        let entry = inc.entry(bid.clone()).or_insert_with(|| In {
            item: BatchItem {
                id: bid.clone(),
                node_id: from.to_string(),
                incoming: true,
                name: name.clone(),
                kind: kind.into(),
                count: n("count").min(MAX_FILES as u64),
                total: n("bytes"),
                state: "active".into(),
                path: (kind == "folder")
                    .then(|| safe_folder(&name).map(|f| dir.join(f).to_string_lossy().into_owned()))
                    .flatten(),
                started_at: now_ms(),
                ..BatchItem::default()
            },
            files: HashMap::new(),
            ended_by_sender: None,
        });
        if entry.item.node_id != from {
            return None;
        }
        // Sent again (`retry`): it's going again.
        if entry.item.finished() {
            entry.item.state = "active".into();
            entry.item.error = None;
            entry.ended_by_sender = None;
        }
        let fresh = !entry.files.contains_key(fid);
        let f = entry
            .files
            .entry(fid.to_string())
            .or_insert((size, 0, None));
        if f.2 == Some(false) {
            *f = (size, 0, None);
        }
        fresh
    };
    st(ctx).by_file.lock().insert(fid.to_string(), bid.clone());
    if fresh {
        save(ctx);
        emit(ctx, &bid);
    }
    Some(bid)
}

/// A file of a batch moved on (`done` bytes here, or confirmed).
pub(crate) fn progress<P: Platform>(ctx: &NetCtx<P>, fid: &str, done: u64, incoming: bool) {
    let Some(bid) = batch_of(ctx, fid) else {
        return;
    };
    if incoming {
        if let Some(i) = st(ctx).inc.lock().get_mut(&bid) {
            if let Some(f) = i.files.get_mut(fid) {
                f.1 = done.min(f.0);
            }
        }
    }
    let due = st(ctx)
        .shown
        .lock()
        .get(&bid)
        .is_none_or(|at| at.elapsed() >= EVENT_EVERY);
    if due {
        emit(ctx, &bid);
    }
}

/// An incoming file of a batch ended.
pub(crate) fn incoming_ended<P: Platform>(ctx: &NetCtx<P>, fid: &str, ok: bool) {
    let Some(bid) = batch_of(ctx, fid) else {
        return;
    };
    {
        let mut inc = st(ctx).inc.lock();
        let Some(i) = inc.get_mut(&bid) else { return };
        let Some(f) = i.files.get_mut(fid) else {
            return;
        };
        f.2 = Some(ok);
        if ok {
            f.1 = f.0;
        }
        check_incoming_end(i);
    }
    save(ctx);
    emit(ctx, &bid);
    trim(ctx);
}

/// All its files ended (or the sender said it's over, and the files it
/// sent ended): the batch ends.
fn check_incoming_end(i: &mut In) {
    if i.item.finished() {
        return;
    }
    let ended = i.files.values().filter(|f| f.2.is_some()).count() as u64;
    let all_in = i.files.values().all(|f| f.2.is_some());
    let over = (ended >= i.item.count && all_in) || (i.ended_by_sender.is_some() && all_in);
    if !over {
        return;
    }
    let failed = i.files.values().any(|f| f.2 == Some(false))
        || ended < i.item.count && i.ended_by_sender.as_deref() != Some("done");
    i.item.state = match i.ended_by_sender.as_deref() {
        Some("cancelled") => "cancelled",
        _ if failed => "failed",
        _ => "done",
    }
    .into();
}

// --- pausing, cancelling, retrying -------------------------------------------------

fn tell_pause<P: Platform>(ctx: &NetCtx<P>, node: &str, id: &str, paused: bool) {
    let msg = json!({ "type": "file_pause", "from": me(ctx), "id": id, "paused": paused });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
}

/// Pause or resume batch `id` (`by_peer`: the other device asked).
/// `Ok(false)` if it isn't a batch.
pub(crate) fn pause<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    paused: bool,
    by_peer: Option<&str>,
) -> crate::Result<bool> {
    restore(ctx);
    let state = if paused { "paused" } else { "active" };
    let out_files = {
        let mut out = st(ctx).out.lock();
        match out.get_mut(id) {
            Some(o) if by_peer.is_some_and(|p| p != o.item.node_id) => return Ok(true),
            Some(o) if o.item.finished() => {
                return Err(CoreError::InvalidInput("that transfer has ended".into()))
            }
            Some(o) => {
                o.item.state = state.into();
                Some((
                    o.item.node_id.clone(),
                    o.going.keys().cloned().collect::<Vec<_>>(),
                ))
            }
            None => None,
        }
    };
    if let Some((node, files)) = out_files {
        for fid in files {
            let _ = super::files::set_paused(ctx, &fid, paused, Some(&node));
        }
        if by_peer.is_none() && supports(ctx, &node) {
            tell_pause(ctx, &node, id, paused);
        }
        save(ctx);
        emit(ctx, id);
        if !paused {
            pump(ctx);
        }
        return Ok(true);
    }
    let node = {
        let inc = st(ctx).inc.lock();
        match inc.get(id) {
            Some(i) if by_peer.is_some_and(|p| p != i.item.node_id) => return Ok(true),
            Some(i) if i.item.finished() => {
                return Err(CoreError::InvalidInput("that transfer has ended".into()))
            }
            Some(i) => i.item.node_id.clone(),
            None => return Ok(false),
        }
    };
    if by_peer.is_none() {
        if !supports(ctx, &node) {
            return Err(CoreError::InvalidInput(
                "that device can't pause transfers yet (update my-domain on it)".into(),
            ));
        }
        tell_pause(ctx, &node, id, paused);
    }
    if let Some(i) = st(ctx).inc.lock().get_mut(id) {
        i.item.state = state.into();
    }
    save(ctx);
    emit(ctx, id);
    Ok(true)
}

/// Stop batch `id`, either way, and tell the other device. False if it
/// isn't a batch this device knows (or it ended).
pub(crate) fn cancel<P: Platform>(ctx: &NetCtx<P>, id: &str, by_peer: Option<&str>) -> bool {
    restore(ctx);
    let out = {
        let mut out = st(ctx).out.lock();
        match out.get_mut(id) {
            Some(o) if by_peer.is_some_and(|p| p != o.item.node_id) => return false,
            Some(o) if !o.item.finished() => {
                let why = if by_peer.is_some() {
                    "cancelled on the other device"
                } else {
                    "cancelled"
                };
                let mut files = Vec::new();
                for (fid, e) in o.going.drain() {
                    files.push(fid);
                    o.failed.push((e, why.into()));
                }
                let left: Vec<Entry> = o.queue.drain(..).collect();
                o.failed
                    .extend(left.into_iter().map(|e| (e, why.to_string())));
                o.item.state = "cancelled".into();
                o.item.error = Some(why.into());
                Some((o.item.node_id.clone(), files))
            }
            Some(_) => return true,
            None => None,
        }
    };
    let (node, files) = match out {
        Some(x) => x,
        None => {
            let mut inc = st(ctx).inc.lock();
            match inc.get_mut(id) {
                Some(i) if by_peer.is_some_and(|p| p != i.item.node_id) => return false,
                Some(i) if !i.item.finished() => {
                    i.item.state = "cancelled".into();
                    i.item.error = Some(
                        if by_peer.is_some() {
                            "cancelled on the other device"
                        } else {
                            "cancelled"
                        }
                        .into(),
                    );
                    let files: Vec<String> = i
                        .files
                        .iter()
                        .filter(|(_, f)| f.2.is_none())
                        .map(|(fid, _)| fid.clone())
                        .collect();
                    for fid in &files {
                        if let Some(f) = i.files.get_mut(fid) {
                            f.2 = Some(false);
                        }
                    }
                    (i.item.node_id.clone(), files)
                }
                Some(_) => return true,
                None => return false,
            }
        }
    };
    {
        let mut by_file = st(ctx).by_file.lock();
        for fid in &files {
            by_file.remove(fid);
        }
    }
    for fid in files {
        super::files::cancel(ctx, &fid);
    }
    if by_peer.is_none() {
        // Older devices ignore a cancel for an id they don't know.
        let msg = json!({ "type": "file_cancel", "from": me(ctx), "id": id });
        crate::engine::send_reliably(ctx, &node, &msg.to_string());
    }
    save(ctx);
    emit(ctx, id);
    trim(ctx);
    true
}

/// Send the files of batch `id` that didn't go again. `Ok(false)` if it
/// isn't a batch sent from here.
pub(crate) fn retry<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<bool> {
    restore(ctx);
    {
        let mut out = st(ctx).out.lock();
        let Some(o) = out.get_mut(id) else {
            return Ok(st(ctx).inc.lock().contains_key(id)).and_then(|incoming| {
                if incoming {
                    Err(CoreError::InvalidInput(
                        "ask the other device to send it again".into(),
                    ))
                } else {
                    Ok(false)
                }
            });
        };
        if o.failed.is_empty() {
            return Err(CoreError::InvalidInput("nothing in it failed".into()));
        }
        let again: Vec<Entry> = o.failed.drain(..).map(|(e, _)| e).collect();
        o.queue.extend(again);
        o.item.state = "active".into();
        o.item.error = None;
    }
    save(ctx);
    emit(ctx, id);
    pump(ctx);
    Ok(true)
}

pub struct BatchesHandler;

impl<P: Platform> ControlHandler<P> for BatchesHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["file_pause", "batch_end"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        // Whoever sends these knows them.
        remember(ctx, &from, &json!(VERSION));
        let id = msg.str("id");
        match msg.kind() {
            "file_pause" => {
                let paused = msg.flag("paused");
                if !matches!(pause(ctx, &id, paused, Some(&from)), Ok(true)) {
                    let _ = super::files::set_paused(ctx, &id, paused, Some(&from));
                }
            }
            "batch_end" => {
                {
                    let mut inc = st(ctx).inc.lock();
                    let Some(i) = inc.get_mut(&id) else { return };
                    if i.item.node_id != from {
                        return;
                    }
                    let state = msg.str("state");
                    i.ended_by_sender = Some(state);
                    check_incoming_end(i);
                }
                save(ctx);
                emit(ctx, &id);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inc(count: u64, files: &[Option<bool>]) -> In {
        In {
            item: BatchItem {
                count,
                state: "active".into(),
                ..BatchItem::default()
            },
            files: files
                .iter()
                .enumerate()
                .map(|(n, e)| (n.to_string(), (10, 0, *e)))
                .collect(),
            ended_by_sender: None,
        }
    }

    #[test]
    fn an_incoming_batch_ends_when_all_its_files_did() {
        let mut i = inc(3, &[Some(true), Some(true)]);
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "active", "one still to come");
        i.files.insert("2".into(), (10, 10, Some(true)));
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "done");

        let mut i = inc(2, &[Some(true), Some(false)]);
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "failed");

        // The sender skipped one it couldn't read and said it's over.
        let mut i = inc(3, &[Some(true), Some(true)]);
        i.ended_by_sender = Some("failed".into());
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "failed");

        let mut i = inc(3, &[Some(true), None]);
        i.ended_by_sender = Some("cancelled".into());
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "active", "a file still going");
        i.files.insert("1".into(), (10, 0, Some(false)));
        check_incoming_end(&mut i);
        assert_eq!(i.item.state, "cancelled");
    }

    #[test]
    fn the_item_sums_its_files() {
        let mut i = inc(3, &[Some(true), None, Some(false)]);
        i.item.total = 30;
        i.files.insert("1".into(), (10, 4, None));
        i.files.insert("0".into(), (10, 10, Some(true)));
        let it = in_item(&i);
        assert_eq!((it.done, it.files_done, it.files_failed), (14, 1, 1));
    }

    #[test]
    fn which_failures_skip_one_file() {
        assert!(only_this_file("the file arrived damaged"));
        assert!(only_this_file("can't read: gone"));
        assert!(!only_this_file("cancelled"));
        assert!(!only_this_file("the device wasn't reachable"));
    }
}
