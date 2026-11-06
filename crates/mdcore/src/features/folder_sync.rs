//! Synced folders: one folder kept the same on two of your devices (say
//! `~/Documents/Notes` on the Mac and a folder on the phone), both ways,
//! over the same end-to-end encrypted links as everything else. No cloud
//! storage: the files only ever go from one device to the other.
//!
//! **A pair** is (local folder, the other device, the pair id both sides
//! share, this side's mode). One device creates it (`create`: the folder,
//! the device, the mode) and invites the other; the other accepts and picks
//! where the folder goes there (`accept`), or declines. The modes:
//! `both` (changes go both ways), `send` (this side only sends its changes)
//! and `receive` (this side only takes the other's); the invited side gets
//! the mirror of the creator's mode.
//!
//!   sync_invite   { from, pair, name, mode }        creator → other
//!   sync_accept   { from, pair }                    other → creator
//!   sync_remove   { from, pair }                    either way (also "no")
//!   sync_manifest { from, pair, round, page, pages, reply,
//!                   files: [[path, size, mtime_ms, sha256], ...] }
//!   sync_get      { from, pair, files: [[path, sha256, id], ...] }
//!   sync_nope     { from, pair, ids: [...] }        "can't send those"
//!
//! **State, per pair, on each side**: an index kept in the folder itself
//! (`<folder>/.mydomain-sync/<pair>.json`): for every relative path the
//! size, modification time and SHA-256 seen at the last scan (a file whose
//! size and time didn't change isn't hashed again), and the *base*: what
//! both sides last agreed that path holds (a hash, "absent", or never
//! agreed). Comparing each side with its base tells who changed what since
//! the last sync, without clocks having to agree.
//!
//! **A round**: every `CoreConfig::sync_every` (30 s) while there's a live
//! link to the other device (never waking it just for this), a side scans
//! its folder (a plain walk: no watcher crate) and sends its manifest (the
//! files it has: path, size, time, hash) with `reply`. The other side
//! scans too, decides what it has to do (`plan`), does it, and answers
//! with its own manifest; the first side then decides likewise. Each side
//! only ever changes its own folder:
//!
//! - the same on both sides: remembered as the new base;
//! - changed only here: nothing to do here (the other side fetches it);
//! - changed only there: fetched (`sync_get`; the other side sends it with
//!   the normal resumable file transfer, `features::files`, so a dropped
//!   link resumes and the SHA-256 is checked), or, if it was deleted there,
//!   moved to `<folder>/.mydomain-trash/<date>/` here, kept 7 days, never
//!   deleted outright;
//! - changed on both sides (a conflict, or two folders with different
//!   files of the same name at the first sync): both are kept. The copy
//!   changed last keeps the name; the other is saved on both devices as
//!   "name (conflict from <device> <date>).ext" (the device and the date
//!   of the copy that lost, so both sides pick the same name). Nothing is
//!   ever silently overwritten: an incoming file that would replace one
//!   changed here since the decision goes in as a conflict copy instead.
//!   Deleted on one side and edited on the other: the edit wins.
//! - a rename is a delete and an add; one that only changes the letter
//!   case of the name (`Notes.txt` -> `notes.txt`), of a file the other
//!   side still has as it was, is just a rename there.
//!
//! **Letter case**: "Notes.txt" and "notes.txt" are two files on Linux and
//! in an Android app's own storage, but one on a Mac (APFS as it comes),
//! on Windows and in Android's shared storage. Each side asks its disk
//! once per folder (a file made in `.mydomain-sync`, looked up by its
//! upper-case name; `CoreConfig::sync_fold_case` says yes without asking).
//! Where the disk can't tell them apart, a fetched file whose path differs
//! only by case from another file here (or from another one fetched in
//! the same round) is saved as "name (case conflict from <device>).ext",
//! and one whose folder is spelled differently here goes in that folder as
//! it's spelled here. The index marks it (`Entry::a`) as holding the other
//! side's path: this side's manifest lists it under that path, so it's
//! neither fetched again every round nor sent back as a new file, and its
//! edits and deletes go both ways like any file's. Once its name is free
//! here (the other file went), it takes it. Nothing is deleted or
//! overwritten because of case: what's in the way stays, and an incoming
//! file that would replace it goes in as a conflict copy.
//!
//! **Unicode spelling**: "é" can be one character (NFC) or "e" and an
//! accent (NFD). A Mac's disk takes both for one name and may list a file
//! under either; Linux, Android and Windows keep them as two names. Names
//! are always compared in NFC (and in lower case where the disk ignores
//! case) and sent in NFC: a file whose name is spelled another way here is
//! marked (`Entry::a`) as holding its NFC path, so the other side's
//! "café.txt" is this side's "cafe\u{301}.txt", neither fetched again nor
//! sent to the trash, and its edits go both ways; a file listed under the
//! other spelling than at the last scan is the same file. Only where a
//! file of the NFC name is here too does the other keep its own spelling
//! on the wire. Each side asks its disk once per folder whether it takes
//! the two spellings for one name (`CoreConfig::sync_fold_norm` says yes
//! without asking); where it does, two incoming names that are one in NFC
//! are kept apart as for letter case ("name (case conflict from
//! <device>).ext"), never one over the other. A file kept under its own
//! name spelled another way isn't counted as renamed.
//!
//! **Left out**: hidden and system files (`.DS_Store`, `.git`, anything
//! starting with a dot, `Thumbs.db`, `desktop.ini`, `~$...` lock files,
//! `*.part`, the `.mydomain-*` folders), symlinks, names another system
//! couldn't keep (`a:b`), and files over the size limit (`set_max_size`,
//! 1 GB unless changed) or unreadable: those are counted as `skipped` and
//! the other side neither fetches nor deletes them.
//!
//! **Only to devices that understand it**: a device that does says
//! `"sync": 1` in its status (`CoreConfig::folder_sync`); the core
//! remembers what each peer said (or showed by sending a `sync_*`
//! message), and sends no `sync_*` message to a device that didn't say it:
//! an older app would show them as chat.
//!
//! Pausing a pair stops new rounds and serving files for it (a file half
//! way finishes); removing it (either side; the other is told) stops it for
//! good and forgets its state, never the files. Received files arrive in
//! `<folder>/.mydomain-sync/` first (same disk, so the final move is a
//! rename) and get the sender's modification time.
//!
//! **Saving battery** (`set_power`, from the phone's battery saver): while
//! heavy work is held back, no round starts by itself, the other side's
//! rounds aren't answered and its `sync_get`s aren't served (it asks again
//! later); "Sync now" still works. On a metered network the host may set a
//! size cap: bigger files are neither asked for nor sent until it's lifted
//! (they stay waiting, never dropped). A file half way finishes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_normalization::{is_nfc, UnicodeNormalization};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::SyncPair;
use crate::platform::Platform;

use super::files::{is_transfer_id, safe_folder, safe_name, sha256_file};
use super::{ControlHandler, ControlMsg};

/// What a device that syncs folders says in its status.
pub const FLAG: &str = "sync";
/// The version it says.
pub const VERSION: u64 = 1;
/// The folder (inside the synced one) for this feature's own files.
pub const STATE_DIR: &str = ".mydomain-sync";
/// Where files deleted on the other side go.
pub const TRASH_DIR: &str = ".mydomain-trash";
/// How long the trash keeps them.
const TRASH_DAYS: u64 = 7;
/// The size limit unless the user changed it.
pub const DEFAULT_MAX_SIZE: u64 = 1 << 30;
/// Most files in one synced folder.
pub const MAX_FILES: usize = 20_000;
/// Deepest subfolder walked into.
const MAX_DEPTH: usize = 32;
/// Manifest entries per message.
const PAGE: usize = 300;
/// Files asked for per `sync_get`.
const GET_PAGE: usize = 200;
/// A file asked for that hasn't come this long after: asked for again
/// (same transfer id, so it resumes).
const PULL_RETRY: u64 = 10 * 60;
/// A manifest whose pages didn't all come within this is dropped.
const MANIFEST_WAIT: Duration = Duration::from_secs(120);
/// Staged files and partial files nobody finished, older than this, go.
const STAGE_KEEP: Duration = Duration::from_secs(25 * 60 * 60);
/// Index hash for a file that's left out (too big, unreadable).
const EXCLUDED: &str = "-";
/// What's in a conflict copy's name.
const CONFLICT_MARK: &str = " (conflict from ";
/// What's in the name of a file kept under another name for letter case.
const CASE_MARK: &str = " (case conflict from ";

const PAIRS_KEY: &str = "sync_pairs";
const MAX_SIZE_KEY: &str = "sync_max_size";
const CAPS_PREFIX: &str = "sync_caps:";

/// One side's part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Both,
    Send,
    Receive,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "both" => Some(Mode::Both),
            "send" => Some(Mode::Send),
            "receive" => Some(Mode::Receive),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Both => "both",
            Mode::Send => "send",
            Mode::Receive => "receive",
        }
    }

    /// The other side's part.
    fn mirror(self) -> Mode {
        match self {
            Mode::Both => Mode::Both,
            Mode::Send => Mode::Receive,
            Mode::Receive => Mode::Send,
        }
    }

    /// This side applies the other's changes.
    fn takes(self) -> bool {
        self != Mode::Send
    }

    /// This side sends its files.
    fn gives(self) -> bool {
        self != Mode::Receive
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    /// The other device asked; waiting for this side's answer.
    Invited,
    /// Asked the other device; waiting for its answer.
    Pending,
    Active,
}

/// A pair as kept in the platform's store.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PairCfg {
    id: String,
    peer: String,
    name: String,
    #[serde(default)]
    path: String,
    mode: Mode,
    #[serde(default)]
    paused: bool,
    phase: Phase,
    #[serde(default)]
    last_synced: u64,
}

/// What the index knows about one path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// Size and modification time (ms since 1970) when it was hashed.
    pub(crate) s: u64,
    pub(crate) m: u64,
    /// SHA-256 (hex); "" = not here any more (a tombstone); `EXCLUDED` =
    /// here but left out.
    pub(crate) h: String,
    /// What both sides last agreed the path holds: `None` never agreed,
    /// `Some("")` absent on both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) b: Option<String>,
    /// When it went missing (unix seconds; tombstones only).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) t: u64,
    /// This file is the other side's file of this path, kept here under
    /// another name: its own differs only by letter case from another
    /// file's, and this disk can't tell them apart (or by Unicode
    /// spelling); or its name here is just spelled another way than the
    /// NFC path it's sent under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) a: Option<String>,
}

impl Entry {
    /// Kept here (at `key`) under a name of its own for the other side's
    /// path, not just under the same name spelled another way.
    fn renamed(&self, key: &str) -> bool {
        self.a.as_deref().is_some_and(|p| nfc(p) != nfc(key))
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One file of the other side's manifest.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Remote {
    pub(crate) s: u64,
    pub(crate) m: u64,
    /// SHA-256, or `EXCLUDED`.
    pub(crate) h: String,
}

/// How a fetched file goes into place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
pub(crate) enum How {
    /// Over what's there, which must still be `expect` (`None`: nothing
    /// there); else it goes in as a conflict copy.
    Replace { expect: Option<String> },
    /// What's there moves to `to` (a conflict copy) first.
    Displace { to: String },
    /// A conflict copy of the other side's file.
    Copy,
}

/// A file asked for, until it arrives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pull {
    /// The transfer id (chosen here, so a repeat resumes).
    id: String,
    /// Its path on the other side.
    src: String,
    sha: String,
    s: u64,
    m: u64,
    how: How,
    /// It goes in under another name than `src` because of letter case
    /// (`Entry::a`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    alias: bool,
    /// When it was last asked for (unix seconds).
    #[serde(default)]
    at: u64,
}

/// The index kept in the folder.
#[derive(Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
    /// Files on their way here, by where they go.
    #[serde(default)]
    pulls: BTreeMap<String, Pull>,
}

/// What a round decided for one path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    /// Both hold the same: remember it as agreed ("" = absent).
    Agree { path: String, base: String },
    /// Deleted on the other side: to the trash here.
    Trash { path: String },
    /// Fetch the other side's `src` into `target`.
    Pull {
        target: String,
        src: String,
        sha: String,
        s: u64,
        m: u64,
        how: How,
    },
}

/// One side, for naming conflict copies and breaking ties.
#[derive(Clone, Copy)]
pub(crate) struct Side<'a> {
    pub(crate) node: &'a str,
    pub(crate) name: &'a str,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Plan {
    pub(crate) acts: Vec<Act>,
    /// Paths where the two sides differ.
    pub(crate) differing: u32,
}

