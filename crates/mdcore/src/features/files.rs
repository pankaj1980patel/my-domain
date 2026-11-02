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
}

struct Incoming {
    from: String,
    name: String,
    size: u64,
    sha: String,
    part: PathBuf,
    file: File,
    len: u64,
    hasher: Sha256,
    progress_at: Option<Instant>,
}

/// Transfers in progress, both ways.
#[derive(Default)]
pub struct Files {
    /// Where received files go; `None` = this device doesn't take files.
    pub download_dir: Mutex<Option<PathBuf>>,
    out: Mutex<HashMap<String, Outgoing>>,
    acked: Condvar,
    inc: Mutex<HashMap<String, Incoming>>,
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
    let clean = clean.trim().trim_start_matches('.').trim().to_string();
    if clean.is_empty() {
        "file".to_string()
    } else {
        clean
    }
}

/// `dir/name`, or `dir/name (1).ext`, ... if that's taken.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
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
pub fn start<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    path: &Path,
) -> crate::Result<String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("can't read the file: {e}"))?;
    if !meta.is_file() {
        return Err("not a file".to_string().into());
    }
    let id = crate::transport::outbox::new_id();
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
    let (ctx, node, path, id2) = (ctx.clone(), node.to_string(), path.to_path_buf(), id.clone());
    std::thread::spawn(move || {
        let sha = match sha256_file(&path) {
            Ok(s) => s,
            Err(e) => {
                return failed(&ctx, &id2, &node, &name, false, &format!("can't read: {e}"));
            }
        };
        let msg = json!({
            "type": "file_offer", "from": me(&ctx), "id": id2,
            "name": name, "size": size, "sha256": sha,
        });
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
            },
        );
        crate::engine::send_reliably(&ctx, &node, &msg.to_string());
    });
    Ok(id)
}

fn failed<P: Platform>(ctx: &NetCtx<P>, id: &str, node: &str, name: &str, incoming: bool, why: &str) {
    ctx.sink.emit(CoreEvent::FileFailed {
        id: id.to_string(),
        node_id: node.to_string(),
        name: name.to_string(),
        incoming,
        reason: why.to_string(),
    });
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
    let stop = |ctx: &NetCtx<P>| {
        if let Some(t) = ctx.files.out.lock().get_mut(&id) {
            t.running = false;
        }
    };
    let Ok(mut file) = File::open(&path) else {
        stop(&ctx);
        return;
    };
    let mut pos = from;
    let mut buf = vec![0u8; CHUNK];
    let mut stalls = 0;
    let mut waited = Duration::ZERO;
    loop {
        // Wait for room in the window (or the end), rewinding on a stall.
        {
            let mut out = files.out.lock();
            let Some(t) = out.get(&id) else { return };
            let acked = t.acked;
            if acked >= size {
                return; // all there; `file_done` finishes it
            }
            if pos >= size || pos.saturating_sub(acked) >= WINDOW {
                let timed_out = files.acked.wait_for(&mut out, LINK_CHECK).timed_out();
                let Some(t) = out.get_mut(&id) else { return };
                if !ctx.ws_conns.lock().contains_key(&node) {
                    t.running = false; // link gone: offered again on the next
                    return;
                }
                waited = if timed_out { waited + LINK_CHECK } else { Duration::ZERO };
                if waited < STALL {
                    continue;
                }
                waited = Duration::ZERO;
                if t.acked == acked {
                    stalls += 1;
                    if stalls >= MAX_STALLS {
                        t.running = false; // paused: offered again on a link
                        return;
                    }
                    pos = t.acked;
                } else {
                    stalls = 0;
                }
                continue;
            }
        }
        let n = match file.seek(SeekFrom::Start(pos)).and_then(|_| file.read(&mut buf)) {
            Ok(n) if n > 0 => n,
            _ => {
                stop(&ctx);
                return;
            }
        };
        let msg = json!({
            "type": "file_chunk", "from": me(&ctx), "id": id, "off": pos,
            "data": base64::engine::general_purpose::STANDARD.encode(&buf[..n]),
        });
        if send(&ctx, &node, &msg).is_err() {
            stop(&ctx); // no link: offered again when there is one
            return;
        }
        pos += n as u64;
    }
}

