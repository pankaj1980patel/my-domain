//! Device labels shared between your devices: a nickname, an icon and a
//! colour you give one of your devices on one of them show up on all of
//! them. Every device of an account shares one key (`crypto`), so every
//! peer is one of your own devices: sharing is on until turned off
//! (`set_sharing`); off, this device neither sends its labels nor takes
//! anyone else's.
//!
//!   labels { from, labels: { <node>: {nickname?, icon?, color?,
//!                                      updated_at, by} } }
//!
//! The apps keep their own labels (what screens read); the core keeps a
//! small replicated map beside them (`kv`, key `labels_map`): target node
//! id -> its label, when it was set (unix ms) and on which device. Merging
//! is last-writer-wins per device by (`updated_at`, `by`, then the label
//! itself), so every device ends with the same map whatever order the
//! messages came in. Clearing a label keeps a cleared entry (a tombstone)
//! with its time, so a clear wins over the older label everywhere too.
//!
//! The whole map goes (it's small): when a link comes up, when a device is
//! first heard to understand it, and to every linked device when a label
//! is set here. A device that has something newer than what it was sent
//! answers with its own map once; the answer brings nothing newer back, so
//! it ends there. Devices not linked get it at their next link.
//!
//! Only devices that said `labels: 1` (in their status, or in the
//! `addr_update` every link starts with) get the message: older apps would
//! show it as a chat message. A `labels` message arriving says the same.
//!
//! What arrives and is newer makes a `CoreEvent::LabelsChanged` with the
//! changed labels; the apps apply them to their own labels. Setting a label
//! from the apps (`set`) makes no event, so applying one never echoes.

use std::collections::{BTreeMap, HashMap};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::SharedLabel;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The longest nickname, in characters (as the apps cut it).
pub const MAX_NICKNAME: usize = 40;
/// The most labels kept (cleared ones included).
pub const MAX_LABELS: usize = 256;
/// The longest icon or colour name.
const MAX_TOKEN: usize = 24;
/// The longest node id.
const MAX_NODE: usize = 128;
/// When a label the apps had before sharing counts as set: older than any
/// label set since, so a seeded one never overrides a real change.
pub const SEEDED_AT: u64 = 1;

/// The status (and `addr_update`) flag saying this device understands
/// `labels`, and its version.
pub const FLAG: &str = "labels";
const VERSION: u64 = 1;

const MAP_KEY: &str = "labels_map";
const SHARE_KEY: &str = "labels_share";
const CAPS_PREFIX: &str = "labels_caps:";

/// One device's label as it goes on the wire and into `kv`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub by: String,
}

/// Target node id -> its label.
pub type LabelMap = BTreeMap<String, Entry>;

impl Entry {
    fn is_cleared(&self) -> bool {
        self.nickname.is_none() && self.icon.is_none() && self.color.is_none()
    }

    /// The order merges go by: the newest wins; the same time goes by the
    /// device that set it, then by the label, so every device agrees.
    #[allow(clippy::type_complexity)]
    fn order(&self) -> (u64, &str, Option<&str>, Option<&str>, Option<&str>) {
        (
            self.updated_at,
            &self.by,
            self.nickname.as_deref(),
            self.icon.as_deref(),
            self.color.as_deref(),
        )
    }

    /// This label replaces `other` in a merge.
    pub fn wins_over(&self, other: &Entry) -> bool {
        self.order() > other.order()
    }

    fn shared(&self, node: &str) -> SharedLabel {
        SharedLabel {
            node: node.to_string(),
            nickname: self.nickname.clone(),
            icon: self.icon.clone(),
            color: self.color.clone(),
            updated_at: self.updated_at,
            by: self.by.clone(),
        }
    }
}

/// A nickname tidied as the apps do: spaces collapsed, control characters
/// dropped, cut to `MAX_NICKNAME`; blank: none.
pub fn tidy_nickname(nickname: Option<&str>) -> Option<String> {
    let n = nickname?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NICKNAME)
        .collect::<String>();
    let n = n.trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// An icon or colour name: a short lowercase word; anything else: none.
