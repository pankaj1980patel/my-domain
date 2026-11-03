//! Photo backup: the phone sends its new photos and videos to one chosen
//! computer, by itself, one file at a time, with the normal resumable file
//! push (`features::files`). The computer saves them under
//! `<downloads>/Phone backup/<phone name>/<YYYY-MM>/`.
//!
//! The host app finds new media (Android: MediaStore, from
//! `BackupStatus::since` on) and hands it over with `Engine::backup_enqueue`;
//! it reports the conditions with `Engine::backup_conditions` (on unmetered
//! Wi-Fi? charging?). The core decides when to send, and remembers every
//! item it sent (`kv`, survives restarts), so nothing goes twice:
//!
//! - Only while allowed: enabled, a computer chosen, unmetered Wi-Fi, and
//!   charging if the user asked for that. When that stops mid-file, the
//!   transfer is dropped quietly (`files::forget`); the computer keeps its
//!   partial file and the same transfer id resumes it later.
//! - Transfer ids come from (computer, item, attempt), so a file resumes
//!   across restarts of either side. A failed try waits `RETRY_AFTER` and
//!   goes again under a new id (the computer remembers how the old one
//!   ended); an item that fails `MAX_TRIES` times is left for the next
//!   scan.
//! - The offer carries `"backup": {"device", "month"}`; the receiver picks
//!   the folder from it (`offer_dest`), and answers "done" at once when the
//!   same file is already there (`already_there`): even a phone that lost
//!   its memory doesn't send a photo twice.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::{BackupItem, BackupStatus};
use crate::platform::Platform;

/// The folder under the computer's downloads that backups go into.
pub const FOLDER: &str = "Phone backup";
/// Wait after a failed try; doubled for each time in a row the computer
/// wasn't reachable (each try may wake it by push), up to `MAX_RETRY_AFTER`.
const RETRY_AFTER: Duration = Duration::from_secs(30);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
/// Tries per item before it's left for the next scan.
const MAX_TRIES: u32 = 5;
/// Sent keys per stored chunk (each new key rewrites only the last one).
const CHUNK_KEYS: usize = 500;
/// Most items waiting at once.
const MAX_QUEUE: usize = 20_000;

const SETTINGS_KEY: &str = "backup_settings";
const CHUNKS_KEY: &str = "backup_sent_chunks";

#[derive(Clone, Default, Serialize, Deserialize)]
struct Settings {
    enabled: bool,
    target: String,
    charging_only: bool,
    since: u64,
    #[serde(default)]
    last_sent_at: u64,
}

struct Current {
    id: String,
    key: String,
    name: String,
}

#[derive(Default)]
struct State {
    settings: Settings,
    sent: HashSet<String>,
    /// Stored chunks of `sent`; the last one is `last_chunk`.
    chunks: usize,
    last_chunk: Vec<String>,
    queue: VecDeque<BackupItem>,
    unmetered: bool,
    charging: bool,
    current: Option<Current>,
    tries: HashMap<String, u32>,
    retry_at: Option<Instant>,
    /// Tries in a row that ended "the device wasn't reachable".
    unreachable: u32,
    last_error: Option<String>,
    /// What the last `BackupState` event said (sent again only on change).
    shown: Option<BackupStatus>,
}

impl State {
    fn allowed(&self) -> bool {
        self.waiting_for().is_empty()
    }

    fn waiting_for(&self) -> &'static str {
        let s = &self.settings;
        if !s.enabled {
            "off"
        } else if s.target.is_empty() {
            "target"
        } else if !self.unmetered {
            "wifi"
        } else if s.charging_only && !self.charging {
            "charging"
        } else if self.retry_at.is_some_and(|at| Instant::now() < at) {
            "retry"
        } else {
            ""
        }
    }
}

/// The phone's backup state (in `NetCtx::extras`).
#[derive(Default)]
pub struct Backup {
    state: Mutex<State>,
    loaded: AtomicBool,
}

fn chunk_key(n: usize) -> String {
    format!("backup_sent_{n}")
}

/// A usable item key: not empty, not huge, no control characters.
pub fn is_item_key(key: &str) -> bool {
    !key.trim().is_empty() && key.len() <= 200 && !key.chars().any(char::is_control)
}

