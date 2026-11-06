// Search: one query over what this computer already keeps, for the command
// palette and the Search screen.
//
//  - devices (their names and the nicknames you gave them), the saved
//    messages (chat_history.rs), the notes (the core's notepad; deleted
//    ones are never listed), the links saved to read later (by title,
//    host and link), the clipboard history (clip_history.rs),
//    the notification history (notif_history.rs),
//    transfers (transfers.rs), synced folders (their names and folders),
//    automations, schedules and keyboard shortcuts;
//  - read through each module's own accessors, as copies: nothing is
//    indexed and no file format changes. Help topics, Settings sections and
//    messages not saved (with "Keep message history" off) live in the
//    window, which searches those itself with the same rules
//    (ui/src/lib/search.ts);
//  - every word of the query must be somewhere in an item, ignoring case
//    and accents ("cafe" finds "Café", "CAFÉ" and "café" spelled with a
//    combining accent). A word in the title counts most, then in the
//    secondary field (an app, a time, keys), then in the text; a word at
//    the start of a word counts a little more; newer items come first among
//    equals;
//  - only local data: nothing here leaves the computer.
//
// Folding is a simple one: lower case, accents taken off the Latin letters
// (Latin-1 and Latin Extended-A) and combining accents dropped, typographic
// apostrophes as plain ones. Each character folds to at most one, so a
// match's place in the folded text points back into the original for the
// snippet.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use mdcore::model::{Note, ReadLaterItem};
use tauri::{AppHandle, Manager};

use crate::automations::Automation;
use crate::chat_history::ChatRecord;
use crate::clip_history::{ClipDirection, ClipRecord};
use crate::device_labels::DeviceLabel;
use crate::notif_history::HistoryEntry;
use crate::schedule::Schedule;
use crate::shortcuts::ShortcutView;
use crate::sync_folders::SyncPairView;
use crate::transfers::{TransferEntry, TransferState};

/// What can be searched here (`kinds`), in the order the groups come back.
pub const KINDS: [&str; 11] = [
    "devices",
    "messages",
    "notes",
    "read_later",
    "clipboard",
    "notifications",
    "transfers",
    "folders",
    "automations",
    "schedules",
    "shortcuts",
];

/// The most hits one group returns.
const MAX_LIMIT: u32 = 200;
/// The longest snippet, in characters.
const SNIPPET: usize = 140;

/// How much a word counts in each field.
const TITLE: f64 = 3.0;
const SECONDARY: f64 = 2.0;
const BODY: f64 = 1.0;
/// Extra for a word found at the start of a word.
const WORD_START: f64 = 0.5;
/// A day, in milliseconds.
const DAY_MS: f64 = 24.0 * 3600.0 * 1000.0;

/// Narrowing a search: one device, and a time span (ms since 1970, either
/// end open). With a span, items that have no time are left out.
#[derive(serde::Deserialize, specta::Type, Clone, Debug, Default)]
pub struct SearchFilter {
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub until: Option<f64>,
}

/// One thing found.
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct SearchHit {
    /// Its id in its own list (a clip's number, a transfer's id, a node id
    /// for a device…), for opening it.
    pub id: String,
    /// What to show first: a name, a title, or the clip's text.
    pub title: String,
    /// A line of its text around the first match ("" when none).
    pub detail: String,
    /// The device it belongs to, if any.
    pub node: Option<String>,
    /// When, in ms since 1970, if it has a time.
    pub ts: Option<f64>,
    /// A kind's own short note: a clip's way ("in" / "out"), a
    /// notification's app, a transfer's state ("active" / "done" /
    /// "failed"), a folder's state, a shortcut's keys, "conflict" for a
    /// note that's a conflict copy, "read" / "unread" for a saved link.
    pub tag: Option<String>,
    pub score: f64,
}

/// The hits of one kind, best first, and how many matched in all.
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct SearchGroup {
    pub kind: String,
    pub total: u32,
    pub hits: Vec<SearchHit>,
}

/// Every kind that had a match, in `KINDS` order.
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct SearchResults {
    pub groups: Vec<SearchGroup>,
}

// --- folding and matching ------------------------------------------------------

/// A combining accent (dropped when folding).
fn is_mark(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'
        | '\u{1AB0}'..='\u{1AFF}'
        | '\u{1DC0}'..='\u{1DFF}'
        | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}')
}

/// A lower-case letter without its accent (Latin-1 and Latin Extended-A),
/// typographic apostrophes as plain ones; anything else as it is.
fn base(c: char) -> char {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => 's',
        'ţ' | 'ť' | 'ŧ' | 'ț' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        '\u{2018}' | '\u{2019}' => '\'',
        other => other,
    }
}

/// `s` folded for matching: lower case, no accents. Borrowed when it's
/// already plain lower-case ASCII (most text), so that costs nothing.
pub fn fold(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        if s.bytes().any(|b| b.is_ascii_uppercase()) {
            return Cow::Owned(s.to_ascii_lowercase());
        }
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_mark(c) {
            continue;
        }
        let lower = c.to_lowercase().next().unwrap_or(c);
        out.push(base(lower));
    }
    Cow::Owned(out)
}

/// The words of a query, folded.
#[derive(Clone, Debug, Default)]
pub struct Query {
    words: Vec<String>,
}

