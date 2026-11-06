//! Read later: a list of links saved to read later that's the same on all
//! your devices. Save a link (from the share sheet, the clipboard, the
//! command palette) on any device and it shows on all of them; mark it
//! read, or delete it, anywhere. Each link goes end-to-end encrypted over
//! the links your devices already have; nothing is kept on a server, and
//! nothing is fetched from the web (the title is the one the share gave,
//! else the link's host).
//!
//! An item is `{id, url, title, read, saved_at, saved_on, updated_at, by,
//! deleted}`. Its id is made from its URL (`id_for`), so a link saved on
//! two devices is one item. Deleting keeps a tombstone (`deleted`, no URL
//! or title) so the delete reaches every device; a tombstone is forgotten
//! after `TOMBSTONE_DAYS`. Saving a link again (even a deleted one) puts
//! it back at the top, unread.
//!
//! # Merging
//!
//! Each item is merged on its own, last writer wins by (`updated_at`, `by`,
//! then the content), so every device ends the same whatever order things
//! arrive in. An edit here is always newer than what this device had for
//! the item, whatever the clock says. (Unlike a note there's no text that
//! a lost edit would take with it: the worst is a "read" mark undone.)
//!
//! # Sync
//!
//!   read_later_digest { from, digest: { <id>: <version hash> } }
//!   read_later        { from, items: [<item>...], want?: [<id>...] }
//!
//! As notes do: when a link comes up (or a device is first heard to know
//! the list), each end sends its digest, ids and version hashes only. The
//! other end answers with the items whose version differs or the digest
//! lacks (tombstones only for items it has), and asks (`want`) for those
//! it lacks; answers go in batches of at most `BATCH_BYTES`. An item saved
//! here goes at once to every linked device that knows the list. A device
//! answers a `read_later` message once with what it has that's newer; that
//! answer brings nothing newer back, so it ends there.
//!
//! Only devices that said `read_later: 1` (in their status, or in the
//! `addr_update` every link starts with) are sent these messages: older
//! apps would show them as chat. Receiving one says the same.
//!
//! Limits: `MAX_ITEMS` links; past it a new link isn't saved here, nor
//! taken from another device.
//!
//! Stored beside the platform's kv store as `read_later.json`
//! (`Platform::data_file`), or in `kv` (key `read_later`) where there's none.
//!
//! What arrives and changes the list makes a `CoreEvent::ReadLaterChanged`
//! with the changed items and the deleted ids. Saving from the apps makes
//! no event.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::ReadLaterItem;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The most links (not counting deleted ones).
pub const MAX_ITEMS: usize = 5_000;
/// The longest link, in bytes.
pub const MAX_URL: usize = 4_096;
/// The longest title, in characters.
pub const MAX_TITLE: usize = 300;
/// How long a deleted link is remembered.
pub const TOMBSTONE_DAYS: u64 = 30;
const TOMBSTONE_MS: u64 = TOMBSTONE_DAYS * 24 * 3600 * 1000;
/// Links and tombstones kept at most: the oldest tombstones go first.
const MAX_ENTRIES: usize = MAX_ITEMS * 2;
/// The most JSON in one `read_later` message.
const BATCH_BYTES: usize = 256 * 1024;
const MAX_ID: usize = 64;
const MAX_NODE: usize = 128;
const MAX_NAME: usize = 64;

/// The status (and `addr_update`) flag saying this device knows the list.
pub const FLAG: &str = "read_later";
const VERSION: u64 = 1;

const KV_KEY: &str = "read_later";
const FILE: &str = "read_later.json";
const CAPS_PREFIX: &str = "read_later_caps:";

fn not(b: &bool) -> bool {
    !*b
}

/// One link as it's kept and goes on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "not")]
    pub read: bool,
    /// When it was (last) saved, unix ms: the list's order.
    #[serde(default)]
    pub saved_at: u64,
    /// The name of the device it was saved on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub saved_on: String,
    #[serde(default)]
    pub updated_at: u64,
    /// The device that made this version (node id).
    #[serde(default)]
    pub by: String,
    #[serde(default, skip_serializing_if = "not")]
    pub deleted: bool,
}