/// Decide what this side does after comparing its index (`local`) with
/// the other side's manifest. `takes`: this side applies the other's
/// changes (false for `send` mode: then it only remembers what agrees).
pub(crate) fn plan(
    local: &BTreeMap<String, Entry>,
    remote: &BTreeMap<String, Remote>,
    takes: bool,
    me: Side,
    peer: Side,
) -> Plan {
    let mut out = Plan::default();
    let paths: BTreeSet<&String> = local.keys().chain(remote.keys()).collect();
    for path in paths {
        let le = local.get(path);
        let re = remote.get(path);
        // A file left out on either side: no opinion either way.
        if le.is_some_and(|e| e.h == EXCLUDED) || re.is_some_and(|r| r.h == EXCLUDED) {
            continue;
        }
        let l = le.map(|e| e.h.as_str()).filter(|h| !h.is_empty());
        let r = re.map(|r| r.h.as_str());
        if l == r {
            if let Some(e) = le {
                let agreed = l.unwrap_or("");
                if e.b.as_deref() != Some(agreed) {
                    out.acts.push(Act::Agree {
                        path: path.clone(),
                        base: agreed.to_string(),
                    });
                }
            }
            continue;
        }
        out.differing += 1;
        if !takes {
            continue;
        }
        // `None`: never agreed; `Some(None)`: agreed absent.
        let base: Option<Option<&str>> = le
            .and_then(|e| e.b.as_deref())
            .map(|b| (!b.is_empty()).then_some(b));
        let changed_here = base.is_none_or(|b| b != l);
        let changed_there = base.is_none_or(|b| b != r);
        if !changed_there {
            continue; // only ours changed: the other side fetches it
        }
        let pull = |target: &str, how: How| {
            let re = re.cloned().unwrap_or(Remote {
                s: 0,
                m: 0,
                h: String::new(),
            });
            Act::Pull {
                target: target.to_string(),
                src: path.clone(),
                sha: re.h,
                s: re.s,
                m: re.m,
                how,
            }
        };
        match (l, r) {
            (Some(_), None) if !changed_here => out.acts.push(Act::Trash { path: path.clone() }),
            (_, Some(_)) if !changed_here => out.acts.push(pull(
                path,
                How::Replace {
                    expect: l.map(str::to_string),
                },
            )),
            // Edited here, deleted there: the edit stays (and goes back).
            (Some(_), None) => {}
            // Deleted here, edited (or added) there: the edit comes.
            (None, Some(_)) => out.acts.push(pull(path, How::Replace { expect: None })),
            (Some(_), Some(rh)) => {
                let (Some(le), Some(re)) = (le, re) else {
                    continue;
                };
                // The copy changed last keeps the name (ties: the larger
                // node id), on both sides alike.
                if (le.m, me.node) > (re.m, peer.node) {
                    let target = conflict_name(path, peer.name, re.m);
                    if local.get(&target).is_some_and(|e| e.h == rh) {
                        continue; // already here
                    }
                    out.acts.push(pull(&target, How::Copy));
                } else {
                    let to = conflict_name(path, me.name, le.m);
                    out.acts.push(pull(path, How::Displace { to }));
                }
            }
            (None, None) => {}
        }
    }
    out
}

/// "dir/name (conflict from <device> <YYYY-MM-DD>).ext" for `rel`, the
/// date of `mtime_ms` (UTC).
pub(crate) fn conflict_name(rel: &str, device: &str, mtime_ms: u64) -> String {
    let (dir, name) = match rel.rsplit_once('/') {
        Some((d, n)) => (Some(d), n),
        None => (None, rel),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let device = safe_name(device);
    let date = date_of(mtime_ms / 1000);
    let file = safe_name(&format!("{stem}{CONFLICT_MARK}{device} {date}){ext}"));
    match dir {
        Some(d) => format!("{d}/{file}"),
        None => file,
    }
}

/// "YYYY-MM-DD" of unix time `secs` (UTC).
pub(crate) fn date_of(secs: u64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A file or folder name that's left out of syncing.
pub fn ignored(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || lower.ends_with(".part")
}

/// A name that syncs: not left out, and one every system can keep.
fn fits(name: &str) -> bool {
    !ignored(name) && safe_name(name) == name
}

/// A relative path from the other side that may be written here.
pub(crate) fn rel_ok(rel: &str) -> bool {
    safe_folder(rel).is_some() && rel.split('/').all(fits)
}

fn is_sha(h: &str) -> bool {
    h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_pair_id(id: &str) -> bool {
    is_transfer_id(id) && id.len() <= 32
}

fn mtime_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    crate::transport::now_secs()
}

// --- scanning ------------------------------------------------------------------

#[derive(Default)]
struct ScanCounts {
    skipped: u32,
}

/// Walk `root` and bring `entries` up to date: new and changed files are
/// hashed, missing ones become tombstones, settled tombstones go.
fn scan(
    root: &Path,
    entries: &mut BTreeMap<String, Entry>,
    max_size: u64,
    now: u64,
) -> Result<ScanCounts, String> {
    let mut found: Vec<(String, PathBuf, u64, u64)> = Vec::new();
    walk(root, "", 0, &mut found).map_err(|e| format!("can't read the folder: {e}"))?;
    if found.len() > MAX_FILES {
        return Err(format!("the folder has more than {MAX_FILES} files"));
    }
    let seen: HashSet<String> = found.iter().map(|f| f.0.clone()).collect();
    // What the index knows that isn't listed now, by its name in NFC: a
    // file listed under that name spelled another way is the same file.
    let mut gone: HashMap<String, String> = entries
        .keys()
        .filter(|k| !seen.contains(*k))
        .map(|k| (nfc(k), k.clone()))
        .collect();
    // The paths the other side knows the files here by.
    let mut wires: HashSet<String> = entries
        .iter()
        .map(|(k, e)| e.a.clone().unwrap_or_else(|| k.clone()))
        .collect();
    let mut counts = ScanCounts::default();
    for (rel, path, size, mtime) in found {
        let mut old = entries.get(&rel).cloned();
        if old.is_none() {
            let n = nfc(&rel);
            let moved = gone
                .remove(&n)
                .and_then(|k| entries.remove(&k).map(|e| (k, e)));
            if let Some((k, e)) = moved {
                // The same file: it keeps the path it's known by.
                let wire = e.a.clone().unwrap_or(k);
                old = Some(Entry {
                    a: (wire != rel).then_some(wire),
                    ..e
                });
            } else if n != rel && !wires.contains(&n) && !seen.contains(&n) {
                // Spelled otherwise than in NFC: sent under its NFC name.
                old = Some(Entry {
                    a: Some(n.clone()),
                    ..Entry::default()
                });
                wires.insert(n);
            }
        }
        let same = old
            .as_ref()
            .is_some_and(|e| e.s == size && e.m == mtime && !e.h.is_empty() && e.h != EXCLUDED);
        if same && size <= max_size {
            if let Some(e) = old {
                entries.insert(rel, e);
            }
            continue;
        }
        let h = if size > max_size {
            EXCLUDED.to_string()
        } else {
            sha256_file(&path).unwrap_or_else(|_| EXCLUDED.to_string())
        };
        if h == EXCLUDED {
            counts.skipped += 1;
        }
        let (b, a) = old.map(|e| (e.b, e.a)).unwrap_or_default();
        entries.insert(
            rel,
            Entry {
                s: size,
                m: mtime,
                h,
                b,
                t: 0,
                a,
            },
        );
    }
    entries.retain(|rel, e| {
        if seen.contains(rel) {
            return true;
        }
        if !e.h.is_empty() {
            e.h.clear();
            e.t = now;
        }
        // A tombstone matters only while the other side may still have
        // what both last agreed on.
        e.b.as_deref().is_some_and(|b| !b.is_empty())
    });
    // A file standing for another path, once a file of that very name is
    // here too: that file takes the path (and what both last agreed it
    // holds), and this one goes by its own name from now on.
    let taken: Vec<(String, String)> = entries
        .iter()
        .filter_map(|(k, e)| {
            let p = e.a.as_ref()?;
            let plain = entries
                .get(p)
                .is_some_and(|o| o.a.is_none() && !o.h.is_empty());
            (p != k && plain).then(|| (k.clone(), p.clone()))
        })
        .collect();
    for (k, p) in taken {
        let Some(e) = entries.remove(&k) else {
            continue;
        };
        if let Some(o) = entries.get_mut(&p) {
            if o.b.is_none() {
                o.b.clone_from(&e.b);
            }
        }
        if !e.h.is_empty() {
            entries.insert(
                k,
                Entry {
                    a: None,
                    b: None,
                    ..e
                },
            );
        }
    }
    Ok(counts)
}

/// The files under `dir`, each as (relative path, path, size, mtime ms).
fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    out: &mut Vec<(String, PathBuf, u64, u64)>,
) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)?.filter_map(Result::ok) {
        if out.len() > MAX_FILES {
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if !fits(&name) {
            continue;
        }
        let Ok(kind) = e.file_type() else { continue };
        let sub = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if kind.is_file() {
            if let Ok(meta) = e.metadata() {
                out.push((sub, e.path(), meta.len(), mtime_ms(&meta)));
            }
        } else if kind.is_dir() && depth < MAX_DEPTH {
            // A subfolder we may not read is left out, not the whole scan.
            let _ = walk(&e.path(), &sub, depth + 1, out);
        }
    }
    Ok(())
}

// --- letter case ---------------------------------------------------------------

/// `s` in NFC ("é" as one character), as names are sent.
pub(crate) fn nfc(s: &str) -> String {
    if is_nfc(s) {
        s.to_string()
    } else {
        s.nfc().collect()
    }
}

/// How names are compared: always in NFC, and in lower case where the
/// disk ignores letter case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fold {
    case: bool,
}

impl Fold {
    /// The loosest: names any disk may take for one file.
    const LOOSE: Fold = Fold { case: true };

    /// `rel` as compared.
    fn key(self, rel: &str) -> String {
        let n = nfc(rel);
        if self.case {
            n.to_lowercase()
        } else {
            n
        }
    }
}

/// What the disk of a synced folder takes for one name.
#[derive(Clone, Copy, Debug, Default)]
struct Disk {
    /// Names that differ only by letter case.
    case: bool,
    /// Names that differ only by Unicode spelling (NFC / NFD).
    norm: bool,
}

impl Disk {
    /// Two different names can be one file here.
    fn merges(self) -> bool {
        self.case || self.norm
    }

    fn fold(self) -> Fold {
        Fold { case: self.case }
    }
}

/// (folder, name) of a relative path ("" for the top).
fn split_rel(rel: &str) -> (&str, &str) {
    rel.rsplit_once('/').unwrap_or(("", rel))
}

/// The disk under `root` takes `made` and `other` for one file: a file
/// made in the state folder as `made` is looked up as `other` (`guess`
/// if it can't be made).
fn same_file(root: &Path, made: &str, other: &str, guess: bool) -> bool {
    let dir = root.join(STATE_DIR);
    let path = dir.join(made);
    let ok = std::fs::create_dir_all(&dir).is_ok()
        && std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_ok();
    if !ok {
        return guess;
    }
    let same = std::fs::symlink_metadata(dir.join(other)).is_ok();
    let _ = std::fs::remove_file(&path);
    same
}

/// The disk under `root` takes names that differ only by letter case for
/// one file.
fn disk_folds_case(root: &Path) -> bool {
    let name = format!("case-probe-{}", crate::transport::outbox::new_id());
    // Can't tell: what these systems' disks usually do.
    let guess = cfg!(any(target_os = "macos", target_os = "windows"));
    same_file(root, &name, &name.to_uppercase(), guess)
}

/// The disk under `root` takes "é" as one character and as "e" and an
/// accent for one name.
fn disk_folds_norm(root: &Path) -> bool {
    let id = crate::transport::outbox::new_id();
    let (made, other) = (
        format!("norm-probe-{id}-\u{e9}"),
        format!("norm-probe-{id}-e\u{301}"),
    );
    same_file(root, &made, &other, cfg!(target_os = "macos"))
}

/// What the disk of the folder at `root` (pair `slot`'s) takes for one
/// name (asked once).
fn disk_of<P: Platform>(ctx: &NetCtx<P>, slot: &Slot, root: &Path) -> Disk {
    let c = &ctx.config;
    *slot.disk.lock().get_or_insert_with(|| Disk {
        case: c.sync_fold_case || disk_folds_case(root),
        norm: c.sync_fold_norm || disk_folds_norm(root),
    })
}

/// "dir/name (case conflict from <device>).ext" for `rel` (`n` > 1: with
/// the number too, "... <device> 2).ext").
pub(crate) fn case_name(rel: &str, device: &str, n: u32) -> String {
    let (dir, name) = split_rel(rel);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let device = safe_name(device);
    let num = if n > 1 {
        format!(" {n}")
    } else {
        String::new()
    };
    let file = safe_name(&format!("{stem}{CASE_MARK}{device}{num}){ext}"));
    if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    }
}

/// The index as the other side knows it: a file kept here under another
/// name for letter case (`Entry::a`) stands under the other side's path
/// (unless a file of that very name is here). Returns that view, and for
/// those files, the other side's path -> the path here.
pub(crate) fn local_view(
    entries: &BTreeMap<String, Entry>,
) -> (BTreeMap<String, Entry>, HashMap<String, String>) {
    let mut view: BTreeMap<String, Entry> = entries
        .iter()
        .filter(|(_, e)| e.a.is_none())
        .map(|(k, e)| (k.clone(), e.clone()))
        .collect();
    let mut keys = HashMap::new();
    for (k, e) in entries {
        let Some(p) = &e.a else { continue };
        let free = !keys.contains_key(p) && view.get(p).is_none_or(|r| r.h.is_empty());
        if p != k && rel_ok(p) && free {
            view.insert(p.clone(), e.clone());
            keys.insert(p.clone(), k.clone());
        }
    }
    (view, keys)
}

