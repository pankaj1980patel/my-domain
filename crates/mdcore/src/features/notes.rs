//! Notes: a small shared notepad that's the same on all your devices (quick
//! text notes, lists, things to remember). Each note goes end-to-end
//! encrypted over the links your devices already have; nothing is kept on
//! a server or in a cloud.
//!
//! A note is `{id, title, body, pinned, checklist, color?, created_at,
//! updated_at, by, by_name, deleted, conflict_of?, seen, remind?}`. The body is plain
//! text (at most `MAX_BODY` bytes); `checklist` only tells the apps to draw
//! lines starting with `[ ] ` / `[x] ` as checkboxes. Deleting keeps a
//! tombstone (`deleted`, no text) so the delete reaches every device; a
//! tombstone is forgotten after `TOMBSTONE_DAYS` (a device that was away
//! longer than that and still has the note brings it back).
//!
//! # Merging
//!
//! Each note is merged on its own, last writer wins by (`updated_at`, `by`,
//! then the content), so every device ends the same whatever order things
//! arrive in. But two devices editing the same note before they hear of
//! each other's edit must not silently lose text, so each note carries a
//! short lineage, `seen`: hashes of the versions it was edited from (the
//! last `LINEAGE` versions another device could have). A device's own
//! edits that no other device has seen yet (`dirty`, never sent) don't add
//! to it, so a long typing session or a day offline is one step. That is
//! how a device tracks "the last synced version" of each note: when a
//! version arrives,
//!
//! - the same version: nothing to do;
//! - its lineage names ours: it was edited from ours, so it replaces ours;
//! - our lineage names it: ours was edited from it, so ours stays (and
//!   goes back);
//! - neither: both were edited since they last agreed (a conflict). The
//!   newer one keeps the note, and the other, if it has text the winner
//!   doesn't, is kept as a new note titled "Title (conflict from
//!   <device>)" (`conflict_of` names the note it split from). Its id is
//!   made from the losing version, so every device that sees the conflict
//!   makes the very same copy and they never multiply. A delete that
//!   conflicts with an edit keeps the edited text the same way.
//!
//! # Sync
//!
//!   notes_digest { from, digest: { <id>: <version hash> } }
//!   notes        { from, notes: [<note>...], want?: [<id>...] }
//!
//! When a link comes up (or a device is first heard to understand notes),
//! each end sends its digest: just ids and version hashes, so a large set
//! isn't resent. The other end answers with the notes whose version
//! differs or the digest lacks (tombstones only for notes it has), and
//! asks (`want`) for those it lacks; the answer goes in batches of at most
//! `BATCH_BYTES`. A note saved here goes at once to every linked device
//! that understands notes. A device answers a `notes` message once with
//! what it has that's newer (or a conflict copy it made); that answer
//! brings nothing newer back, so it ends there. Devices without a link get
//! everything at their next link (nothing wakes a device for a note).
//!
//! Only devices that said `notes: 1` or more (in their status, or in the
//! `addr_update` every link starts with) are sent notes: older apps would
//! show the message as chat. Receiving one says the same (and its `v`,
//! which version).
//!
//! # Reminders (`notes: 2`)
//!
//! A note may carry a reminder, `remind: {at?, done?}` (`at` unix ms;
//! `done` once someone marked it Done on any device). Every device that
//! has the note shows it at `at` (the apps schedule that; the core only
//! syncs it). Setting, snoozing (a new `at`) and Done are edits like any
//! other, so they merge the same way.
//!
//! An app from before reminders (`notes: 1`) keeps only the fields it
//! knows: what it saves or passes on has no `remind` at all, which here
//! means "unknown", not "no reminder" (an app that knows reminders always
//! sends `remind`, `{}` for none). So:
//!
//! - `remind` goes only to devices that said `notes: 2`, in `notes`
//!   messages that say `v: 2`; their digests (also `v: 2`) hash the
//!   reminder too (`full_version`), so two devices that know reminders
//!   and hold one version with different reminders find out;
//! - a version with no `remind` takes the one of the version it replaces
//!   here (an older app's edit of the text keeps the reminder), or the
//!   one another device has for that very version (`remind` beats none;
//!   two reminders for one version: Done wins, then the later time, so
//!   every device ends the same);
//! - when two edits conflict, the winner keeps a Done made on the other
//!   side for the same time (marking Done on the phone while the text was
//!   edited on the computer doesn't bring the reminder back).
//!
//! The version hash (`version`) stays as it was, so an older app's
//! digests and lineage still match.
//!
//! Limits: `MAX_NOTES` notes, `MAX_TOTAL` bytes of text in all; past them a
//! new note isn't made here, nor taken from another device (conflict copies
//! are always kept: they hold text that would be lost otherwise).
//!
//! Stored beside the platform's kv store as `notes.json`
//! (`Platform::data_file`), or in `kv` (key `notes`) where there's none.
//!
//! What arrives and changes a note makes a `CoreEvent::NotesChanged` with
//! the changed notes and the deleted ids. Saving from the apps makes no
//! event (the app already shows what it saved).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::Note;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The most notes (not counting deleted ones).
pub const MAX_NOTES: usize = 2_000;
/// The most text in all notes together, in bytes.
pub const MAX_TOTAL: usize = 16 * 1024 * 1024;
/// The longest body, in bytes.
pub const MAX_BODY: usize = 64 * 1024;
/// The longest title, in characters.
pub const MAX_TITLE: usize = 200;
/// How long a deleted note is remembered.
pub const TOMBSTONE_DAYS: u64 = 30;
const TOMBSTONE_MS: u64 = TOMBSTONE_DAYS * 24 * 3600 * 1000;
/// Notes and tombstones kept at most: the oldest tombstones go first.
const MAX_ENTRIES: usize = MAX_NOTES * 2;
/// Versions a note remembers it was edited from.
pub const LINEAGE: usize = 32;
/// The most JSON in one `notes` message.
const BATCH_BYTES: usize = 256 * 1024;
const MAX_ID: usize = 64;
const MAX_NODE: usize = 128;
const MAX_NAME: usize = 64;
const MAX_TOKEN: usize = 24;

/// The status (and `addr_update`) flag saying this device understands
/// notes, and its version: 1 notes, 2 reminders too.
pub const FLAG: &str = "notes";
const VERSION: u64 = 2;

const KV_KEY: &str = "notes";
const FILE: &str = "notes.json";
const CAPS_PREFIX: &str = "notes_caps:";

fn not(b: &bool) -> bool {
    !*b
}

/// One note as it's kept and goes on the wire (`dirty` never does).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "not")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "not")]
    pub checklist: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    /// The device that made this version (node id) ...
    #[serde(default)]
    pub by: String,
    /// ... and its name then (names conflict copies the same everywhere).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by_name: String,
    #[serde(default, skip_serializing_if = "not")]
    pub deleted: bool,
    /// A conflict copy: the note it split from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_of: Option<String>,
    /// Hashes of the versions this one was edited from, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<u64>,
    /// Edited here since it last went to another device (local only).
    #[serde(default, skip_serializing_if = "not")]
    pub dirty: bool,
    /// The reminder; none: unknown (the version came from, or through, an
    /// app before reminders; see "Reminders").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remind: Option<Remind>,
}