/// Item id -> item.
pub type ItemMap = BTreeMap<String, Entry>;

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

/// The id of the item for `url` (as `clean_url` keeps it): the same on
/// every device, so a link saved twice is one item.
pub fn id_for(url: &str) -> String {
    format!("u{:016x}", fnv(&[url.as_bytes()]))
}

impl Entry {
    /// Which version this is, as a hash (digests).
    pub fn version(&self) -> u64 {
        fnv(&[&self.updated_at.to_le_bytes(), self.by.as_bytes()])
    }

    fn same_version(&self, other: &Entry) -> bool {
        self.updated_at == other.updated_at && self.by == other.by
    }

    /// The order merges go by: the newest wins; the same time goes by the
    /// device, then by the content, so every device agrees.
    #[allow(clippy::type_complexity)]
    fn order(&self) -> (u64, &str, bool, bool, u64, &str, &str, &str) {
        (
            self.updated_at,
            &self.by,
            self.deleted,
            self.read,
            self.saved_at,
            &self.url,
            &self.title,
            &self.saved_on,
        )
    }

    /// This version replaces `other`.
    pub fn wins_over(&self, other: &Entry) -> bool {
        self.order() > other.order()
    }

    /// As the apps see it.
    pub fn item(&self) -> ReadLaterItem {
        ReadLaterItem {
            id: self.id.clone(),
            url: self.url.clone(),
            title: self.title.clone(),
            host: host_of(&self.url).unwrap_or_default(),
            read: self.read,
            saved_at: self.saved_at,
            saved_on: self.saved_on.clone(),
            updated_at: self.updated_at,
            by: self.by.clone(),
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

/// `url` as it's kept, if it's a web link: trimmed, the scheme in lower
/// case, nothing else changed (the id is made from it). Only `http` and
/// `https` with a host, no spaces or control characters, at most
/// `MAX_URL` bytes.
pub fn clean_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL {
        return None;
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let out = format!("{scheme}://{rest}");
    host_of(&out)?;
    Some(out)
}

/// A link's host, for showing (lower case, without `www.`, a port or a
/// user): `https://www.Example.com:8080/a` -> `example.com`. None when it
/// has none.
pub fn host_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        // An IPv6 address: `[::1]:8080`.
        let end = v6.find(']')?;
        &host_port[..end + 2]
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty() && host != "[]").then_some(host)
}

/// The first web link in `text` (a share, a clipboard), as `clean_url`
/// keeps it. Punctuation right after a link (`.`, `,`, a closing bracket
/// the link didn't open...) isn't part of it.
pub fn find_url(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while from < lower.len() {
        let rest = &lower[from..];
        let start = match (rest.find("http://"), rest.find("https://")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return None,
        } + from;
        let tail = &text[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"'))
            .unwrap_or(tail.len());
        let mut url = &tail[..end];
        loop {
            let before = url;
            url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '*']);
            for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
                if url.ends_with(close) && url.matches(close).count() > url.matches(open).count() {
                    url = &url[..url.len() - 1];
                }
            }
            if url == before {
                break;
            }
        }
        if let Some(u) = clean_url(url) {
            return Some(u);
        }
        from = start + end.max(1);
    }
    None
}

/// A title as it's kept: one line, no control characters, spaces run
/// together, at most `MAX_TITLE` characters.
pub fn tidy_title(title: &str) -> String {
    let one_line: String = title
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    one_line
        .split(' ')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE)
        .collect::<String>()
        .trim()
        .to_string()
}

