//! Sending files between your devices, over whatever link the peer has
//! (QUIC, punched, WebRTC or the relay).
//!
//!   file_offer  { from, id, name, size, sha256 }   sender → receiver (reliable)
//!               (+ folder? for a file of a folder, see `features::folders`)
//!   file_accept { from, id, offset }               receiver → sender
//!   file_chunk  { from, id, off, data }            sender → receiver (base64)
//!   file_ack    { from, id, upto }                 receiver → sender
//!   file_done   { from, id, ok, error? }           receiver → sender
//!   file_cancel { from, id }                       either way
//!
//! The file goes in `CHUNK`-sized pieces with at most `WINDOW` bytes
//! unacknowledged. The receiver writes to `<downloads>/.mydomain-partial/
//! <id>.part` and acks how much it has; a chunk that doesn't continue the
//! file is answered with the current length, and the sender rewinds to it.
//! If the link drops, the transfer pauses and is offered again once the
//! peer has a link; the receiver answers with the length of its partial
//! file, so it resumes where it stopped. At the end the SHA-256 is checked
//! and the file moved into the downloads folder under a free name.
//!
//! Resuming survives more than a dropped link:
//! - The receiver keeps `<id>.json` next to the partial file (who sends it,
//!   its name, size and SHA-256). A partial file is only resumed for an
//!   offer of the same file; anything else starts from zero.
//! - The sender keeps its transfers (not a folder's or a backup's files:
//!   those features restart their own) in the platform's store, so after a
//!   restart it offers them again, for up to `RESUME_FOR`, as long as the
//!   file is still there unchanged (same size and modification time). A
//!   changed file fails the transfer and the receiver drops its part.
//! - Progress after a resume starts from the offset, with `resumed` set.
//!
//! Nothing new on the wire: `file_accept.offset` was always there (an
//! older receiver that leaves it out, or always answers 0, simply gets the
//! whole file again), so peers of any version still talk.
//!
//! Your own devices are trusted, so offers are accepted without asking
//! (Phase 4's grants add an `ask`).
//!
//! A file of a synced folder (`"sync": {"pair"}` in the offer, see
//! `features::folder_sync`) goes the same way, but into that folder's
//! `.mydomain-sync` (partial file included, so the final move is a rename
//! on the same disk); the sync feature then puts it in its place.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use parking_lot::{Condvar, Mutex};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// Raw bytes per chunk (about 120 KB on the wire after base64, JSON and
/// encryption: well inside every transport's frame limit).
pub const CHUNK: usize = 64 * 1024;
/// Most bytes in flight without an ack.
const WINDOW: u64 = 16 * CHUNK as u64;
/// No ack for this long: rewind to the last acked byte.
const STALL: Duration = Duration::from_secs(5);
/// How often a waiting sender checks that the link is still there.
const LINK_CHECK: Duration = Duration::from_millis(500);
/// Stalls in a row before the transfer pauses until the next link.
const MAX_STALLS: u32 = 3;
/// Paused transfers are offered again at most this often over a link...
const REOFFER_EVERY: Duration = Duration::from_secs(10);
/// ...and, with no link (each try connects, maybe waking it by push), after
/// a wait that doubles from this, up to `MAX_BACKOFF`.
const FIRST_BACKOFF: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(10 * 60);
/// An offer nobody accepted is given up after this (it has left the
/// outbox by then, see `transport::outbox`).
const NEVER_ACCEPTED: Duration = Duration::from_secs(11 * 60);
/// A transfer paused this long (the device never came back) is given up;
/// a sender that restarts drops the transfers it started longer ago.
const RESUME_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// A partial incoming file nobody has fed for this long is dropped (the
/// sender gave up): a little longer than the sender keeps trying.
const INCOMING_IDLE: Duration = Duration::from_secs(25 * 60 * 60);
/// How often the partial folder is swept for files of transfers this run
/// never heard of (from before a restart) that are past `INCOMING_IDLE`.
const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);
/// A gap ack rewinds the sender at most this often (each chunk in flight
/// after the gap is answered with one).
const REWIND_EVERY: Duration = Duration::from_millis(500);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const PARTIAL_DIR: &str = ".mydomain-partial";
/// Why a transfer whose file changed under it ends.
const CHANGED: &str = "the file changed since it was offered";

struct Outgoing {
    node: String,
    path: PathBuf,
    name: String,
    size: u64,
    sha: String,
    /// Bytes the receiver confirmed.
    acked: u64,
    /// A sender thread is running.
    running: bool,
    /// The receiver accepted at least once (so it's worth re-offering).
    started: bool,
    offered_at: Option<Instant>,
    /// Wait before the next re-offer while the peer has no link.
    backoff: Duration,
    progress_at: Option<Instant>,
    /// When `acked` last moved (or the sender last rewound): no movement
    /// for `STALL` means chunks were lost.
    last_progress: Instant,
    /// The receiver reported a gap: resend from `acked` now.
    rewind: bool,
    rewound_at: Option<Instant>,
    /// Since when the transfer is paused (no sender running, not done).
    paused_since: Option<Instant>,
    /// More fields for the offer (e.g. `features::backup`'s folder).
    extra: Option<serde_json::Value>,
    /// The file's modification time when it was offered (ns since the
    /// epoch, if the platform has one): with `size`, how a resume knows
    /// it's still the same file.
    mtime: Option<u64>,
    /// When the transfer started (unix seconds; see `RESUME_FOR`).
    created: u64,
    /// Kept in the platform's store, to be offered again after a restart.
    persist: bool,
    /// It carried on from a partial file at least once.
    resumed: bool,
}

struct Incoming {
    from: String,
    name: String,
    size: u64,
    sha: String,
    part: PathBuf,
    /// The folder it's moved into when complete.
    dest: PathBuf,
    file: File,
    len: u64,
    hasher: Sha256,
    /// A copied picture (`"clipboard": true` in the offer): the receiving
    /// host also puts it on its clipboard.
    clipboard: bool,
    progress_at: Option<Instant>,
    /// Last offer or chunk for it (see `INCOMING_IDLE`).
    last_seen: Instant,
    /// It carried on from a partial file at least once.
    resumed: bool,
    /// A synced folder's file: the pair it belongs to.
    sync: Option<String>,
}