fn load<P: Platform>(ctx: &NetCtx<P>) {
    let b = &ctx.extras.backup;
    if b.loaded.swap(true, Ordering::SeqCst) {
        return;
    }
    let kv = |k: &str| ctx.platform.kv_get(k);
    let settings: Settings = kv(SETTINGS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let chunks: usize = kv(CHUNKS_KEY).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut sent = HashSet::new();
    let mut last_chunk = Vec::new();
    for n in 0..chunks {
        let keys: Vec<String> = kv(&chunk_key(n))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if n + 1 == chunks {
            last_chunk.clone_from(&keys);
        }
        sent.extend(keys);
    }
    let mut st = b.state.lock();
    st.settings = settings;
    st.sent = sent;
    st.chunks = chunks;
    st.last_chunk = last_chunk;
}

fn save_settings<P: Platform>(ctx: &NetCtx<P>, s: &Settings) {
    if let Ok(json) = serde_json::to_string(s) {
        ctx.platform.kv_set(SETTINGS_KEY, &json);
    }
}

fn status_of<P: Platform>(ctx: &NetCtx<P>, st: &State) -> BackupStatus {
    let s = &st.settings;
    let (done, total) = st
        .current
        .as_ref()
        .and_then(|c| super::files::outgoing_progress(ctx, &c.id))
        .unwrap_or_default();
    BackupStatus {
        enabled: s.enabled,
        target: s.target.clone(),
        charging_only: s.charging_only,
        since: s.since,
        queued: st.queue.len() as u32,
        sent: st.sent.len() as u32,
        waiting_for: st.waiting_for().to_string(),
        current: st.current.as_ref().map(|c| c.name.clone()),
        done,
        total,
        last_error: st.last_error.clone(),
        last_sent_at: s.last_sent_at,
    }
}

/// Where the backup stands now.
pub fn status<P: Platform>(ctx: &NetCtx<P>) -> BackupStatus {
    load(ctx);
    let st = ctx.extras.backup.state.lock();
    status_of(ctx, &st)
}

/// Emit `BackupState` if it changed since the last one.
fn announce<P: Platform>(ctx: &NetCtx<P>) {
    let changed = {
        let mut st = ctx.extras.backup.state.lock();
        let now = status_of(ctx, &st);
        if st.shown.as_ref() == Some(&now) {
            None
        } else {
            st.shown = Some(now.clone());
            Some(now)
        }
    };
    if let Some(s) = changed {
        ctx.sink.emit(CoreEvent::BackupState(s));
    }
}

/// Turn the backup on or off, and choose the computer. Turning it on the
/// first time sets `since` to now: photos added from then on are backed up.
pub fn configure<P: Platform>(
    ctx: &NetCtx<P>,
    enabled: bool,
    target: &str,
    charging_only: bool,
) -> crate::Result<()> {
    load(ctx);
    if enabled && target.is_empty() {
        return Err("choose the computer to back up to".to_string().into());
    }
    if !target.is_empty() && !ctx.peers.lock().contains_key(target) {
        return Err(crate::error::CoreError::PeerNotFound);
    }
    let stop = {
        let mut st = ctx.extras.backup.state.lock();
        let s = &mut st.settings;
        let moved = s.target != target;
        s.enabled = enabled;
        s.target = target.to_string();
        s.charging_only = charging_only;
        if enabled && s.since == 0 {
            s.since = crate::transport::now_secs();
        }
        let saved = s.clone();
        save_settings(ctx, &saved);
        st.retry_at = None;
        // Off, or another computer: the file going now stops.
        if !enabled || moved {
            st.current.take()
        } else {
            None
        }
    };
    if let Some(c) = stop {
        super::files::forget(ctx, &c.id);
    }
    tick(ctx);
    announce(ctx);
    Ok(())
}

/// The phone is on unmetered Wi-Fi (`unmetered`) and/or charging.
pub fn conditions<P: Platform>(ctx: &NetCtx<P>, unmetered: bool, charging: bool) {
    load(ctx);
    {
        let mut st = ctx.extras.backup.state.lock();
        st.unmetered = unmetered;
        st.charging = charging;
    }
    tick(ctx);
    announce(ctx);
}

/// New photos/videos to back up; ones already sent or waiting are skipped.
/// Returns how many were added.
pub fn enqueue<P: Platform>(ctx: &NetCtx<P>, items: Vec<BackupItem>) -> u32 {
    load(ctx);
    let added = {
        let mut st = ctx.extras.backup.state.lock();
        let mut waiting: HashSet<String> = st.queue.iter().map(|i| i.key.clone()).collect();
        if let Some(c) = &st.current {
            waiting.insert(c.key.clone());
        }
        let mut added = 0;
        for item in items {
            if !is_item_key(&item.key)
                || item.path.is_empty()
                || st.sent.contains(&item.key)
                || waiting.contains(&item.key)
                || st.queue.len() >= MAX_QUEUE
            {
                continue;
            }
            waiting.insert(item.key.clone());
            st.tries.remove(&item.key);
            st.queue.push_back(item);
            added += 1;
        }
        added
    };
    if added > 0 {
        tick(ctx);
        announce(ctx);
    }
    added
}

/// The transfer id for `key` to `target`, try `attempt`.
fn transfer_id(target: &str, key: &str, attempt: u32) -> String {
    let h = Sha256::digest(format!("backup\n{target}\n{key}\n{attempt}").as_bytes());
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// "YYYY-MM" of unix time `secs` (UTC).
pub fn month_of(secs: u64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}")
}

/// A "YYYY-MM" month.
pub fn is_month(m: &str) -> bool {
    let b = m.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || c.is_ascii_digit())
        && matches!(m[5..].parse::<u8>(), Ok(1..=12))
}