fn tidy_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// An item as it arrived, or `None` if it can't be kept as it is (nothing
/// that makes the version is changed: an item that isn't valid is left
/// out instead).
pub fn sanitize(mut e: Entry) -> Option<Entry> {
    let live_ok = e.deleted
        || (clean_url(&e.url).as_deref() == Some(e.url.as_str()) && id_for(&e.url) == e.id);
    let ok = valid_id(&e.id)
        && valid_node(&e.by)
        && live_ok
        && e.url.len() <= MAX_URL
        && e.title.chars().count() <= MAX_TITLE
        && !e.title.chars().any(char::is_control);
    if !ok {
        return None;
    }
    e.saved_on = tidy_name(&e.saved_on);
    if e.deleted {
        e.url.clear();
        e.title.clear();
    }
    Some(e)
}

/// Items as they arrived (a JSON array), each checked (`sanitize`).
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

/// Every item's version, for a `read_later_digest`.
pub fn digest(map: &ItemMap) -> BTreeMap<String, u64> {
    map.iter()
        .map(|(id, e)| (id.clone(), e.version()))
        .collect()
}

/// What a digest from another device asks of this one: the ids to send it
/// (its version differs, or it lacks an item that isn't deleted here) and
/// the ids to ask it for (items this device doesn't have).
pub fn diff(map: &ItemMap, theirs: &HashMap<String, u64>) -> (Vec<String>, Vec<String>) {
    let send = map
        .iter()
        .filter(|(id, e)| match theirs.get(*id) {
            Some(v) => *v != e.version(),
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

fn live_count(map: &ItemMap) -> usize {
    map.values().filter(|e| !e.deleted).count()
}

/// Forget tombstones older than `TOMBSTONE_DAYS`. True if any went.
pub fn prune(map: &mut ItemMap, now: u64) -> bool {
    let before = map.len();
    map.retain(|_, e| !e.deleted || e.updated_at.saturating_add(TOMBSTONE_MS) >= now);
    map.len() != before
}

/// Make room for one more entry: drop the oldest tombstones while the map
/// is at `MAX_ENTRIES`.
fn make_room(map: &mut ItemMap) {
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
fn fits(map: &ItemMap, e: &Entry, prev: Option<&Entry>) -> bool {
    let was_live = prev.is_some_and(|p| !p.deleted);
    e.deleted || was_live || live_count(map) < MAX_ITEMS
}

/// What a merge did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// Items that changed here.
    pub changed: Vec<String>,
    /// Items worth sending back: this device has a newer version.
    pub answer: Vec<String>,
}

/// Merge items that arrived (checked: `parse`) into `map`. `now` (unix ms)
/// drops tombstones too old to keep.
pub fn merge(map: &mut ItemMap, incoming: Vec<Entry>, now: u64) -> Merged {
    let mut out = Merged::default();
    for inc in incoming {
        let id = inc.id.clone();
        match map.get(&id) {
            None => {
                let stale = inc.deleted && inc.updated_at.saturating_add(TOMBSTONE_MS) < now;
                if !stale && fits(map, &inc, None) {
                    make_room(map);
                    map.insert(id.clone(), inc);
                    out.changed.push(id);
                }
            }
            Some(cur) if cur.same_version(&inc) => {}
            Some(cur) if inc.wins_over(cur) => {
                if fits(map, &inc, Some(cur)) {
                    map.insert(id.clone(), inc);
                    out.changed.push(id);
                }
            }
            Some(_) => out.answer.push(id),
        }
    }
    if prune(map, now) {
        out.changed.retain(|id| map.contains_key(id));
        out.answer.retain(|id| map.contains_key(id));
    }
    out
}

/// The version fields of an edit of `prev` made here: always newer than it.
fn step(prev: Option<&Entry>, me: &str, now: u64) -> (u64, String) {
    (
        now.max(prev.map_or(0, |p| p.updated_at.saturating_add(1))),
        me.to_string(),
    )
}

/// Save `url` here (with `title`; none or blank: its host), as made at
/// `now` by `me` (node id, name). A link already in the list goes back to
/// the top, unread (keeping its title unless a new one is given); a deleted
/// one comes back.
pub fn put(
    map: &mut ItemMap,
    url: &str,
    title: Option<&str>,
    me: (&str, &str),
    now: u64,
) -> crate::Result<Entry> {
    let url =
        clean_url(url).ok_or_else(|| CoreError::InvalidInput("That isn't a web link.".into()))?;
    let id = id_for(&url);
    let prev = map.get(&id);
    let title = title.map(tidy_title).filter(|t| !t.is_empty());
    let title = match (title, prev.filter(|p| !p.deleted)) {
        (Some(t), _) => t,
        (None, Some(p)) if !p.title.is_empty() => p.title.clone(),
        _ => host_of(&url).unwrap_or_default(),
    };
    let (updated_at, by) = step(prev, me.0, now);
    let e = Entry {
        id: id.clone(),
        url,
        title,
        read: false,
        saved_at: updated_at,
        saved_on: tidy_name(me.1),
        updated_at,
        by,
        deleted: false,
    };
    if !fits(map, &e, prev) {
        return Err(CoreError::InvalidInput(format!(
            "at most {MAX_ITEMS} links to read later"
        )));
    }
    if prev.is_none() {
        make_room(map);
    }
    map.insert(id, e.clone());
    Ok(e)
}

/// Mark item `id` read (or unread again) here.
pub fn set_read(
    map: &mut ItemMap,
    id: &str,
    read: bool,
    me: (&str, &str),
    now: u64,
) -> crate::Result<Entry> {
    let Some(prev) = map.get(id).filter(|p| !p.deleted) else {
        return Err(CoreError::InvalidInput("no such link".into()));
    };
    let (updated_at, by) = step(Some(prev), me.0, now);
    let e = Entry {
        read,
        updated_at,
        by,
        ..prev.clone()
    };
    map.insert(id.to_string(), e.clone());
    Ok(e)
}

/// Delete item `id` here (a tombstone, as made at `now` by `me`).
pub fn remove(map: &mut ItemMap, id: &str, me: (&str, &str), now: u64) -> crate::Result<Entry> {
    let Some(prev) = map.get(id).filter(|p| !p.deleted) else {
        return Err(CoreError::InvalidInput("no such link".into()));
    };
    let (updated_at, by) = step(Some(prev), me.0, now);
    let e = Entry {
        id: id.to_string(),
        saved_at: prev.saved_at,
        updated_at,
        by,
        deleted: true,
        ..Entry::default()
    };
    map.insert(id.to_string(), e.clone());
    Ok(e)
}

/// The links, the most recently saved first.
pub fn listed(map: &ItemMap) -> Vec<ReadLaterItem> {
    let mut v: Vec<&Entry> = map.values().filter(|e| !e.deleted).collect();
    v.sort_by(|a, b| b.saved_at.cmp(&a.saved_at).then(a.id.cmp(&b.id)));
    v.into_iter().map(Entry::item).collect()
}

/// Split items into `read_later` messages of at most `BATCH_BYTES` (one
/// item over that goes alone); `want` rides on the first.
fn batches(from: &str, items: Vec<Entry>, want: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<Value> = Vec::new();
    let mut size = 0;
    let mut want = Some(want);
    let mut flush = |cur: &mut Vec<Value>, want: &mut Option<&[String]>| {
        let mut msg = json!({
            "type": "read_later",
            "from": from,
            "v": VERSION,
            "items": std::mem::take(cur),
        });
        if let Some(w) = want.take().filter(|w| !w.is_empty()) {
            msg["want"] = json!(w);
        }
        out.push(msg.to_string());
    };
    for e in items {
        let one = serde_json::to_value(e).unwrap_or_default();
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

/// The list and which devices know it.
#[derive(Default)]
pub struct ReadLater {
    /// `None` until read.
    map: Mutex<Option<ItemMap>>,
    /// Node -> the version it said (0: none).
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

fn load<P: Platform>(ctx: &NetCtx<P>) -> ItemMap {
    let json = match store_path(ctx) {
        Some(path) => std::fs::read_to_string(path).ok(),
        None => ctx.platform.kv_get(KV_KEY),
    };
    let items = json
        .and_then(|j| serde_json::from_str::<Vec<Entry>>(&j).ok())
        .unwrap_or_default();
    let mut map = ItemMap::new();
    for e in items.into_iter().filter_map(sanitize) {
        map.insert(e.id.clone(), e);
    }
    prune(&mut map, now_ms());
    map
}

fn save<P: Platform>(ctx: &NetCtx<P>, map: &ItemMap) {
    let Ok(json) = serde_json::to_string(&map.values().collect::<Vec<_>>()) else {
        return;
    };
    match store_path(ctx) {
        // Nowhere to report it: the list stays in memory until the next save.
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

/// Run `f` on the list (read the first time); saved when `f` says it
/// changed it.
fn with_map<P: Platform, R>(ctx: &NetCtx<P>, f: impl FnOnce(&mut ItemMap) -> (R, bool)) -> R {
    let mut slot = ctx.extras.read_later.map.lock();
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

/// Every link, the most recently saved first.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> Vec<ReadLaterItem> {
    with_map(ctx, |m| (listed(m), false))
}

/// Run an edit made here, keep it, and send it to the linked devices that
/// know the list.
fn edit<P: Platform>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut ItemMap, (&str, &str), u64) -> crate::Result<Entry>,
) -> crate::Result<ReadLaterItem> {
    let (node, name) = me(ctx);
    let now = now_ms();
    let e = with_map(ctx, |m| {
        let r = f(m, (&node, &name), now);
        if r.is_ok() {
            prune(m, now);
        }
        let ok = r.is_ok();
        (r, ok)
    })?;
    send_all(ctx, &e.id);
    Ok(e.item())
}

/// The apps save a link (see `put`). Makes no event.
pub fn save_item<P: Platform>(
    ctx: &NetCtx<P>,
    url: &str,
    title: Option<&str>,
) -> crate::Result<ReadLaterItem> {
    edit(ctx, |m, me, now| put(m, url, title, me, now))
}

/// The apps mark a link read or unread.
pub fn mark_read<P: Platform>(
    ctx: &NetCtx<P>,
    id: &str,
    read: bool,
) -> crate::Result<ReadLaterItem> {
    edit(ctx, |m, me, now| set_read(m, id, read, me, now))
}

/// The apps delete a link (saving it again brings it back).
pub fn delete_item<P: Platform>(ctx: &NetCtx<P>, id: &str) -> crate::Result<()> {
    edit(ctx, |m, me, now| remove(m, id, me, now)).map(|_| ())
}

/// Say in this device's status (or `addr_update`) that it knows the list.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.read_later {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `read_later` value of its status or
/// `addr_update`) whether it knows the list. Kept across restarts; newly
/// known, it gets this device's digest now.
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
    ctx.extras
        .read_later
        .caps
        .lock()
        .insert(node.to_string(), level);
    let kept = if level == 0 {
        String::new()
    } else {
        level.to_string()
    };
    ctx.platform.kv_set(&format!("{CAPS_PREFIX}{node}"), &kept);
    true
}

/// The version `node` said (0: none).
fn caps<P: Platform>(ctx: &NetCtx<P>, node: &str) -> u64 {
    if let Some(level) = ctx.extras.read_later.caps.lock().get(node) {
        return *level;
    }
    let level = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    ctx.extras
        .read_later
        .caps
        .lock()
        .insert(node.to_string(), level);
    level
}

/// `node` said it knows the list.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    caps(ctx, node) >= 1
}

/// A link to `node` just came up: send it the digest.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    send_digest(ctx, node);
}

/// The list goes only to real devices that know it, with this end on.
fn may_send<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.read_later && !node.starts_with("manual:") && supports(ctx, node)
}

fn send_digest<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !may_send(ctx, node) {
        return;
    }
    let (from, _) = me(ctx);
    let digest = with_map(ctx, |m| (digest(m), false));
    let msg = json!({
        "type": "read_later_digest",
        "from": from,
        "v": VERSION,
        "digest": digest,
    });
    send_control_to(ctx, node, &msg.to_string());
}

/// Send items `ids` (and ask for `want`) to `node`.
fn send_items<P: Platform>(ctx: &NetCtx<P>, node: &str, ids: &[String], want: &[String]) {
    if !may_send(ctx, node) || (ids.is_empty() && want.is_empty()) {
        return;
    }
    let (from, _) = me(ctx);
    let items = with_map(ctx, |m| {
        (
            ids.iter().filter_map(|id| m.get(id).cloned()).collect(),
            false,
        )
    });
    for msg in batches(&from, items, want) {
        send_control_to(ctx, node, &msg);
    }
}

/// Send item `id` to every linked device that knows the list.
fn send_all<P: Platform>(ctx: &NetCtx<P>, id: &str) {
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    let ids = [id.to_string()];
    for node in linked {
        send_items(ctx, &node, &ids, &[]);
    }
}

pub struct ReadLaterHandler;

impl<P: Platform> ControlHandler<P> for ReadLaterHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["read_later", "read_later_digest"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return; // only your own devices share your list
        }
        // Sending it says it knows it. Heard here first (before its status
        // or `addr_update`), it gets this device's digest now.
        let v = msg.json("v").as_u64().unwrap_or(1).max(1);
        if set_caps(ctx, &from, v) {
            send_digest(ctx, &from);
        }
        if msg.kind() == "read_later_digest" {
            let theirs = parse_digest(&msg.json("digest"));
            let (send, want) = with_map(ctx, |m| (diff(m, &theirs), false));
            send_items(ctx, &from, &send, &want);
            return;
        }
        let incoming = parse(&msg.json("items"));
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
        let (merged, items, removed) = with_map(ctx, |m| {
            let merged = merge(m, incoming, now);
            let mut items = Vec::new();
            let mut removed = Vec::new();
            for id in &merged.changed {
                match m.get(id) {
                    Some(e) if !e.deleted => items.push(e.item()),
                    _ => removed.push(id.clone()),
                }
            }
            let dirty = !merged.changed.is_empty();
            ((merged, items, removed), dirty)
        });
        if !items.is_empty() || !removed.is_empty() {
            ctx.sink
                .emit(CoreEvent::ReadLaterChanged { items, removed });
        }
        let mut answer = merged.answer;
        for id in want {
            if !answer.contains(&id) {
                answer.push(id);
            }
        }
        send_items(ctx, &from, &answer, &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: (&str, &str) = ("node-a", "Mac");
    const B: (&str, &str) = ("node-b", "Pixel");

    /// What `from` would send of item `id` (as it goes on the wire).
    fn sent(from: &ItemMap, id: &str) -> Vec<Entry> {
        parse(&json!([from[id]]))
    }

    #[test]
    fn links_are_found_in_text_and_kept_as_given() {
        assert_eq!(
            find_url("Look at this https://example.com/a?b=1#c, it's good."),
            Some("https://example.com/a?b=1#c".into())
        );
        assert_eq!(
            find_url("(see https://en.wikipedia.org/wiki/Rust_(programming_language))"),
            Some("https://en.wikipedia.org/wiki/Rust_(programming_language)".into())
        );
        assert_eq!(
            find_url("HTTPS://Example.com/Path."),
            Some("https://Example.com/Path".into())
        );
        assert_eq!(
            find_url("A title\nhttp://x.org"),
            Some("http://x.org".into())
        );
        assert_eq!(find_url("ftp://x.org and https://"), None);
        assert_eq!(
            find_url("https:// then https://ok.io"),
            Some("https://ok.io".into())
        );
        assert_eq!(find_url("no link here"), None);
        assert_eq!(clean_url("javascript://x"), None);
        assert_eq!(clean_url("https://a b"), None);
        assert_eq!(clean_url("https:///path"), None);
        assert_eq!(
            clean_url(&format!("https://x.io/{}", "a".repeat(MAX_URL))),
            None
        );
    }

    #[test]
    fn a_host_is_shown_plainly() {
        assert_eq!(
            host_of("https://www.Example.com:8080/a").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            host_of("http://user:pw@news.site.org?x").as_deref(),
            Some("news.site.org")
        );
        assert_eq!(host_of("http://[::1]:80/").as_deref(), Some("[::1]"));
        assert_eq!(host_of("not a link"), None);
    }

    #[test]
    fn a_title_is_the_shares_else_the_host() {
        let mut m = ItemMap::new();
        let e = put(&mut m, "https://www.blog.dev/post", None, A, 10).unwrap();
        assert_eq!(e.title, "blog.dev");
        assert_eq!(e.saved_on, "Mac");
        let e = put(
            &mut m,
            "https://www.blog.dev/post",
            Some("  A long\n read\t"),
            A,
            5,
        )
        .unwrap();
        assert_eq!(e.title, "A long read");
        // Saved again with no title: it keeps the one it had.
        let e = put(&mut m, "https://www.blog.dev/post", Some(" "), A, 20).unwrap();
        assert_eq!(e.title, "A long read");
        assert_eq!(m.len(), 1);
        assert!(put(&mut m, "mailto:x@y", None, A, 30).is_err());
    }

    #[test]
    fn edits_go_to_the_other_device_and_the_newer_wins() {
        let mut a = ItemMap::new();
        let e = put(&mut a, "https://x.io/1", Some("One"), A, 10).unwrap();
        let mut b = ItemMap::new();
        let m = merge(&mut b, sent(&a, &e.id), 11);
        assert_eq!(m.changed, std::slice::from_ref(&e.id));
        assert_eq!(a, b);
        // Read on b; it goes to a.
        set_read(&mut b, &e.id, true, B, 20).unwrap();
        let m = merge(&mut a, sent(&b, &e.id), 21);
        assert_eq!(m.changed, std::slice::from_ref(&e.id));
        assert!(a[&e.id].read);
        // The older version arriving again changes nothing, and is answered.
        let old = Entry {
            read: false,
            updated_at: 10,
            by: A.0.into(),
            ..a[&e.id].clone()
        };
        let m = merge(&mut a, vec![old], 30);
        assert!(m.changed.is_empty());
        assert_eq!(m.answer, std::slice::from_ref(&e.id));
        // The same version: nothing either way.
        assert_eq!(merge(&mut b, sent(&a, &e.id), 31), Merged::default());
    }

    #[test]
    fn a_delete_reaches_the_other_device_and_saving_brings_it_back() {
        let mut a = ItemMap::new();
        let e = put(&mut a, "https://x.io/2", None, A, 10).unwrap();
        let mut b = ItemMap::new();
        merge(&mut b, sent(&a, &e.id), 11);
        remove(&mut b, &e.id, B, 20).unwrap();
        assert!(b[&e.id].url.is_empty(), "a tombstone keeps no link");
        merge(&mut a, sent(&b, &e.id), 21);
        assert!(listed(&a).is_empty());
        // An older version from a third device doesn't bring it back.
        let mut c = ItemMap::new();
        let old = put(&mut c, "https://x.io/2", None, ("node-c", "PC"), 15).unwrap();
        let m = merge(&mut a, vec![old], 22);
        assert_eq!(m.answer, std::slice::from_ref(&e.id));
        assert!(listed(&a).is_empty());
        // Saving it again (here, with an older clock) does.
        let again = put(&mut a, "https://x.io/2", None, A, 5).unwrap();
        assert!(again.updated_at > 20 && !again.read);
        merge(&mut b, sent(&a, &e.id), 23);
        assert_eq!(listed(&b).len(), 1);
        // Tombstones are forgotten after a while.
        remove(&mut b, &e.id, B, 30).unwrap();
        assert!(prune(&mut b, 30 + TOMBSTONE_MS + 1));
        assert!(b.is_empty());
    }

    #[test]
    fn devices_agree_whatever_the_order() {
        let mut a = ItemMap::new();
        let e = put(&mut a, "https://x.io/3", None, A, 10).unwrap();
        let mut b = ItemMap::new();
        merge(&mut b, sent(&a, &e.id), 11);
        // Both change it at the same time before hearing of the other.
        set_read(&mut a, &e.id, true, A, 20).unwrap();
        remove(&mut b, &e.id, B, 20).unwrap();
        let (to_b, to_a) = (sent(&a, &e.id), sent(&b, &e.id));
        merge(&mut a, to_a, 21);
        merge(&mut b, to_b, 21);
        assert_eq!(a, b);
    }

    #[test]
    fn digests_ask_for_only_what_differs() {
        let mut a = ItemMap::new();
        let one = put(&mut a, "https://x.io/a", None, A, 10).unwrap();
        let two = put(&mut a, "https://x.io/b", None, A, 11).unwrap();
        let gone = put(&mut a, "https://x.io/c", None, A, 12).unwrap();
        remove(&mut a, &gone.id, A, 13).unwrap();
        let mut b = ItemMap::new();
        merge(&mut b, sent(&a, &one.id), 14);
        let theirs: HashMap<String, u64> = digest(&b).into_iter().collect();
        let (send, want) = diff(&a, &theirs);
        assert_eq!(send, vec![two.id.clone()], "not the tombstone it never had");
        assert!(want.is_empty());
        let mine: HashMap<String, u64> = digest(&a).into_iter().collect();
        let (send, want) = diff(&b, &mine);
        assert!(send.is_empty());
        assert_eq!(want.len(), 2);
    }

    #[test]
    fn what_arrives_is_checked() {
        let mut a = ItemMap::new();
        let e = put(&mut a, "https://x.io/ok", None, A, 10).unwrap();
        let good = serde_json::to_value(&a[&e.id]).unwrap();
        let mut wrong_id = good.clone();
        wrong_id["id"] = "u0000000000000000".into();
        let mut bad_url = good.clone();
        bad_url["url"] = "javascript:alert(1)".into();
        let mut bad_title = good.clone();
        bad_title["title"] = "a\u{7}b".into();
        let got = parse(&json!([good, wrong_id, bad_url, bad_title, 5]));
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn batches_split_large_lists() {
        let mut a = ItemMap::new();
        for i in 0..400 {
            let long = format!("https://x.io/{i}/{}", "p".repeat(2_000));
            put(&mut a, &long, Some(&"t".repeat(MAX_TITLE)), A, 10 + i).unwrap();
        }
        let items: Vec<Entry> = a.values().cloned().collect();
        let msgs = batches("me", items, &["want-1".into()]);
        assert!(msgs.len() > 1);
        assert!(msgs.iter().all(|m| m.len() <= BATCH_BYTES + 4_096));
        let first: Value = serde_json::from_str(&msgs[0]).unwrap();
        assert_eq!(first["want"], json!(["want-1"]));
        let total: usize = msgs
            .iter()
            .map(|m| {
                serde_json::from_str::<Value>(m).unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .len()
            })
            .sum();
        assert_eq!(total, 400);
    }

    #[test]
    fn listed_newest_saved_first() {
        let mut a = ItemMap::new();
        put(&mut a, "https://x.io/old", None, A, 10).unwrap();
        put(&mut a, "https://x.io/new", None, A, 20).unwrap();
        let l = listed(&a);
        assert_eq!(l[0].url, "https://x.io/new");
        assert_eq!(l[0].host, "x.io");
        // Saved again: back on top.
        put(&mut a, "https://x.io/old", None, A, 30).unwrap();
        assert_eq!(listed(&a)[0].url, "https://x.io/old");
    }
}