/// Transfers in progress, both ways.
#[derive(Default)]
pub struct Files {
    /// Where received files go; `None` = this device doesn't take files.
    pub download_dir: Mutex<Option<PathBuf>>,
    out: Mutex<HashMap<String, Outgoing>>,
    acked: Condvar,
    inc: Mutex<HashMap<String, Incoming>>,
    /// Transfers this device finished receiving lately, with how they ended
    /// (`Some(error)` if they failed): an offer repeated because our
    /// `file_done` got lost is answered again instead of starting over.
    finished: Mutex<std::collections::VecDeque<(String, Option<String>)>>,
    /// `finished` was read back from the platform's store (once per run).
    finished_loaded: std::sync::atomic::AtomicBool,
    /// Folders other devices may browse and download from (see
    /// `features::browse`): (name, path). Empty = nothing is shared.
    pub shared: Mutex<Vec<(String, PathBuf)>>,
    /// Folders being sent, file by file (see `features::folders`).
    pub(crate) folders: Mutex<Vec<super::folders::FolderSend>>,
    /// The outgoing transfers were read back from the platform's store.
    out_restored: std::sync::atomic::AtomicBool,
    /// What was last written there (to skip writing the same again).
    out_saved: Mutex<String>,
    /// When the partial folder was last swept.
    swept_at: Mutex<Option<Instant>>,
    /// Devices a cancel was sent to: node -> (last connect try, since).
    cancels: Mutex<HashMap<String, (Instant, Instant)>>,
    /// Tests only: answer offers like a device from before resuming (no
    /// `offset` in the accept, the partial file thrown away).
    pub(crate) legacy_receiver: std::sync::atomic::AtomicBool,
}

/// An outgoing transfer as kept in the platform's store.
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedOutgoing {
    id: String,
    node: String,
    path: PathBuf,
    name: String,
    size: u64,
    sha: String,
    #[serde(default)]
    mtime: Option<u64>,
    created: u64,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    extra: Option<serde_json::Value>,
}

/// What the receiver keeps next to a partial file (`<id>.json`): the offer
/// it belongs to.
#[derive(serde::Serialize, serde::Deserialize, PartialEq)]
struct PartMeta {
    from: String,
    name: String,
    size: u64,
    sha256: String,
}

/// Where `Files`' outgoing transfers are kept between runs.
const OUTGOING_KEY: &str = "files_outgoing";

/// How many finished transfers are remembered.
const FINISHED_KEPT: usize = 64;

/// Where `Files::finished` is kept between runs (ids and outcomes only),
/// so a receiver that restarted still recognises a repeated offer.
const FINISHED_KEY: &str = "files_finished";

impl Files {
    fn load_finished<P: Platform>(&self, ctx: &NetCtx<P>) {
        use std::sync::atomic::Ordering;
        if self.finished_loaded.swap(true, Ordering::Relaxed) {
            return;
        }
        let stored: Vec<(String, Option<String>)> = ctx
            .platform
            .kv_get(FINISHED_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut f = self.finished.lock();
        for e in stored.into_iter().rev() {
            f.push_front(e);
        }
        while f.len() > FINISHED_KEPT {
            f.pop_front();
        }
    }

    fn finished<P: Platform>(&self, ctx: &NetCtx<P>, id: &str, error: Option<&str>) {
        self.load_finished(ctx);
        let json = {
            let mut f = self.finished.lock();
            if f.len() >= FINISHED_KEPT {
                f.pop_front();
            }
            f.push_back((id.to_string(), error.map(str::to_string)));
            serde_json::to_string(&*f).unwrap_or_default()
        };
        ctx.platform.kv_set(FINISHED_KEY, &json);
    }

    fn how_it_ended<P: Platform>(&self, ctx: &NetCtx<P>, id: &str) -> Option<Option<String>> {
        self.load_finished(ctx);
        self.finished
            .lock()
            .iter()
            .find(|(x, _)| x == id)
            .map(|(_, e)| e.clone())
    }
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

/// Send a control message over the peer's current link.
fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &serde_json::Value) -> crate::Result<()> {
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(
        &ctx.ws_conns,
        &ctx.p2p_conns,
        &ctx.key,
        &name,
        node,
        &msg.to_string(),
    )
}

/// Only a plain file name: no folders, no hidden or empty names.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .take(200)
        .collect();
    let clean = clean
        .trim()
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_string();
    if clean.is_empty() {
        "file".to_string()
    } else {
        clean
    }
}

/// The subfolder an offer's `folder` names (`Trip/day 1`), if it's safe
/// to make under the downloads folder: relative, separated by `/`, every
/// part a plain name (`safe_name` leaves it as it is), so no `..`, no
/// absolute path, no drive letter.
pub fn safe_folder(folder: &str) -> Option<PathBuf> {
    if folder.is_empty() || folder.len() > 1024 {
        return None;
    }
    let parts: Vec<&str> = folder.split('/').collect();
    if parts.len() > 33 || parts.iter().any(|p| p.is_empty() || safe_name(p) != *p) {
        return None;
    }
    Some(parts.iter().collect())
}

/// `dir/name` for `n == 0`, else `dir/name (n).ext`.
fn numbered(dir: &Path, name: &str, n: u32) -> PathBuf {
    if n == 0 {
        return dir.join(name);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    dir.join(format!("{stem} ({n}){ext}"))
}

/// `dir/name`, or `dir/name (1).ext`, ... if that's taken.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    (0..10_000)
        .map(|n| numbered(dir, name, n))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(name))
}