/// A note's reminder. Ordered so that of two for one version Done wins,
/// then the later time.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Remind {
    /// Marked Done (on any device).
    #[serde(default, skip_serializing_if = "not")]
    pub done: bool,
    /// When, unix ms; none: no reminder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u64>,
}

impl Remind {
    /// A reminder as it's kept: Done only means something with a time.
    pub fn new(at: Option<u64>, done: bool) -> Remind {
        Remind {
            done: done && at.is_some(),
            at,
        }
    }
}

/// Note id -> note.
pub type NoteMap = BTreeMap<String, Entry>;

/// FNV-1a, 64 bits: small, stable on every platform.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.iter().chain(std::iter::once(&0xff)) {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

impl Entry {
    /// Which version this is, as a hash (digests and lineage).
    pub fn version(&self) -> u64 {
        fnv(&[&self.updated_at.to_le_bytes(), self.by.as_bytes()])
    }

    /// The version and its reminder, as a hash (digests between devices
    /// that know reminders).
    pub fn full_version(&self) -> u64 {
        let remind = match self.remind {
            None => vec![0],
            Some(r) => {
                let mut b = vec![1, u8::from(r.done), u8::from(r.at.is_some())];
                b.extend_from_slice(&r.at.unwrap_or(0).to_le_bytes());
                b
            }
        };
        fnv(&[&self.updated_at.to_le_bytes(), self.by.as_bytes(), &remind])
    }

    fn same_version(&self, other: &Entry) -> bool {
        self.updated_at == other.updated_at && self.by == other.by
    }

    /// The order merges go by: the newest wins; the same time goes by the
    /// device, then by the content, so every device agrees.
    #[allow(clippy::type_complexity)]
    fn order(&self) -> (u64, &str, bool, &str, &str, bool, bool, Option<&str>) {
        (
            self.updated_at,
            &self.by,
            self.deleted,
            &self.title,
            &self.body,
            self.pinned,
            self.checklist,
            self.color.as_deref(),
        )
    }

    /// This version replaces `other` when neither was edited from the other.
    pub fn wins_over(&self, other: &Entry) -> bool {
        self.order() > other.order()
    }

    /// Edited (perhaps over several steps) from `other`.
    fn descends_from(&self, other: &Entry) -> bool {
        self.seen.contains(&other.version())
    }

    /// Text that would be lost if `winner` replaced this.
    fn text_lost_to(&self, winner: &Entry) -> bool {
        !self.deleted
            && !(self.title.trim().is_empty() && self.body.trim().is_empty())
            && (self.title != winner.title || self.body != winner.body)
    }

    fn size(&self) -> usize {
        if self.deleted {
            0
        } else {
            self.title.len() + self.body.len()
        }
    }

    /// As it goes to another device (`reminders`: one that knows them).
    fn wire(&self, reminders: bool) -> Entry {
        Entry {
            dirty: false,
            remind: if reminders { self.remind } else { None },
            ..self.clone()
        }
    }

    /// As the apps see it.
    pub fn note(&self) -> Note {
        Note {
            id: self.id.clone(),
            title: self.title.clone(),
            body: self.body.clone(),
            pinned: self.pinned,
            checklist: self.checklist,
            color: self.color.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            by: self.by.clone(),
            by_name: self.by_name.clone(),
            conflict_of: self.conflict_of.clone(),
            remind_at: self.remind.and_then(|r| r.at),
            remind_done: self.remind.is_some_and(|r| r.done),
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn valid_node(node: &str) -> bool {
    !node.trim().is_empty() && node.len() <= MAX_NODE && !node.chars().any(char::is_control)
}

/// A colour name: a short lowercase word.
fn valid_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= MAX_TOKEN
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A title as it's kept: one line, no control characters, ends trimmed,
/// at most `MAX_TITLE` characters.
pub fn tidy_title(title: &str) -> String {
    let t: String = title
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .take(MAX_TITLE)
        .collect();
    t.trim().to_string()
}

/// A body as it's kept: line ends as `\n`.
fn tidy_body(body: &str) -> String {
    body.replace("\r\n", "\n").replace('\r', "\n")
}

/// A device name as it's kept with a note.
fn tidy_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A note as it arrived, or `None` if it can't be kept as it is. Nothing
/// that makes the version is changed (that would split one version into
/// two contents): a note that isn't valid is left out instead.
pub fn sanitize(mut e: Entry) -> Option<Entry> {
    let ok = valid_id(&e.id)
        && valid_node(&e.by)
        && e.body.len() <= MAX_BODY
        && e.title.chars().count() <= MAX_TITLE
        && !e.title.chars().any(char::is_control)
        && e.color.as_deref().is_none_or(valid_token)
        && e.conflict_of.as_deref().is_none_or(valid_id);
    if !ok {
        return None;
    }
    e.by_name = tidy_name(&e.by_name);
    if e.seen.len() > LINEAGE {
        e.seen.drain(..e.seen.len() - LINEAGE);
    }
    e.dirty = false;
    Some(e)
}

/// Notes as they arrived (a JSON array), each checked (`sanitize`).
pub fn parse(v: &Value) -> Vec<Entry> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|e| sanitize(serde_json::from_value::<Entry>(e.clone()).ok()?))
        .take(MAX_ENTRIES)
        .collect()
}

/// A digest as it arrived: id -> version hash.
pub fn parse_digest(v: &Value) -> HashMap<String, u64> {
    let Some(obj) = v.as_object() else {
        return HashMap::new();
    };
    obj.iter()
        .filter(|(id, _)| valid_id(id))
        .filter_map(|(id, h)| Some((id.clone(), h.as_u64()?)))
        .take(MAX_ENTRIES)
        .collect()
}

/// The hash a digest names a note by: its version, and with `full` (both
/// ends know reminders) its reminder too.
fn hash(e: &Entry, full: bool) -> u64 {
    if full {
        e.full_version()
    } else {
        e.version()
    }
}

/// Every note's version, for a `notes_digest` (`full`: see `hash`).
pub fn digest(map: &NoteMap, full: bool) -> BTreeMap<String, u64> {
    map.iter()
        .map(|(id, e)| (id.clone(), hash(e, full)))
        .collect()
}

/// What a digest from another device asks of this one: the ids to send it
/// (its version differs, or it lacks a note that isn't deleted here) and
/// the ids to ask it for (notes this device doesn't have). `full`: the
/// digest hashes reminders too (see `hash`).
pub fn diff(
    map: &NoteMap,
    theirs: &HashMap<String, u64>,
    full: bool,
) -> (Vec<String>, Vec<String>) {
    let send = map
        .iter()
        .filter(|(id, e)| match theirs.get(*id) {
            Some(v) => *v != hash(e, full),
            None => !e.deleted,
        })
        .map(|(id, _)| id.clone())
        .collect();
    let want = theirs
        .keys()
        .filter(|id| !map.contains_key(*id))
        .cloned()
        .collect();
    (send, want)
}

fn live_count(map: &NoteMap) -> usize {
    map.values().filter(|e| !e.deleted).count()
}

fn total_size(map: &NoteMap) -> usize {
    map.values().map(Entry::size).sum()
}

/// Forget tombstones older than `TOMBSTONE_DAYS`. True if any went.
pub fn prune(map: &mut NoteMap, now: u64) -> bool {
    let before = map.len();
    map.retain(|_, e| !e.deleted || e.updated_at.saturating_add(TOMBSTONE_MS) >= now);
    map.len() != before
}

/// Make room for one more entry: drop the oldest tombstone if the map is
/// at `MAX_ENTRIES`.
fn make_room(map: &mut NoteMap) {
    while map.len() >= MAX_ENTRIES {
        let oldest = map
            .values()
            .filter(|e| e.deleted)
            .min_by(|a, b| a.order().cmp(&b.order()))
            .map(|e| e.id.clone());
        match oldest {
            Some(id) => {
                map.remove(&id);
            }
            None => return,
        }
    }
}

/// `e` (new to `map`, or replacing `prev`) fits the limits.
fn fits(map: &NoteMap, e: &Entry, prev: Option<&Entry>) -> bool {
    let was_live = prev.is_some_and(|p| !p.deleted);
    let count_ok = e.deleted || was_live || live_count(map) < MAX_NOTES;
    let size_ok = total_size(map) - prev.map(Entry::size).unwrap_or(0) + e.size() <= MAX_TOTAL;
    count_ok && size_ok
}

/// The copy of `loser` kept when it lost a conflict: the same on every
/// device that sees the conflict.
pub fn conflict_copy(loser: &Entry) -> Entry {
    let id = format!(
        "c{:016x}",
        fnv(&[
            loser.id.as_bytes(),
            &loser.updated_at.to_le_bytes(),
            loser.by.as_bytes()
        ])
    );
    let from = if loser.by_name.is_empty() {
        loser.by.chars().take(8).collect()
    } else {
        loser.by_name.clone()
    };
    let suffix = format!(" (conflict from {from})");
    let room = MAX_TITLE.saturating_sub(suffix.chars().count());
    let base: String = loser.title.chars().take(room).collect();
    let title: String = format!("{}{suffix}", base.trim_end())
        .trim()
        .chars()
        .take(MAX_TITLE)
        .collect();
    Entry {
        id,
        title,
        body: loser.body.clone(),
        pinned: false,
        checklist: loser.checklist,
        color: loser.color.clone(),
        created_at: loser.updated_at,
        updated_at: loser.updated_at,
        by: loser.by.clone(),
        by_name: loser.by_name.clone(),
        deleted: false,
        conflict_of: Some(loser.id.clone()),
        seen: Vec::new(),
        dirty: false,
        // A copy never reminds again (the note it split from does).
        remind: None,
    }
}

/// What a merge did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// Notes that changed here (conflict copies included).
    pub changed: Vec<String>,
    /// Notes worth sending back: this device has a newer version, or made
    /// a conflict copy the sender should have too.
    pub answer: Vec<String>,
    /// Conflict copies made.
    pub conflicts: Vec<String>,
}