/// Where the other side's `src` is here: the file kept under another name
/// for it (`keys`, from `local_view`), or the file of that name.
fn local_key(
    entries: &BTreeMap<String, Entry>,
    keys: &HashMap<String, String>,
    src: &str,
) -> Option<String> {
    match keys.get(src) {
        Some(k) => Some(k.clone()),
        None => entries
            .get(src)
            .is_some_and(|e| e.a.is_none())
            .then(|| src.to_string()),
    }
}

/// Every folder the index knows, as it's spelled here, by its folded
/// spelling.
fn dir_map(entries: &BTreeMap<String, Entry>, fold: Fold) -> HashMap<String, String> {
    let mut dirs = HashMap::new();
    for (k, _) in entries.iter().filter(|(_, e)| !e.h.is_empty()) {
        for (i, _) in k.match_indices('/') {
            let prefix = &k[..i];
            dirs.entry(fold.key(prefix))
                .or_insert_with(|| prefix.to_string());
        }
    }
    dirs
}

/// `rel` with its folders spelled as they are here (a disk that ignores
/// case puts a file for "docs/a.txt" into "Docs" if that's there, a Mac's
/// one for "cafe\u{301}/a.txt" into "café"): from `dirs` (what's known,
/// and decided this round), else the disk; folders not here yet keep
/// `rel`'s spelling and go into `dirs`.
fn resolve_dirs(root: &Path, rel: &str, dirs: &mut HashMap<String, String>, fold: Fold) -> String {
    let (dir, name) = split_rel(rel);
    if dir.is_empty() {
        return rel.to_string();
    }
    let mut actual = String::new();
    for part in dir.split('/') {
        let wanted = if actual.is_empty() {
            part.to_string()
        } else {
            format!("{actual}/{part}")
        };
        let key = fold.key(&wanted);
        let got = match dirs.get(&key) {
            Some(a) => a.clone(),
            None => {
                let names: Vec<String> = std::fs::read_dir(root.join(&actual))
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                let folded = fold.key(part);
                let here = names
                    .iter()
                    .find(|n| *n == part)
                    .or_else(|| names.iter().find(|n| fold.key(n) == folded))
                    .map_or(part, String::as_str);
                let a = if actual.is_empty() {
                    here.to_string()
                } else {
                    format!("{actual}/{here}")
                };
                dirs.insert(key, a.clone());
                a
            }
        };
        actual = got;
    }
    format!("{actual}/{name}")
}

/// Rename `from` to `to` in the same folder, if `from` is still the file
/// `e` knows and no other file there has `to`'s name in any case or
/// Unicode spelling (so nothing is ever renamed over, whatever the disk
/// takes for one name).
fn rename_in_place(root: &Path, from: &str, to: &str, e: &Entry) -> bool {
    let ((dir, from_name), (to_dir, to_name)) = (split_rel(from), split_rel(to));
    if dir != to_dir || from == to {
        return false;
    }
    let src = root.join(from);
    let Ok(meta) = std::fs::symlink_metadata(&src) else {
        return false;
    };
    if !meta.is_file() || meta.len() != e.s || mtime_ms(&meta) != e.m {
        return false;
    }
    let Ok(names) = std::fs::read_dir(root.join(dir)) else {
        return false;
    };
    let folded = Fold::LOOSE.key(to_name);
    let in_the_way = names.flatten().any(|n| {
        let n = n.file_name().to_string_lossy().into_owned();
        n != from_name && Fold::LOOSE.key(&n) == folded
    });
    !in_the_way && std::fs::rename(&src, root.join(to)).is_ok()
}

/// Files kept under another name for letter case (or Unicode spelling)
/// take their own name once it's free here. One whose name here is just
/// its own spelled another way stays as it is: the disk may well list it
/// so whatever it's called.
fn settle_names(root: &Path, entries: &mut BTreeMap<String, Entry>, fold: Fold) {
    let kept: Vec<(String, String)> = entries
        .iter()
        .filter(|(k, e)| !e.h.is_empty() && e.renamed(k))
        .filter_map(|(k, e)| e.a.clone().map(|p| (k.clone(), p)))
        .collect();
    for (k, p) in kept {
        let folded = fold.key(&p);
        let taken = entries
            .iter()
            .any(|(o, e)| *o != k && !e.h.is_empty() && fold.key(o) == folded);
        let Some(e) = entries.get(&k).cloned() else {
            continue;
        };
        if taken || !rel_ok(&p) || !rename_in_place(root, &k, &p, &e) {
            continue;
        }
        entries.remove(&k);
        entries.insert(p, Entry { a: None, ..e });
    }
}

/// A file renamed there only in letter case or Unicode spelling (gone
/// under one name, come under the other with the same content) that's
/// here as it was under the old name: renamed here too, instead of going
/// to the trash while the same file is fetched again. With `dirs` (a disk
/// that takes different names for one) the new name's folders are
/// spelled as they are here (compared as `fold` says); when that leaves
/// the path as it is (only a folder's case changed there), the file just
/// stands for the new path.
fn case_renames(
    root: &Path,
    entries: &mut BTreeMap<String, Entry>,
    trash: &mut Vec<String>,
    pulls: &mut Vec<(String, Pull)>,
    mut dirs: Option<&mut HashMap<String, String>>,
    fold: Fold,
) {
    trash.retain(|from| {
        let Some(e) = entries
            .get(from)
            .filter(|e| e.a.is_none() && is_sha(&e.h))
            .cloned()
        else {
            return true;
        };
        let folded = Fold::LOOSE.key(from);
        let Some(i) = pulls.iter().position(|(t, p)| {
            !p.alias
                && *t == p.src
                && p.how == How::Replace { expect: None }
                && p.sha == e.h
                && Fold::LOOSE.key(t) == folded
        }) else {
            return true;
        };
        let target = pulls[i].0.clone();
        let dest = match dirs.as_deref_mut() {
            Some(d) => resolve_dirs(root, &target, d, fold),
            None => target.clone(),
        };
        if dest != *from && !rename_in_place(root, from, &dest, &e) {
            return true;
        }
        entries.remove(from);
        let h = e.h.clone();
        entries.insert(
            dest.clone(),
            Entry {
                b: Some(h),
                t: 0,
                a: (dest != target).then_some(target),
                ..e
            },
        );
        pulls.remove(i);
        false
    });
}

/// On a disk that takes different names for one file (letter case,
/// Unicode spelling: compared as `fold` says): a file coming to a path
/// that's new here, which is one with a file here (or with one coming
/// this round), goes in as "name (case conflict from <device>).ext"; one
/// whose folder is spelled differently here goes in that folder as it's
/// spelled here. Either way it's marked as standing for the other side's
/// path. Names in NFC come first, so of two that differ only by Unicode
/// spelling the NFC one keeps it. What's on its way already keeps where it
/// goes; a file no free name can be found for isn't fetched.
fn make_room(
    root: &Path,
    idx: &Index,
    mut pulls: Vec<(String, Pull)>,
    dirs: &mut HashMap<String, String>,
    peer: &str,
    fold: Fold,
) -> Vec<(String, Pull)> {
    let is_new =
        |t: &str, p: &Pull| !p.alias && t == p.src && p.how == How::Replace { expect: None };
    let going = |t: &str, p: &Pull| {
        idx.pulls
            .get(t)
            .is_some_and(|q| q.src == p.src && q.sha == p.sha && q.alias == p.alias)
    };
    pulls.sort_by_key(|(t, _)| !is_nfc(t));
    let mut taken: HashSet<String> = idx
        .entries
        .iter()
        .filter(|(_, e)| !e.h.is_empty())
        .map(|(k, _)| fold.key(k))
        .collect();
    for (t, p) in &pulls {
        if !is_new(t, p) || going(t, p) {
            taken.insert(fold.key(t));
        }
    }
    let mut out = Vec::with_capacity(pulls.len());
    for (mut t, mut p) in pulls {
        if is_new(&t, &p) && !going(&t, &p) {
            let kept = idx
                .pulls
                .iter()
                .find(|(_, q)| q.alias && q.src == p.src && q.sha == p.sha)
                .map(|(k, _)| k.clone());
            if let Some(k) = kept {
                t = k;
                p.alias = true;
            } else {
                let at = resolve_dirs(root, &t, dirs, fold);
                if taken.contains(&fold.key(&at)) {
                    // Its own name in NFC, in the folder as it's here.
                    let (dir, name) = split_rel(&at);
                    let at = if dir.is_empty() {
                        nfc(name)
                    } else {
                        format!("{dir}/{}", nfc(name))
                    };
                    let free = (1..1000)
                        .map(|n| case_name(&at, peer, n))
                        .find(|c| !taken.contains(&fold.key(c)) && !root.join(c).exists());
                    let Some(free) = free else { continue };
                    t = free;
                    p.alias = true;
                } else if at != t {
                    t = at;
                    p.alias = true;
                }
            }
            taken.insert(fold.key(&t));
        }
        out.push((t, p));
    }
    out
}

// --- state ---------------------------------------------------------------------

/// Per pair, in memory.
#[derive(Default)]
struct Slot {
    /// The index (`None` until read from the folder). Held for a whole
    /// round, so rounds and arriving files of one pair take turns.
    work: Mutex<Option<Index>>,
    live: Mutex<Live>,
    /// What the folder's disk takes for one name (`None` until asked).
    disk: Mutex<Option<Disk>>,
}

#[derive(Default)]
struct Live {
    last_round: Option<Instant>,
    error: Option<String>,
    files: u32,
    skipped: u32,
    conflicts: u32,
    renamed: u32,
    differing: u32,
    pulls: u32,
    /// Transfers sending this pair's files to the other side.
    serving: HashSet<String>,
}

/// A manifest whose pages are coming in.
struct Inbound {
    round: String,
    pages: Vec<Option<Vec<(String, Remote)>>>,
    reply: bool,
    at: Instant,
}

/// All synced folders (in `NetCtx::extras`).
#[derive(Default)]
pub struct FolderSync {
    loaded: AtomicBool,
    pairs: Mutex<BTreeMap<String, PairCfg>>,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    /// Whether each peer said it syncs (read from `kv` on first use).
    caps: Mutex<HashMap<String, bool>>,
    /// Transfer id -> pair, for files coming and going.
    transfers: Mutex<HashMap<String, String>>,
    inbound: Mutex<HashMap<String, Inbound>>,
    shown: Mutex<Option<Vec<SyncPair>>>,
    /// 0 = not read yet.
    max_size: AtomicU64,
    /// Heavy work is held back to save battery (`set_power`).
    held: AtomicBool,
    /// On a metered network: files bigger than this (bytes) wait; 0 = no cap.
    metered_cap: AtomicU64,
}

/// Hold back rounds (`held`) and cap file sizes on a metered network
/// (`metered_cap` bytes, 0 = none). Set by the host's battery saver.
pub fn set_power<P: Platform>(ctx: &NetCtx<P>, held: bool, metered_cap: u64) {
    let s = &ctx.extras.sync;
    s.held.store(held, Ordering::Relaxed);
    s.metered_cap.store(metered_cap, Ordering::Relaxed);
}

fn held<P: Platform>(ctx: &NetCtx<P>) -> bool {
    ctx.extras.sync.held.load(Ordering::Relaxed)
}

/// A file of `size` bytes waits under the metered cap `cap` (0 = none).
fn over_cap(size: u64, cap: u64) -> bool {
    cap != 0 && size > cap
}

fn load<P: Platform>(ctx: &NetCtx<P>) {
    let s = &ctx.extras.sync;
    if s.loaded.swap(true, Ordering::SeqCst) {
        return;
    }
    let stored: Vec<PairCfg> = ctx
        .platform
        .kv_get(PAIRS_KEY)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let mut pairs = s.pairs.lock();
    for p in stored {
        if is_pair_id(&p.id) {
            pairs.insert(p.id.clone(), p);
        }
    }
}

fn save_pairs<P: Platform>(ctx: &NetCtx<P>) {
    let list: Vec<PairCfg> = ctx.extras.sync.pairs.lock().values().cloned().collect();
    if let Ok(j) = serde_json::to_string(&list) {
        ctx.platform.kv_set(PAIRS_KEY, &j);
    }
}

fn cfg<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<PairCfg> {
    load(ctx);
    ctx.extras.sync.pairs.lock().get(id).cloned()
}

fn update_cfg<P: Platform>(ctx: &NetCtx<P>, id: &str, f: impl FnOnce(&mut PairCfg)) {
    let changed = {
        let mut pairs = ctx.extras.sync.pairs.lock();
        match pairs.get_mut(id) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        }
    };
    if changed {
        save_pairs(ctx);
    }
}

fn slot<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Arc<Slot> {
    ctx.extras
        .sync
        .slots
        .lock()
        .entry(id.to_string())
        .or_default()
        .clone()
}

fn index_path(root: &Path, id: &str) -> PathBuf {
    root.join(STATE_DIR).join(format!("{id}.json"))
}