/// Move `part` into `dir` under `name` or the first free numbered name,
/// never over an existing file: the name is claimed with `create_new`
/// (atomic), so two files finishing at once can't take the same one.
fn move_into(part: &Path, dir: &Path, name: &str) -> Option<PathBuf> {
    for n in 0..10_000 {
        let dest = numbered(dir, name, n);
        match OpenOptions::new().write(true).create_new(true).open(&dest) {
            Ok(_) => {
                // Ours now (empty): replace it with the file.
                if std::fs::rename(part, &dest).is_ok() {
                    return Some(dest);
                }
                let _ = std::fs::remove_file(&dest);
                return None;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Transfer ids are letters and digits (they name the partial file).
pub fn is_transfer_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

pub(crate) fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A file's modification time in ns since the epoch, if the platform has
/// one.
fn mtime_of(meta: &std::fs::Metadata) -> Option<u64> {
    let d = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    u64::try_from(d.as_nanos()).ok()
}

/// `path` is still the file that was offered: there, same size, same
/// modification time.
fn unchanged(path: &Path, size: u64, mtime: Option<u64>) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|m| m.is_file() && m.len() == size && (mtime.is_none() || mtime_of(&m) == mtime))
}

/// A cancel must arrive, or the other side keeps a partial file (or keeps
/// offering it) for a day: through the outbox, which holds it until the
/// peer acknowledges it, connecting if there's no link (or the link is
/// closing under us).
fn send_cancel<P: Platform>(ctx: &NetCtx<P>, node: &str, id: &str) {
    let msg = json!({ "type": "file_cancel", "from": me(ctx), "id": id });
    crate::engine::send_reliably(ctx, node, &msg.to_string());
    // If it went out on a link that was closing, nothing else would bring
    // a new one: `nudge_cancels` does.
    let now = Instant::now();
    ctx.files
        .cancels
        .lock()
        .insert(node.to_string(), (now, now));
}

/// A cancel still waiting in the outbox for a device with no link: connect
/// (every `REOFFER_EVERY`, for as long as the outbox keeps it).
fn nudge_cancels<P: Platform>(ctx: &NetCtx<P>) {
    let mut due = Vec::new();
    ctx.files.cancels.lock().retain(|node, (last, since)| {
        let waiting = crate::transport::outbox::waiting(ctx, node) > 0;
        if !waiting || since.elapsed() >= NEVER_ACCEPTED {
            return false;
        }
        if !ctx.ws_conns.lock().contains_key(node) && last.elapsed() >= REOFFER_EVERY {
            *last = Instant::now();
            due.push(node.clone());
        }
        true
    });
    for node in due {
        crate::engine::start_ladder(ctx, &node);
    }
}

fn offer_msg(me: &str, id: &str, t: &Outgoing) -> serde_json::Value {
    let mut msg = json!({
        "type": "file_offer", "from": me, "id": id,
        "name": t.name, "size": t.size, "sha256": t.sha,
    });
    add_extra(&mut msg, t.extra.as_ref());
    msg
}

/// A folder's and a backup's files are restarted by their own feature, so
/// only other transfers outlive a restart here.
fn persists(extra: Option<&serde_json::Value>) -> bool {
    extra.is_none_or(|e| {
        e.get("folder").is_none() && e.get("backup").is_none() && e.get("sync").is_none()
    })
}

/// Write the transfers worth resuming after a restart to the platform's
/// store (if they changed since last time).
fn save_outgoing<P: Platform>(ctx: &NetCtx<P>) {
    // Not over what the last run left before it's taken back.
    restore_outgoing(ctx);
    let mut saved: Vec<SavedOutgoing> = ctx
        .files
        .out
        .lock()
        .iter()
        .filter(|(_, t)| t.persist)
        .map(|(id, t)| SavedOutgoing {
            id: id.clone(),
            node: t.node.clone(),
            path: t.path.clone(),
            name: t.name.clone(),
            size: t.size,
            sha: t.sha.clone(),
            mtime: t.mtime,
            created: t.created,
            started: t.started,
            extra: t.extra.clone(),
        })
        .collect();
    saved.sort_by(|a, b| a.id.cmp(&b.id));
    let json = serde_json::to_string(&saved).unwrap_or_default();
    let mut last = ctx.files.out_saved.lock();
    if *last != json {
        ctx.platform.kv_set(OUTGOING_KEY, &json);
        *last = json;
    }
}

/// Once per run: take back the transfers a previous run was sending. One
/// that's too old, or whose file is gone or changed, fails (and the
/// receiver is told, so it drops its part); the rest are offered again.
fn restore_outgoing<P: Platform>(ctx: &NetCtx<P>) {
    use std::sync::atomic::Ordering;
    if ctx.files.out_restored.swap(true, Ordering::Relaxed) {
        return;
    }
    let saved: Vec<SavedOutgoing> = ctx
        .platform
        .kv_get(OUTGOING_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let me = me(ctx);
    let now = now_secs();
    for s in saved {
        if !is_transfer_id(&s.id) || ctx.files.out.lock().contains_key(&s.id) {
            continue;
        }
        let why = if now.saturating_sub(s.created) >= RESUME_FOR.as_secs() {
            Some("the device wasn't reachable")
        } else if !unchanged(&s.path, s.size, s.mtime) {
            Some("the file changed or is gone")
        } else {
            None
        };
        if let Some(why) = why {
            failed(ctx, &s.id, &s.node, &s.name, false, why);
            send_cancel(ctx, &s.node, &s.id);
            continue;
        }
        let t = Outgoing {
            node: s.node.clone(),
            path: s.path,
            name: s.name.clone(),
            size: s.size,
            sha: s.sha,
            acked: 0,
            running: false,
            started: s.started,
            // Offered again at once: started ones by `resume_paused`, the
            // others just below.
            offered_at: (!s.started).then(Instant::now),
            backoff: FIRST_BACKOFF,
            progress_at: None,
            last_progress: Instant::now(),
            rewind: false,
            rewound_at: None,
            paused_since: s.started.then(Instant::now),
            extra: s.extra,
            mtime: s.mtime,
            created: s.created,
            persist: true,
            resumed: false,
        };
        let offer = (!s.started).then(|| offer_msg(&me, &s.id, &t));
        ctx.files.out.lock().insert(s.id.clone(), t);
        ctx.sink.emit(CoreEvent::FileProgress {
            id: s.id.clone(),
            node_id: s.node.clone(),
            name: s.name,
            done: 0,
            total: s.size,
            incoming: false,
            resumed: false,
        });
        if let Some(offer) = offer {
            crate::engine::send_reliably(ctx, &s.node, &offer.to_string());
        }
    }
}

/// The receiver's note next to partial file `part`.
fn meta_path(part: &Path) -> PathBuf {
    part.with_extension("json")
}

fn remove_partial(part: &Path) {
    let _ = std::fs::remove_file(part);
    let _ = std::fs::remove_file(meta_path(part));
}

/// Remove the partial file of transfer `id` that this run never took up
/// (it's from before a restart), if it's from `from` (any device for
/// `None`). Returns the device it was from.
fn drop_stale_partial<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    from: Option<&str>,
) -> Option<String> {
    if !is_transfer_id(id) {
        return None;
    }
    let dir = ctx.files.download_dir.lock().clone()?;
    let part = dir.join(PARTIAL_DIR).join(format!("{id}.part"));
    let note: PartMeta = std::fs::read_to_string(meta_path(&part))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    if from.is_some_and(|f| f != note.from) {
        return None;
    }
    remove_partial(&part);
    Some(note.from)
}

/// Drop partial files (and their notes) nobody has fed for
/// `INCOMING_IDLE`: from transfers this run never heard of again.
fn sweep_partials<P: Platform>(ctx: &NetCtx<P>) {
    let Some(dir) = ctx.files.download_dir.lock().clone() else {
        return;
    };
    {
        let mut at = ctx.files.swept_at.lock();
        if at.is_some_and(|t| t.elapsed() < SWEEP_EVERY) {
            return;
        }
        *at = Some(Instant::now());
    }
    let Ok(entries) = std::fs::read_dir(dir.join(PARTIAL_DIR)) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let id = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if ctx.files.inc.lock().contains_key(&id) {
            continue;
        }
        // A note is as old as its partial file (written to last).
        let part = path.with_extension("part");
        let fed = if part.exists() { part } else { path.clone() };
        let old = std::fs::metadata(&fed)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age >= INCOMING_IDLE);
        if old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// --- sending ---------------------------------------------------------------

/// Start sending `path` to `node`: hashes it (off the caller's thread) and
/// offers it reliably (`send` queues it while the peer is unreachable).
/// Returns the transfer id at once.
pub fn start<P: Platform>(ctx: &NetCtx<P>, node: &str, path: &Path) -> crate::Result<String> {
    start_with_id(ctx, node, path, &crate::transport::outbox::new_id())
}

/// `start`, under a transfer id the caller chose: a device that asked for
/// the file (`features::browse`) picked it, so it already knows the
/// transfer when the offer arrives.
pub fn start_with_id<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    path: &Path,
    id: &str,
) -> crate::Result<String> {
    start_with_extra(ctx, node, path, id, None)
}

/// The fields of `extra` (an object) go into every offer of this transfer,
/// e.g. a photo backup's `"backup": {...}` (see `backup::offer_dest`).
pub fn start_with_extra<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    path: &Path,
    id: &str,
    extra: Option<serde_json::Value>,
) -> crate::Result<String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("can't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("not a file".to_string().into());
    }
    if !is_transfer_id(id) || ctx.files.out.lock().contains_key(id) {
        return Err("bad transfer id".to_string().into());
    }
    let id = id.to_string();
    let name = safe_name(&path.file_name().unwrap_or_default().to_string_lossy());
    let size = meta.len();
    let mtime = mtime_of(&meta);
    ctx.sink.emit(CoreEvent::FileProgress {
        id: id.clone(),
        node_id: node.to_string(),
        name: name.clone(),
        done: 0,
        total: size,
        incoming: false,
        resumed: false,
    });
    let (ctx, node, path, id2) = (
        ctx.clone(),
        node.to_string(),
        path.to_path_buf(),
        id.clone(),
    );
    std::thread::spawn(move || {
        let sha = match sha256_file(&path) {
            Ok(s) => s,
            Err(e) => {
                return failed(&ctx, &id2, &node, &name, false, &format!("can't read: {e}"));
            }
        };
        let persist = persists(extra.as_ref());
        let t = Outgoing {
            node: node.clone(),
            path,
            name,
            size,
            sha,
            acked: 0,
            running: false,
            started: false,
            offered_at: Some(Instant::now()),
            backoff: FIRST_BACKOFF,
            progress_at: None,
            last_progress: Instant::now(),
            rewind: false,
            rewound_at: None,
            paused_since: None,
            extra,
            mtime,
            created: now_secs(),
            persist,
            resumed: false,
        };
        let msg = offer_msg(&me(&ctx), &id2, &t);
        ctx.files.out.lock().insert(id2.clone(), t);
        if persist {
            save_outgoing(&ctx);
        }
        crate::engine::send_reliably(&ctx, &node, &msg.to_string());
    });
    Ok(id)
}