fn token(t: Option<&str>) -> Option<String> {
    let t = t?.trim();
    let ok = !t.is_empty()
        && t.len() <= MAX_TOKEN
        && t.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    ok.then(|| t.to_string())
}

fn valid_node(node: &str) -> bool {
    !node.trim().is_empty() && node.len() <= MAX_NODE && !node.chars().any(char::is_control)
}

/// An entry as it's kept: tidied, or `None` if it can't be kept (no device
/// that set it).
pub fn sanitize(e: Entry) -> Option<Entry> {
    if !valid_node(&e.by) {
        return None;
    }
    Some(Entry {
        nickname: tidy_nickname(e.nickname.as_deref()),
        icon: token(e.icon.as_deref()),
        color: token(e.color.as_deref()),
        updated_at: e.updated_at,
        by: e.by,
    })
}

/// A map as it arrived (JSON object node -> entry), each entry tidied;
/// the ones that can't be read are left out.
pub fn parse(v: &Value) -> LabelMap {
    let Some(obj) = v.as_object() else {
        return LabelMap::new();
    };
    obj.iter()
        .filter(|(node, _)| valid_node(node))
        .filter_map(|(node, e)| {
            let e = serde_json::from_value::<Entry>(e.clone()).ok()?;
            Some((node.clone(), sanitize(e)?))
        })
        .take(MAX_LABELS * 2)
        .collect()
}

/// Make room for a new label in a full map: drop the oldest cleared one.
/// False if there's none (the map is full of labels).
fn make_room(map: &mut LabelMap) -> bool {
    if map.len() < MAX_LABELS {
        return true;
    }
    let oldest = map
        .iter()
        .filter(|(_, e)| e.is_cleared())
        .min_by(|a, b| a.1.order().cmp(&b.1.order()))
        .map(|(n, _)| n.clone());
    match oldest {
        Some(n) => {
            map.remove(&n);
            true
        }
        None => false,
    }
}

/// Merge `incoming` (tidied: `parse`) into `map`, last writer wins per
/// device. Returns the devices whose label changed.
pub fn merge(map: &mut LabelMap, incoming: &LabelMap) -> Vec<String> {
    let mut changed = Vec::new();
    for (node, e) in incoming {
        let fits = match map.get(node) {
            Some(cur) => e.wins_over(cur),
            None => make_room(map),
        };
        if !fits {
            continue;
        }
        map.insert(node.clone(), e.clone());
        changed.push(node.clone());
    }
    changed
}

/// `ours` has a label newer than `theirs` has (or one it lacks): worth
/// answering with.
pub fn has_newer(ours: &LabelMap, theirs: &LabelMap) -> bool {
    ours.iter()
        .any(|(node, e)| theirs.get(node).is_none_or(|t| e.wins_over(t)))
}

/// Set `node`'s label here (all none: clear it), as set at `now` by `me`.
/// Always newer than what the map had for it, whatever the clock says.
pub fn put(
    map: &mut LabelMap,
    node: &str,
    label: (Option<&str>, Option<&str>, Option<&str>),
    me: &str,
    now: u64,
) -> crate::Result<Entry> {
    if !valid_node(node) {
        return Err(CoreError::InvalidInput("which device?".into()));
    }
    let (nickname, icon, color) = label;
    let prev = map.get(node).map(|e| e.updated_at).unwrap_or(0);
    let e = Entry {
        nickname: tidy_nickname(nickname),
        icon: token(icon),
        color: token(color),
        updated_at: now.max(prev.saturating_add(1)),
        by: me.to_string(),
    };
    if !map.contains_key(node) && !make_room(map) {
        return Err(CoreError::InvalidInput(format!(
            "at most {MAX_LABELS} labels"
        )));
    }
    map.insert(node.to_string(), e.clone());
    Ok(e)
}

