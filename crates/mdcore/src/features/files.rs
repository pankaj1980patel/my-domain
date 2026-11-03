//! Sending files between your devices, over whatever link the peer has
//! (QUIC, punched, WebRTC or the relay).
//!
//!   file_offer  { from, id, name, size, sha256 }   sender → receiver (reliable)
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
//! Your own devices are trusted, so offers are accepted without asking
//! (Phase 4's grants add an `ask`).

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
/// A transfer paused this long in total (the device never came back) is
/// given up.
const GIVE_UP_AFTER: Duration = Duration::from_secs(60 * 60);
/// A partial incoming file nobody has fed for this long is dropped (the
/// sender gave up or restarted).
const INCOMING_IDLE: Duration = Duration::from_secs(2 * 60 * 60);
/// A gap ack rewinds the sender at most this often (each chunk in flight
/// after the gap is answered with one).
const REWIND_EVERY: Duration = Duration::from_millis(500);
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const PARTIAL_DIR: &str = ".mydomain-partial";

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
    progress_at: Option<Instant>,
    /// Last offer or chunk for it (see `INCOMING_IDLE`).
    last_seen: Instant,
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
}

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
    ctx.sink.emit(CoreEvent::FileProgress {
        id: id.clone(),
        node_id: node.to_string(),
        name: name.clone(),
        done: 0,
        total: size,
        incoming: false,
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
        let mut msg = json!({
            "type": "file_offer", "from": me(&ctx), "id": id2,
            "name": name, "size": size, "sha256": sha,
        });
        add_extra(&mut msg, extra.as_ref());
        ctx.files.out.lock().insert(
            id2.clone(),
            Outgoing {
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
            },
        );
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
    }
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
    let Some((node, path, size)) = files
        .out
        .lock()
        .get(&id)
        .map(|t| (t.node.clone(), t.path.clone(), t.size))
    else {
        return;
    };
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
    failed(ctx, id, &t.node, &t.name, false, why);
    let _ = send(
        ctx,
        &t.node,
        &json!({ "type": "file_cancel", "from": me(ctx), "id": id }),
    );
}

/// Offer paused transfers again (called on a timer); the receiver answers
/// with how much it already has. Every `REOFFER_EVERY` while the peer has a
/// live link; without one, after a backoff, since each try may wake it by
/// push.
pub fn resume_paused<P: Platform>(ctx: &NetCtx<P>) {
    let me = me(ctx);
    let mut offers = Vec::new();
    let mut expired = Vec::new();
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
            t.offered_at = Some(Instant::now());
            if !linked {
                t.backoff = (t.backoff * 2).min(MAX_BACKOFF);
            }
            let mut msg = json!({
                "type": "file_offer", "from": me, "id": id,
                "name": t.name, "size": t.size, "sha256": t.sha,
            });
            add_extra(&mut msg, t.extra.as_ref());
            offers.push((t.node.clone(), msg));
        }
    }
    for (id, node, name) in expired {
        failed(ctx, &id, &node, &name, false, "the device wasn't reachable");
    }
    let stuck: Vec<String> = ctx
        .files
        .out
        .lock()
        .iter()
        .filter(|(_, t)| !t.running && t.paused_since.is_some_and(|p| p.elapsed() >= GIVE_UP_AFTER))
        .map(|(id, _)| id.clone())
        .collect();
    for id in stuck {
        give_up(ctx, &id, "the device wasn't reachable");
    }
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
        let _ = std::fs::remove_file(&t.part);
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
        failed(ctx, id, &t.node, &t.name, false, "cancelled");
        Some(t.node)
    } else if let Some(t) = incoming {
        let _ = std::fs::remove_file(&t.part);
        // A re-offer still on its way is answered "cancelled", not restarted.
        ctx.files
            .finished(ctx, id, Some("cancelled on the other device"));
        failed(ctx, id, &t.from, &t.name, true, "cancelled");
        Some(t.from)
    } else {
        None
    };
    ctx.files.acked.notify_all();
    if let Some(node) = peer {
        let _ = send(
            ctx,
            &node,
            &json!({ "type": "file_cancel", "from": me(ctx), "id": id }),
        );
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
    let Some(dir) = ctx.files.download_dir.lock().clone() else {
        return reject("this device can't receive files");
    };
    let dest = super::backup::offer_dest(&dir, msg);
    // A backup of a photo that's already here (the phone forgot it sent
    // it): done, without sending it again.
    if dest != dir && super::backup::already_there(&dest, &name, size, &sha) {
        ctx.files.finished(ctx, &id, None);
        return reply(json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": true }));
    }

    let known = ctx.files.inc.lock().get_mut(&id).map(|t| {
        t.last_seen = Instant::now();
        t.len
    });
    let offset = if let Some(len) = known {
        len
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
        // A partial file from before (an earlier link, or a restart):
        // resume after it, hashing what's already there. Outside the lock:
        // a big partial file takes a while, and chunks of other transfers
        // shouldn't wait for it.
        let mut hasher = Sha256::new();
        let mut len = 0u64;
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
        if len > size {
            let _ = file.set_len(0);
            hasher = Sha256::new();
            len = 0;
        }
        let mut inc = ctx.files.inc.lock();
        // Another copy of the offer may have got here first.
        if let Some(t) = inc.get(&id) {
            t.len
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
                    progress_at: None,
                    last_seen: Instant::now(),
                },
            );
            len
        }
    };
    ctx.sink.emit(CoreEvent::FileProgress {
        id: id.clone(),
        node_id: from.to_string(),
        name,
        done: offset,
        total: size,
        incoming: true,
    });
    reply(json!({ "type": "file_accept", "from": me(ctx), "id": id, "offset": offset }));
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
                let _ = std::fs::remove_file(&t.part);
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
    let (len, size, name) = (t.len, t.size, t.name.clone());
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
        ctx.sink.emit(CoreEvent::FileReceived {
            id: id.to_string(),
            from: t.from.clone(),
            name: t.name.clone(),
            path: path.to_string_lossy().into_owned(),
            size: t.size,
        });
        let _ = send(
            ctx,
            &t.from,
            &json!({ "type": "file_done", "from": me, "id": id, "ok": true }),
        );
    } else {
        let _ = std::fs::remove_file(&t.part);
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
                t.started = true;
                t.backoff = FIRST_BACKOFF;
                t.paused_since = None;
                t.last_progress = Instant::now();
                t.acked = offset.min(t.size);
                files.acked.notify_all();
                if !t.running && t.acked < t.size {
                    t.running = true;
                    let (ctx, id, at) = (ctx.clone(), id.clone(), t.acked);
                    std::thread::spawn(move || run_sender(ctx, id, at));
                }
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
                if msg.json("ok").as_bool() == Some(true) {
                    ctx.sink.emit(CoreEvent::FileSent {
                        id: id.clone(),
                        node_id: from,
                        name: t.name,
                    });
                    super::backup::outgoing_ended(ctx, &id, None);
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
                    let _ = std::fs::remove_file(&t.part);
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