fn load_index(root: &Path, id: &str) -> Index {
    std::fs::read_to_string(index_path(root, id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_index(root: &Path, id: &str, idx: &Index) {
    let path = index_path(root, id);
    let Ok(json) = serde_json::to_string(idx) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    let ok = std::fs::create_dir_all(root.join(STATE_DIR)).is_ok()
        && std::fs::write(&tmp, json).is_ok()
        && std::fs::rename(&tmp, &path).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> (String, String) {
    let id = ctx.identity.lock();
    (id.node_id.clone(), id.name.clone())
}

fn peer_name<P: Platform>(ctx: &NetCtx<P>, node: &str) -> String {
    ctx.peers
        .lock()
        .get(node)
        .map(|p| p.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "other device".to_string())
}

/// The size limit for synced files.
pub fn max_size<P: Platform>(ctx: &NetCtx<P>) -> u64 {
    let s = &ctx.extras.sync;
    let v = s.max_size.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let v = ctx
        .platform
        .kv_get(MAX_SIZE_KEY)
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_SIZE);
    s.max_size.store(v, Ordering::Relaxed);
    v
}

/// Change the size limit (bytes, at least 1 MB).
pub fn set_max_size<P: Platform>(ctx: &NetCtx<P>, bytes: u64) -> crate::Result<()> {
    if bytes < 1 << 20 {
        return Err("the size limit must be at least 1 MB".to_string().into());
    }
    ctx.extras.sync.max_size.store(bytes, Ordering::Relaxed);
    ctx.platform.kv_set(MAX_SIZE_KEY, &bytes.to_string());
    Ok(())
}

// --- who understands it ----------------------------------------------------------

/// Add `"sync": 1` to this device's status reply.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.folder_sync {
        status[FLAG] = VERSION.into();
    }
}

/// Remember whether `node` said it syncs folders (kept across restarts).
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, status: &Value) {
    let on = status[FLAG].as_u64().is_some_and(|v| v >= VERSION);
    set_caps(ctx, node, on);
}

fn set_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) {
    if supports(ctx, node) == on {
        return;
    }
    ctx.extras.sync.caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
}

/// `node` said it syncs folders.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if let Some(on) = ctx.extras.sync.caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    ctx.extras.sync.caps.lock().insert(node.to_string(), on);
    on
}

/// Send a `sync_*` message, only to a device that understands it.
/// `reliably`: through the outbox (waits for a link); else only over the
/// live link. False if it didn't go.
fn send<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value, reliably: bool) -> bool {
    if !ctx.config.folder_sync || !supports(ctx, node) {
        return false;
    }
    if reliably {
        crate::engine::send_reliably(ctx, node, &msg.to_string());
        return true;
    }
    let name = ctx.identity.lock().name.clone();
    crate::transport::send_payload(
        &ctx.ws_conns,
        &ctx.p2p_conns,
        &ctx.key,
        &name,
        node,
        &msg.to_string(),
    )
    .is_ok()
}

// --- status ------------------------------------------------------------------------

fn status_of(cfg: &PairCfg, live: &Live) -> SyncPair {
    let busy = live.pulls + live.serving.len() as u32;
    let state = match cfg.phase {
        Phase::Invited => "invited",
        Phase::Pending => "pending",
        Phase::Active if cfg.paused => "paused",
        Phase::Active if live.error.is_some() => "error",
        Phase::Active if busy > 0 || live.differing > 0 => "syncing",
        Phase::Active if cfg.last_synced == 0 => "waiting",
        Phase::Active => "in_sync",
    };
    SyncPair {
        id: cfg.id.clone(),
        peer: cfg.peer.clone(),
        name: cfg.name.clone(),
        path: cfg.path.clone(),
        mode: cfg.mode.as_str().to_string(),
        paused: cfg.paused,
        state: state.to_string(),
        syncing: busy.max(live.differing),
        files: live.files,
        conflicts: live.conflicts,
        skipped: live.skipped,
        renamed: live.renamed,
        last_synced: cfg.last_synced,
        error: live.error.clone(),
    }
}

/// Every pair, as the apps show them.
pub fn pairs<P: Platform>(ctx: &NetCtx<P>) -> Vec<SyncPair> {
    load(ctx);
    let cfgs: Vec<PairCfg> = ctx.extras.sync.pairs.lock().values().cloned().collect();
    cfgs.iter()
        .map(|c| {
            let slot = slot(ctx, &c.id);
            let live = slot.live.lock();
            status_of(c, &live)
        })
        .collect()
}

/// Emit `SyncPairs` if anything changed since the last one.
fn announce<P: Platform>(ctx: &NetCtx<P>) {
    let now = pairs(ctx);
    {
        let mut shown = ctx.extras.sync.shown.lock();
        if shown.as_ref() == Some(&now) {
            return;
        }
        *shown = Some(now.clone());
    }
    ctx.sink.emit(CoreEvent::SyncPairs { pairs: now });
}

// --- creating, answering, pausing, removing -------------------------------------------

/// The folder is inside another pair's folder, or holds one.
fn overlaps<P: Platform>(ctx: &NetCtx<P>, path: &Path, except: &str) -> bool {
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    ctx.extras.sync.pairs.lock().values().any(|p| {
        p.id != except
            && !p.path.is_empty()
            && std::fs::canonicalize(&p.path)
                .is_ok_and(|other| other.starts_with(&path) || path.starts_with(&other))
    })
}

/// Start syncing the folder at `path` with `peer` (`mode` "both", "send"
/// or "receive": this side's part). The other device is asked; the pair
/// starts once it accepts. Returns the pair id.
pub fn create<P: Platform>(
    ctx: &NetCtx<P>,
    peer: &str,
    path: &str,
    mode: &str,
) -> crate::Result<String> {
    load(ctx);
    if !ctx.config.folder_sync {
        return Err("synced folders are off".to_string().into());
    }
    let Some(mode) = Mode::parse(mode) else {
        return Err(format!("not a sync mode: {mode}").into());
    };
    if !ctx.peers.lock().contains_key(peer) {
        return Err(crate::error::CoreError::PeerNotFound);
    }
    if !supports(ctx, peer) {
        return Err("that device's app can't sync folders yet: update it"
            .to_string()
            .into());
    }
    let root = std::fs::canonicalize(path).map_err(|e| format!("can't read the folder: {e}"))?;
    if !root.is_dir() {
        return Err("not a folder".to_string().into());
    }
    if overlaps(ctx, &root, "") {
        return Err("that folder is already synced (or holds one that is)"
            .to_string()
            .into());
    }
    let name = safe_name(&root.file_name().unwrap_or_default().to_string_lossy());
    let id = crate::transport::outbox::new_id();
    let cfg = PairCfg {
        id: id.clone(),
        peer: peer.to_string(),
        name: name.clone(),
        path: root.to_string_lossy().into_owned(),
        mode,
        paused: false,
        phase: Phase::Pending,
        last_synced: 0,
    };
    ctx.extras.sync.pairs.lock().insert(id.clone(), cfg);
    save_pairs(ctx);
    let (node, _) = me(ctx);
    let msg = json!({
        "type": "sync_invite", "from": node, "pair": id,
        "name": name, "mode": mode.as_str(),
    });
    send(ctx, peer, &msg, true);
    announce(ctx);
    Ok(id)
}

/// Accept the other device's invitation `id`, keeping the folder at
/// `path` here (created if it isn't there; files already in it are
/// merged, never overwritten).
pub fn accept<P: Platform>(ctx: &NetCtx<P>, id: &str, path: &str) -> crate::Result<()> {
    let Some(c) = cfg(ctx, id) else {
        return Err("no such synced folder".to_string().into());
    };
    if c.phase != Phase::Invited {
        return Err("that folder isn't waiting for an answer".to_string().into());
    }
    if path.trim().is_empty() {
        return Err("choose a folder".to_string().into());
    }
    std::fs::create_dir_all(path).map_err(|e| format!("can't make the folder: {e}"))?;
    let root = std::fs::canonicalize(path).map_err(|e| format!("can't read the folder: {e}"))?;
    if !root.is_dir() {
        return Err("not a folder".to_string().into());
    }
    if overlaps(ctx, &root, id) {
        return Err("that folder is already synced (or holds one that is)"
            .to_string()
            .into());
    }
    update_cfg(ctx, id, |p| {
        p.path = root.to_string_lossy().into_owned();
        p.phase = Phase::Active;
    });
    let (node, _) = me(ctx);
    send(
        ctx,
        &c.peer,
        &json!({ "type": "sync_accept", "from": node, "pair": id }),
        true,
    );
    announce(ctx);
    sync_now(ctx, id)
}

/// Stop syncing `id` (or decline the invitation) on both devices. The
/// files stay where they are; only the pair's own state goes.
pub fn remove<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<()> {
    let Some(c) = forget(ctx, id) else {
        return Err("no such synced folder".to_string().into());
    };
    let (node, _) = me(ctx);
    send(
        ctx,
        &c.peer,
        &json!({ "type": "sync_remove", "from": node, "pair": id }),
        true,
    );
    announce(ctx);
    Ok(())
}

/// Drop pair `id` here: its transfers stop, its index goes.
fn forget<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<PairCfg> {
    load(ctx);
    let c = ctx.extras.sync.pairs.lock().remove(id)?;
    save_pairs(ctx);
    let slot = ctx.extras.sync.slots.lock().remove(id);
    ctx.extras.sync.inbound.lock().remove(id);
    let ids: Vec<String> = ctx
        .extras
        .sync
        .transfers
        .lock()
        .iter()
        .filter(|(_, p)| *p == id)
        .map(|(t, _)| t.clone())
        .collect();
    for t in &ids {
        ctx.extras.sync.transfers.lock().remove(t);
        super::files::cancel(ctx, t);
    }
    if let Some(slot) = slot {
        *slot.work.lock() = None;
    }
    if !c.path.is_empty() {
        let root = Path::new(&c.path);
        let _ = std::fs::remove_file(index_path(root, id));
        // The state folder too, once nothing else is in it.
        let _ = std::fs::remove_dir(root.join(STATE_DIR));
    }
    Some(c)
}

/// Pause or resume pair `id` (this device only).
pub fn set_paused<P: Platform>(ctx: &NetCtx<P>, id: &str, paused: bool) -> crate::Result<()> {
    if cfg(ctx, id).is_none() {
        return Err("no such synced folder".to_string().into());
    }
    update_cfg(ctx, id, |p| p.paused = paused);
    if !paused {
        slot(ctx, id).live.lock().last_round = None;
    }
    announce(ctx);
    Ok(())
}

/// Scan pair `id` and compare with the other device now (connecting to
/// it if there's no link).
pub fn sync_now<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<()> {
    let Some(c) = cfg(ctx, id) else {
        return Err("no such synced folder".to_string().into());
    };
    if c.phase != Phase::Active {
        return Err("that folder isn't synced yet".to_string().into());
    }
    if !crate::transport::is_live(ctx, &c.peer) {
        // The round runs once the link is up (`on_timer`).
        slot(ctx, id).live.lock().last_round = None;
        crate::engine::start_ladder(ctx, &c.peer);
        return Ok(());
    }
    spawn_round(ctx, id, None);
    Ok(())
}

