// Search everything: one query over what this computer keeps. The backend
// (search.rs, the `search` command) looks through devices, the saved
// messages, the notes, the clipboard and notification histories, transfers, synced
// folders, automations, schedules and shortcuts; this file adds what only the
// window has (this session's messages that weren't saved, the Help topics,
// the Settings sections) with the same rules, and
// the pieces the palette and the Search screen share: folding, highlighting,
// and where a result opens.
//
// The rules (same as search.rs): every word somewhere in the item, ignoring
// case and accents; a word in the title counts most, then a secondary field,
// then the text; a word at a word's start a little more; newer first among
// equals. Nothing leaves the computer.

import { createSignal } from "solid-js";
import { commands, type SearchFilter_Deserialize, type SearchHit_Serialize, type SearchResults_Serialize } from "../bindings";
import { TOPICS } from "../help/topics";
import { t, type Key } from "../i18n";
import { state } from "./core";

/** What can be found, in the order the groups show. */
export const FOUND_KINDS = [
  "devices",
  "messages",
  "notes",
  "clipboard",
  "notifications",
  "transfers",
  "folders",
  "automations",
  "schedules",
  "shortcuts",
  "settings",
  "help",
] as const;
export type FoundKind = (typeof FOUND_KINDS)[number];

/** The kinds the backend searches. */
const BACKEND: FoundKind[] = ["devices", "messages", "notes", "clipboard", "notifications", "transfers", "folders", "automations", "schedules", "shortcuts"];

export const KIND_LABEL: Record<FoundKind, Key> = {
  devices: "search.kind.devices",
  messages: "search.kind.messages",
  notes: "search.kind.notes",
  clipboard: "search.kind.clipboard",
  notifications: "search.kind.notifications",
  transfers: "search.kind.transfers",
  folders: "search.kind.folders",
  automations: "search.kind.automations",
  schedules: "search.kind.schedules",
  shortcuts: "search.kind.shortcuts",
  settings: "search.kind.settings",
  help: "search.kind.help",
};

/** One thing found. `title` empty: show `detail`'s snippet as the line. */
export type Found = {
  kind: FoundKind;
  id: string;
  title: string;
  detail: string;
  node?: string;
  ts?: number;
  /** Per kind: a clip's way, a notification's app, a transfer's state, a
   *  shortcut's keys, a schedule's time, "conflict" for a note that's a
   *  conflict copy. */
  tag?: string;
  score: number;
};
export type FoundGroup = { kind: FoundKind; total: number; hits: Found[] };

export type SearchOptions = {
  limit: number;
  /** Only these kinds (all when absent or empty). */
  kinds?: FoundKind[];
  /** A device; a span in ms since 1970. */
  node?: string | null;
  since?: number | null;
  until?: number | null;
};

// --- folding ------------------------------------------------------------------

const MARK = /\p{M}/u;
const ASCII = /^[\x00-\x7f]*$/;

/** One character folded: "" for a combining accent, else one character
 *  (lower case, accent off, typographic apostrophe as a plain one). */
function foldChar(ch: string): string {
  if (MARK.test(ch)) return "";
  let base = ch;
  const parts = Array.from(ch.normalize("NFD"));
  // Only when the rest is accents: Hangul and the like stay whole.
  if (parts.length > 1 && parts.slice(1).every((c) => MARK.test(c))) base = parts[0];
  const lower = Array.from(base.toLowerCase())[0] ?? base;
  return lower === "‘" || lower === "’" ? "'" : lower;
}

/** `s` folded for matching (lower case, no accents). */
export function fold(s: string): string {
  if (ASCII.test(s)) return s.toLowerCase();
  let out = "";
  for (const ch of s) out += foldChar(ch);
  return out;
}

/** The words of a query, folded. */
export function queryWords(query: string): string[] {
  return fold(query).split(/\s+/).filter(Boolean);
}

/** `text` folded, with each folded code unit's place in the original. */
function foldMapped(text: string): { folded: string; at: number[] } {
  let folded = "";
  const at: number[] = [];
  let i = 0;
  for (const ch of text) {
    const f = foldChar(ch);
    for (let k = 0; k < f.length; k++) at.push(i);
    folded += f;
    i += ch.length;
  }
  return { folded, at };
}

/** Where `words` are in `text`: [start, end) ranges in the original,
 *  sorted and merged (accents after a match are in it). */
export function matchRanges(text: string, words: string[]): [number, number][] {
  if (!text || !words.length) return [];
  const { folded, at } = foldMapped(text);
  const ranges: [number, number][] = [];
  for (const w of words) {
    let from = 0;
    for (;;) {
      const i = folded.indexOf(w, from);
      if (i < 0) break;
      const end = i + w.length;
      ranges.push([at[i], end < at.length ? at[end] : text.length]);
      from = i + Math.max(1, w.length);
    }
  }
  ranges.sort((a, b) => a[0] - b[0]);
  const merged: [number, number][] = [];
  for (const r of ranges) {
    const last = merged[merged.length - 1];
    if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]);
    else merged.push([r[0], r[1]]);
  }
  return merged;
}