impl Query {
    pub fn new(q: &str) -> Self {
        Query {
            words: fold(q).split_whitespace().map(str::to_string).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// Whether `at` (a byte offset in `hay`) starts a word.
fn at_word_start(hay: &str, at: usize) -> bool {
    hay[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric())
}

/// How much `word` counts in `hay` (folded) worth `weight`: 0 when absent.
fn word_in(hay: &str, word: &str, weight: f64) -> f64 {
    let mut best = 0.0;
    for (at, _) in hay.match_indices(word) {
        if at_word_start(hay, at) {
            return weight + WORD_START;
        }
        best = weight;
    }
    best
}

/// An item's fields to match: title, secondary and text.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fields<'a> {
    pub title: &'a str,
    pub secondary: &'a str,
    pub body: &'a str,
}

/// How well `f` matches `q`: None unless every word is in some field; each
/// word counts for the best field it's in. An empty query matches with 0.
pub fn score(q: &Query, f: &Fields) -> Option<f64> {
    if q.is_empty() {
        return Some(0.0);
    }
    let title = fold(f.title);
    let secondary = fold(f.secondary);
    // The text is folded only when a word isn't in the title.
    let mut body: Option<Cow<str>> = None;
    let mut total = 0.0;
    for w in &q.words {
        let mut s = word_in(&title, w, TITLE);
        if s < TITLE {
            s = s.max(word_in(&secondary, w, SECONDARY));
        }
        if s == 0.0 {
            let b = body.get_or_insert_with(|| fold(f.body));
            s = word_in(b, w, BODY);
        }
        if s == 0.0 {
            return None;
        }
        total += s;
    }
    Some(total)
}

/// Up to 1 for something new, towards 0 with age (half after a week).
pub fn recency(ts: Option<f64>, now: f64) -> f64 {
    match ts {
        Some(ts) if ts > 0.0 => {
            let days = ((now - ts) / DAY_MS).max(0.0);
            1.0 / (1.0 + days / 7.0)
        }
        _ => 0.0,
    }
}

/// The index in `original`'s characters of folded character `folded_at`
/// (the fold only drops combining accents, one for one otherwise).
fn original_char(original: &str, folded_at: usize) -> usize {
    let mut seen = 0;
    for (i, c) in original.chars().enumerate() {
        if is_mark(c) {
            continue;
        }
        if seen == folded_at {
            return i;
        }
        seen += 1;
    }
    original.chars().count()
}

/// `text` on one line, at most `max` characters, around the first word of
/// `q` it has (from the start when none); an ellipsis where it was cut.
pub fn snippet(text: &str, q: &Query, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = flat.chars().count();
    if len <= max {
        return flat;
    }
    let folded = fold(&flat);
    let first = q.words.iter().filter_map(|w| folded.find(w.as_str())).min();
    let at = first.map_or(0, |b| original_char(&flat, folded[..b].chars().count()));
    // A little before the match, so it reads in context.
    let mut begin = at.saturating_sub(max / 4);
    let end = (begin + max).min(len);
    begin = end.saturating_sub(max);
    let mut out = String::new();
    if begin > 0 {
        out.push('\u{2026}');
    }
    out.extend(flat.chars().skip(begin).take(end - begin));
    if end < len {
        out.push('\u{2026}');
    }
    out
}

// --- ranking -----------------------------------------------------------------

/// One thing that may be found, borrowed from its list.
pub struct Candidate<'a> {
    pub id: Cow<'a, str>,
    pub node: Option<&'a str>,
    pub ts: Option<f64>,
    pub fields: Fields<'a>,
    /// Shown as the hit's title ("" for a snippet of the text instead).
    pub title: Cow<'a, str>,
    pub tag: Option<Cow<'a, str>>,
}

/// Whether `c` is within `filter`.
fn within(c: &Candidate, filter: &SearchFilter) -> bool {
    if let Some(node) = filter.node.as_deref().filter(|n| !n.is_empty()) {
        if c.node != Some(node) {
            return false;
        }
    }
    if filter.since.is_some() || filter.until.is_some() {
        let Some(ts) = c.ts else {
            return false;
        };
        if filter.since.is_some_and(|s| ts < s) || filter.until.is_some_and(|u| ts > u) {
            return false;
        }
    }
    true
}

/// The best `limit` of `items` for `q`, as a group (None when nothing
/// matched). An empty query needs a filter: then it lists the newest.
pub fn rank<'a>(
    kind: &str,
    q: &Query,
    filter: &SearchFilter,
    limit: usize,
    now: f64,
    items: impl IntoIterator<Item = Candidate<'a>>,
) -> Option<SearchGroup> {
    let mut found: Vec<(f64, Candidate<'a>)> = items
        .into_iter()
        .filter(|c| within(c, filter))
        .filter_map(|c| score(q, &c.fields).map(|s| (s + recency(c.ts, now), c)))
        .collect();
    if found.is_empty() {
        return None;
    }
    let total = found.len() as u32;
    found.sort_by(|(sa, a), (sb, b)| {
        sb.total_cmp(sa)
            .then_with(|| b.ts.unwrap_or(0.0).total_cmp(&a.ts.unwrap_or(0.0)))
            .then_with(|| a.title.cmp(&b.title))
    });
    found.truncate(limit);
    let hits = found
        .into_iter()
        .map(|(score, c)| {
            // The text around the match, unless the title says it all.
            let (title, detail) = if c.title.is_empty() {
                (snippet(c.fields.body, q, SNIPPET), String::new())
            } else {
                let detail = if c.fields.body.is_empty() {
                    String::new()
                } else {
                    snippet(c.fields.body, q, SNIPPET)
                };
                (c.title.into_owned(), detail)
            };
            SearchHit {
                id: c.id.into_owned(),
                title,
                detail,
                node: c.node.map(str::to_string),
                ts: c.ts,
                tag: c.tag.map(Cow::into_owned),
                score: (score * 1000.0).round() / 1000.0,
            }
        })
        .collect();
    Some(SearchGroup {
        kind: kind.to_string(),
        total,
        hits,
    })
}

// --- the sources ---------------------------------------------------------------

/// A device as searched: its node id and its own name.
pub struct DeviceRow {
    pub node: String,
    pub name: String,
}

/// Copies of what each module keeps, gathered for one search.
#[derive(Default)]
pub struct Sources {
    pub devices: Vec<DeviceRow>,
    pub labels: BTreeMap<String, DeviceLabel>,
    pub messages: Vec<ChatRecord>,
    /// The notes (`Engine::notes`: deleted ones aren't in it).
    pub notes: Vec<Note>,
    /// The links saved to read later (`Engine::read_later`).
    pub read_later: Vec<ReadLaterItem>,
    pub clips: Vec<ClipRecord>,
    pub notifications: Vec<HistoryEntry>,
    pub transfers: Vec<TransferEntry>,
    pub folders: Vec<SyncPairView>,
    pub automations: Vec<Automation>,
    pub schedules: Vec<Schedule>,
    pub shortcuts: Vec<ShortcutView>,
}

/// A note's name in a list: its title, else its text's first line.
pub fn note_title(n: &Note) -> &str {
    if !n.title.trim().is_empty() {
        return n.title.trim();
    }
    n.body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// Notes as candidates: found by title and text, dated by their last
/// change, belonging to the device that made it; conflict copies tagged.
fn note_candidates(notes: &[Note]) -> impl Iterator<Item = Candidate<'_>> {
    notes.iter().map(|n| {
        let title = note_title(n);
        Candidate {
            id: Cow::Borrowed(&n.id),
            node: (!n.by.is_empty()).then_some(n.by.as_str()),
            ts: (n.updated_at > 0).then_some(n.updated_at as f64),
            fields: Fields {
                title,
                secondary: "",
                body: &n.body,
            },
            title: Cow::Borrowed(title),
            tag: n.conflict_of.is_some().then_some(Cow::Borrowed("conflict")),
        }
    })
}

/// Saved links as candidates: found by title, host and link, dated by when
/// they were saved, tagged read or unread.
fn read_later_candidates(items: &[ReadLaterItem]) -> impl Iterator<Item = Candidate<'_>> {
    items.iter().map(|i| Candidate {
        id: Cow::Borrowed(&i.id),
        node: (!i.by.is_empty()).then_some(i.by.as_str()),
        ts: (i.saved_at > 0).then_some(i.saved_at as f64),
        fields: Fields {
            title: &i.title,
            secondary: &i.host,
            body: &i.url,
        },
        title: Cow::Borrowed(non_empty(&i.title, &i.url)),
        tag: Some(Cow::Borrowed(if i.read { "read" } else { "unread" })),
    })
}