fn keep_copy(map: &mut NoteMap, loser: &Entry, out: &mut Merged) {
    let copy = conflict_copy(loser);
    let id = copy.id.clone();
    if !map.contains_key(&id) {
        make_room(map);
        map.insert(id.clone(), copy);
        out.changed.push(id.clone());
        out.conflicts.push(id.clone());
    }
    out.answer.push(id);
}

/// The reminder `winner` keeps when it replaces (or beats) `other`: its
/// own, or `other`'s when it has none (unknown: from an app before
/// reminders); and a Done made on `other`'s side for the same time. True
/// if `winner` changed.
fn carry(winner: &mut Entry, other: &Entry) -> bool {
    match (&mut winner.remind, other.remind) {
        (w @ None, Some(o)) => {
            *w = Some(o);
            true
        }
        (Some(w), Some(o)) if w.at == o.at && o.done && !w.done => {
            w.done = true;
            true
        }
        _ => false,
    }
}

/// Merge notes that arrived (checked: `parse`) into `map` (see the module
/// notes). `now` (unix ms) drops tombstones too old to keep.
pub fn merge(map: &mut NoteMap, incoming: Vec<Entry>, now: u64) -> Merged {
    let mut out = Merged::default();
    for mut inc in incoming {
        let id = inc.id.clone();
        let Some(cur) = map.get_mut(&id) else {
            let stale = inc.deleted && inc.updated_at.saturating_add(TOMBSTONE_MS) < now;
            if !stale && fits(map, &inc, None) {
                make_room(map);
                map.insert(id.clone(), inc);
                out.changed.push(id);
            }
            continue;
        };
        if cur.same_version(&inc) {
            // The same version: only its reminder can differ (see
            // "Reminders"); the greater one is kept everywhere.
            if let Some(r) = inc.remind.filter(|r| cur.remind.is_none_or(|c| *r > c)) {
                cur.remind = Some(r);
                out.changed.push(id);
            }
            continue;
        }
        if inc.descends_from(cur) {
            if inc.remind.is_none() {
                inc.remind = cur.remind;
            }
            let cur = map.get(&id);
            if fits(map, &inc, cur) {
                map.insert(id.clone(), inc);
                out.changed.push(id);
            }
            continue;
        }
        if cur.descends_from(&inc) {
            out.answer.push(id);
            continue;
        }
        // A conflict: both were edited since they last agreed.
        if !inc.wins_over(cur) {
            if carry(cur, &inc) {
                out.changed.push(id.clone());
            }
            let cur = cur.clone();
            if inc.text_lost_to(&cur) {
                keep_copy(map, &inc, &mut out);
            }
            out.answer.push(id);
            continue;
        }
        let cur = cur.clone();
        carry(&mut inc, &cur);
        if cur.text_lost_to(&inc) {
            keep_copy(map, &cur, &mut out);
        }
        map.insert(id.clone(), inc);
        out.changed.push(id);
    }
    if prune(map, now) {
        out.changed.retain(|id| map.contains_key(id));
        out.answer.retain(|id| map.contains_key(id));
    }
    out
}

/// What the apps save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub checklist: bool,
    pub color: Option<String>,
    /// The reminder (none: this end doesn't know reminders).
    pub remind: Option<Remind>,
}