/** `text` in pieces, the matching ones marked (for <mark>). */
export function highlight(text: string, words: string[]): { s: string; hit: boolean }[] {
  const out: { s: string; hit: boolean }[] = [];
  let last = 0;
  for (const [a, b] of matchRanges(text, words)) {
    if (a > last) out.push({ s: text.slice(last, a), hit: false });
    out.push({ s: text.slice(a, b), hit: true });
    last = b;
  }
  if (last < text.length) out.push({ s: text.slice(last), hit: false });
  return out;
}

/** `text` on one line, at most `max` characters, around the first match. */
export function snippet(text: string, words: string[], max = 140): string {
  const flat = text.split(/\s+/).filter(Boolean).join(" ");
  if (flat.length <= max) return flat;
  const first = matchRanges(flat, words)[0]?.[0] ?? 0;
  let begin = Math.max(0, first - Math.floor(max / 4));
  const end = Math.min(flat.length, begin + max);
  begin = Math.max(0, end - max);
  return `${begin > 0 ? "…" : ""}${flat.slice(begin, end)}${end < flat.length ? "…" : ""}`;
}

// --- scoring (as search.rs) ---------------------------------------------------

const TITLE = 3;
const SECONDARY = 2;
const BODY = 1;
const WORD_START = 0.5;
const DAY_MS = 24 * 3600 * 1000;

const isWordChar = (c: string) => /[\p{L}\p{N}]/u.test(c);

function wordIn(hay: string, word: string, weight: number): number {
  let best = 0;
  let from = 0;
  for (;;) {
    const i = hay.indexOf(word, from);
    if (i < 0) return best;
    if (i === 0 || !isWordChar(hay[i - 1])) return weight + WORD_START;
    best = weight;
    from = i + 1;
  }
}

/** How well the fields match: null unless every word is in one. */
export function scoreFields(words: string[], f: { title: string; secondary?: string; body?: string }): number | null {
  if (!words.length) return 0;
  const title = fold(f.title);
  const secondary = fold(f.secondary ?? "");
  let body: string | undefined;
  let total = 0;
  for (const w of words) {
    let s = wordIn(title, w, TITLE);
    if (s < TITLE) s = Math.max(s, wordIn(secondary, w, SECONDARY));
    if (!s) s = wordIn((body ??= fold(f.body ?? "")), w, BODY);
    if (!s) return null;
    total += s;
  }
  return total;
}

function recency(ts: number | undefined, now: number): number {
  if (!ts || ts <= 0) return 0;
  return 1 / (1 + Math.max(0, (now - ts) / DAY_MS) / 7);
}

/** The best `limit`, as a group (null when nothing matched). */
function rankLocal(kind: FoundKind, items: Found[], limit: number): FoundGroup | null {
  if (!items.length) return null;
  items.sort((a, b) => b.score - a.score || (b.ts ?? 0) - (a.ts ?? 0) || a.title.localeCompare(b.title));
  return { kind, total: items.length, hits: items.slice(0, limit) };
}

// --- what the window searches itself -------------------------------------------

/** Settings sections, by their element id (`settings-<id>`). */
export const SETTINGS_SECTIONS: { id: string; title: Key }[] = [
  { id: "account", title: "settings.account" },
  { id: "device", title: "settings.thisDevice" },
  { id: "shortcuts", title: "shortcuts.title" },
  { id: "calls", title: "calls.title" },
  { id: "computer-notifs", title: "computerNotifs.title" },
  { id: "wake", title: "wake.title" },
  { id: "file-manager", title: "sendTo.settingTitle" },
  { id: "sync", title: "sync.title" },
  { id: "commands", title: "commands.title" },
  { id: "schedules", title: "schedule.local.title" },
  { id: "automations", title: "automation.title" },
  { id: "webhooks", title: "webhooks.title" },
  { id: "key", title: "settings.deviceKey" },
  { id: "appearance", title: "settings.appearance" },
];