fn non_empty<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.trim().is_empty() {
        b
    } else {
        a
    }
}

/// Run a search over `src`: for each wanted kind (all when `kinds` is
/// empty), its best `limit` hits.
pub fn run(
    src: &Sources,
    query: &str,
    limit: u32,
    kinds: &[String],
    filter: &SearchFilter,
    now: f64,
) -> SearchResults {
    let q = Query::new(query);
    let has_filter = filter.node.as_deref().is_some_and(|n| !n.is_empty())
        || filter.since.is_some()
        || filter.until.is_some();
    if q.is_empty() && !has_filter {
        return SearchResults { groups: Vec::new() };
    }
    let limit = limit.clamp(1, MAX_LIMIT) as usize;
    let wanted = |k: &str| kinds.is_empty() || kinds.iter().any(|x| x == k);
    let mut groups = Vec::new();
    for kind in KINDS {
        if !wanted(kind) {
            continue;
        }
        let group = match kind {
            "devices" => {
                let names: Vec<(String, &DeviceRow)> = src
                    .devices
                    .iter()
                    .map(|d| {
                        let shown = crate::device_labels::name_for(&src.labels, &d.node, &d.name);
                        (shown, d)
                    })
                    .collect();
                rank(
                    kind,
                    &q,
                    filter,
                    limit,
                    now,
                    names.iter().map(|(shown, d)| Candidate {
                        id: Cow::Borrowed(&d.node),
                        node: Some(&d.node),
                        ts: None,
                        fields: Fields {
                            title: shown,
                            secondary: &d.name,
                            body: "",
                        },
                        title: Cow::Borrowed(shown),
                        tag: None,
                    }),
                )
            }
            "messages" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.messages.iter().map(|m| Candidate {
                    id: Cow::Borrowed(&m.id),
                    node: Some(m.node()),
                    ts: Some(m.at),
                    fields: Fields {
                        body: &m.text,
                        ..Fields::default()
                    },
                    title: Cow::Borrowed(""),
                    tag: Some(Cow::Borrowed(if m.incoming() { "in" } else { "out" })),
                }),
            ),
            "notes" => rank(kind, &q, filter, limit, now, note_candidates(&src.notes)),
            "read_later" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                read_later_candidates(&src.read_later),
            ),
            "clipboard" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.clips.iter().map(|c| Candidate {
                    id: Cow::Owned(c.id.to_string()),
                    node: c.node(),
                    ts: Some(c.at),
                    fields: Fields {
                        body: &c.text,
                        ..Fields::default()
                    },
                    title: Cow::Borrowed(""),
                    tag: Some(Cow::Borrowed(match c.direction {
                        ClipDirection::In => "in",
                        ClipDirection::Out => "out",
                    })),
                }),
            ),
            "notifications" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.notifications.iter().map(|n| Candidate {
                    id: Cow::Owned(n.id.to_string()),
                    node: Some(&n.from),
                    ts: Some(n.ts),
                    fields: Fields {
                        title: &n.title,
                        secondary: &n.app,
                        body: &n.body,
                    },
                    title: Cow::Borrowed(non_empty(&n.title, &n.app)),
                    tag: (!n.app.is_empty()).then_some(Cow::Borrowed(n.app.as_str())),
                }),
            ),
            "transfers" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.transfers.iter().map(|t| Candidate {
                    id: Cow::Borrowed(&t.id),
                    node: Some(&t.node_id),
                    ts: Some(t.ended.unwrap_or(t.started)),
                    fields: Fields {
                        title: &t.name,
                        secondary: "",
                        body: t.path.as_deref().unwrap_or(""),
                    },
                    title: Cow::Borrowed(&t.name),
                    tag: Some(Cow::Borrowed(match t.state {
                        TransferState::Active => "active",
                        TransferState::Done => "done",
                        TransferState::Failed => "failed",
                    })),
                }),
            ),
            "folders" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.folders.iter().map(|p| Candidate {
                    id: Cow::Borrowed(&p.id),
                    node: Some(&p.peer),
                    ts: (p.last_synced > 0.0).then_some(p.last_synced * 1000.0),
                    fields: Fields {
                        title: &p.name,
                        secondary: "",
                        body: &p.path,
                    },
                    title: Cow::Borrowed(&p.name),
                    tag: Some(Cow::Borrowed(&p.state)),
                }),
            ),
            "automations" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.automations.iter().map(|a| Candidate {
                    id: Cow::Borrowed(&a.id),
                    node: Some(&a.node),
                    ts: None,
                    fields: Fields {
                        title: non_empty(&a.label, &a.action),
                        secondary: &a.trigger.kind,
                        body: &a.action,
                    },
                    title: Cow::Borrowed(non_empty(&a.label, &a.action)),
                    tag: Some(Cow::Borrowed(&a.trigger.kind)),
                }),
            ),
            "schedules" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.schedules.iter().map(|s| Candidate {
                    id: Cow::Borrowed(&s.id),
                    node: Some(&s.node),
                    ts: None,
                    fields: Fields {
                        title: non_empty(&s.label, &s.action),
                        secondary: &s.time,
                        body: &s.action,
                    },
                    title: Cow::Borrowed(non_empty(&s.label, &s.action)),
                    tag: Some(Cow::Borrowed(&s.time)),
                }),
            ),
            "shortcuts" => rank(
                kind,
                &q,
                filter,
                limit,
                now,
                src.shortcuts.iter().map(|s| Candidate {
                    id: Cow::Borrowed(&s.binding.id),
                    node: (!s.binding.node.is_empty()).then_some(s.binding.node.as_str()),
                    ts: None,
                    fields: Fields {
                        title: &s.name,
                        secondary: &s.label,
                        body: "",
                    },
                    title: Cow::Borrowed(&s.name),
                    tag: Some(Cow::Borrowed(&s.label)),
                }),
            ),
            _ => None,
        };
        groups.extend(group);
    }
    SearchResults { groups }
}