/// Save a note here: `id` (none: a new note) with `edit`, as made at `now`
/// by `me` (`name`). Always newer than what the map had for it, whatever
/// the clock says. Saving a deleted note brings it back (undo).
pub fn put(
    map: &mut NoteMap,
    id: Option<&str>,
    edit: Edit,
    me: (&str, &str),
    now: u64,
) -> crate::Result<Entry> {
    let id = match id {
        Some(id) if valid_id(id) => id.to_string(),
        Some(_) => return Err(CoreError::InvalidInput("which note?".into())),
        None => uuid::Uuid::new_v4().simple().to_string(),
    };
    let body = tidy_body(&edit.body);
    if body.len() > MAX_BODY {
        return Err(CoreError::InvalidInput(format!(
            "a note holds at most {} KB",
            MAX_BODY / 1024
        )));
    }
    let color = edit.color.filter(|c| !c.is_empty());
    if color.as_deref().is_some_and(|c| !valid_token(c)) {
        return Err(CoreError::InvalidInput("which colour?".into()));
    }
    let prev = map.get(&id);
    let e = Entry {
        id: id.clone(),
        title: tidy_title(&edit.title),
        body,
        pinned: edit.pinned,
        checklist: edit.checklist,
        color,
        created_at: prev
            .filter(|p| !p.deleted && p.created_at > 0)
            .map(|p| p.created_at)
            .unwrap_or(now),
        conflict_of: prev.and_then(|p| p.conflict_of.clone()),
        remind: edit.remind.map(|r| Remind::new(r.at, r.done)),
        ..step(prev, me, now)
    };
    if !fits(map, &e, prev) {
        return Err(CoreError::InvalidInput(if live_count(map) >= MAX_NOTES {
            format!("at most {MAX_NOTES} notes")
        } else {
            format!("at most {} MB of notes", MAX_TOTAL / (1024 * 1024))
        }));
    }
    if prev.is_none() {
        make_room(map);
    }
    map.insert(id, e.clone());
    Ok(e)
}

/// The version fields of an edit of `prev` made here: newer than it, and
/// with it in the lineage unless it's an edit of ours nobody has seen.
fn step(prev: Option<&Entry>, me: (&str, &str), now: u64) -> Entry {
    let mut seen = prev.map(|p| p.seen.clone()).unwrap_or_default();
    if let Some(p) = prev {
        if !(p.dirty && p.by == me.0) {
            seen.push(p.version());
            if seen.len() > LINEAGE {
                seen.drain(..seen.len() - LINEAGE);
            }
        }
    }
    Entry {
        updated_at: now.max(prev.map_or(0, |p| p.updated_at.saturating_add(1))),
        by: me.0.to_string(),
        by_name: tidy_name(me.1),
        seen,
        dirty: true,
        ..Entry::default()
    }
}

/// Delete note `id` here (a tombstone, as made at `now` by `me`).
pub fn remove(map: &mut NoteMap, id: &str, me: (&str, &str), now: u64) -> crate::Result<Entry> {
    let Some(prev) = map.get(id).filter(|p| !p.deleted) else {
        return Err(CoreError::InvalidInput("no such note".into()));
    };
    let e = Entry {
        id: id.to_string(),
        created_at: prev.created_at,
        deleted: true,
        ..step(Some(prev), me, now)
    };
    map.insert(id.to_string(), e.clone());
    Ok(e)
}

/// The notes, pinned first, then the most recently changed.
pub fn listed(map: &NoteMap) -> Vec<Note> {
    let mut v: Vec<&Entry> = map.values().filter(|e| !e.deleted).collect();
    v.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
    v.into_iter().map(Entry::note).collect()
}

/// Split notes into `notes` messages of at most `BATCH_BYTES` (one note
/// over that goes alone); `want` rides on the first. Each says `v`, this
/// end's version; `reminders`: for a device that knows them (else they
/// stay out).
fn batches(from: &str, notes: Vec<Entry>, want: &[String], v: u64, reminders: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<Value> = Vec::new();
    let mut size = 0;
    let mut want = Some(want);
    let mut flush = |cur: &mut Vec<Value>, want: &mut Option<&[String]>| {
        let mut msg =
            json!({ "type": "notes", "from": from, "v": v, "notes": std::mem::take(cur) });
        if let Some(w) = want.take().filter(|w| !w.is_empty()) {
            msg["want"] = json!(w);
        }
        out.push(msg.to_string());
    };
    for n in notes {
        let one = serde_json::to_value(n.wire(reminders)).unwrap_or_default();
        let len = one.to_string().len();
        if !cur.is_empty() && size + len > BATCH_BYTES {
            flush(&mut cur, &mut want);
            size = 0;
        }
        size += len;
        cur.push(one);
    }
    if !cur.is_empty() || want.is_some_and(|w| !w.is_empty()) {
        flush(&mut cur, &mut want);
    }
    out
}

// --- the engine's side -------------------------------------------------------

/// The notes and which devices understand them.
#[derive(Default)]
pub struct Notes {
    /// `None` until read.
    map: Mutex<Option<NoteMap>>,
    /// Node -> the notes version it said (0: none).
    caps: Mutex<HashMap<String, u64>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn store_path<P: Platform>(ctx: &NetCtx<P>) -> Option<PathBuf> {
    ctx.platform.data_file(FILE)
}

fn load<P: Platform>(ctx: &NetCtx<P>) -> NoteMap {
    let json = match store_path(ctx) {
        Some(path) => std::fs::read_to_string(path).ok(),
        None => ctx.platform.kv_get(KV_KEY),
    };
    let notes = json
        .and_then(|j| serde_json::from_str::<Vec<Entry>>(&j).ok())
        .unwrap_or_default();
    let mut map = NoteMap::new();
    for n in notes {
        // Kept as saved (with `dirty`), but checked like what arrives.
        let dirty = n.dirty;
        if let Some(mut n) = sanitize(n) {
            n.dirty = dirty;
            map.insert(n.id.clone(), n);
        }
    }
    prune(&mut map, now_ms());
    map
}

fn save<P: Platform>(ctx: &NetCtx<P>, map: &NoteMap) {
    let Ok(json) = serde_json::to_string(&map.values().collect::<Vec<_>>()) else {
        return;
    };
    match store_path(ctx) {
        // Nowhere to report it: the notes stay in memory until the next save.
        Some(path) => {
            let _ = write_atomic(&path, &json);
        }
        None => ctx.platform.kv_set(KV_KEY, &json),
    }
}

/// Write `path` whole or not at all: a temporary file renamed over it.
fn write_atomic(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Run `f` on the notes (read the first time); saved when `f` says it
/// changed them.
fn with_map<P: Platform, R>(ctx: &NetCtx<P>, f: impl FnOnce(&mut NoteMap) -> (R, bool)) -> R {
    let mut slot = ctx.extras.notes.map.lock();
    let map = slot.get_or_insert_with(|| load(ctx));
    let (r, changed) = f(map);
    if changed {
        save(ctx, map);
    }
    r
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> (String, String) {
    let id = ctx.identity.lock();
    (id.node_id.clone(), id.name.clone())
}

/// Every note, pinned first, then the most recently changed.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> Vec<Note> {
    with_map(ctx, |m| (listed(m), false))
}

/// One note, if it's there (and not deleted).
pub fn get<P: Platform>(ctx: &NetCtx<P>, id: &str) -> Option<Note> {
    with_map(ctx, |m| {
        (m.get(id).filter(|e| !e.deleted).map(Entry::note), false)
    })
}

/// The apps save a note (`id` none: a new one). Sent to the linked devices
/// that understand notes. Makes no event.
pub fn save_note<P: Platform>(
    ctx: &NetCtx<P>,
    id: Option<&str>,
    edit: Edit,
) -> crate::Result<Note> {
    let (node, name) = me(ctx);
    let now = now_ms();
    let edit = Edit {
        remind: edit.remind.filter(|_| ctx.config.note_reminders),
        ..edit
    };
    let e = with_map(ctx, |m| {
        let r = put(m, id, edit, (&node, &name), now);
        if r.is_ok() {
            prune(m, now);
        }
        let ok = r.is_ok();
        (r, ok)
    })?;
    send_all(ctx, &e.id);
    Ok(e.note())
}

/// The apps delete a note (it can be brought back by saving it again).
pub fn delete_note<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<()> {
    let (node, name) = me(ctx);
    with_map(ctx, |m| {
        let r = remove(m, id, (&node, &name), now_ms());
        let ok = r.is_ok();
        (r, ok)
    })?;
    send_all(ctx, id);
    Ok(())
}

/// This end's notes version: 2 with reminders, 1 without (an app from
/// before them, as tests play one).
fn here<P: Platform>(ctx: &NetCtx<P>) -> u64 {
    if ctx.config.note_reminders {
        VERSION
    } else {
        1
    }
}

/// Say in this device's status (or `addr_update`) that it understands
/// notes, and which version.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.notes {
        status[FLAG] = here(ctx).into();
    }
}