/// Copy the fields of `extra` (an object) into `msg`.
fn add_extra(msg: &mut serde_json::Value, extra: Option<&serde_json::Value>) {
    if let (Some(serde_json::Value::Object(fields)), serde_json::Value::Object(m)) = (extra, msg) {
        for (k, v) in fields {
            m.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
}

fn failed<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    node: &str,
    name: &str,
    incoming: bool,
    why: &str,
) {
    ctx.sink.emit(CoreEvent::FileFailed {
        id: id.to_string(),
        node_id: node.to_string(),
        name: name.to_string(),
        incoming,
        reason: why.to_string(),
    });
    if !incoming {
        super::backup::outgoing_ended(ctx, id, Some(why));
        super::folders::outgoing_ended(ctx, id, Some(why));
    }
    super::folder_sync::transfer_ended(ctx, id, Some(why), incoming);
}

/// Drop an outgoing transfer without a word to anyone: its sender stops,
/// and the receiver keeps the partial file, so starting it again under the
/// same id resumes it (`features::backup` pausing off Wi-Fi).
pub fn forget<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    ctx.files.out.lock().remove(id);
    ctx.files.acked.notify_all();
}

/// (bytes confirmed, size) of outgoing transfer `id`, if it's going.
pub fn outgoing_progress<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<(u64, u64)> {
    ctx.files.out.lock().get(id).map(|t| (t.acked, t.size))
}