/// The background timer (every couple of seconds): start the rounds that
/// are due, over live links only.
pub fn on_timer<P: Platform>(ctx: &NetCtx<P>) {
    if !ctx.config.folder_sync {
        return;
    }
    load(ctx);
    let every = ctx.config.sync_every;
    let cfgs: Vec<PairCfg> = ctx.extras.sync.pairs.lock().values().cloned().collect();
    let held = held(ctx);
    for c in cfgs {
        if c.phase != Phase::Active || c.paused || held {
            continue;
        }
        let due = {
            let slot = slot(ctx, &c.id);
            let mut live = slot.live.lock();
            let due = live.last_round.is_none_or(|t| t.elapsed() >= every);
            if due && crate::transport::is_live(ctx, &c.peer) && supports(ctx, &c.peer) {
                live.last_round = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if due {
            spawn_round(ctx, &c.id, None);
        }
    }
    ctx.extras
        .sync
        .inbound
        .lock()
        .retain(|_, m| m.at.elapsed() < MANIFEST_WAIT);
    announce(ctx);
}

fn spawn_round<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    remote: Option<(BTreeMap<String, Remote>, bool)>,
) {
    let (ctx, id) = (ctx.clone(), id.to_string());
    std::thread::spawn(move || round(&ctx, &id, remote));
}

// --- a round ---------------------------------------------------------------------------

/// Scan the folder; with the other side's manifest (`remote`), act on it;
/// then send this side's manifest (asking for an answer when this side
/// starts the round, or when `remote` asked for one).
fn round<P: Platform>(ctx: &NetCtx<P>, id: &str, remote: Option<(BTreeMap<String, Remote>, bool)>) {
    let Some(c) = cfg(ctx, id) else { return };
    if c.phase != Phase::Active || c.paused {
        return;
    }
    let slot = slot(ctx, id);
    // A round started here waits for none: one is going already.
    let mut work = if remote.is_some() {
        slot.work.lock()
    } else {
        match slot.work.try_lock() {
            Some(g) => g,
            None => return,
        }
    };
    slot.live.lock().last_round = Some(Instant::now());
    let root = PathBuf::from(&c.path);
    if !root.is_dir() {
        // Never compared while missing: an unplugged drive must not look
        // like every file was deleted.
        slot.live.lock().error = Some("the folder is missing".to_string());
        drop(work);
        announce(ctx);
        return;
    }
    let idx = work.get_or_insert_with(|| load_index(&root, id));
    let now = now_secs();
    let counts = match scan(&root, &mut idx.entries, max_size(ctx), now) {
        Ok(c) => c,
        Err(e) => {
            slot.live.lock().error = Some(e);
            drop(work);
            announce(ctx);
            return;
        }
    };
    clean_up(&root, now);
    let disk = disk_of(ctx, &slot, &root);
    let (merges, fold) = (disk.merges(), disk.fold());
    if merges {
        settle_names(&root, &mut idx.entries, fold);
    }
    let (my_node, my_name) = me(ctx);
    let peer = peer_name(ctx, &c.peer);
    let mut gets: Vec<(String, String, String)> = Vec::new();
    let mut cancel: Vec<String> = Vec::new();
    let mut differing = None;
    if let Some((remote, _)) = &remote {
        // Files kept under another name for letter case stand under the
        // other side's path; what's decided for it is done to them.
        let (view, keys) = local_view(&idx.entries);
        let plan = plan(
            &view,
            remote,
            c.mode.takes(),
            Side {
                node: &my_node,
                name: &my_name,
            },
            Side {
                node: &c.peer,
                name: &peer,
            },
        );
        let here = |p: String| keys.get(&p).cloned().unwrap_or(p);
        let mut trash: Vec<String> = Vec::new();
        let mut pulls: Vec<(String, Pull)> = Vec::new();
        for act in plan.acts {
            match act {
                Act::Agree { path, base } => {
                    let path = here(path);
                    if let Some(e) = idx.entries.get_mut(&path) {
                        if base.is_empty() && e.h.is_empty() {
                            idx.entries.remove(&path);
                        } else {
                            e.b = Some(base);
                        }
                    }
                }
                Act::Trash { path } => trash.push(here(path)),
                Act::Pull {
                    target,
                    src,
                    sha,
                    s,
                    m,
                    how,
                } => {
                    if rel_ok(&target) && rel_ok(&src) && is_sha(&sha) {
                        let kept = keys.get(&target).cloned();
                        let pull = Pull {
                            id: String::new(),
                            src,
                            sha,
                            s,
                            m,
                            how,
                            alias: kept.is_some(),
                            at: 0,
                        };
                        pulls.push((kept.unwrap_or(target), pull));
                    }
                }
            }
        }
        let mut dirs = dir_map(&idx.entries, fold);
        case_renames(
            &root,
            &mut idx.entries,
            &mut trash,
            &mut pulls,
            merges.then_some(&mut dirs),
            fold,
        );
        for path in trash {
            if to_trash(&root, &path, idx.entries.get(&path), now) {
                idx.entries.remove(&path);
            }
        }
        if merges {
            pulls = make_room(&root, idx, pulls, &mut dirs, &peer, fold);
        }
        let wanted: BTreeMap<String, Pull> = pulls.into_iter().collect();
        // What's no longer wanted stops; what's new is asked for; what's
        // been on its way too long is asked for again (same id: resumes).
        idx.pulls.retain(|target, p| {
            let keep = wanted.get(target).is_some_and(|w| {
                w.src == p.src && w.sha == p.sha && w.how == p.how && w.alias == p.alias
            });
            if !keep {
                cancel.push(p.id.clone());
            }
            keep
        });
        let cap = ctx.extras.sync.metered_cap.load(Ordering::Relaxed);
        for (target, mut w) in wanted {
            match idx.pulls.get_mut(&target) {
                // Too big for a metered network: waits (and is asked for
                // as soon as the cap is lifted: `at` isn't moved).
                Some(p) if over_cap(p.s, cap) => {}
                Some(p) => {
                    if now.saturating_sub(p.at) >= PULL_RETRY {
                        p.at = now;
                        gets.push((p.src.clone(), p.sha.clone(), p.id.clone()));
                    }
                }
                None => {
                    w.id = crate::transport::outbox::new_id();
                    if !over_cap(w.s, cap) {
                        w.at = now;
                        gets.push((w.src.clone(), w.sha.clone(), w.id.clone()));
                    }
                    idx.pulls.insert(target, w);
                }
            }
        }
        differing = Some(plan.differing);
    }
    let pulls = idx.pulls.len() as u32;
    let synced = differing == Some(0) && pulls == 0;
    {
        let mut live = slot.live.lock();
        live.error = None;
        live.skipped = counts.skipped;
        live.files = idx
            .entries
            .values()
            .filter(|e| !e.h.is_empty() && e.h != EXCLUDED)
            .count() as u32;
        live.conflicts = idx
            .entries
            .iter()
            .filter(|(p, e)| !e.h.is_empty() && p.contains(CONFLICT_MARK))
            .count() as u32;
        live.renamed = idx
            .entries
            .iter()
            .filter(|(k, e)| !e.h.is_empty() && e.renamed(k))
            .count() as u32;
        live.pulls = pulls;
        if let Some(d) = differing {
            live.differing = d;
        }
    }
    save_index(&root, id, idx);
    let manifest: Vec<Value> = local_view(&idx.entries)
        .0
        .iter()
        .filter(|(_, e)| !e.h.is_empty())
        .map(|(p, e)| json!([p, e.s, e.m, e.h]))
        .collect();
    drop(work);

    if synced {
        update_cfg(ctx, id, |p| p.last_synced = now);
    }
    {
        let mut t = ctx.extras.sync.transfers.lock();
        for x in &cancel {
            t.remove(x);
        }
        for (_, _, tid) in &gets {
            t.insert(tid.clone(), id.to_string());
        }
    }
    for x in cancel {
        super::files::cancel(ctx, &x);
    }
    for page in gets.chunks(GET_PAGE) {
        let files: Vec<Value> = page.iter().map(|(p, h, t)| json!([p, h, t])).collect();
        let msg = json!({ "type": "sync_get", "from": my_node, "pair": id, "files": files });
        send(ctx, &c.peer, &msg, false);
    }
    let reply = match &remote {
        None => Some(true),
        Some((_, true)) => Some(false),
        Some((_, false)) => None,
    };
    if let Some(ask) = reply {
        send_manifest(ctx, &c.peer, id, &my_node, &manifest, ask);
    }
    announce(ctx);
}

fn send_manifest<P: Platform>(
    ctx: &NetCtx<P>,
    peer: &str,
    id: &str,
    me: &str,
    files: &[Value],
    reply: bool,
) {
    let round = crate::transport::outbox::new_id();
    let pages: Vec<&[Value]> = if files.is_empty() {
        vec![&[]]
    } else {
        files.chunks(PAGE).collect()
    };
    let n = pages.len();
    for (i, page) in pages.into_iter().enumerate() {
        let msg = json!({
            "type": "sync_manifest", "from": me, "pair": id, "round": round,
            "page": i, "pages": n, "reply": reply, "files": page,
        });
        if !send(ctx, peer, &msg, false) {
            return;
        }
    }
}

/// Move `rel` to the trash, if it's still the file the index knows (else
/// it changed meanwhile: left for the next round). True when it's gone.
fn to_trash(root: &Path, rel: &str, entry: Option<&Entry>, now: u64) -> bool {
    let Some(e) = entry else { return false };
    let path = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return true; // already gone
    };
    if !meta.is_file() || meta.len() != e.s || mtime_ms(&meta) != e.m {
        return false;
    }
    let day = root.join(TRASH_DIR).join(date_of(now));
    let dest = day.join(rel);
    let Some(parent) = dest.parent() else {
        return false;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return false;
    }
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest = super::files::free_path(parent, &name);
    std::fs::rename(&path, dest).is_ok()
}

/// Empty the trash of days older than `TRASH_DAYS`, and drop staged and
/// partial files nobody finished.
fn clean_up(root: &Path, now: u64) {
    let oldest = date_of(now.saturating_sub(TRASH_DAYS * 86_400));
    if let Ok(days) = std::fs::read_dir(root.join(TRASH_DIR)) {
        for d in days.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            if name.len() == 10 && name < oldest {
                let _ = std::fs::remove_dir_all(d.path());
            }
        }
    }
    for sub in ["in", ".mydomain-partial"] {
        let Ok(files) = std::fs::read_dir(root.join(STATE_DIR).join(sub)) else {
            continue;
        };
        for f in files.flatten() {
            let old = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age >= STAGE_KEEP);
            if old {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

// --- files arriving ----------------------------------------------------------------------

/// Where `from`'s file for pair `pair` is received (`features::files` puts
/// the partial file under it, and the finished file in its `in`), or why
/// it isn't taken.
///
/// Only a file this side asked for (transfer `id`, of the `size` it was
/// when asked): the other device can't fill the folder with files nobody
/// wanted.
pub fn offer_dir<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    pair: &str,
    id: &str,
    size: u64,
) -> Result<PathBuf, &'static str> {
    let Some(c) = cfg(ctx, pair) else {
        return Err("that synced folder isn't here");
    };
    if c.peer != from || c.phase != Phase::Active || c.path.is_empty() {
        return Err("that synced folder isn't here");
    }
    if !c.mode.takes() {
        return Err("this side of the synced folder only sends");
    }
    let root = Path::new(&c.path);
    if !root.is_dir() {
        return Err("the synced folder is missing");
    }
    let asked = {
        let slot = slot(ctx, pair);
        let mut work = slot.work.lock();
        let idx = work.get_or_insert_with(|| load_index(root, pair));
        idx.pulls.values().any(|p| p.id == id && p.s == size)
    };
    if !asked {
        return Err("that file wasn't asked for");
    }
    Ok(root.join(STATE_DIR))
}

/// A synced file arrived complete (hash checked) at `staged`: put it in
/// its place (off the caller's thread), then say so.
#[allow(clippy::too_many_arguments)]
pub fn received<P: Platform>(
    ctx: &NetCtx<P>,
    pair: &str,
    id: &str,
    staged: PathBuf,
    from: &str,
    name: &str,
    size: u64,
    sha: &str,
) {
    let (ctx, pair, id, from, name, sha) = (
        ctx.clone(),
        pair.to_string(),
        id.to_string(),
        from.to_string(),
        name.to_string(),
        sha.to_string(),
    );
    std::thread::spawn(move || {
        ctx.extras.sync.transfers.lock().remove(&id);
        match place(&ctx, &pair, &id, &staged, &from, &sha) {
            Some(path) => ctx.sink.emit(CoreEvent::FileReceived {
                id,
                from,
                name,
                path: path.to_string_lossy().into_owned(),
                size,
                clipboard: false,
                sync: true,
            }),
            None => {
                let _ = std::fs::remove_file(&staged);
                ctx.sink.emit(CoreEvent::FileFailed {
                    id,
                    node_id: from,
                    name,
                    incoming: true,
                    reason: "not needed any more".to_string(),
                });
            }
        }
        announce(&ctx);
    });
}

/// `root/rel`, with its folders made, if it's inside `root` (no link in
/// the way leads out).
fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let sub = safe_folder(rel)?;
    let path = root.join(sub);
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    let real_root = std::fs::canonicalize(root).ok()?;
    let real_parent = std::fs::canonicalize(parent).ok()?;
    real_parent.starts_with(&real_root).then_some(path)
}