/// `node` said (`flag`: the `notes` value of its status or `addr_update`)
/// whether it understands notes, and which version. Kept across restarts;
/// newly understood (or newer), it gets this device's digest now.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let level = flag.as_u64().unwrap_or(0);
    if set_caps(ctx, node, level) && level >= 1 {
        send_digest(ctx, node);
    }
}

/// True if it changed.
fn set_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, level: u64) -> bool {
    if node.is_empty() || caps(ctx, node) == level {
        return false;
    }
    ctx.extras.notes.caps.lock().insert(node.to_string(), level);
    let kept = if level == 0 {
        String::new()
    } else {
        level.to_string()
    };
    ctx.platform.kv_set(&format!("{CAPS_PREFIX}{node}"), &kept);
    true
}

/// The notes version `node` said (0: none).
fn caps<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    if let Some(level) = ctx.extras.notes.caps.lock().get(node) {
        return *level;
    }
    let level = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    ctx.extras.notes.caps.lock().insert(node.to_string(), level);
    level
}

/// `node` said it understands notes.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    caps(ctx, node) >= 1
}

/// Reminders go to `node` (both ends know them).
pub fn reminders_with<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.note_reminders && caps(ctx, node) >= VERSION
}

/// A link to `node` just came up: send it the digest.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    send_digest(ctx, node);
}

/// Notes go only to real devices that understand them, with this end on.
fn may_send<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.notes && !node.starts_with("manual:") && supports(ctx, node)
}

fn send_digest<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !may_send(ctx, node) {
        return;
    }
    let (from, _) = me(ctx);
    let full = reminders_with(ctx, node);
    let digest = with_map(ctx, |m| (digest(m, full), false));
    let msg = json!({
        "type": "notes_digest",
        "from": from,
        "v": here(ctx),
        "full": full,
        "digest": digest,
    });
    send_control_to(ctx, node, &msg.to_string());
}

/// Send notes `ids` (and ask for `want`) to `node`. What goes is no longer
/// only this device's (`dirty` cleared).
fn send_notes<P: Platform>(ctx: &NetCtx<P>, node: &str, ids: &[String], want: &[String]) {
    if !may_send(ctx, node) || (ids.is_empty() && want.is_empty()) {
        return;
    }
    let (from, _) = me(ctx);
    let notes = with_map(ctx, |m| {
        let mut out = Vec::new();
        let mut cleared = false;
        for id in ids {
            if let Some(e) = m.get_mut(id) {
                cleared |= std::mem::take(&mut e.dirty);
                out.push(e.clone());
            }
        }
        (out, cleared)
    });
    let reminders = reminders_with(ctx, node);
    for msg in batches(&from, notes, want, here(ctx), reminders) {
        send_control_to(ctx, node, &msg);
    }
}

/// Send note `id` to every linked device that understands notes.
fn send_all<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    let ids = [id.to_string()];
    for node in linked {
        send_notes(ctx, &node, &ids, &[]);
    }
}

pub struct NotesHandler;