/// Push chunks from `from` on, keeping the window full, until everything is
/// acked, the link fails, or the transfer is gone.
fn run_sender<P: Platform>(ctx: NetCtx<P>, id: String, from: u64) {
    let files = &ctx.files;
    let Some((node, path, size, mtime)) = files
        .out
        .lock()
        .get(&id)
        .map(|t| (t.node.clone(), t.path.clone(), t.size, t.mtime))
    else {
        return;
    };
    // The offer's hash (and the receiver's partial file) is of the file as
    // it was: a changed one can't go on.
    if !unchanged(&path, size, mtime) {
        return give_up(&ctx, &id, CHANGED);
    }
    // Paused: offered again on the next link (see `resume_paused`).
    let pause = |t: &mut Outgoing| {
        t.running = false;
        t.paused_since.get_or_insert_with(Instant::now);
    };
    let Ok(mut file) = File::open(&path) else {
        return give_up(&ctx, &id, "the file couldn't be read");
    };
    let mut pos = from;
    let mut buf = vec![0u8; CHUNK];
    let (mut stalls, mut seen_acked) = (0, from);
    loop {
        // Wait for room in the window (or the end), rewinding on a stall
        // or when the receiver reports a gap.
        {
            let mut out = files.out.lock();
            let Some(t) = out.get_mut(&id) else { return };
            if t.acked >= size {
                // All there; `file_done` finishes it. If that got lost, the
                // re-offer is answered from the receiver's memory.
                pause(t);
                return;
            }
            if t.acked > seen_acked {
                (stalls, seen_acked) = (0, t.acked);
            }
            if t.rewind {
                t.rewind = false;
                pos = t.acked;
            }
            pos = pos.max(t.acked);
            if pos >= size || pos - t.acked >= WINDOW {
                files.acked.wait_for(&mut out, LINK_CHECK);
                let Some(t) = out.get_mut(&id) else { return };
                if !ctx.ws_conns.lock().contains_key(&node) {
                    return pause(t); // link gone
                }
                if t.last_progress.elapsed() >= STALL {
                    stalls += 1;
                    if stalls >= MAX_STALLS {
                        return pause(t);
                    }
                    pos = t.acked;
                    t.last_progress = Instant::now();
                }
                continue;
            }
        }
        let n = match file
            .seek(SeekFrom::Start(pos))
            .and_then(|_| file.read(&mut buf))
        {
            Ok(n) if n > 0 => n,
            // Shorter than when it was offered, or unreadable now.
            _ => return give_up(&ctx, &id, "the file couldn't be read"),
        };
        let msg = json!({
            "type": "file_chunk", "from": me(&ctx), "id": id, "off": pos,
            "data": base64::engine::general_purpose::STANDARD.encode(&buf[..n]),
        });
        if send(&ctx, &node, &msg).is_err() {
            if let Some(t) = files.out.lock().get_mut(&id) {
                pause(t); // no link
            }
            return;
        }
        pos += n as u64;
    }
}

/// End an outgoing transfer for good: tell the user and the receiver.
fn give_up<P: Platform>(ctx: &NetCtx<P>, id: &str, why: &str) {
    let Some(t) = ctx.files.out.lock().remove(id) else {
        return;
    };
    ctx.files.acked.notify_all();
    save_outgoing(ctx);
    failed(ctx, id, &t.node, &t.name, false, why);
    // Even with no link now: the receiver drops its partial file.
    send_cancel(ctx, &t.node, id);
}

/// Offer paused transfers again (called on a timer); the receiver answers
/// with how much it already has. Every `REOFFER_EVERY` while the peer has a
/// live link; without one, after a backoff, since each try may wake it by
/// push.
pub fn resume_paused<P: Platform>(ctx: &NetCtx<P>) {
    restore_outgoing(ctx);
    let me = me(ctx);
    let mut offers = Vec::new();
    let mut expired = Vec::new();
    let mut changed = Vec::new();
    {
        let mut out = ctx.files.out.lock();
        // Never accepted, and the queued offer has expired from the outbox.
        out.retain(|id, t| {
            let stale = !t.started
                && t.offered_at
                    .is_some_and(|at| at.elapsed() >= NEVER_ACCEPTED);
            if stale {
                expired.push((id.clone(), t.node.clone(), t.name.clone()));
            }
            !stale
        });
        for (id, t) in out.iter_mut() {
            if t.running || !t.started {
                continue;
            }
            // With no link, each try may wake the device by push: back off.
            let linked = crate::transport::is_live(ctx, &t.node);
            let every = if linked { REOFFER_EVERY } else { t.backoff };
            if t.offered_at.is_some_and(|at| at.elapsed() < every) {
                continue;
            }
            // Only the file that was offered is resumed.
            if !unchanged(&t.path, t.size, t.mtime) {
                changed.push(id.clone());
                continue;
            }
            t.offered_at = Some(Instant::now());
            if !linked {
                t.backoff = (t.backoff * 2).min(MAX_BACKOFF);
            }
            offers.push((t.node.clone(), offer_msg(&me, id, t)));
        }
    }
    for (id, node, name) in expired {
        failed(ctx, &id, &node, &name, false, "the device wasn't reachable");
    }
    for id in changed {
        give_up(ctx, &id, CHANGED);
    }
    let stuck: Vec<String> = ctx
        .files
        .out
        .lock()
        .iter()
        .filter(|(_, t)| !t.running && t.paused_since.is_some_and(|p| p.elapsed() >= RESUME_FOR))
        .map(|(id, _)| id.clone())
        .collect();
    for id in stuck {
        give_up(ctx, &id, "the device wasn't reachable");
    }
    save_outgoing(ctx);
    sweep_partials(ctx);
    nudge_cancels(ctx);
    // Partial incoming files nobody feeds any more.
    let idle: Vec<Incoming> = {
        let mut inc = ctx.files.inc.lock();
        let ids: Vec<String> = inc
            .iter()
            .filter(|(_, t)| t.last_seen.elapsed() >= INCOMING_IDLE)
            .map(|(id, _)| id.clone())
            .collect();
        ids.iter().filter_map(|id| inc.remove(id)).collect()
    };
    for t in idle {
        remove_partial(&t.part);
    }
    // Reliably, linked or not: the outbox resends it on the next link,
    // connects (waking the device by push) when there's none, and notices
    // a link that stopped answering (a killed app, a phone that left
    // without a word) and replaces it.
    for (node, msg) in offers {
        crate::engine::send_reliably(ctx, &node, &msg.to_string());
    }
}