/// `rel`, or `rel` with a number ("a (1).txt") if that's taken.
fn free_rel(root: &Path, rel: &str) -> String {
    let (dir, name) = match rel.rsplit_once('/') {
        Some((d, n)) => (format!("{d}/"), n),
        None => (String::new(), rel),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (0..10_000)
        .map(|n| {
            if n == 0 {
                rel.to_string()
            } else {
                format!("{dir}{stem} ({n}){ext}")
            }
        })
        .find(|r| !root.join(r).exists())
        .unwrap_or_else(|| rel.to_string())
}

/// Put fetched file `id` in its place; returns where it went.
fn place<P: Platform>(
    ctx: &NetCtx<P>,
    pair: &str,
    id: &str,
    staged: &Path,
    from: &str,
    sha: &str,
) -> Option<PathBuf> {
    let c = cfg(ctx, pair)?;
    if c.peer != from {
        return None;
    }
    let root = PathBuf::from(&c.path);
    let slot = slot(ctx, pair);
    let mut work = slot.work.lock();
    let idx = work.get_or_insert_with(|| load_index(&root, pair));
    let (target, pull) = idx
        .pulls
        .iter()
        .find(|(_, p)| p.id == id)
        .map(|(t, p)| (t.clone(), p.clone()))?;
    idx.pulls.remove(&target);
    let placed = (|| {
        if pull.sha != sha {
            return None; // it changed there since: the next round sees
        }
        // The sender's modification time, so both sides agree on it.
        let when = UNIX_EPOCH + Duration::from_millis(pull.m);
        if let Ok(f) = std::fs::File::options().write(true).open(staged) {
            let _ = f.set_modified(when);
        }
        let peer = peer_name(ctx, &c.peer);
        let conflict = || free_rel(&root, &conflict_name(&target, &peer, pull.m));
        let (dest, agreed) = match &pull.how {
            How::Replace { expect } => {
                if current(&root, &idx.entries, &target) == *expect {
                    (target.clone(), true)
                } else {
                    // Changed here since the decision: keep both.
                    (conflict(), false)
                }
            }
            How::Displace { to } => {
                let old = root.join(&target);
                if std::fs::symlink_metadata(&old).is_ok() {
                    let to = free_rel(&root, to);
                    let to_path = inside(&root, &to)?;
                    std::fs::rename(&old, &to_path).ok()?;
                    let meta = std::fs::metadata(&to_path).ok()?;
                    let h = match idx.entries.get(&target) {
                        Some(e) if e.s == meta.len() && e.m == mtime_ms(&meta) => e.h.clone(),
                        _ => sha256_file(&to_path).ok()?,
                    };
                    idx.entries.insert(
                        to,
                        Entry {
                            s: meta.len(),
                            m: mtime_ms(&meta),
                            h,
                            b: None,
                            t: 0,
                            a: None,
                        },
                    );
                }
                (target.clone(), true)
            }
            How::Copy => {
                let same = idx.entries.get(&target).is_some_and(|e| e.h == sha)
                    && root.join(&target).is_file();
                if same {
                    let _ = std::fs::remove_file(staged);
                    return Some(root.join(&target));
                }
                (free_rel(&root, &target), false)
            }
        };
        let path = inside(&root, &dest)?;
        std::fs::rename(staged, &path).ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        // Kept under another name for letter case: it stands for the other
        // side's path (whose old tombstone here, if any, it replaces).
        let kept = pull.alias && dest == target;
        if kept
            && idx
                .entries
                .get(&pull.src)
                .is_some_and(|e| e.h.is_empty() && e.a.is_none())
        {
            idx.entries.remove(&pull.src);
        }
        idx.entries.insert(
            dest,
            Entry {
                s: meta.len(),
                m: mtime_ms(&meta),
                h: sha.to_string(),
                b: agreed.then(|| sha.to_string()),
                t: 0,
                a: kept.then(|| pull.src.clone()),
            },
        );
        Some(path)
    })();
    save_index(&root, pair, idx);
    let left = idx.pulls.len() as u32;
    drop(work);
    {
        let mut live = slot.live.lock();
        live.pulls = left;
        if left == 0 {
            // Compare again soon: likely all in sync now.
            live.last_round = None;
        }
    }
    placed
}

/// What `rel` holds here now, as far as the index can tell: `None` if
/// nothing's there, its hash if it's the file the index knows, else a
/// marker that matches no hash.
fn current(root: &Path, entries: &BTreeMap<String, Entry>, rel: &str) -> Option<String> {
    let meta = std::fs::symlink_metadata(root.join(rel)).ok()?;
    match entries.get(rel) {
        Some(e)
            if meta.is_file()
                && e.s == meta.len()
                && e.m == mtime_ms(&meta)
                && !e.h.is_empty()
                && e.h != EXCLUDED =>
        {
            Some(e.h.clone())
        }
        _ => Some("changed".to_string()),
    }
}

/// A transfer ended that may be this feature's: `incoming` and failed, the
/// fetch is dropped (the next round asks again); outgoing, it's no longer
/// being served. (A fetched file that arrived goes through `received`.)
pub fn transfer_ended<P: Platform>(ctx: &NetCtx<P>, id: &str, why: Option<&str>, incoming: bool) {
    let Some(pair) = ctx.extras.sync.transfers.lock().remove(id) else {
        return;
    };
    let (ctx, id) = (ctx.clone(), id.to_string());
    let failed = why.is_some();
    std::thread::spawn(move || {
        let slot = slot(&ctx, &pair);
        if incoming && failed {
            if let Some(c) = cfg(&ctx, &pair) {
                let root = PathBuf::from(&c.path);
                let mut work = slot.work.lock();
                let idx = work.get_or_insert_with(|| load_index(&root, &pair));
                idx.pulls.retain(|_, p| p.id != id);
                save_index(&root, &pair, idx);
                slot.live.lock().pulls = idx.pulls.len() as u32;
            }
        } else if !incoming {
            let mut live = slot.live.lock();
            live.serving.remove(&id);
            if live.serving.is_empty() {
                live.last_round = None;
            }
        }
        announce(&ctx);
    });
}

// --- messages ------------------------------------------------------------------------------

fn parse_manifest(files: &Value) -> Option<Vec<(String, Remote)>> {
    let arr = files.as_array()?;
    if arr.len() > PAGE {
        return None;
    }
    Some(
        arr.iter()
            .filter_map(|f| {
                let p = f.get(0)?.as_str()?;
                let s = f.get(1)?.as_u64()?;
                let m = f.get(2)?.as_u64()?;
                let h = f.get(3)?.as_str()?;
                (rel_ok(p) && (h == EXCLUDED || is_sha(h))).then(|| {
                    (
                        p.to_string(),
                        Remote {
                            s,
                            m,
                            h: h.to_string(),
                        },
                    )
                })
            })
            .collect(),
    )
}

fn on_manifest<P: Platform>(ctx: &NetCtx<P>, c: &PairCfg, msg: &ControlMsg) {
    if c.phase != Phase::Active || c.paused || held(ctx) {
        return;
    }
    let round = msg.str("round");
    let page = msg.json("page").as_u64().unwrap_or(u64::MAX) as usize;
    let pages = msg.json("pages").as_u64().unwrap_or(0) as usize;
    if round.is_empty() || pages == 0 || pages > MAX_FILES / PAGE + 1 || page >= pages {
        return;
    }
    let Some(files) = parse_manifest(&msg.json("files")) else {
        return;
    };
    let reply = msg.json("reply").as_bool() == Some(true);
    let done = {
        let mut inbound = ctx.extras.sync.inbound.lock();
        let m = inbound.entry(c.id.clone()).or_insert_with(|| Inbound {
            round: round.clone(),
            pages: Vec::new(),
            reply,
            at: Instant::now(),
        });
        if m.round != round {
            // A newer round: the old one is dropped.
            *m = Inbound {
                round: round.clone(),
                pages: Vec::new(),
                reply,
                at: Instant::now(),
            };
        }
        if m.pages.len() != pages {
            m.pages = vec![None; pages];
        }
        m.pages[page] = Some(files);
        if m.pages.iter().all(Option::is_some) {
            inbound.remove(&c.id)
        } else {
            None
        }
    };
    if let Some(m) = done {
        let remote: BTreeMap<String, Remote> = m.pages.into_iter().flatten().flatten().collect();
        spawn_round(ctx, &c.id, Some((remote, m.reply)));
    }
}

fn on_get<P: Platform>(ctx: &NetCtx<P>, c: &PairCfg, msg: &ControlMsg) {
    if c.phase != Phase::Active || c.paused || !c.mode.gives() || held(ctx) {
        return;
    }
    let cap = ctx.extras.sync.metered_cap.load(Ordering::Relaxed);
    let Some(files) = msg.json("files").as_array().cloned() else {
        return;
    };
    let (ctx, c) = (ctx.clone(), c.clone());
    // Off the link's thread: it reads the index (maybe waiting for a scan).
    std::thread::spawn(move || {
        let root = PathBuf::from(&c.path);
        let slot = slot(&ctx, &c.id);
        let mut nope = Vec::new();
        {
            let mut work = slot.work.lock();
            let idx = work.get_or_insert_with(|| load_index(&root, &c.id));
            let (_, keys) = local_view(&idx.entries);
            for f in files.iter().take(GET_PAGE) {
                let (Some(src), Some(sha), Some(tid)) = (
                    f.get(0).and_then(Value::as_str),
                    f.get(1).and_then(Value::as_str),
                    f.get(2).and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !is_transfer_id(tid) {
                    continue;
                }
                // The file of that name, or the one kept under another
                // name for it (letter case).
                let Some(key) = local_key(&idx.entries, &keys, src) else {
                    nope.push(tid.to_string());
                    continue;
                };
                let path = root.join(&key);
                let ok = rel_ok(src)
                    && idx.entries.get(&key).is_some_and(|e| {
                        e.h == sha
                            && std::fs::symlink_metadata(&path)
                                .is_ok_and(|m| m.is_file() && m.len() == e.s && mtime_ms(&m) == e.m)
                    });
                if !ok {
                    nope.push(tid.to_string());
                    continue;
                }
                // Too big for a metered network: no answer, so the other
                // side asks again later (a "nope" would end it).
                if idx.entries.get(&key).is_some_and(|e| over_cap(e.s, cap)) {
                    continue;
                }
                let extra = json!({ "sync": { "pair": c.id } });
                match super::files::start_with_extra(&ctx, &c.peer, &path, tid, Some(extra)) {
                    Ok(_) => {
                        ctx.extras
                            .sync
                            .transfers
                            .lock()
                            .insert(tid.to_string(), c.id.clone());
                        slot.live.lock().serving.insert(tid.to_string());
                    }
                    // Already going (it resumes by itself), or can't.
                    Err(_) => {
                        if super::files::outgoing_progress(&ctx, tid).is_none() {
                            nope.push(tid.to_string());
                        }
                    }
                }
            }
        }
        if !nope.is_empty() {
            let (node, _) = me(&ctx);
            let msg = json!({ "type": "sync_nope", "from": node, "pair": c.id, "ids": nope });
            send(&ctx, &c.peer, &msg, false);
        }
        announce(&ctx);
    });
}

fn on_nope<P: Platform>(ctx: &NetCtx<P>, c: &PairCfg, msg: &ControlMsg) {
    let ids: HashSet<String> = msg
        .json("ids")
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let (ctx, c) = (ctx.clone(), c.clone());
    std::thread::spawn(move || {
        let root = PathBuf::from(&c.path);
        let slot = slot(&ctx, &c.id);
        {
            let mut work = slot.work.lock();
            let idx = work.get_or_insert_with(|| load_index(&root, &c.id));
            idx.pulls.retain(|_, p| !ids.contains(&p.id));
            save_index(&root, &c.id, idx);
            slot.live.lock().pulls = idx.pulls.len() as u32;
        }
        let mut t = ctx.extras.sync.transfers.lock();
        for id in &ids {
            t.remove(id);
        }
        drop(t);
        announce(&ctx);
    });
}

fn on_invite<P: Platform>(ctx: &NetCtx<P>, from: &str, msg: &ControlMsg) {
    let id = msg.str("pair");
    let Some(mode) = Mode::parse(&msg.str("mode")) else {
        return;
    };
    if !is_pair_id(&id) || !ctx.peers.lock().contains_key(from) {
        return;
    }
    {
        let mut pairs = ctx.extras.sync.pairs.lock();
        if pairs.contains_key(&id) {
            return; // a repeat
        }
        pairs.insert(
            id.clone(),
            PairCfg {
                id: id.clone(),
                peer: from.to_string(),
                name: safe_name(&msg.str("name")),
                path: String::new(),
                mode: mode.mirror(),
                paused: false,
                phase: Phase::Invited,
                last_synced: 0,
            },
        );
    }
    save_pairs(ctx);
    announce(ctx);
}

pub struct FolderSyncHandler;

impl<P: Platform> ControlHandler<P> for FolderSyncHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &[
            "sync_invite",
            "sync_accept",
            "sync_remove",
            "sync_manifest",
            "sync_get",
            "sync_nope",
        ]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() {
            return;
        }
        load(ctx);
        // It sent one: it understands them.
        set_caps(ctx, &from, true);
        if msg.kind() == "sync_invite" {
            return on_invite(ctx, &from, msg);
        }
        let Some(c) = cfg(ctx, &msg.str("pair")) else {
            return;
        };
        if c.peer != from {
            return;
        }
        match msg.kind() {
            "sync_accept" => {
                if c.phase == Phase::Pending {
                    update_cfg(ctx, &c.id, |p| p.phase = Phase::Active);
                    slot(ctx, &c.id).live.lock().last_round = None;
                    announce(ctx);
                }
            }
            "sync_remove" => {
                forget(ctx, &c.id);
                announce(ctx);
            }
            "sync_manifest" => on_manifest(ctx, &c, msg),
            "sync_get" => on_get(ctx, &c, msg),
            "sync_nope" => on_nope(ctx, &c, msg),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn me() -> Side<'static> {
        Side {
            node: "node-m",
            name: "Mac",
        }
    }
    fn peer() -> Side<'static> {
        Side {
            node: "node-p",
            name: "Phone",
        }
    }

    fn entry(h: &str, b: Option<&str>, m: u64) -> Entry {
        Entry {
            s: 1,
            m,
            h: h.to_string(),
            b: b.map(str::to_string),
            t: 0,
            a: None,
        }
    }

    fn remote(h: &str, m: u64) -> Remote {
        Remote {
            s: 1,
            m,
            h: h.to_string(),
        }
    }

    fn run(local: &[(&str, Entry)], rem: &[(&str, Remote)], takes: bool) -> Plan {
        let l = local
            .iter()
            .map(|(p, e)| (p.to_string(), e.clone()))
            .collect();
        let r = rem
            .iter()
            .map(|(p, e)| (p.to_string(), e.clone()))
            .collect();
        plan(&l, &r, takes, me(), peer())
    }

    fn pull(target: &str, src: &str, sha: &str, m: u64, how: How) -> Act {
        Act::Pull {
            target: target.into(),
            src: src.into(),
            sha: sha.into(),
            s: 1,
            m,
            how,
        }
    }

    #[test]
    fn only_files_over_the_metered_cap_wait() {
        assert!(!over_cap(u64::MAX, 0), "no cap: nothing waits");
        assert!(!over_cap(10 << 20, 10 << 20));
        assert!(over_cap((10 << 20) + 1, 10 << 20));
    }

    #[test]
    fn the_same_on_both_sides_is_remembered_as_agreed() {
        let p = run(
            &[("a.txt", entry(A, None, 1))],
            &[("a.txt", remote(A, 1))],
            true,
        );
        assert_eq!(
            p.acts,
            vec![Act::Agree {
                path: "a.txt".into(),
                base: A.into()
            }]
        );
        assert_eq!(p.differing, 0);
        // Already agreed: nothing to do.
        let p = run(
            &[("a.txt", entry(A, Some(A), 1))],
            &[("a.txt", remote(A, 1))],
            true,
        );
        assert_eq!(p, Plan::default());
        // Gone on both sides after being agreed: agreed absent.
        let p = run(&[("a.txt", entry("", Some(A), 1))], &[], true);
        assert_eq!(
            p.acts,
            vec![Act::Agree {
                path: "a.txt".into(),
                base: String::new()
            }]
        );
    }

    #[test]
    fn new_files_there_are_fetched_and_new_ones_here_wait_for_the_other_side() {
        let p = run(&[], &[("new.txt", remote(A, 5))], true);
        assert_eq!(
            p.acts,
            vec![pull(
                "new.txt",
                "new.txt",
                A,
                5,
                How::Replace { expect: None }
            )]
        );
        let p = run(&[("mine.txt", entry(A, None, 5))], &[], true);
        assert_eq!(p.acts, vec![]);
        assert_eq!(p.differing, 1);
    }

    #[test]
    fn an_edit_there_replaces_the_unchanged_copy_here() {
        let p = run(
            &[("a.txt", entry(A, Some(A), 1))],
            &[("a.txt", remote(B, 9))],
            true,
        );
        assert_eq!(
            p.acts,
            vec![pull(
                "a.txt",
                "a.txt",
                B,
                9,
                How::Replace {
                    expect: Some(A.into())
                }
            )]
        );
        // An edit here only: the other side fetches it.
        let p = run(
            &[("a.txt", entry(B, Some(A), 9))],
            &[("a.txt", remote(A, 1))],
            true,
        );
        assert_eq!(p.acts, vec![]);
    }

    #[test]
    fn a_deletion_there_moves_the_unchanged_copy_to_the_trash() {
        let p = run(&[("a.txt", entry(A, Some(A), 1))], &[], true);
        assert_eq!(
            p.acts,
            vec![Act::Trash {
                path: "a.txt".into()
            }]
        );
        // Deleted here: nothing here (the other side trashes its copy).
        let p = run(
            &[("a.txt", entry("", Some(A), 1))],
            &[("a.txt", remote(A, 1))],
            true,
        );
        assert_eq!(p.acts, vec![]);
    }

    #[test]
    fn deleted_on_one_side_and_edited_on_the_other_keeps_the_edit() {
        // Edited here, deleted there: stays.
        let p = run(&[("a.txt", entry(B, Some(A), 9))], &[], true);
        assert_eq!(p.acts, vec![]);
        // Deleted here, edited there: comes back.
        let p = run(
            &[("a.txt", entry("", Some(A), 1))],
            &[("a.txt", remote(B, 9))],
            true,
        );
        assert_eq!(
            p.acts,
            vec![pull("a.txt", "a.txt", B, 9, How::Replace { expect: None })]
        );
    }

    #[test]
    fn a_conflict_keeps_both_and_both_sides_agree_who_keeps_the_name() {
        // Here changed later: the other copy comes as a conflict copy.
        let p = run(
            &[("n.txt", entry(B, Some(A), 2_000))],
            &[("n.txt", remote(C, 1_000))],
            true,
        );
        assert_eq!(
            p.acts,
            vec![pull(
                "n (conflict from Phone 1970-01-01).txt",
                "n.txt",
                C,
                1_000,
                How::Copy
            )]
        );
        // The other side's view of the same: its copy moves aside under
        // the same name, and ours takes the name.
        let other = plan(
            &[("n.txt".to_string(), entry(C, Some(A), 1_000))].into(),
            &[("n.txt".to_string(), remote(B, 2_000))].into(),
            true,
            peer(),
            me(),
        );
        assert_eq!(
            other.acts,
            vec![pull(
                "n.txt",
                "n.txt",
                B,
                2_000,
                How::Displace {
                    to: "n (conflict from Phone 1970-01-01).txt".into()
                }
            )]
        );
        // Two different files of one name at the first sync: a conflict
        // too, never an overwrite.
        let p = run(&[("x", entry(A, None, 5))], &[("x", remote(B, 5))], true);
        assert!(
            matches!(
                &p.acts[..],
                [Act::Pull {
                    how: How::Displace { .. },
                    ..
                }]
            ),
            "{p:?}"
        );
        // The conflict copy is already here: nothing more.
        let p = run(
            &[
                ("n.txt", entry(B, Some(A), 2_000)),
                (
                    "n (conflict from Phone 1970-01-01).txt",
                    entry(C, None, 1_000),
                ),
            ],
            &[("n.txt", remote(C, 1_000))],
            true,
        );
        assert!(
            p.acts.iter().all(|a| !matches!(a, Act::Pull { .. })),
            "{p:?}"
        );
    }

    #[test]
    fn send_only_never_changes_this_side() {
        let p = run(
            &[("a.txt", entry(A, Some(A), 1)), ("b", entry(A, None, 1))],
            &[("b", remote(B, 1)), ("c", remote(C, 1))],
            false,
        );
        assert!(
            p.acts.iter().all(|a| matches!(a, Act::Agree { .. })),
            "{p:?}"
        );
        assert_eq!(p.differing, 3);
    }

    #[test]
    fn files_left_out_on_either_side_are_left_alone() {
        let p = run(
            &[("big", entry(EXCLUDED, Some(A), 1))],
            &[("big", remote(B, 1))],
            true,
        );
        assert_eq!(p, Plan::default());
        let p = run(
            &[("big", entry(A, Some(A), 1))],
            &[("big", remote(EXCLUDED, 1))],
            true,
        );
        assert_eq!(p, Plan::default());
    }

    #[test]
    fn hidden_and_system_files_and_odd_names_are_left_out() {
        for name in [
            ".DS_Store",
            ".git",
            "Thumbs.db",
            "desktop.ini",
            "~$report.docx",
            "movie.mp4.part",
            ".mydomain-trash",
        ] {
            assert!(ignored(name), "{name}");
        }
        assert!(!ignored("notes.txt"));
        assert!(rel_ok("Trip/day 1/a.txt"));
        for bad in [
            "../x",
            "/etc/passwd",
            "a/.git/HEAD",
            "a//b",
            "a:b",
            "C:/x",
            "",
        ] {
            assert!(!rel_ok(bad), "{bad}");
        }
    }

    #[test]
    fn conflict_names_keep_the_folder_and_extension() {
        assert_eq!(
            conflict_name("Notes/todo.md", "Pankaj’s Phone", 1_793_750_400_000),
            "Notes/todo (conflict from Pankaj’s Phone 2026-11-04).md"
        );
        assert_eq!(
            conflict_name("Makefile", "Mac", 0),
            "Makefile (conflict from Mac 1970-01-01)"
        );
        assert!(rel_ok(&conflict_name("a/b.txt", "x:y", 0)));
    }

    fn temp(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("md-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn new_pull(path: &str, sha: &str) -> (String, Pull) {
        let p = Pull {
            id: String::new(),
            src: path.into(),
            sha: sha.into(),
            s: 1,
            m: 1,
            how: How::Replace { expect: None },
            alias: false,
            at: 0,
        };
        (path.into(), p)
    }

    #[test]
    fn case_conflict_names_keep_the_folder_and_extension() {
        assert_eq!(
            case_name("Notes/todo.md", "Pankaj’s Phone", 1),
            "Notes/todo (case conflict from Pankaj’s Phone).md"
        );
        assert_eq!(
            case_name("README", "x:y", 2),
            "README (case conflict from xy 2)"
        );
        assert!(rel_ok(&case_name("a/b.txt", "Linux", 3)));
        assert!(!case_name("a.txt", "Mac", 1).contains(CONFLICT_MARK));
    }

    #[test]
    fn a_file_kept_under_another_name_stands_for_the_other_sides_path() {
        let mut kept = entry(A, Some(A), 1);
        kept.a = Some("notes.txt".into());
        let local: BTreeMap<String, Entry> = [
            ("Notes.txt".to_string(), entry(B, Some(B), 1)),
            ("notes (case conflict from Phone).txt".to_string(), kept),
        ]
        .into();
        let (view, keys) = local_view(&local);
        assert_eq!(
            view.keys().map(String::as_str).collect::<Vec<_>>(),
            ["Notes.txt", "notes.txt"]
        );
        assert_eq!(keys["notes.txt"], "notes (case conflict from Phone).txt");
        assert_eq!(
            local_key(&local, &keys, "notes.txt").as_deref(),
            Some("notes (case conflict from Phone).txt")
        );
        assert_eq!(
            local_key(&local, &keys, "Notes.txt").as_deref(),
            Some("Notes.txt")
        );
        assert_eq!(local_key(&local, &keys, "nope.txt"), None);
        // The same on both sides: nothing to fetch, nothing differs.
        let remote: BTreeMap<String, Remote> = [
            ("Notes.txt".to_string(), remote(B, 1)),
            ("notes.txt".to_string(), remote(A, 1)),
        ]
        .into();
        assert_eq!(plan(&view, &remote, true, me(), peer()), Plan::default());
        // A file of that very name here wins.
        let mut both = local.clone();
        both.insert("notes.txt".into(), entry(C, None, 1));
        let (view, keys) = local_view(&both);
        assert_eq!(view["notes.txt"].h, C);
        assert!(keys.is_empty());
    }

    #[test]
    fn folders_are_spelled_as_they_are_here() {
        let root = temp("dirs");
        std::fs::create_dir_all(root.join("Docs").join("Old")).unwrap();
        let mut dirs = HashMap::new();
        assert_eq!(
            resolve_dirs(&root, "docs/old/a.txt", &mut dirs, Fold::LOOSE),
            "Docs/Old/a.txt"
        );
        assert_eq!(
            resolve_dirs(&root, "Docs/Old/b.txt", &mut dirs, Fold::LOOSE),
            "Docs/Old/b.txt"
        );
        // A folder decided this round is used for the next file too.
        assert_eq!(
            resolve_dirs(&root, "New/a.txt", &mut dirs, Fold::LOOSE),
            "New/a.txt"
        );
        assert_eq!(
            resolve_dirs(&root, "NEW/b.txt", &mut dirs, Fold::LOOSE),
            "New/b.txt"
        );
        assert_eq!(
            resolve_dirs(&root, "top.txt", &mut dirs, Fold::LOOSE),
            "top.txt"
        );
        let known: BTreeMap<String, Entry> =
            [("Photos/2024/a.jpg".to_string(), entry(A, None, 1))].into();
        let mut dirs = dir_map(&known, Fold::LOOSE);
        assert_eq!(
            resolve_dirs(&root, "photos/2024/b.jpg", &mut dirs, Fold::LOOSE),
            "Photos/2024/b.jpg"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_file_whose_name_differs_only_by_case_gets_a_name_of_its_own() {
        let root = temp("room");
        let mut idx = Index::default();
        idx.entries.insert("Notes.txt".into(), entry(A, Some(A), 1));
        idx.entries
            .insert("Docs/x.txt".into(), entry(A, Some(A), 1));
        let mut dirs = dir_map(&idx.entries, Fold::LOOSE);
        let pulls = vec![
            new_pull("NOTES.TXT", B),
            new_pull("docs/y.txt", B),
            new_pull("Two.txt", B),
            new_pull("two.txt", C),
            new_pull("fine.txt", C),
        ];
        let out = make_room(&root, &idx, pulls, &mut dirs, "Phone", Fold::LOOSE);
        let got: Vec<(&str, &str, bool)> = out
            .iter()
            .map(|(t, p)| (t.as_str(), p.src.as_str(), p.alias))
            .collect();
        assert_eq!(
            got,
            [
                ("NOTES (case conflict from Phone).TXT", "NOTES.TXT", true),
                ("Docs/y.txt", "docs/y.txt", true),
                ("Two.txt", "Two.txt", false),
                ("two (case conflict from Phone).txt", "two.txt", true),
                ("fine.txt", "fine.txt", false),
            ]
        );
        // On its way already under a name of its own: it keeps that name
        // next round.
        let mut idx2 = Index {
            entries: idx.entries.clone(),
            ..Index::default()
        };
        for (t, p) in out {
            idx2.pulls.insert(t, p);
        }
        let again = make_room(
            &root,
            &idx2,
            vec![new_pull("NOTES.TXT", B), new_pull("two.txt", C)],
            &mut dir_map(&idx2.entries, Fold::LOOSE),
            "Phone",
            Fold::LOOSE,
        );
        assert_eq!(again[0].0, "NOTES (case conflict from Phone).TXT");
        assert_eq!(again[1].0, "two (case conflict from Phone).txt");
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn file(root: &Path, rel: &str, body: &[u8]) -> Entry {
        let path = root.join(rel);
        std::fs::write(&path, body).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        Entry {
            s: meta.len(),
            m: mtime_ms(&meta),
            h: sha256_file(&path).unwrap(),
            b: None,
            t: 0,
            a: None,
        }
    }

    fn names(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_rename_only_in_case_there_is_a_rename_here() {
        let root = temp("case-rename");
        let e = file(&root, "Notes.txt", b"notes");
        let agreed = Entry {
            b: Some(e.h.clone()),
            ..e.clone()
        };
        let mut entries: BTreeMap<String, Entry> = [("Notes.txt".to_string(), agreed)].into();
        let mut trash = vec!["Notes.txt".to_string()];
        let mut pulls = vec![new_pull("notes.txt", &e.h)];
        case_renames(
            &root,
            &mut entries,
            &mut trash,
            &mut pulls,
            None,
            Fold::LOOSE,
        );
        assert!(trash.is_empty() && pulls.is_empty());
        assert_eq!(names(&root), ["notes.txt"]);
        assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), b"notes");
        assert_eq!(entries["notes.txt"].b.as_deref(), Some(e.h.as_str()));
        assert!(!entries.contains_key("Notes.txt"));
        // Different content: not a rename (it's fetched as usual).
        let mut trash = vec!["notes.txt".to_string()];
        let mut pulls = vec![new_pull("NOTES.txt", B)];
        case_renames(
            &root,
            &mut entries,
            &mut trash,
            &mut pulls,
            None,
            Fold::LOOSE,
        );
        assert_eq!((trash.len(), pulls.len()), (1, 1));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_renamed_only_in_case_there_stays_as_it_is_here() {
        let root = temp("dir-rename");
        std::fs::create_dir_all(root.join("Docs")).unwrap();
        let e = file(&root, "Docs/a.txt", b"a");
        let mut entries: BTreeMap<String, Entry> = [("Docs/a.txt".to_string(), e.clone())].into();
        let mut dirs = dir_map(&entries, Fold::LOOSE);
        let mut trash = vec!["Docs/a.txt".to_string()];
        let mut pulls = vec![new_pull("docs/a.txt", &e.h)];
        case_renames(
            &root,
            &mut entries,
            &mut trash,
            &mut pulls,
            Some(&mut dirs),
            Fold::LOOSE,
        );
        assert!(trash.is_empty() && pulls.is_empty());
        assert_eq!(entries["Docs/a.txt"].a.as_deref(), Some("docs/a.txt"));
        assert!(root.join("Docs").join("a.txt").is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nothing_is_renamed_over_another_file() {
        let root = temp("in-the-way");
        let e = file(&root, "a.txt", b"a");
        file(&root, "b.txt", b"b");
        // "B.txt" is "b.txt" on a disk that ignores case.
        assert!(!rename_in_place(&root, "a.txt", "B.txt", &e));
        assert!(!rename_in_place(&root, "a.txt", "b.txt", &e));
        // Changed since it was scanned: left alone.
        let stale = Entry { s: 99, ..e.clone() };
        assert!(!rename_in_place(&root, "a.txt", "A.txt", &stale));
        assert!(rename_in_place(&root, "a.txt", "A.txt", &e));
        assert_eq!(names(&root), ["A.txt", "b.txt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_kept_file_takes_its_own_name_once_free() {
        let root = temp("settle");
        let mut kept = file(&root, "n (case conflict from Phone).txt", b"n");
        kept.a = Some("n.txt".into());
        let other = file(&root, "N.txt", b"N");
        let mut entries: BTreeMap<String, Entry> = [
            ("n (case conflict from Phone).txt".to_string(), kept),
            ("N.txt".to_string(), other),
        ]
        .into();
        settle_names(&root, &mut entries, Fold::LOOSE);
        assert!(entries.contains_key("n (case conflict from Phone).txt"));
        // The other file went: the name is free.
        std::fs::remove_file(root.join("N.txt")).unwrap();
        entries.get_mut("N.txt").unwrap().h.clear();
        settle_names(&root, &mut entries, Fold::LOOSE);
        assert_eq!(entries["n.txt"].a, None);
        assert_eq!(names(&root), ["n.txt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_disk_is_asked_whether_it_ignores_case() {
        let root = temp("probe");
        std::fs::write(root.join("x"), b"x").unwrap();
        assert_eq!(disk_folds_case(&root), root.join("X").exists());
        // The probe leaves nothing behind.
        assert_eq!(std::fs::read_dir(root.join(STATE_DIR)).unwrap().count(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    // "café.txt" with "é" as one character, and as "e" and an accent.
    const CAFE_NFC: &str = "caf\u{e9}.txt";
    const CAFE_NFD: &str = "cafe\u{301}.txt";

    #[test]
    fn names_are_compared_in_nfc_and_in_lower_case_only_where_the_disk_ignores_it() {
        let exact = Fold { case: false };
        assert_eq!(nfc(CAFE_NFD), CAFE_NFC);
        assert_eq!(nfc(CAFE_NFC), CAFE_NFC);
        assert_eq!(exact.key(CAFE_NFD), exact.key(CAFE_NFC));
        assert_eq!(exact.key("Docs/Cafe\u{301}"), "Docs/Caf\u{e9}");
        assert_ne!(exact.key("Notes.txt"), exact.key("notes.txt"));
        assert_eq!(Fold::LOOSE.key("CAFE\u{301}.TXT"), CAFE_NFC);
        assert_eq!(Disk::default().fold(), exact);
        assert!(!Disk::default().merges());
        let mac = Disk {
            case: true,
            norm: true,
        };
        assert!(mac.merges() && mac.fold() == Fold::LOOSE);
        let renamed = |k: &str, a: &str| {
            Entry {
                a: Some(a.into()),
                ..Entry::default()
            }
            .renamed(k)
        };
        assert!(!renamed(CAFE_NFD, CAFE_NFC));
        assert!(renamed("cafe (case conflict from Phone).txt", CAFE_NFC));
    }

    #[test]
    fn the_disk_is_asked_whether_it_takes_both_spellings_for_one_name() {
        let root = temp("norm-probe");
        std::fs::write(root.join(CAFE_NFC), b"x").unwrap();
        assert_eq!(disk_folds_norm(&root), root.join(CAFE_NFD).exists());
        assert_eq!(std::fs::read_dir(root.join(STATE_DIR)).unwrap().count(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_name_spelled_otherwise_is_sent_in_nfc_and_stays_one_file() {
        let root = temp("nfd");
        std::fs::write(root.join(CAFE_NFD), b"caf").unwrap();
        let mut entries = BTreeMap::new();
        scan(&root, &mut entries, 1 << 20, 10).unwrap();
        // As the disk lists it (a Mac's may list it either way).
        let (k, e) = entries.iter().next().unwrap();
        let k = k.clone();
        assert_eq!(nfc(&k), CAFE_NFC);
        if k == CAFE_NFD {
            assert_eq!(e.a.as_deref(), Some(CAFE_NFC));
            assert!(!e.renamed(&k));
        }
        let (view, keys) = local_view(&entries);
        assert_eq!(
            view.keys().map(String::as_str).collect::<Vec<_>>(),
            [CAFE_NFC]
        );
        assert_eq!(
            local_key(&entries, &keys, CAFE_NFC).as_deref(),
            Some(k.as_str())
        );
        // Listed under the other spelling now: the same file, still agreed.
        let h = e.h.clone();
        entries.get_mut(&k).unwrap().b = Some(h.clone());
        let other = if k == CAFE_NFD { CAFE_NFC } else { CAFE_NFD };
        std::fs::rename(root.join(&k), root.join(other)).unwrap();
        let listed = names(&root).pop().unwrap();
        scan(&root, &mut entries, 1 << 20, 20).unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        let e = &entries[&listed];
        assert_eq!(
            (e.h.as_str(), e.b.as_deref()),
            (h.as_str(), Some(h.as_str()))
        );
        assert_eq!(e.a.as_deref(), (listed != CAFE_NFC).then_some(CAFE_NFC));
        assert_eq!(
            local_view(&entries).0.keys().collect::<Vec<_>>(),
            [CAFE_NFC]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_of_the_nfc_name_takes_the_path_from_one_spelled_otherwise() {
        let root = temp("nfc-twin");
        std::fs::write(root.join(CAFE_NFD), b"nfd").unwrap();
        let mut entries = BTreeMap::new();
        scan(&root, &mut entries, 1 << 20, 10).unwrap();
        std::fs::write(root.join(CAFE_NFC), b"nfc").unwrap();
        if names(&root).len() < 2 {
            // This disk takes both for one name.
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
        let agreed = entries[CAFE_NFD].h.clone();
        entries.get_mut(CAFE_NFD).unwrap().b = Some(agreed.clone());
        scan(&root, &mut entries, 1 << 20, 20).unwrap();
        assert_eq!(entries[CAFE_NFC].a, None);
        assert_eq!(entries[CAFE_NFC].b.as_deref(), Some(agreed.as_str()));
        assert_eq!((&entries[CAFE_NFD].a, &entries[CAFE_NFD].b), (&None, &None));
        let (view, keys) = local_view(&entries);
        assert_eq!(
            view.keys().map(String::as_str).collect::<Vec<_>>(),
            [CAFE_NFD, CAFE_NFC]
        );
        assert!(keys.is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn two_spellings_of_one_name_get_names_of_their_own_where_the_disk_merges_them() {
        let root = temp("norm-room");
        let exact = Fold { case: false };
        let mut idx = Index::default();
        idx.entries
            .insert("x\u{e9}.txt".into(), entry(A, Some(A), 1));
        let pulls = vec![
            new_pull("xe\u{301}.txt", B),
            new_pull(CAFE_NFD, B),
            new_pull(CAFE_NFC, C),
            new_pull("Caf\u{e9}.txt", C),
        ];
        let out = make_room(&root, &idx, pulls, &mut HashMap::new(), "Phone", exact);
        let got: Vec<(&str, &str, bool)> = out
            .iter()
            .map(|(t, p)| (t.as_str(), p.src.as_str(), p.alias))
            .collect();
        assert_eq!(
            got,
            [
                // The NFC one keeps the name.
                (CAFE_NFC, CAFE_NFC, false),
                // Letter case tells them apart on this disk.
                ("Caf\u{e9}.txt", "Caf\u{e9}.txt", false),
                (
                    "x\u{e9} (case conflict from Phone).txt",
                    "xe\u{301}.txt",
                    true
                ),
                ("caf\u{e9} (case conflict from Phone).txt", CAFE_NFD, true),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_file_whose_name_is_just_spelled_otherwise_isnt_renamed() {
        let root = temp("norm-settle");
        let mut e = file(&root, CAFE_NFD, b"c");
        e.a = Some(CAFE_NFC.into());
        let key = names(&root).pop().unwrap();
        let mut entries: BTreeMap<String, Entry> = [(key.clone(), e)].into();
        settle_names(&root, &mut entries, Fold::LOOSE);
        assert!(entries.contains_key(&key));
        assert_eq!(names(&root), [key]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dates_from_unix_time() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(date_of(1_798_761_599), "2026-12-31");
        assert_eq!(date_of(1_798_761_600), "2027-01-01");
    }

    #[test]
    fn modes_mirror() {
        assert_eq!(Mode::Send.mirror(), Mode::Receive);
        assert_eq!(Mode::Receive.mirror(), Mode::Send);
        assert_eq!(Mode::Both.mirror(), Mode::Both);
        assert!(!Mode::Send.takes() && Mode::Send.gives());
        assert!(Mode::Receive.takes() && !Mode::Receive.gives());
        assert_eq!(Mode::parse("both"), Some(Mode::Both));
        assert_eq!(Mode::parse("sideways"), None);
    }

    #[test]
    fn a_scan_hashes_new_files_and_leaves_tombstones_only_where_needed() {
        let root = std::env::temp_dir().join(format!("md-sync-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("sub").join("b.txt"), b"b").unwrap();
        std::fs::write(root.join(".DS_Store"), b"x").unwrap();
        std::fs::write(root.join(".git").join("HEAD"), b"x").unwrap();
        std::fs::write(root.join("big.bin"), vec![0u8; 100]).unwrap();
        let mut entries = BTreeMap::new();
        let counts = scan(&root, &mut entries, 50, 10).unwrap();
        assert_eq!(counts.skipped, 1);
        let keys: Vec<&str> = entries.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a.txt", "big.bin", "sub/b.txt"]);
        assert_eq!(entries["big.bin"].h, EXCLUDED);
        assert!(is_sha(&entries["a.txt"].h));
        // Agreed, then deleted: a tombstone; never agreed: just forgotten.
        entries.get_mut("a.txt").unwrap().b = Some(entries["a.txt"].h.clone());
        std::fs::remove_file(root.join("a.txt")).unwrap();
        std::fs::remove_file(root.join("sub").join("b.txt")).unwrap();
        scan(&root, &mut entries, 50, 20).unwrap();
        assert_eq!(entries["a.txt"].h, "");
        assert_eq!(entries["a.txt"].t, 20);
        assert!(!entries.contains_key("sub/b.txt"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