/// Offer paused transfers again (called on a timer); the receiver answers
/// with how much it already has. Over the peer's link if it has one;
/// otherwise once through the reliable path, which connects (waking the
/// device by push) and delivers the offer when the link is up.
pub fn resume_paused<P: Platform>(ctx: &NetCtx<P>) {
    let me = me(ctx);
    let mut offers = Vec::new();
    {
        let mut out = ctx.files.out.lock();
        for (id, t) in out.iter_mut() {
            if t.running || !t.started {
                continue;
            }
            // With no link, each try may wake the device by push: back off.
            let linked = ctx.ws_conns.lock().contains_key(&t.node);
            let every = if linked { REOFFER_EVERY } else { t.backoff };
            if t.offered_at.is_some_and(|at| at.elapsed() < every) {
                continue;
            }
            t.offered_at = Some(Instant::now());
            if !linked {
                t.backoff = (t.backoff * 2).min(MAX_BACKOFF);
            }
            let msg = json!({
                "type": "file_offer", "from": me, "id": id,
                "name": t.name, "size": t.size, "sha256": t.sha,
            });
            offers.push((t.node.clone(), msg, linked));
        }
    }
    for (node, msg, linked) in offers {
        if linked {
            let _ = send(ctx, &node, &msg);
        } else {
            crate::engine::send_reliably(ctx, &node, &msg.to_string());
        }
    }
}

/// Stop a transfer, either way, and tell the other side.
pub fn cancel<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let peer = if let Some(t) = ctx.files.out.lock().remove(id) {
        failed(ctx, id, &t.node, &t.name, false, "cancelled");
        Some(t.node)
    } else if let Some(t) = ctx.files.inc.lock().remove(id) {
        let _ = std::fs::remove_file(&t.part);
        failed(ctx, id, &t.from, &t.name, true, "cancelled");
        Some(t.from)
    } else {
        None
    };
    ctx.files.acked.notify_all();
    if let Some(node) = peer {
        let _ = send(ctx, &node, &json!({ "type": "file_cancel", "from": me(ctx), "id": id }));
    }
}

// --- receiving -------------------------------------------------------------

fn on_offer<P: Platform>(ctx: &NetCtx<P>, msg: &ControlMsg, from: &str) {
    let id = msg.str("id");
    let reply = |m: serde_json::Value| {
        let _ = send(ctx, from, &m);
    };
    // Ids are ours (hex); anything else could escape the partial folder.
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return;
    }
    let size = msg.json("size").as_u64().unwrap_or(0);
    let name = safe_name(&msg.str("name"));
    let sha = msg.str("sha256");
    let reject = |why: &str| {
        reply(json!({ "type": "file_done", "from": me(ctx), "id": id, "ok": false, "error": why }))
    };
    let Some(dir) = ctx.files.download_dir.lock().clone() else {
        return reject("this device can't receive files");
    };

    let mut inc = ctx.files.inc.lock();
    let offset = if let Some(t) = inc.get(&id) {
        t.len
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
            drop(inc);
            return reject("can't write to the downloads folder");
        };
        // A partial file from before (an earlier link, or a restart):
        // resume after it, hashing what's already there.
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
        inc.insert(
            id.clone(),
            Incoming {
                from: from.to_string(),
                name: name.clone(),
                size,
                sha,
                part,
                file,
                len,
                hasher,
                progress_at: None,
            },
        );
        len
    };
    drop(inc);
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
    if off == t.len {
        let Ok(data) = base64::engine::general_purpose::STANDARD.decode(msg.str("data")) else {
            return;
        };
        if t.len + data.len() as u64 > t.size || t.file.write_all(&data).is_err() {
            return;
        }
        t.hasher.update(&data);
        t.len += data.len() as u64;
    }
    // Ack what we have: after a gap the sender rewinds to it.
    let (len, size, name) = (t.len, t.size, t.name.clone());
    let show = t.progress_at.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY) || len == size;
    if show {
        t.progress_at = Some(Instant::now());
    }
    drop(inc);
    let _ = send(ctx, from, &json!({ "type": "file_ack", "from": me(ctx), "id": id, "upto": len }));
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
    let dir = t.part.parent().and_then(Path::parent).map(Path::to_path_buf);
    let dest = dir.map(|d| free_path(&d, &t.name));
    let moved = match (&dest, ok) {
        (Some(dest), true) => std::fs::rename(&t.part, dest).is_ok(),
        _ => false,
    };
    let me = me(ctx);
    if moved {
        let path = dest.unwrap_or_default();
        ctx.sink.emit(CoreEvent::FileReceived {
            id: id.to_string(),
            from: t.from.clone(),
            name: t.name.clone(),
            path: path.to_string_lossy().into_owned(),
            size: t.size,
        });
        let _ = send(ctx, &t.from, &json!({ "type": "file_done", "from": me, "id": id, "ok": true }));
    } else {
        let _ = std::fs::remove_file(&t.part);
        let why = if ok { "couldn't save the file" } else { "the file arrived damaged" };
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
        &["file_offer", "file_accept", "file_chunk", "file_ack", "file_done", "file_cancel"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.str("from");
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
                t.acked = upto.min(t.size);
                files.acked.notify_all();
                let show = t.progress_at.is_none_or(|at| at.elapsed() >= PROGRESS_EVERY);
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
                        id,
                        node_id: from,
                        name: t.name,
                    });
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
                    failed(ctx, &id, &from, &t.name, false, "cancelled on the other device");
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
                    failed(ctx, &id, &from, &t.name, true, "cancelled on the other device");
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