/// Stop a transfer, either way, and tell the other side.
pub fn cancel<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let outgoing = ctx.files.out.lock().remove(id);
    let incoming = match outgoing {
        Some(_) => None,
        None => ctx.files.inc.lock().remove(id),
    };
    let peer = if let Some(t) = outgoing {
        save_outgoing(ctx);
        failed(ctx, id, &t.node, &t.name, false, "cancelled");
        Some(t.node)
    } else if let Some(t) = incoming {
        drop(t.file);
        remove_partial(&t.part);
        // A re-offer still on its way is answered "cancelled", not restarted.
        ctx.files
            .finished(ctx, id, Some("cancelled on the other device"));
        failed(ctx, id, &t.from, &t.name, true, "cancelled");
        Some(t.from)
    } else {
        // A partial file from before a restart, not offered again yet.
        let from = drop_stale_partial(ctx, id, None);
        if from.is_some() {
            ctx.files
                .finished(ctx, id, Some("cancelled on the other device"));
        }
        from
    };
    ctx.files.acked.notify_all();
    // Even with no link now: a paused transfer's other side drops its
    // partial file (or stops offering it) once it hears.
    if let Some(node) = peer {
        send_cancel(ctx, &node, id);
    }
}

// --- receiving -------------------------------------------------------------

fn on_offer<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    // The offer may have come in before we have a link back (it was
    // queued and delivered as the link came up): then queue the answer
    // too, or the sender would wait for an accept that never comes.
    let reply = |m: serde_json::Value| {
        if send(ctx, from, &m).is_err() {
            crate::engine::send_reliably(ctx, from, &m.to_string());
        }
    };
    // Ids are ours (hex); anything else could escape the partial folder.
    if !is_transfer_id(&id) {
        return;
    }
    let size = msg.json("size").as_u64().unwrap_or(0);
    let name = safe_name(&msg.str("name"));
    let sha = msg.str("sha256");
    let reject = |why: &str| {
        reply(json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": false, "error": why }))
    };
    // Already received (our `file_done` was lost): say so again.
    if let Some(ended) = ctx.files.how_it_ended(ctx, &id) {
        let mut done =
            json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": ended.is_none() });
        if let Some(why) = ended {
            done["error"] = why.into();
        }
        return reply(done);
    }
    // A synced folder's file: received inside that folder.
    let sync = msg
        .json("sync")
        .get("pair")
        .and_then(|p| p.as_str())
        .map(str::to_string);
    let (dir, dest) = if let Some(pair) = &sync {
        match super::folder_sync::offer_dir(ctx, from, pair) {
            Ok(dir) => {
                let dest = dir.join("in");
                (dir, dest)
            }
            Err(why) => return reject(why),
        }
    } else {
        let Some(dir) = ctx.files.download_dir.lock().clone() else {
            return reject("this device can't receive files");
        };
        let mut dest = super::backup::offer_dest(&dir, msg);
        // A file of a folder: into its subfolder (a backup has its own).
        if let Some(folder) = msg.json("folder").as_str().filter(|_| dest == dir) {
            let Some(sub) = safe_folder(folder) else {
                return reject("the folder name isn't allowed");
            };
            dest = dir.join(sub);
        }
        // A backup of a photo that's already here (the phone forgot it
        // sent it): done, without sending it again.
        if dest != dir && super::backup::already_there(&dest, &name, size, &sha) {
            ctx.files.finished(ctx, &id, None);
            return reply(json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": true }));
        }
        (dir, dest)
    };

    let legacy = ctx
        .files
        .legacy_receiver
        .load(std::sync::atomic::Ordering::Relaxed);
    let meta = PartMeta {
        from: from.to_string(),
        name: name.clone(),
        size,
        sha256: sha.clone(),
    };
    // Already going: the same file carries on where it is; a different
    // one under this id starts over (from another device: ignored).
    let known = {
        let mut inc = ctx.files.inc.lock();
        match inc.get_mut(&id) {
            Some(t) if t.from != from => return,
            Some(t) if !legacy && t.size == size && t.sha == sha => {
                t.last_seen = Instant::now();
                t.resumed |= t.len > 0;
                Some((t.len, t.resumed))
            }
            Some(_) => {
                if let Some(t) = inc.remove(&id) {
                    drop(t.file);
                    remove_partial(&t.part);
                }
                None
            }
            None => None,
        }
    };
    let (offset, resumed) = if let Some(known) = known {
        known
    } else {
        let partial = dir.join(PARTIAL_DIR);
        let part = partial.join(format!("{id}.part"));
        let opened = std::fs::create_dir_all(&partial).and_then(|_| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(&part)
        });
        let Ok(file) = opened else {
            return reject("can't write to the downloads folder");
        };
        // A partial file from before (an earlier link, or a restart) of
        // this very file (its note says so): resume after it, hashing
        // what's already there. Outside the lock: a big partial file takes
        // a while, and chunks of other transfers shouldn't wait for it.
        let note = meta_path(&part);
        let same = !legacy
            && std::fs::read_to_string(&note)
                .ok()
                .and_then(|s| serde_json::from_str::<PartMeta>(&s).ok())
                .is_some_and(|m| m == meta);
        let mut hasher = Sha256::new();
        let mut len = 0u64;
        if same {
            if let Ok(mut f) = File::open(&part) {
                let mut buf = vec![0u8; CHUNK];
                while let Ok(n) = f.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                    len += n as u64;
                }
            }
        }
        if !same || len > size {
            let _ = file.set_len(0);
            hasher = Sha256::new();
            len = 0;
            if !legacy {
                let note_json = serde_json::to_string(&meta).unwrap_or_default();
                let _ = std::fs::write(&note, note_json);
            }
        }
        let mut inc = ctx.files.inc.lock();
        // Another copy of the offer may have got here first.
        if let Some(t) = inc.get(&id) {
            (t.len, t.resumed)
        } else {
            inc.insert(
                id.clone(),
                Incoming {
                    from: from.to_string(),
                    name: name.clone(),
                    size,
                    sha,
                    part,
                    dest,
                    file,
                    len,
                    hasher,
                    clipboard: msg.json("clipboard").as_bool().unwrap_or(false),
                    progress_at: None,
                    last_seen: Instant::now(),
                    resumed: len > 0,
                    sync,
                },
            );
            (len, len > 0)
        }
    };
    ctx.sink.emit(CoreEvent::FileProgress {
        id: id.clone(),
        node_id: from.to_string(),
        name,
        done: offset,
        total: size,
        incoming: true,
        resumed,
    });
    if legacy {
        // As a device from before resuming answers: no offset.
        reply(json!({ "type": "file_accept", "from": me(ctx), "id": id }));
    } else {
        reply(json!({ "type": "file_accept", "from": me(ctx), "id": id, "offset": offset }));
    }
    if offset == size {
        finish(ctx, &id);
    }
}