/// Start the next file if allowed and nothing is going; stop the one going
/// if no longer allowed. Called on a timer and after every change.
pub fn tick<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.extras.backup.loaded.load(Ordering::SeqCst) {
        return; // never configured in this run: nothing to do
    }
    // Decide under the lock; act (files, events) outside it.
    enum Step {
        Stop(String),
        Start {
            target: String,
            item: BackupItem,
            id: String,
        },
    }
    loop {
        let step = {
            let mut st = ctx.extras.backup.state.lock();
            if st.current.is_some() {
                if st.allowed() {
                    return;
                }
                st.current.take().map(|c| Step::Stop(c.id))
            } else if !st.allowed() {
                return;
            } else {
                st.retry_at = None;
                let Some(item) = st.queue.front().cloned() else {
                    return;
                };
                if !Path::new(&item.path).is_file() {
                    // Deleted before it went: nothing to back up.
                    st.queue.pop_front();
                    continue;
                }
                let attempt = st.tries.get(&item.key).copied().unwrap_or(0);
                let target = st.settings.target.clone();
                let id = transfer_id(&target, &item.key, attempt);
                st.current = Some(Current {
                    id: id.clone(),
                    key: item.key.clone(),
                    name: super::files::safe_name(
                        &Path::new(&item.path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    ),
                });
                Some(Step::Start { target, item, id })
            }
        };
        match step {
            None => return,
            Some(Step::Stop(id)) => {
                super::files::forget(ctx, &id);
                announce(ctx);
                return;
            }
            Some(Step::Start { target, item, id }) => {
                let month = item
                    .month
                    .clone()
                    .filter(|m| is_month(m))
                    .unwrap_or_else(|| month_of(item.taken));
                let device = ctx.identity.lock().name.clone();
                let extra = json!({ "backup": { "device": device, "month": month } });
                // A transfer under this id may still be in the files table
                // (the backup was turned off and on): start it afresh.
                super::files::forget(ctx, &id);
                let started = super::files::start_with_extra(
                    ctx,
                    &target,
                    Path::new(&item.path),
                    &id,
                    Some(extra),
                );
                if let Err(e) = started {
                    ended(ctx, &id, Some(&e.to_string()));
                }
                announce(ctx);
                return;
            }
        }
    }
}

/// The background timer (every couple of seconds): start or stop files as
/// the conditions say, and report progress.
pub fn on_timer<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.extras.backup.loaded.load(Ordering::SeqCst) {
        return;
    }
    tick(ctx);
    announce(ctx);
}

/// An outgoing transfer ended (`why`: `None` = the computer saved it).
/// Called by `features::files` for every outgoing transfer.
pub fn outgoing_ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>) {
    if !ctx.extras.backup.loaded.load(Ordering::SeqCst) {
        return;
    }
    if ended(ctx, id, why) {
        tick(ctx);
        announce(ctx);
    }
}