/// Copies of the wanted kinds' lists, from the app's state.
fn gather(app: &AppHandle, kinds: &[String]) -> Sources {
    let wanted = |k: &str| kinds.is_empty() || kinds.iter().any(|x| x == k);
    let mut src = Sources::default();
    if wanted("devices") {
        if let Some(engine) = app.try_state::<crate::Eng>() {
            src.devices = engine
                .get_peers()
                .into_iter()
                .map(|p| DeviceRow {
                    node: p.node_id,
                    name: p.name,
                })
                .collect();
        }
        if let Some(labels) = app.try_state::<crate::device_labels::DeviceLabels>() {
            src.labels = crate::device_labels::get_device_labels(labels);
        }
    }
    if wanted("messages") {
        if let Some(h) = app.try_state::<crate::chat_history::ChatHistory>() {
            src.messages = h.snapshot();
        }
    }
    if wanted("notes") {
        if let Some(engine) = app.try_state::<crate::Eng>() {
            src.notes = engine.notes();
        }
    }
    if wanted("read_later") {
        if let Some(engine) = app.try_state::<crate::Eng>() {
            src.read_later = engine.read_later();
        }
    }
    if wanted("clipboard") {
        if let Some(h) = app.try_state::<crate::clip_history::ClipHistory>() {
            src.clips = h.snapshot();
        }
    }
    if wanted("notifications") {
        if let Some(h) = app.try_state::<crate::notif_history::NotifHistory>() {
            src.notifications = h.snapshot();
        }
    }
    if wanted("transfers") {
        if let Some(h) = app.try_state::<crate::transfers::TransferHistory>() {
            src.transfers = h.snapshot();
        }
    }
    if wanted("folders") {
        if let Some(engine) = app.try_state::<crate::Eng>() {
            src.folders = engine.sync_pairs().into_iter().map(Into::into).collect();
        }
    }
    if wanted("automations") {
        if let Some(s) = app.try_state::<crate::automations::Automations>() {
            src.automations = crate::automations::list_automations(s);
        }
    }
    if wanted("schedules") {
        if let Some(s) = app.try_state::<crate::schedule::Schedules>() {
            src.schedules = crate::schedule::list_schedules(s);
        }
    }
    if wanted("shortcuts") {
        if let Some(s) = app.try_state::<crate::shortcuts::Shortcuts>() {
            src.shortcuts = crate::shortcuts::list_shortcuts(app.clone(), s).bindings;
        }
    }
    src
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Search what this computer keeps for `query` (every word, ignoring case
/// and accents): for each kind (`KINDS`; all when `kinds` is none or
/// empty) its best `limit_per_kind` hits and how many matched. `filter`
/// narrows to a device and a time span. Only local data is read.
#[tauri::command]
#[specta::specta]
pub async fn search(
    query: String,
    limit_per_kind: u32,
    kinds: Option<Vec<String>>,
    filter: Option<SearchFilter>,
    app: AppHandle,
) -> SearchResults {
    let kinds = kinds.unwrap_or_default();
    let src = gather(&app, &kinds);
    run(
        &src,
        &query,
        limit_per_kind,
        &kinds,
        &filter.unwrap_or_default(),
        now_ms(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::Trigger;
    use crate::shortcuts::Binding;

    const NOW: f64 = 1_700_000_000_000.0;

    fn q(s: &str) -> Query {
        Query::new(s)
    }

    fn fields<'a>(title: &'a str, secondary: &'a str, body: &'a str) -> Fields<'a> {
        Fields {
            title,
            secondary,
            body,
        }
    }

    #[test]
    fn folding_drops_case_and_accents() {
        assert_eq!(fold("hello"), "hello");
        assert!(matches!(fold("hello"), Cow::Borrowed(_)));
        assert_eq!(fold("HeLLo"), "hello");
        assert_eq!(fold("Café"), "cafe");
        assert_eq!(fold("CAFÉ"), "cafe");
        // A combining accent (NFD, as macOS names files) goes too.
        assert_eq!(fold("cafe\u{0301}"), "cafe");
        assert_eq!(fold("Ångström Łódź Ñandú"), "angstrom lodz nandu");
        assert_eq!(fold("Straße"), "straße");
        assert_eq!(fold("it\u{2019}s"), "it's");
        // Other scripts stay as they are (lower-cased where they have case).
        assert_eq!(fold("日本語"), "日本語");
        assert_eq!(fold("ΣΟΦΙΑ"), "σοφια");
    }

    #[test]
    fn every_word_must_match_somewhere() {
        let f = fields("Meeting notes", "Mail", "See you at the blue door");
        assert!(score(&q("meeting"), &f).is_some());
        assert!(score(&q("MEETING blue"), &f).is_some());
        assert!(score(&q("mail door"), &f).is_some());
        assert!(score(&q("meeting window"), &f).is_none());
        assert_eq!(score(&q("  "), &f), Some(0.0));
    }

    #[test]
    fn matching_ignores_accents_both_ways() {
        let f = fields("Café receipts", "", "");
        assert!(score(&q("cafe"), &f).is_some());
        assert!(score(&q("CAFÉ"), &f).is_some());
        let nfd = fields("cafe\u{0301} receipts", "", "");
        assert!(score(&q("café"), &nfd).is_some());
        assert!(score(&q("it’s"), &fields("it's here", "", "")).is_some());
    }

    #[test]
    fn title_beats_secondary_beats_body() {
        let w = q("door");
        let title = score(&w, &fields("door", "", "")).unwrap();
        let secondary = score(&w, &fields("x", "door", "")).unwrap();
        let body = score(&w, &fields("x", "", "door")).unwrap();
        assert!(
            title > secondary && secondary > body,
            "{title} {secondary} {body}"
        );
        // A word's start counts more than its middle.
        let start = score(&q("door"), &fields("door bell", "", "")).unwrap();
        let middle = score(&q("oor"), &fields("door bell", "", "")).unwrap();
        assert!(start > middle);
    }

    #[test]
    fn newer_ranks_first_among_equals() {
        assert!(recency(Some(NOW), NOW) > recency(Some(NOW - 7.0 * DAY_MS), NOW));
        assert!((recency(Some(NOW - 7.0 * DAY_MS), NOW) - 0.5).abs() < 1e-9);
        assert_eq!(recency(None, NOW), 0.0);
        // But recency never beats a better field.
        let old_title = TITLE + recency(Some(NOW - 3650.0 * DAY_MS), NOW);
        let new_body = BODY + WORD_START + recency(Some(NOW), NOW);
        assert!(old_title > new_body);
    }

    #[test]
    fn snippets_center_on_the_match() {
        let text = format!("{} needle {}", "a ".repeat(200), "b ".repeat(200));
        let s = snippet(&text, &q("needle"), 40);
        assert!(s.contains("needle"), "{s}");
        assert!(s.starts_with('\u{2026}') && s.ends_with('\u{2026}'));
        assert!(s.chars().count() <= 42);
        // Short text: as it is, on one line.
        assert_eq!(snippet("one\ntwo   three", &q("x"), 40), "one two three");
        // A combining accent before the match doesn't shift it.
        let nfd = format!(
            "{}cafe\u{0301} {} needle {}",
            "x ".repeat(50),
            "y ".repeat(50),
            "z ".repeat(50)
        );
        assert!(snippet(&nfd, &q("needle"), 30).contains("needle"));
    }

    fn clip(id: u32, dir: ClipDirection, node: &str, text: &str, at: f64) -> ClipRecord {
        let n = Some(node.to_string());
        let (from_node, to_node) = match dir {
            ClipDirection::In => (n, None),
            ClipDirection::Out => (None, n),
        };
        ClipRecord {
            id,
            text: text.into(),
            truncated: false,
            from_node,
            to_node,
            direction: dir,
            at,
        }
    }

    fn note(id: u32, from: &str, app: &str, title: &str, body: &str, ts: f64) -> HistoryEntry {
        HistoryEntry {
            id,
            from: from.into(),
            app: app.into(),
            title: title.into(),
            body: body.into(),
            key: None,
            ts,
            dismissed: false,
        }
    }

    fn transfer(id: &str, node: &str, name: &str, state: TransferState, ts: f64) -> TransferEntry {
        TransferEntry {
            id: id.into(),
            node_id: node.into(),
            name: name.into(),
            incoming: true,
            state,
            done: 1.0,
            total: 1.0,
            resumed_from: None,
            path: Some(format!("/Users/me/Downloads/{name}")),
            reason: None,
            started: ts,
            ended: Some(ts),
            exists: false,
        }
    }

    fn seeded() -> Sources {
        let mut labels = BTreeMap::new();
        labels.insert(
            "p".to_string(),
            DeviceLabel {
                nickname: Some("Work phone".into()),
                icon: None,
                color: None,
            },
        );
        Sources {
            devices: vec![
                DeviceRow {
                    node: "p".into(),
                    name: "Pixel 8".into(),
                },
                DeviceRow {
                    node: "m".into(),
                    name: "MacBook-Air".into(),
                },
            ],
            labels,
            messages: Vec::new(),
            notes: Vec::new(),
            read_later: Vec::new(),
            clips: vec![
                clip(
                    3,
                    ClipDirection::In,
                    "p",
                    "Café on Rue Cler, 8pm",
                    NOW - DAY_MS,
                ),
                clip(
                    2,
                    ClipDirection::Out,
                    "m",
                    "invoice 2041 total",
                    NOW - 2.0 * DAY_MS,
                ),
                clip(
                    1,
                    ClipDirection::In,
                    "p",
                    "the old invoice",
                    NOW - 30.0 * DAY_MS,
                ),
            ],
            notifications: vec![
                note(
                    9,
                    "p",
                    "Mail",
                    "Invoice ready",
                    "Your invoice is attached",
                    NOW - DAY_MS,
                ),
                note(
                    8,
                    "p",
                    "Chat",
                    "Sam",
                    "lunch at the café?",
                    NOW - 3.0 * DAY_MS,
                ),
            ],
            transfers: vec![
                transfer(
                    "t1",
                    "p",
                    "invoice.pdf",
                    TransferState::Done,
                    NOW - 5.0 * DAY_MS,
                ),
                transfer(
                    "t2",
                    "m",
                    "holiday.jpg",
                    TransferState::Failed,
                    NOW - DAY_MS,
                ),
            ],
            folders: vec![SyncPairView {
                id: "s1".into(),
                peer: "m".into(),
                name: "Invoices".into(),
                path: "/Users/me/Invoices".into(),
                mode: "both".into(),
                paused: false,
                state: "in_sync".into(),
                syncing: 0,
                files: 3,
                conflicts: 0,
                skipped: 0,
                renamed: 0,
                last_synced: (NOW - DAY_MS) / 1000.0,
                error: None,
            }],
            automations: vec![Automation {
                id: "a1".into(),
                trigger: Trigger {
                    kind: "connects".into(),
                    node: "p".into(),
                    level: None,
                },
                node: "m".into(),
                action: "ringer".into(),
                args: String::new(),
                label: "Ringer to normal".into(),
                enabled: true,
                last_fired: None,
            }],
            schedules: vec![Schedule {
                id: "s1".into(),
                node: "p".into(),
                action: "ring".into(),
                args: String::new(),
                label: "Wake-up ring".into(),
                time: "07:30".into(),
                days: vec![1, 2, 3],
                enabled: true,
                last_run: None,
            }],
            shortcuts: vec![ShortcutView {
                binding: Binding {
                    id: "k1".into(),
                    action: "ring".into(),
                    node: "p".into(),
                    automation: String::new(),
                    keys: "CmdOrCtrl+Shift+R".into(),
                    enabled: true,
                },
                label: "⇧⌘R".into(),
                name: "Ring Work phone".into(),
                problem: None,
            }],
        }
    }

    fn kinds_of(r: &SearchResults) -> Vec<&str> {
        r.groups.iter().map(|g| g.kind.as_str()).collect()
    }

    #[test]
    fn finds_across_every_kind_in_order() {
        let src = seeded();
        let r = run(&src, "invoice", 10, &[], &SearchFilter::default(), NOW);
        assert_eq!(
            kinds_of(&r),
            ["clipboard", "notifications", "transfers", "folders"]
        );
        let clips = &r.groups[0];
        assert_eq!(clips.total, 2);
        // Newer first (same field).
        assert_eq!(clips.hits[0].id, "2");
        assert_eq!(clips.hits[0].tag.as_deref(), Some("out"));
        assert_eq!(clips.hits[0].node.as_deref(), Some("m"));
        // A clip's text is its title.
        assert_eq!(clips.hits[0].title, "invoice 2041 total");
        let notes = &r.groups[1];
        assert_eq!(notes.hits[0].title, "Invoice ready");
        assert_eq!(notes.hits[0].detail, "Your invoice is attached");
        assert_eq!(notes.hits[0].tag.as_deref(), Some("Mail"));
        assert_eq!(r.groups[2].hits[0].id, "t1");
        assert_eq!(r.groups[3].hits[0].ts, Some(NOW - DAY_MS));
    }

    #[test]
    fn devices_by_name_or_nickname() {
        let src = seeded();
        let none = SearchFilter::default();
        let r = run(&src, "work", 10, &[], &none, NOW);
        assert_eq!(r.groups[0].kind, "devices");
        assert_eq!(r.groups[0].hits[0].title, "Work phone");
        assert_eq!(r.groups[0].hits[0].id, "p");
        let r = run(&src, "pixel", 10, &["devices".into()], &none, NOW);
        assert_eq!(r.groups[0].hits[0].id, "p");
        // Shown tidied ("MacBook Air"), found either way.
        let r = run(&src, "macbook air", 10, &["devices".into()], &none, NOW);
        assert_eq!(r.groups[0].hits[0].title, "MacBook Air");
        let r = run(&src, "macbook-air", 10, &["devices".into()], &none, NOW);
        assert_eq!(r.groups[0].hits[0].id, "m");
        assert!(run(&src, "ipad", 10, &["devices".into()], &none, NOW)
            .groups
            .is_empty());
    }

    #[test]
    fn accents_and_case_across_stores() {
        let src = seeded();
        let r = run(&src, "CAFE", 10, &[], &SearchFilter::default(), NOW);
        assert_eq!(kinds_of(&r), ["clipboard", "notifications"]);
        assert_eq!(r.groups[1].hits[0].detail, "lunch at the café?");
    }

    #[test]
    fn settings_things_by_name() {
        let src = seeded();
        let none = SearchFilter::default();
        assert_eq!(
            kinds_of(&run(&src, "ringer", 10, &[], &none, NOW)),
            ["automations"]
        );
        let r = run(&src, "wake", 10, &[], &none, NOW);
        assert_eq!(kinds_of(&r), ["schedules"]);
        assert_eq!(r.groups[0].hits[0].tag.as_deref(), Some("07:30"));
        let r = run(&src, "ring work", 10, &[], &none, NOW);
        assert_eq!(kinds_of(&r), ["shortcuts"]);
        assert_eq!(r.groups[0].hits[0].tag.as_deref(), Some("⇧⌘R"));
    }

    #[test]
    fn kinds_limit_and_totals() {
        let src = seeded();
        let none = SearchFilter::default();
        let r = run(&src, "invoice", 1, &["clipboard".into()], &none, NOW);
        assert_eq!(kinds_of(&r), ["clipboard"]);
        assert_eq!(r.groups[0].total, 2);
        assert_eq!(r.groups[0].hits.len(), 1);
        // Unknown kinds find nothing.
        assert!(run(&src, "invoice", 5, &["nope".into()], &none, NOW)
            .groups
            .is_empty());
        // An empty query with no filter finds nothing.
        assert!(run(&src, "  ", 5, &[], &none, NOW).groups.is_empty());
    }

    #[test]
    fn filters_by_device_and_time() {
        let src = seeded();
        let on_p = SearchFilter {
            node: Some("p".into()),
            ..SearchFilter::default()
        };
        let r = run(&src, "invoice", 10, &[], &on_p, NOW);
        assert_eq!(kinds_of(&r), ["clipboard", "notifications", "transfers"]);
        assert_eq!(r.groups[0].hits[0].id, "1");
        // The last week: the old clip and the transfer are out; things
        // without a time too.
        let week = SearchFilter {
            since: Some(NOW - 7.0 * DAY_MS),
            until: Some(NOW),
            ..SearchFilter::default()
        };
        let r = run(&src, "invoice", 10, &[], &week, NOW);
        assert_eq!(
            kinds_of(&r),
            ["clipboard", "notifications", "transfers", "folders"]
        );
        assert_eq!(r.groups[0].total, 1);
        let r = run(&src, "ring", 10, &[], &week, NOW);
        assert!(r.groups.is_empty());
        // A filter alone lists the newest.
        let r = run(&src, "", 10, &["clipboard".into()], &on_p, NOW);
        let ids: Vec<&str> = r.groups[0].hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["3", "1"]);
    }

    #[test]
    fn title_matches_rank_above_text_matches() {
        let mut src = seeded();
        src.notifications = vec![
            note(1, "p", "Chat", "hello", "about the report", NOW),
            note(2, "p", "Chat", "Report due", "", NOW - 60.0 * DAY_MS),
        ];
        let r = run(
            &src,
            "report",
            10,
            &["notifications".into()],
            &SearchFilter::default(),
            NOW,
        );
        let ids: Vec<&str> = r.groups[0].hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["2", "1"]);
    }

    #[test]
    fn saved_messages_are_found() {
        use crate::chat_history::{ChatStatus, Store};
        let mut store = Store::default();
        let mut out = |id: &str, to: &str, text: &str, at: f64, status: ChatStatus| {
            store.add(ChatRecord {
                id: id.into(),
                from_node: None,
                to_node: Some(to.into()),
                text: text.into(),
                truncated: false,
                at,
                status,
            });
        };
        out(
            "a",
            "p",
            "Dinner at the café?",
            NOW - DAY_MS,
            ChatStatus::Delivered,
        );
        out(
            "b",
            "m",
            "dinner moved",
            NOW - 30.0 * DAY_MS,
            ChatStatus::Sent,
        );
        store.add(ChatRecord {
            id: "c".into(),
            from_node: Some("p".into()),
            to_node: None,
            text: "Cafe is fine".into(),
            truncated: false,
            at: NOW,
            status: ChatStatus::Received,
        });
        let mut src = seeded();
        // What `ChatHistory::snapshot` hands over.
        src.messages = store.to_list();
        let none = SearchFilter::default();
        let r = run(&src, "cafe", 10, &[], &none, NOW);
        assert_eq!(kinds_of(&r)[0], "messages");
        let ids: Vec<&str> = r.groups[0].hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["c", "a"]);
        assert_eq!(r.groups[0].hits[0].tag.as_deref(), Some("in"));
        assert_eq!(r.groups[0].hits[1].tag.as_deref(), Some("out"));
        assert_eq!(r.groups[0].hits[1].node.as_deref(), Some("p"));
        let on_m = SearchFilter {
            node: Some("m".into()),
            ..SearchFilter::default()
        };
        let r = run(&src, "dinner", 10, &["messages".into()], &on_m, NOW);
        assert_eq!(r.groups[0].total, 1);
        assert_eq!(r.groups[0].hits[0].id, "b");
    }

    fn a_note(id: &str, title: &str, body: &str, by: &str, at: f64) -> Note {
        Note {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            created_at: at as u64,
            updated_at: at as u64,
            by: by.into(),
            by_name: by.into(),
            ..Note::default()
        }
    }

    #[test]
    fn notes_are_found_by_title_and_text() {
        let mut src = seeded();
        let mut conflict = a_note(
            "n3",
            "Groceries (conflict from Pixel 8)",
            "milk\neggs",
            "p",
            NOW - DAY_MS,
        );
        conflict.conflict_of = Some("n1".into());
        // What `Engine::notes` hands over (deleted ones never are).
        src.notes = vec![
            a_note(
                "n1",
                "Groceries",
                "milk\nbread\nCafé beans",
                "m",
                NOW - 2.0 * DAY_MS,
            ),
            a_note("n2", "  ", "\n  Trip plan\nbook the café", "p", NOW),
            conflict,
        ];
        let none = SearchFilter::default();

        // A title word: both titled notes, the newer (the conflict copy)
        // first, tagged; the text around the match as the detail.
        let r = run(&src, "groceries", 10, &[], &none, NOW);
        assert_eq!(kinds_of(&r), ["notes"]);
        let g = &r.groups[0];
        let ids: Vec<&str> = g.hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["n3", "n1"]);
        assert_eq!(g.hits[0].tag.as_deref(), Some("conflict"));
        assert_eq!(g.hits[1].tag, None);
        assert_eq!(g.hits[1].title, "Groceries");
        assert_eq!(g.hits[1].detail, "milk bread Café beans");
        assert_eq!(g.hits[1].ts, Some(NOW - 2.0 * DAY_MS));
        assert_eq!(g.hits[1].node.as_deref(), Some("m"));

        // No title: the text's first line stands in, and counts as one.
        let r = run(&src, "trip", 10, &["notes".into()], &none, NOW);
        assert_eq!(r.groups[0].hits[0].id, "n2");
        assert_eq!(r.groups[0].hits[0].title, "Trip plan");

        // Text only, accents ignored; a title match ranks above a text one.
        let r = run(&src, "cafe", 10, &["notes".into()], &none, NOW);
        let ids: Vec<&str> = r.groups[0].hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["n2", "n1"]);
        let r = run(&src, "plan", 10, &["notes".into()], &none, NOW);
        assert_eq!(r.groups[0].total, 1);

        // By the device that last changed it, and by when.
        let on_m = SearchFilter {
            node: Some("m".into()),
            ..SearchFilter::default()
        };
        let r = run(&src, "milk", 10, &["notes".into()], &on_m, NOW);
        assert_eq!(r.groups[0].total, 1);
        assert_eq!(r.groups[0].hits[0].id, "n1");
        let today = SearchFilter {
            since: Some(NOW - DAY_MS / 2.0),
            ..SearchFilter::default()
        };
        let r = run(&src, "", 10, &["notes".into()], &today, NOW);
        assert_eq!(r.groups[0].hits.len(), 1);
        assert_eq!(r.groups[0].hits[0].id, "n2");
        assert!(run(&src, "nothing", 10, &[], &none, NOW).groups.is_empty());
    }

    #[test]
    fn saved_links_are_found_by_title_host_and_link() {
        let mut src = seeded();
        let link = |id: &str, url: &str, title: &str, read: bool, at: f64| ReadLaterItem {
            id: id.into(),
            url: url.into(),
            title: title.into(),
            host: mdcore::features::read_later::host_of(url).unwrap(),
            read,
            saved_at: at as u64,
            updated_at: at as u64,
            by: "p".into(),
            ..ReadLaterItem::default()
        };
        // What `Engine::read_later` hands over (deleted ones never are).
        src.read_later = vec![
            link(
                "u1",
                "https://www.rust-lang.org/learn",
                "Learn Rust",
                false,
                NOW,
            ),
            link(
                "u2",
                "https://blog.dev/posts/zebra-crossings",
                "blog.dev",
                true,
                NOW - DAY_MS,
            ),
        ];
        let none = SearchFilter::default();
        let only = ["read_later".to_string()];

        let r = run(&src, "learn rust", 10, &only, &none, NOW);
        assert_eq!(kinds_of(&r), ["read_later"]);
        let hit = &r.groups[0].hits[0];
        assert_eq!((hit.id.as_str(), hit.title.as_str()), ("u1", "Learn Rust"));
        assert_eq!(hit.tag.as_deref(), Some("unread"));
        // By host, and by a word only in the link.
        let r = run(&src, "blog.dev", 10, &only, &none, NOW);
        assert_eq!(r.groups[0].hits[0].id, "u2");
        assert_eq!(r.groups[0].hits[0].tag.as_deref(), Some("read"));
        let r = run(&src, "zebra", 10, &only, &none, NOW);
        assert_eq!(r.groups[0].total, 1);
        assert!(r.groups[0].hits[0].detail.contains("zebra"));
    }

    #[test]
    fn typical_data_is_fast() {
        let mut src = seeded();
        let words = [
            "alpha", "Bravo", "charlie", "délta", "echo", "foxtrot", "golf",
        ];
        let text = |i: usize| {
            (0..40)
                .map(|j| words[(i + j) % words.len()])
                .collect::<Vec<_>>()
                .join(" ")
        };
        src.clips = (0..200)
            .map(|i| {
                clip(
                    i as u32,
                    ClipDirection::In,
                    "p",
                    &text(i),
                    NOW - i as f64 * 1000.0,
                )
            })
            .collect();
        src.notifications = (0..500)
            .map(|i| {
                note(
                    i as u32,
                    "p",
                    "Mail",
                    &format!("Subject {i}"),
                    &text(i),
                    NOW,
                )
            })
            .collect();
        src.transfers = (0..100)
            .map(|i| {
                transfer(
                    &format!("t{i}"),
                    "p",
                    &format!("file {i}.txt"),
                    TransferState::Done,
                    NOW,
                )
            })
            .collect();
        let start = std::time::Instant::now();
        for query in ["delta golf", "zzz", "subject 4", "e"] {
            run(&src, query, 20, &[], &SearchFilter::default(), NOW);
        }
        let each = start.elapsed() / 4;
        // Debug builds on shared CI runners are several times slower.
        let limit = if cfg!(debug_assertions) { 400 } else { 50 };
        assert!(each.as_millis() < limit, "took {each:?}");
    }
}