fn on_chunk<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let off = msg.json("off").as_u64().unwrap_or(u64::MAX);
    let mut inc = ctx.files.inc.lock();
    let Some(t) = inc.get_mut(&id) else { return };
    if t.from != from {
        return;
    }
    t.last_seen = Instant::now();
    // Ahead of what we have: a chunk before it was lost.
    let gap = off > t.len;
    if off == t.len {
        let Ok(data) = base64::engine::general_purpose::STANDARD.decode(msg.str("data")) else {
            return;
        };
        if t.len + data.len() as u64 > t.size {
            return;
        }
        if t.file.write_all(&data).is_err() {
            // Disk full, most likely: what was half-written is cut off.
            let t = inc.remove(&id);
            drop(inc);
            if let Some(t) = t {
                drop(t.file);
                remove_partial(&t.part);
                let why = "couldn't save the file (is the storage full?)";
                ctx.files.finished(ctx, &id, Some(why));
                failed(ctx, &id, from, &t.name, true, why);
                let _ = send(
                    ctx,
                    from,
                    &json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": false, "error": why }),
                );
            }
            return;
        }
        t.hasher.update(&data);
        t.len += data.len() as u64;
    }
    // Ack what we have; a gap makes the sender resend from there.
    let (len, size, name, resumed) = (t.len, t.size, t.name.clone(), t.resumed);
    let show = t
        .progress_at
        .is_none_or(|at| at.elapsed() >= PROGRESS_EVERY)
        || len == size;
    if show {
        t.progress_at = Some(Instant::now());
    }
    drop(inc);
    let _ = send(
        ctx,
        from,
        &json!({ "type": "file_ack", "from": me(ctx), "id": id, "upto": len, "gap": gap }),
    );
    if show {
        ctx.sink.emit(CoreEvent::FileProgress {
            id: id.clone(),
            node_id: from.to_string(),
            name,
            done: len,
            total: size,
            incoming: true,
            resumed,
        });
    }
    if len == size {
        finish(ctx, &id);
    }
}

/// All bytes are in: check the hash, move the file into place, tell both
/// sides.
fn finish<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let Some(t) = ctx.files.inc.lock().remove(id) else {
        return;
    };
    let _ = t.file.sync_all();
    drop(t.file);
    let ok = hex(&t.hasher.finalize()) == t.sha;
    let dest = if ok && std::fs::create_dir_all(&t.dest).is_ok() {
        move_into(&t.part, &t.dest, &t.name)
    } else {
        None
    };
    let moved = dest.is_some();
    let _ = std::fs::remove_file(meta_path(&t.part));
    let me = me(ctx);
    let why = if moved {
        None
    } else if ok {
        Some("couldn't save the file")
    } else {
        Some("the file arrived damaged")
    };
    ctx.files.finished(ctx, id, why);
    if moved {
        let path = dest.unwrap_or_default();
        if let Some(pair) = &t.sync {
            // Into its place in the synced folder (which then says so).
            super::folder_sync::received(ctx, pair, id, path, &t.from, &t.name, t.size, &t.sha);
        } else {
            ctx.sink.emit(CoreEvent::FileReceived {
                id: id.to_string(),
                from: t.from.clone(),
                name: t.name.clone(),
                path: path.to_string_lossy().into_owned(),
                size: t.size,
                clipboard: t.clipboard,
                sync: false,
            });
        }
        let _ = send(
            ctx,
            &t.from,
            &json!({ "type": "file_done", "from": me, "id": id, "ok": true }),
        );
    } else {
        remove_partial(&t.part);
        let why = why.unwrap_or_default();
        failed(ctx, id, &t.from, &t.name, true, why);
        let _ = send(
            ctx,
            &t.from,
            &json!({ "type": "file_done", "from": me, "id": id, "ok": false, "error": why }),
        );
    }
}

pub struct FilesHandler;