function searchLocal(kind: FoundKind, words: string[], o: SearchOptions, now: number): FoundGroup | null {
  const dated = o.since != null || o.until != null;
  const inSpan = (ts: number) => (o.since == null || ts >= o.since) && (o.until == null || ts <= o.until);
  const out: Found[] = [];
  if (kind === "messages") {
    // The saved ones the backend finds.
    for (const m of state.messages) {
      if (m.saved) continue;
      if (o.node && m.node !== o.node) continue;
      if (dated && !inSpan(m.ts)) continue;
      const s = scoreFields(words, { title: "", body: m.text });
      if (s === null) continue;
      out.push({ kind, id: String(m.mid ?? m.id), title: "", detail: snippet(m.text, words), node: m.node, ts: m.ts, tag: m.dir, score: s + recency(m.ts, now) });
    }
  } else if (kind === "help" || kind === "settings") {
    // Not a device's, and no time: a device or a span leaves them out.
    if (o.node || dated || !words.length) return null;
    if (kind === "help") {
      for (const topic of TOPICS) {
        const s = scoreFields(words, { title: topic.title, secondary: topic.keywords.join(" "), body: topic.body });
        if (s !== null) out.push({ kind, id: topic.id, title: topic.title, detail: "", score: s });
      }
    } else {
      for (const sec of SETTINGS_SECTIONS) {
        const title = t(sec.title);
        const s = scoreFields(words, { title, secondary: t("nav.settings") });
        if (s !== null) out.push({ kind, id: sec.id, title, detail: "", score: s });
      }
    }
  }
  return rankLocal(kind, out, o.limit);
}

function fromBackend(kind: FoundKind, h: SearchHit_Serialize): Found {
  return {
    kind,
    id: h.id,
    title: h.title,
    detail: h.detail,
    node: h.node ?? undefined,
    ts: h.ts ?? undefined,
    tag: h.tag ?? undefined,
    score: h.score ?? 0,
  };
}

/** Search everything for `query`: the groups that matched, in
 *  `FOUND_KINDS` order. An empty query finds nothing unless a device or a
 *  span is chosen (then the newest of those). */
export async function searchAll(query: string, o: SearchOptions): Promise<FoundGroup[]> {
  const words = queryWords(query);
  const filtered = !!o.node || o.since != null || o.until != null;
  if (!words.length && !filtered) return [];
  const wanted = (k: FoundKind) => !o.kinds?.length || o.kinds.includes(k);
  const backendKinds = BACKEND.filter(wanted);
  const filter: SearchFilter_Deserialize = { node: o.node ?? null, since: o.since ?? null, until: o.until ?? null };
  const none: SearchResults_Serialize = { groups: [] };
  const remote: Promise<SearchResults_Serialize> = backendKinds.length
    ? commands.search(query, o.limit, backendKinds, filter).catch(() => none)
    : Promise.resolve(none);
  const now = Date.now();
  const local = (["messages", "help", "settings"] as FoundKind[])
    .filter(wanted)
    .map((k) => searchLocal(k, words, o, now))
    .filter((g): g is FoundGroup => g !== null);
  const groups: FoundGroup[] = [];
  // Messages come from both sides (saved, and not saved): one group.
  const add = (g: FoundGroup) => {
    const same = groups.find((x) => x.kind === g.kind);
    if (!same) return groups.push(g);
    same.total += g.total;
    same.hits = [...same.hits, ...g.hits].sort((a, b) => b.score - a.score || (b.ts ?? 0) - (a.ts ?? 0)).slice(0, o.limit);
  };
  local.forEach(add);
  for (const g of (await remote).groups) {
    add({ kind: g.kind as FoundKind, total: g.total, hits: g.hits.map((h) => fromBackend(g.kind as FoundKind, h)) });
  }
  return groups
    .filter((g) => FOUND_KINDS.includes(g.kind))
    .sort((a, b) => FOUND_KINDS.indexOf(a.kind) - FOUND_KINDS.indexOf(b.kind));
}

// --- opening a result -------------------------------------------------------------

/** What a screen should bring into view and mark: set when a result opens. */
export type FoundTarget = { kind: FoundKind; id: string; node?: string };
const [target, setTarget] = createSignal<FoundTarget | null>(null);
/** The result being opened (tiles widen their lists to show it). */
export const foundTarget = target;

/** The element id a screen gives the row of a found item. */
export const foundId = (kind: FoundKind, id: string | number) => `found-${kind}-${id}`;

/** Whether this row is the one just opened from a search. */
export function isFound(kind: FoundKind, id: string | number): boolean {
  const x = target();
  return !!x && x.kind === kind && x.id === String(id);
}

/** Mark `x` and, once its row is on the page, scroll to it and focus it.
 *  `missing` runs when it never shows (gone since). */
export function revealFound(x: FoundTarget, missing: () => void) {
  setTarget(x);
  const started = performance.now();
  const step = () => {
    if (target() !== x) return;
    const el = document.getElementById(foundId(x.kind, x.id));
    if (el) {
      el.scrollIntoView({ block: "center" });
      el.focus({ preventScroll: true });
      // The mark fades after a while.
      window.setTimeout(() => target() === x && setTarget(null), 4000);
      return;
    }
    if (performance.now() - started < 3000) requestAnimationFrame(step);
    else {
      setTarget(null);
      missing();
    }
  };
  requestAnimationFrame(step);
}