/// Book the end of backup transfer `id`; false if it isn't the current one.
fn ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>) -> bool {
    let b = &ctx.extras.backup;
    let mut st = b.state.lock();
    if st.current.as_ref().is_none_or(|c| c.id != id) {
        return false;
    }
    let Some(cur) = st.current.take() else {
        return false;
    };
    match why {
        None => {
            st.queue.retain(|i| i.key != cur.key);
            st.tries.remove(&cur.key);
            st.last_error = None;
            st.unreachable = 0;
            st.sent.insert(cur.key.clone());
            // Remember it: append to the last stored chunk (a new one when
            // that's full), then the chunk count.
            if st.last_chunk.len() >= CHUNK_KEYS || st.chunks == 0 {
                st.last_chunk.clear();
                st.chunks += 1;
            }
            st.last_chunk.push(cur.key);
            let n = st.chunks;
            if let Ok(json) = serde_json::to_string(&st.last_chunk) {
                ctx.platform.kv_set(&chunk_key(n - 1), &json);
            }
            ctx.platform.kv_set(CHUNKS_KEY, &n.to_string());
            st.settings.last_sent_at = crate::transport::now_secs();
            let s = st.settings.clone();
            save_settings(ctx, &s);
        }
        Some(why) => {
            st.last_error = Some(format!("{}: {why}", cur.name));
            let unreachable = why == "the device wasn't reachable";
            st.unreachable = if unreachable { st.unreachable + 1 } else { 0 };
            let wait = RETRY_AFTER
                .saturating_mul(1 << st.unreachable.saturating_sub(1).min(10))
                .min(MAX_RETRY_AFTER);
            st.retry_at = Some(Instant::now() + wait);
            // Unreachable: the same id resumes the partial file. Anything
            // else ended that id for good on the computer: a new one.
            if !unreachable {
                let tries = st.tries.entry(cur.key.clone()).or_insert(0);
                *tries += 1;
                if *tries >= MAX_TRIES {
                    st.queue.retain(|i| i.key != cur.key);
                }
            }
        }
    }
    true
}

// --- the computer's side ------------------------------------------------------

/// The folder an offer's file goes into: the downloads folder, or for a
/// backup `<downloads>/Phone backup/<device>/<YYYY-MM>`.
pub fn offer_dest(dir: &Path, msg: &super::ControlMsg) -> PathBuf {
    let b = msg.json("backup");
    if !b.is_object() {
        return dir.to_path_buf();
    }
    let device = b["device"].as_str().unwrap_or_default();
    let device = if device.trim().is_empty() {
        "Phone".to_string()
    } else {
        super::files::safe_name(device)
    };
    let month = b["month"]
        .as_str()
        .filter(|m| is_month(m))
        .unwrap_or("Unknown date");
    dir.join(FOLDER).join(device).join(month)
}

/// The same file (name, size and SHA-256) is already in `dest`.
pub fn already_there(dest: &Path, name: &str, size: u64, sha: &str) -> bool {
    let file = dest.join(name);
    std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() == size)
        && super::files::sha256_file(&file).is_ok_and(|h| h == sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_from_unix_time() {
        assert_eq!(month_of(0), "1970-01");
        assert_eq!(month_of(951_782_400), "2000-02"); // 2000-02-29
        assert_eq!(month_of(1_793_750_400), "2026-11"); // 2026-11-03
        assert_eq!(month_of(1_798_761_599), "2026-12"); // 2026-12-31 23:59:59
        assert_eq!(month_of(1_798_761_600), "2027-01");
    }

    #[test]
    fn month_names_are_checked() {
        assert!(is_month("2026-11"));
        assert!(is_month("1999-01"));
        for bad in [
            "2026-13", "2026-00", "2026/11", "26-11", "2026-1", "../..", "2026-1x",
        ] {
            assert!(!is_month(bad), "{bad}");
        }
    }

    #[test]
    fn transfer_ids_are_stable_and_change_per_try() {
        let a = transfer_id("laptop", "42:1700000000", 0);
        assert_eq!(a, transfer_id("laptop", "42:1700000000", 0));
        assert_ne!(a, transfer_id("laptop", "42:1700000000", 1));
        assert_ne!(a, transfer_id("desk", "42:1700000000", 0));
        assert!(super::super::files::is_transfer_id(&a));
    }

    #[test]
    fn waiting_for_says_why() {
        let mut st = State::default();
        assert_eq!(st.waiting_for(), "off");
        st.settings.enabled = true;
        assert_eq!(st.waiting_for(), "target");
        st.settings.target = "laptop".into();
        assert_eq!(st.waiting_for(), "wifi");
        st.unmetered = true;
        assert!(st.allowed());
        st.settings.charging_only = true;
        assert_eq!(st.waiting_for(), "charging");
        st.charging = true;
        assert!(st.allowed());
        st.retry_at = Some(Instant::now() + Duration::from_secs(5));
        assert_eq!(st.waiting_for(), "retry");
    }
}