impl<P: Platform> ControlHandler<P> for NotesHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["notes", "notes_digest"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return; // only your own devices share your notes
        }
        // Sending it says it understands it (an app from before
        // reminders doesn't say `v`: 1).
        let v = msg.json("v").as_u64().unwrap_or(1).max(1);
        set_caps(ctx, &from, v);
        if msg.kind() == "notes_digest" {
            let theirs = parse_digest(&msg.json("digest"));
            let full = msg.json("full").as_bool() == Some(true) && ctx.config.note_reminders;
            let (send, want) = with_map(ctx, |m| (diff(m, &theirs, full), false));
            send_notes(ctx, &from, &send, &want);
            return;
        }
        let mut incoming = parse(&msg.json("notes"));
        if !ctx.config.note_reminders {
            // An app from before reminders never kept them.
            for e in &mut incoming {
                e.remind = None;
            }
        }
        let want: Vec<String> = msg
            .json("want")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .filter(|id| valid_id(id))
                    .take(MAX_ENTRIES)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let now = now_ms();
        let (merged, notes, removed) = with_map(ctx, |m| {
            let merged = merge(m, incoming, now);
            let mut notes = Vec::new();
            let mut removed = Vec::new();
            for id in &merged.changed {
                match m.get(id) {
                    Some(e) if !e.deleted => notes.push(e.note()),
                    _ => removed.push(id.clone()),
                }
            }
            let dirty = !merged.changed.is_empty();
            ((merged, notes, removed), dirty)
        });
        if !notes.is_empty() || !removed.is_empty() {
            ctx.sink.emit(CoreEvent::NotesChanged { notes, removed });
        }
        let mut answer = merged.answer;
        for id in want {
            if !answer.contains(&id) {
                answer.push(id);
            }
        }
        send_notes(ctx, &from, &answer, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: (&str, &str) = ("node-a", "Mac");
    const B: (&str, &str) = ("node-b", "Pixel");

    fn edit(title: &str, body: &str) -> Edit {
        Edit {
            title: title.into(),
            body: body.into(),
            ..Edit::default()
        }
    }

    /// What `from` would send of note `id` (as it goes on the wire).
    fn sent(from: &mut NoteMap, id: &str) -> Vec<Entry> {
        let e = from.get_mut(id).unwrap();
        e.dirty = false;
        vec![parse(&json!([e.wire(true)])).remove(0)]
    }

    #[test]
    fn a_later_edit_replaces_the_one_it_was_made_from() {
        let mut a = NoteMap::new();
        let n = put(&mut a, None, edit("Shopping", "milk"), A, 10).unwrap();
        let mut b = NoteMap::new();
        let m = merge(&mut b, sent(&mut a, &n.id), 11);
        assert_eq!(m.changed, std::slice::from_ref(&n.id));
        // b edits it twice before it goes back (one step in the lineage).
        put(&mut b, Some(&n.id), edit("Shopping", "milk, eggs"), B, 20).unwrap();
        put(
            &mut b,
            Some(&n.id),
            edit("Shopping", "milk, eggs, tea"),
            B,
            30,
        )
        .unwrap();
        assert_eq!(b[&n.id].seen.len(), 1);
        let m = merge(&mut a, sent(&mut b, &n.id), 31);
        assert_eq!(m.changed, std::slice::from_ref(&n.id));
        assert!(m.conflicts.is_empty() && m.answer.is_empty());
        assert_eq!(a[&n.id].body, "milk, eggs, tea");
        assert_eq!(a, b);
        // The older version arriving again changes nothing, and is answered.
        let old = Entry {
            body: "milk".into(),
            updated_at: 10,
            by: A.0.into(),
            seen: vec![],
            ..a[&n.id].clone()
        };
        let m = merge(&mut a, vec![old], 40);
        assert!(m.changed.is_empty());
        assert_eq!(m.answer, std::slice::from_ref(&n.id));
        assert_eq!(a[&n.id].body, "milk, eggs, tea");
    }

    #[test]
    fn concurrent_edits_keep_both_texts_the_same_on_both_devices() {
        let mut a = NoteMap::new();
        let n = put(&mut a, None, edit("Plan", "start"), A, 10).unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &n.id), 11);
        // Both edit before hearing of the other.
        put(&mut a, Some(&n.id), edit("Plan", "from the mac"), A, 20).unwrap();
        put(&mut b, Some(&n.id), edit("Plan", "from the phone"), B, 25).unwrap();
        let to_b = sent(&mut a, &n.id);
        let to_a = sent(&mut b, &n.id);
        let ma = merge(&mut a, to_a, 30);
        let mb = merge(&mut b, to_b, 30);
        // The newer (b's) keeps the note; a's text is a copy on both.
        assert_eq!(a[&n.id].body, "from the phone");
        assert_eq!(b[&n.id].body, "from the phone");
        assert_eq!(ma.conflicts, mb.conflicts);
        let copy = a[&ma.conflicts[0]].clone();
        assert_eq!(copy.title, "Plan (conflict from Mac)");
        assert_eq!(copy.body, "from the mac");
        assert_eq!(copy.conflict_of.as_deref(), Some(n.id.as_str()));
        assert_eq!(a, b);
        // Each answers with the copy; it changes nothing on the other side.
        assert!(ma.answer.contains(&copy.id) && mb.answer.contains(&copy.id));
        let again = merge(&mut a, vec![b[&copy.id].wire(true)], 31);
        assert_eq!(again, Merged::default());
    }

    #[test]
    fn an_edit_that_conflicts_with_a_delete_keeps_its_text() {
        let mut a = NoteMap::new();
        let n = put(&mut a, None, edit("Ideas", "one"), A, 10).unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &n.id), 11);
        put(&mut a, Some(&n.id), edit("Ideas", "one, two"), A, 20).unwrap();
        remove(&mut b, &n.id, B, 30).unwrap();
        let m = merge(&mut a, sent(&mut b, &n.id), 31);
        assert!(a[&n.id].deleted, "the newer delete wins");
        assert_eq!(a[&m.conflicts[0]].body, "one, two");
        // A delete that loses makes no copy.
        let mut c = NoteMap::new();
        let n2 = put(&mut c, None, edit("x", "y"), A, 10).unwrap();
        let mut d = NoteMap::new();
        merge(&mut d, sent(&mut c, &n2.id), 11);
        remove(&mut c, &n2.id, A, 20).unwrap();
        put(&mut d, Some(&n2.id), edit("x", "y z"), B, 30).unwrap();
        let m = merge(&mut d, sent(&mut c, &n2.id), 31);
        assert!(m.conflicts.is_empty());
        assert_eq!(d[&n2.id].body, "y z");
    }

    #[test]
    fn same_text_or_only_a_pin_needs_no_copy() {
        let mut a = NoteMap::new();
        let n = put(&mut a, None, edit("T", "same"), A, 10).unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &n.id), 11);
        let pinned = Edit {
            pinned: true,
            ..edit("T", "same")
        };
        put(&mut a, Some(&n.id), pinned, A, 20).unwrap();
        put(&mut b, Some(&n.id), edit("T", "same"), B, 25).unwrap();
        let m = merge(&mut a, sent(&mut b, &n.id), 30);
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn tombstones_go_after_thirty_days() {
        let mut a = NoteMap::new();
        let n = put(&mut a, None, edit("Old", "x"), A, 10).unwrap();
        remove(&mut a, &n.id, A, 20).unwrap();
        assert!(a[&n.id].deleted && a[&n.id].body.is_empty());
        assert!(listed(&a).is_empty());
        assert!(!prune(&mut a, 20 + TOMBSTONE_MS));
        assert!(prune(&mut a, 21 + TOMBSTONE_MS));
        assert!(a.is_empty());
        // One that old isn't taken from another device either.
        let stale = Entry {
            id: "gone".into(),
            deleted: true,
            updated_at: 5,
            by: B.0.into(),
            ..Entry::default()
        };
        assert!(merge(&mut a, vec![stale], 10 + TOMBSTONE_MS)
            .changed
            .is_empty());
        // Saving a deleted note brings it back.
        let mut c = NoteMap::new();
        let n = put(&mut c, None, edit("Back", "again"), A, 10).unwrap();
        remove(&mut c, &n.id, A, 20).unwrap();
        let back = put(&mut c, Some(&n.id), edit("Back", "again"), A, 30).unwrap();
        assert!(!back.deleted);
        assert_eq!(listed(&c).len(), 1);
    }

    #[test]
    fn the_digest_names_only_what_differs() {
        let mut a = NoteMap::new();
        let same = put(&mut a, None, edit("same", ""), A, 1).unwrap();
        let newer = put(&mut a, None, edit("newer here", ""), A, 2).unwrap();
        let only_a = put(&mut a, None, edit("only here", ""), A, 3).unwrap();
        let gone = put(&mut a, None, edit("deleted here", ""), A, 4).unwrap();
        remove(&mut a, &gone.id, A, 5).unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &same.id), 6);
        let mut older = a[&newer.id].clone();
        older.updated_at = 1;
        b.insert(newer.id.clone(), older);
        b.insert(
            "only-b".into(),
            Entry {
                id: "only-b".into(),
                by: B.0.into(),
                updated_at: 3,
                ..Entry::default()
            },
        );
        let theirs: HashMap<String, u64> = digest(&b, false).into_iter().collect();
        let (mut send, want) = diff(&a, &theirs, false);
        send.sort();
        let mut expect = vec![newer.id.clone(), only_a.id.clone()];
        expect.sort();
        // The tombstone isn't sent to a device that never had the note.
        assert_eq!(send, expect);
        assert_eq!(want, ["only-b"]);
        let wire = serde_json::to_value(digest(&a, false)).unwrap();
        assert_eq!(parse_digest(&wire).len(), 4);
    }

    #[test]
    fn limits_hold() {
        let mut m = NoteMap::new();
        let big = "x".repeat(MAX_BODY + 1);
        assert!(put(&mut m, None, edit("big", &big), A, 1).is_err());
        assert!(put(&mut m, Some("bad id!"), edit("x", ""), A, 1).is_err());
        let bad_colour = Edit {
            color: Some("Not A Colour".into()),
            ..edit("x", "")
        };
        assert!(put(&mut m, None, bad_colour, A, 1).is_err());
        for i in 0..MAX_NOTES {
            put(&mut m, Some(&format!("n{i}")), edit("x", ""), A, 1).unwrap();
        }
        assert!(put(&mut m, None, edit("one too many", ""), A, 2).is_err());
        // Editing one that's there still works.
        assert!(put(&mut m, Some("n0"), edit("edited", ""), A, 3).is_ok());
        // Nor is one more taken from another device.
        let other = Entry {
            id: "theirs".into(),
            title: "t".into(),
            by: B.0.into(),
            updated_at: 4,
            ..Entry::default()
        };
        assert!(merge(&mut m, vec![other.clone()], 5).changed.is_empty());
        // After a delete there's room again.
        remove(&mut m, "n1", A, 6).unwrap();
        assert_eq!(merge(&mut m, vec![other], 7).changed, ["theirs"]);

        // Size: 16 MB in all.
        let mut s = NoteMap::new();
        let body = "y".repeat(MAX_BODY);
        for i in 0..(MAX_TOTAL / MAX_BODY) {
            put(&mut s, Some(&format!("s{i}")), edit("", &body), A, 1).unwrap();
        }
        assert!(put(&mut s, None, edit("", "z"), A, 2).is_err());
    }

    #[test]
    fn what_arrives_is_checked_not_changed() {
        let v = json!([
            {"id": "ok", "title": "Fine", "body": "a\nb", "updated_at": 3, "by": "x",
             "by_name": "  Mac  ", "seen": (0..40).collect::<Vec<u64>>()},
            {"id": "bad id", "updated_at": 3, "by": "x"},
            {"id": "noone", "updated_at": 3},
            {"id": "long", "body": "x".repeat(MAX_BODY + 1), "updated_at": 3, "by": "x"},
            {"id": "colour", "color": "Red!", "updated_at": 3, "by": "x"},
            {"id": "dirty", "updated_at": 3, "by": "x", "dirty": true},
            "not a note",
        ]);
        let notes = parse(&v);
        let ids: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["ok", "dirty"]);
        assert_eq!(notes[0].by_name, "Mac");
        assert_eq!(notes[0].seen.len(), LINEAGE);
        assert_eq!(notes[0].seen[0], 40 - LINEAGE as u64);
        assert!(!notes[1].dirty);
    }

    #[test]
    fn titles_are_tidied_and_conflict_titles_fit() {
        assert_eq!(tidy_title("  Two\nlines\t "), "Two lines");
        let loser = Entry {
            id: "n".into(),
            title: "t".repeat(MAX_TITLE),
            by: "abcdefghijk".into(),
            updated_at: 1,
            ..Entry::default()
        };
        let c = conflict_copy(&loser);
        assert_eq!(c.title.chars().count(), MAX_TITLE);
        assert!(c.title.ends_with("(conflict from abcdefgh)"));
        assert_eq!(c, conflict_copy(&loser), "the same everywhere");
        assert!(sanitize(c).is_some());
    }

    #[test]
    fn batches_stay_small_and_carry_want_once() {
        let notes: Vec<Entry> = (0..10)
            .map(|i| Entry {
                id: format!("n{i}"),
                body: "x".repeat(60 * 1024),
                by: "a".into(),
                ..Entry::default()
            })
            .collect();
        let msgs = batches("me", notes, &["w".to_string()], 2, true);
        assert!(msgs.len() >= 3);
        let parsed: Vec<Value> = msgs
            .iter()
            .map(|m| serde_json::from_str(m).unwrap())
            .collect();
        assert!(msgs.iter().all(|m| m.len() <= BATCH_BYTES + 1024));
        assert_eq!(parsed.iter().filter(|m| m.get("want").is_some()).count(), 1);
        let total: usize = parsed
            .iter()
            .map(|m| m["notes"].as_array().unwrap().len())
            .sum();
        assert_eq!(total, 10);
        // Only a want: one message with no notes.
        assert_eq!(batches("me", vec![], &["w".to_string()], 2, true).len(), 1);
        assert!(batches("me", vec![], &[], 2, true).is_empty());
    }

    #[test]
    fn merging_ends_the_same_in_any_order() {
        let base = Entry {
            id: "n".into(),
            title: "T".into(),
            body: "base".into(),
            updated_at: 1,
            by: "o".into(),
            ..Entry::default()
        };
        let v = |body: &str, at: u64, by: &str| Entry {
            body: body.into(),
            updated_at: at,
            by: by.into(),
            by_name: by.into(),
            seen: vec![base.version()],
            ..base.clone()
        };
        let x = v("x", 5, "a");
        let y = v("y", 6, "b");
        let z = v("z", 6, "c");
        let orders = [
            [&base, &x, &y, &z],
            [&z, &y, &x, &base],
            [&y, &base, &z, &x],
            [&x, &z, &base, &y],
        ];
        let results: Vec<NoteMap> = orders
            .iter()
            .map(|o| {
                let mut m = NoteMap::new();
                for e in o {
                    merge(&mut m, vec![(*e).clone()], 10);
                }
                m
            })
            .collect();
        for r in &results {
            assert_eq!(
                r.keys().collect::<Vec<_>>(),
                results[0].keys().collect::<Vec<_>>()
            );
            assert_eq!(r["n"], results[0]["n"]);
        }
        assert_eq!(results[0]["n"].body, "z");
    }

    fn remind(at: u64, done: bool) -> Option<Remind> {
        Some(Remind::new(Some(at), done))
    }

    fn with_remind(title: &str, body: &str, r: Option<Remind>) -> Edit {
        Edit {
            remind: r,
            ..edit(title, body)
        }
    }

    /// What `from` would send of note `id` to an app from before
    /// reminders, as that app keeps it (no `remind`: it doesn't know it).
    fn sent_to_old(from: &mut NoteMap, id: &str) -> Vec<Entry> {
        let e = from.get_mut(id).unwrap();
        e.dirty = false;
        vec![parse(&json!([e.wire(false)])).remove(0)]
    }

    #[test]
    fn a_reminder_and_its_done_sync_like_the_text() {
        let mut a = NoteMap::new();
        let n = put(
            &mut a,
            None,
            with_remind("Call", "the bank", remind(1_000, false)),
            A,
            10,
        )
        .unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &n.id), 11);
        let got = b[&n.id].note();
        assert_eq!((got.remind_at, got.remind_done), (Some(1_000), false));
        // Done on b goes back to a.
        put(
            &mut b,
            Some(&n.id),
            with_remind("Call", "the bank", remind(1_000, true)),
            B,
            20,
        )
        .unwrap();
        let m = merge(&mut a, sent(&mut b, &n.id), 21);
        assert_eq!(m.changed, std::slice::from_ref(&n.id));
        assert!(a[&n.id].note().remind_done);
        // Snooze on a: a new time, not done.
        put(
            &mut a,
            Some(&n.id),
            with_remind("Call", "the bank", remind(5_000, false)),
            A,
            30,
        )
        .unwrap();
        merge(&mut b, sent(&mut a, &n.id), 31);
        let got = b[&n.id].note();
        assert_eq!((got.remind_at, got.remind_done), (Some(5_000), false));
        assert_eq!(a, b);
        // Done without a time means nothing.
        assert_eq!(Remind::new(None, true), Remind::default());
        // The wire: a note with no reminder still says so (`{}`) ...
        let plain = put(
            &mut a,
            None,
            with_remind("x", "", Some(Remind::default())),
            A,
            40,
        )
        .unwrap();
        let v = serde_json::to_value(plain.wire(true)).unwrap();
        assert_eq!(v["remind"], json!({}));
        // ... and an app from before reminders gets none at all.
        assert!(serde_json::to_value(plain.wire(false))
            .unwrap()
            .get("remind")
            .is_none());
    }

    #[test]
    fn an_older_app_editing_the_text_keeps_the_reminder() {
        let mut new = NoteMap::new();
        let n = put(
            &mut new,
            None,
            with_remind("Pay", "rent", remind(9_000, false)),
            A,
            10,
        )
        .unwrap();
        // The older app keeps the note, without the reminder ...
        let mut old = NoteMap::new();
        merge(&mut old, sent_to_old(&mut new, &n.id), 11);
        assert_eq!(old[&n.id].remind, None);
        // ... edits the text (it can't say anything of reminders) ...
        put(&mut old, Some(&n.id), edit("Pay", "rent and water"), B, 20).unwrap();
        let back = sent_to_old(&mut old, &n.id);
        assert_eq!(back[0].remind, None);
        // ... and the reminder is still there where it's known.
        let m = merge(&mut new, back, 21);
        assert_eq!(m.changed, std::slice::from_ref(&n.id));
        assert!(m.conflicts.is_empty());
        let got = new[&n.id].note();
        assert_eq!(got.body, "rent and water");
        assert_eq!((got.remind_at, got.remind_done), (Some(9_000), false));
        // Its own lineage still matches: the older app's digest names the
        // same version, nothing to send either way.
        let theirs: HashMap<String, u64> = digest(&old, false).into_iter().collect();
        assert_eq!(diff(&new, &theirs, false), (vec![], vec![]));
        // Done here, sent to the older app and edited there again: still Done.
        put(
            &mut new,
            Some(&n.id),
            with_remind("Pay", "rent and water", remind(9_000, true)),
            A,
            30,
        )
        .unwrap();
        merge(&mut old, sent_to_old(&mut new, &n.id), 31);
        put(&mut old, Some(&n.id), edit("Pay", "rent, water"), B, 40).unwrap();
        merge(&mut new, sent_to_old(&mut old, &n.id), 41);
        assert!(new[&n.id].note().remind_done);
        // A conflict with an older app's edit keeps the reminder too.
        put(
            &mut new,
            Some(&n.id),
            with_remind("Pay", "rent, water", remind(12_000, false)),
            A,
            50,
        )
        .unwrap();
        put(
            &mut old,
            Some(&n.id),
            edit("Pay", "rent, water, gas"),
            B,
            60,
        )
        .unwrap();
        let m = merge(&mut new, sent_to_old(&mut old, &n.id), 61);
        assert_eq!(new[&n.id].body, "rent, water, gas");
        assert_eq!(new[&n.id].remind, remind(12_000, false));
        assert_eq!(m.conflicts.len(), 1, "the text it beat is kept");
        assert_eq!(new[&m.conflicts[0]].remind, None, "a copy never reminds");
    }

    #[test]
    fn one_version_with_two_reminders_ends_the_same_everywhere() {
        let base = Entry {
            id: "n".into(),
            title: "T".into(),
            updated_at: 5,
            by: "o".into(),
            ..Entry::default()
        };
        let none = base.clone();
        let later = Entry {
            remind: remind(200, false),
            ..base.clone()
        };
        let done = Entry {
            remind: remind(100, true),
            ..base.clone()
        };
        // A reminder beats none (one passed on by an older app); Done beats
        // a later time; and whatever the order, the same.
        for order in [
            [&none, &later, &done],
            [&done, &later, &none],
            [&later, &none, &done],
        ] {
            let mut m = NoteMap::new();
            for e in order {
                merge(&mut m, vec![e.clone()], 10);
            }
            assert_eq!(m["n"].remind, remind(100, true));
        }
        let mut m = NoteMap::new();
        merge(&mut m, vec![none.clone()], 10);
        assert_eq!(merge(&mut m, vec![later.clone()], 10).changed, ["n"]);
        assert!(merge(&mut m, vec![none.clone()], 10).changed.is_empty());
        // Digests between apps that know reminders tell them apart; the
        // older apps' don't (the same version).
        assert_ne!(later.full_version(), none.full_version());
        assert_ne!(later.full_version(), done.full_version());
        assert_eq!(later.version(), none.version());
        let mut x = NoteMap::new();
        x.insert("n".into(), later);
        let mut y = NoteMap::new();
        y.insert("n".into(), none);
        let theirs: HashMap<String, u64> = digest(&y, true).into_iter().collect();
        assert_eq!(diff(&x, &theirs, true).0, ["n"]);
        let theirs: HashMap<String, u64> = digest(&y, false).into_iter().collect();
        assert!(diff(&x, &theirs, false).0.is_empty());
    }

    #[test]
    fn done_on_one_device_survives_an_edit_on_another() {
        let mut a = NoteMap::new();
        let n = put(
            &mut a,
            None,
            with_remind("Pills", "after lunch", remind(500, false)),
            A,
            10,
        )
        .unwrap();
        let mut b = NoteMap::new();
        merge(&mut b, sent(&mut a, &n.id), 11);
        // Apart: a marks it Done, b (later) edits the text.
        put(
            &mut a,
            Some(&n.id),
            with_remind("Pills", "after lunch", remind(500, true)),
            A,
            20,
        )
        .unwrap();
        put(
            &mut b,
            Some(&n.id),
            with_remind("Pills", "after lunch, with water", remind(500, false)),
            B,
            30,
        )
        .unwrap();
        let to_b = sent(&mut a, &n.id);
        let to_a = sent(&mut b, &n.id);
        merge(&mut a, to_a, 40);
        let mb = merge(&mut b, to_b, 40);
        // b's text wins, and the Done stays, on both.
        assert_eq!(a[&n.id].body, "after lunch, with water");
        assert_eq!(a[&n.id].remind, remind(500, true));
        assert!(mb.changed.contains(&n.id), "b's own version took the Done");
        assert_eq!(b[&n.id].remind, remind(500, true));
        assert_eq!(a, b);
        // A Done for an older time doesn't stop a new one set elsewhere.
        put(
            &mut a,
            Some(&n.id),
            with_remind("Pills", "x", remind(500, true)),
            A,
            50,
        )
        .unwrap();
        put(
            &mut b,
            Some(&n.id),
            with_remind("Pills", "y", remind(900, false)),
            B,
            60,
        )
        .unwrap();
        let to_b = sent(&mut a, &n.id);
        let to_a = sent(&mut b, &n.id);
        merge(&mut a, to_a, 70);
        merge(&mut b, to_b, 70);
        assert_eq!(a[&n.id].remind, remind(900, false));
        assert_eq!(a, b);
    }
}