impl<P: Platform> ControlHandler<P> for FilesHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "file_offer",
            "file_accept",
            "file_chunk",
            "file_ack",
            "file_done",
            "file_cancel",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        let id = msg.str("id");
        let files = &ctx.files;
        match msg.kind() {
            "file_offer" => on_offer(ctx, msg, &from),
            "file_chunk" => on_chunk(ctx, msg, &from),
            "file_accept" => {
                let offset = msg.json("offset").as_u64().unwrap_or(0);
                let mut out = files.out.lock();
                let Some(t) = out.get_mut(&id) else { return };
                if t.node != from {
                    return;
                }
                // Kept as started at once: a restart now re-offers it.
                let first = !t.started && t.persist;
                t.started = true;
                t.backoff = FIRST_BACKOFF;
                t.paused_since = None;
                t.last_progress = Instant::now();
                t.acked = offset.min(t.size);
                t.resumed |= t.acked > 0;
                files.acked.notify_all();
                // Where it (re)starts: from the receiver's offset.
                t.progress_at = Some(Instant::now());
                let ev = CoreEvent::FileProgress {
                    id: id.clone(),
                    node_id: from.clone(),
                    name: t.name.clone(),
                    done: t.acked,
                    total: t.size,
                    incoming: false,
                    resumed: t.resumed,
                };
                if !t.running && t.acked < t.size {
                    t.running = true;
                    let (ctx, id, at) = (ctx.clone(), id.clone(), t.acked);
                    std::thread::spawn(move || run_sender(ctx, id, at));
                }
                drop(out);
                if first {
                    save_outgoing(ctx);
                }
                ctx.sink.emit(ev);
            }
            "file_ack" => {
                let upto = msg.json("upto").as_u64().unwrap_or(0);
                let mut out = files.out.lock();
                let Some(t) = out.get_mut(&id) else { return };
                if t.node != from {
                    return;
                }
                let upto = upto.min(t.size);
                // Acks from a replaced link may arrive late: never go back.
                if upto > t.acked {
                    t.acked = upto;
                    t.last_progress = Instant::now();
                }
                // The receiver missed a chunk: resend from what it has,
                // without waiting for the stall timeout.
                if msg.json("gap").as_bool() == Some(true)
                    && t.rewound_at.is_none_or(|at| at.elapsed() >= REWIND_EVERY)
                {
                    t.rewind = true;
                    t.rewound_at = Some(Instant::now());
                }
                files.acked.notify_all();
                let show = t
                    .progress_at
                    .is_none_or(|at| at.elapsed() >= PROGRESS_EVERY);
                if show {
                    t.progress_at = Some(Instant::now());
                    let ev = CoreEvent::FileProgress {
                        id: id.clone(),
                        node_id: from.clone(),
                        name: t.name.clone(),
                        done: t.acked,
                        total: t.size,
                        incoming: false,
                        resumed: t.resumed,
                    };
                    drop(out);
                    ctx.sink.emit(ev);
                }
            }
            "file_done" => {
                let mut out = files.out.lock();
                if out.get(&id).is_none_or(|t| t.node != from) {
                    return;
                }
                let Some(t) = out.remove(&id) else { return };
                drop(out);
                files.acked.notify_all();
                save_outgoing(ctx);
                if msg.json("ok").as_bool() == Some(true) {
                    ctx.sink.emit(CoreEvent::FileSent {
                        id: id.clone(),
                        node_id: from,
                        name: t.name,
                    });
                    super::backup::outgoing_ended(ctx, &id, None);
                    super::folders::outgoing_ended(ctx, &id, None);
                    super::folder_sync::transfer_ended(ctx, &id, None, false);
                } else {
                    failed(ctx, &id, &from, &t.name, false, &msg.str("error"));
                }
            }
            "file_cancel" => {
                let gone_out = {
                    let mut out = files.out.lock();
                    match out.get(&id) {
                        Some(t) if t.node == from => out.remove(&id),
                        _ => None,
                    }
                };
                if let Some(t) = gone_out {
                    files.acked.notify_all();
                    save_outgoing(ctx);
                    failed(
                        ctx,
                        &id,
                        &from,
                        &t.name,
                        false,
                        "cancelled on the other device",
                    );
                }
                let gone_in = {
                    let mut inc = files.inc.lock();
                    match inc.get(&id) {
                        Some(t) if t.from == from => inc.remove(&id),
                        _ => None,
                    }
                };
                if let Some(t) = gone_in {
                    remove_partial(&t.part);
                    ctx.files
                        .finished(ctx, &id, Some("cancelled on the other device"));
                    failed(
                        ctx,
                        &id,
                        &from,
                        &t.name,
                        true,
                        "cancelled on the other device",
                    );
                } else {
                    // A partial file from before a restart here.
                    drop_stale_partial(ctx, &id, Some(&from));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_downloads_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Users\\x\\photo.jpg"), "photo.jpg");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name("a\u{0}b?.txt"), "ab.txt");
        assert_eq!(safe_name(".  .."), "file");
        assert_eq!(safe_name(". ."), "file");
        assert_eq!(safe_name("report. "), "report");
    }

    #[test]
    fn folders_cannot_escape_the_downloads_folder() {
        assert_eq!(safe_folder("Trip"), Some(PathBuf::from("Trip")));
        assert_eq!(
            safe_folder("Trip/day 1/raw"),
            Some(PathBuf::from("Trip").join("day 1").join("raw"))
        );
        for bad in [
            "",
            "..",
            "../etc",
            "Trip/../..",
            "Trip/./x",
            "/etc",
            "/",
            "Trip/",
            "Trip//x",
            "C:/Windows",
            "C:",
            "Trip\\..\\..",
            "..\\x",
            ".ssh",
            "a\u{0}b",
        ] {
            assert_eq!(safe_folder(bad), None, "{bad:?}");
        }
        assert_eq!(safe_folder(&["a"; 40].join("/")), None, "too deep");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("md-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("a (1).txt"), "x").unwrap();
        assert_eq!(free_path(&dir, "a.txt"), dir.join("a (2).txt"));
        assert_eq!(free_path(&dir, "b"), dir.join("b"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