/// The replicated map, whether sharing is on, and which devices
/// understand `labels`.
#[derive(Default)]
pub struct Labels {
    /// `None` until read from `kv`.
    map: Mutex<Option<LabelMap>>,
    share: Mutex<Option<bool>>,
    caps: Mutex<HashMap<String, bool>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run `f` on the map (read from `kv` the first time); saved when `f`
/// says it changed it.
fn with_map<P: Platform, R>(ctx: &NetCtx<P>, f: impl FnOnce(&mut LabelMap) -> (R, bool)) -> R {
    let mut slot = ctx.extras.labels.map.lock();
    let map = slot.get_or_insert_with(|| {
        ctx.platform
            .kv_get(MAP_KEY)
            .and_then(|j| serde_json::from_str::<Value>(&j).ok())
            .map(|v| parse(&v))
            .unwrap_or_default()
    });
    let (r, changed) = f(map);
    if changed {
        if let Ok(json) = serde_json::to_string(&*map) {
            ctx.platform.kv_set(MAP_KEY, &json);
        }
    }
    r
}

/// Whether this device shares labels (on until turned off; never when the
/// feature is off in `CoreConfig::shared_labels`).
pub fn sharing<P: Platform>(ctx: &NetCtx<P>) -> bool {
    if !ctx.config.shared_labels {
        return false;
    }
    *ctx.extras
        .labels
        .share
        .lock()
        .get_or_insert_with(|| ctx.platform.kv_get(SHARE_KEY).as_deref() != Some("off"))
}

/// Turn sharing on or off. Turned on, the labels go to every linked device
/// that understands them (and each answers with anything newer).
pub fn set_sharing<P: Platform>(ctx: &NetCtx<P>, on: bool) {
    *ctx.extras.labels.share.lock() = Some(on);
    ctx.platform
        .kv_set(SHARE_KEY, if on { "on" } else { "off" });
    if on {
        send_all(ctx);
    }
}

/// Every label in the map, cleared ones included.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> Vec<SharedLabel> {
    with_map(ctx, |m| {
        (
            m.iter().map(|(n, e)| e.shared(n)).collect::<Vec<_>>(),
            false,
        )
    })
}

/// The apps set `node`'s label (all none: cleared it). Kept whether or not
/// sharing is on (so turning it on shares what's set by then); sent to the
/// linked devices when it is.
pub fn set<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    nickname: Option<&str>,
    icon: Option<&str>,
    color: Option<&str>,
) -> crate::Result<()> {
    let me = ctx.identity.lock().node_id.clone();
    with_map(ctx, |m| {
        match put(m, node, (nickname, icon, color), &me, now_ms()) {
            Ok(_) => (Ok(()), true),
            Err(e) => (Err(e), false),
        }
    })?;
    if sharing(ctx) {
        send_all(ctx);
    }
    Ok(())
}

/// A label the apps had before the core kept them: added (as set at
/// `SEEDED_AT`, so anything set since wins) unless the map already names
/// `node`. True if it was added.
pub fn seed<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    nickname: Option<&str>,
    icon: Option<&str>,
    color: Option<&str>,
) -> bool {
    let me = ctx.identity.lock().node_id.clone();
    with_map(ctx, |m| {
        if m.contains_key(node) || (nickname.is_none() && icon.is_none() && color.is_none()) {
            return (false, false);
        }
        let ok =
            put(m, node, (nickname, icon, color), &me, SEEDED_AT).is_ok_and(|e| !e.is_cleared());
        if !ok {
            m.remove(node);
        }
        (ok, ok)
    })
}

/// Say in this device's status that it understands `labels`.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.shared_labels {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: the `labels` value of its status or
/// `addr_update`) whether it understands `labels`. Kept across restarts;
/// newly understood, it gets the labels now.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    if set_caps(ctx, node, on) && on && sharing(ctx) {
        send_to(ctx, node);
    }
}

/// True if it changed.
fn set_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, on: bool) -> bool {
    if node.is_empty() || supports(ctx, node) == on {
        return false;
    }
    ctx.extras.labels.caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
    true
}

/// `node` said it understands `labels`.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if let Some(on) = ctx.extras.labels.caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    ctx.extras.labels.caps.lock().insert(node.to_string(), on);
    on
}

/// A link to `node` just came up: send it the labels.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if sharing(ctx) && supports(ctx, node) {
        send_to(ctx, node);
    }
}

fn message<P: Platform>(ctx: &NetCtx<P>) -> String {
    let me = ctx.identity.lock().node_id.clone();
    let labels = with_map(ctx, |m| {
        (serde_json::to_value(&*m).unwrap_or_default(), false)
    });
    json!({ "type": "labels", "from": me, "labels": labels }).to_string()
}

/// Send the whole map to `node`, if it understands it (and it's a real
/// device, not a hand-entered address).
fn send_to<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    if !ctx.config.shared_labels || node.starts_with("manual:") || !supports(ctx, node) {
        return;
    }
    send_control_to(ctx, node, &message(ctx));
}

/// Send the whole map to every linked device that understands it.
fn send_all<P: Platform>(ctx: &NetCtx<P>) {
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    for node in linked {
        send_to(ctx, &node);
    }
}

pub struct LabelsHandler;

impl<P: Platform> ControlHandler<P> for LabelsHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["labels"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return; // only your own devices name your devices
        }
        // Sending it says it understands it.
        set_caps(ctx, &from, true);
        if !sharing(ctx) {
            return;
        }
        let mut theirs = parse(&msg.json("labels"));
        let now = now_ms();
        theirs.retain(|_, e| super::not_ahead(e.updated_at, now));
        let (changed, answer) = with_map(ctx, |m| {
            let changed: Vec<SharedLabel> = merge(m, &theirs)
                .into_iter()
                .filter_map(|n| m.get(&n).map(|e| e.shared(&n)))
                .collect();
            let answer = has_newer(m, &theirs);
            let dirty = !changed.is_empty();
            ((changed, answer), dirty)
        });
        if !changed.is_empty() {
            ctx.sink.emit(CoreEvent::LabelsChanged { labels: changed });
        }
        if answer {
            send_to(ctx, &from);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(nick: Option<&str>, at: u64, by: &str) -> Entry {
        Entry {
            nickname: nick.map(String::from),
            updated_at: at,
            by: by.into(),
            ..Entry::default()
        }
    }

    fn map(entries: &[(&str, Entry)]) -> LabelMap {
        entries
            .iter()
            .map(|(n, e)| (n.to_string(), e.clone()))
            .collect()
    }

    #[test]
    fn the_newest_label_wins() {
        let mut ours = map(&[("tv", e(Some("Den"), 10, "a"))]);
        let theirs = map(&[("tv", e(Some("Lounge"), 20, "b"))]);
        assert_eq!(merge(&mut ours, &theirs), ["tv"]);
        assert_eq!(ours["tv"].nickname.as_deref(), Some("Lounge"));
        // An older one changes nothing.
        let older = map(&[("tv", e(Some("Old"), 5, "c"))]);
        assert!(merge(&mut ours, &older).is_empty());
        assert_eq!(ours["tv"].nickname.as_deref(), Some("Lounge"));
        // The same one again changes nothing either.
        assert!(merge(&mut ours, &theirs).is_empty());
    }

    #[test]
    fn a_clear_is_kept_and_wins_over_older_labels() {
        let mut a = map(&[("tv", e(Some("Den"), 10, "a"))]);
        let clear = put(&mut a, "tv", (None, None, None), "a", 30).unwrap();
        assert!(clear.is_cleared());
        assert_eq!(a.len(), 1, "the clear stays as a tombstone");
        // A device that still has the older label takes the clear...
        let mut b = map(&[("tv", e(Some("Den"), 10, "a"))]);
        assert_eq!(merge(&mut b, &a), ["tv"]);
        assert!(b["tv"].is_cleared());
        // ...and the older label never comes back.
        let stale = map(&[("tv", e(Some("Den"), 10, "a"))]);
        assert!(merge(&mut b, &stale).is_empty());
        assert!(!has_newer(&stale, &b));
        assert!(has_newer(&b, &stale));
    }

    #[test]
    fn merging_ends_the_same_in_any_order() {
        let x = map(&[
            ("tv", e(Some("Den"), 10, "a")),
            ("pc", e(Some("Work"), 7, "a")),
        ]);
        let y = map(&[
            ("tv", e(Some("Lounge"), 10, "b")), // same time: "b" > "a"
            ("ph", e(None, 9, "b")),
        ]);
        let z = map(&[
            ("pc", e(Some("Home"), 8, "c")),
            ("tv", e(Some("Zed"), 10, "a")), // same time, same device: by label
        ]);
        let orders = [[&x, &y, &z], [&z, &y, &x], [&y, &x, &z], [&z, &x, &y]];
        let results: Vec<LabelMap> = orders
            .iter()
            .map(|o| {
                let mut m = LabelMap::new();
                for part in o {
                    merge(&mut m, part);
                }
                m
            })
            .collect();
        for r in &results {
            assert_eq!(r, &results[0]);
        }
        let r = &results[0];
        assert_eq!(r["tv"].nickname.as_deref(), Some("Lounge"));
        assert_eq!(r["pc"].nickname.as_deref(), Some("Home"));
        assert!(r["ph"].is_cleared());
    }

    #[test]
    fn a_local_change_always_beats_what_was_there() {
        let mut m = map(&[("tv", e(Some("Den"), 5_000, "b"))]);
        // This device's clock is behind: still newer.
        let set = put(&mut m, "tv", (Some("Mine"), None, None), "a", 1_000).unwrap();
        assert_eq!(set.updated_at, 5_001);
        assert_eq!(m["tv"].nickname.as_deref(), Some("Mine"));
        assert!(put(&mut m, " ", (Some("x"), None, None), "a", 1).is_err());
    }

    #[test]
    fn what_arrives_is_tidied() {
        let v = json!({
            "tv": {"nickname": "  Den \n  TV ", "icon": "tv", "color": "Not A Colour",
                   "updated_at": 3, "by": "a"},
            "long": {"nickname": "x".repeat(100), "updated_at": 3, "by": "a"},
            "noone": {"nickname": "who", "updated_at": 3},
            "bad": "not an entry",
            "": {"nickname": "blank node", "updated_at": 3, "by": "a"},
        });
        let m = parse(&v);
        assert_eq!(m.len(), 2);
        assert_eq!(m["tv"].nickname.as_deref(), Some("Den TV"));
        assert_eq!(m["tv"].icon.as_deref(), Some("tv"));
        assert_eq!(m["tv"].color, None);
        assert_eq!(
            m["long"].nickname.as_ref().map(|n| n.chars().count()),
            Some(MAX_NICKNAME)
        );
        // Tidied the same on both sides: answering never loops.
        assert!(!has_newer(&m, &parse(&serde_json::to_value(&m).unwrap())));
    }

    #[test]
    fn a_full_map_makes_room_from_old_clears_only() {
        let mut m = LabelMap::new();
        for i in 0..MAX_LABELS {
            m.insert(format!("n{i}"), e(Some("x"), 1, "a"));
        }
        assert!(put(&mut m, "new", (Some("y"), None, None), "a", 2).is_err());
        let clear = map(&[("n0", e(None, 5, "a"))]);
        merge(&mut m, &clear);
        assert!(put(&mut m, "new", (Some("y"), None, None), "a", 6).is_ok());
        assert!(!m.contains_key("n0"));
        assert_eq!(m.len(), MAX_LABELS);
    }

    #[test]
    fn the_map_round_trips_as_json() {
        let m = map(&[("tv", e(Some("Den"), 3, "a")), ("pc", e(None, 4, "b"))]);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            json,
            r#"{"pc":{"updated_at":4,"by":"b"},"tv":{"nickname":"Den","updated_at":3,"by":"a"}}"#
        );
        assert_eq!(parse(&serde_json::from_str(&json).unwrap()), m);
    }
}
