// Notes: the notepad that's the same on all your devices (the core's
// `features::notes`, `notes.rs`). The store keeps every note by id: loaded
// once, changed by what's saved here and by `notes-changed` events (notes
// changed on your other devices; nothing saved here comes back that way).

import { createStore, produce, reconcile } from "solid-js/store";
import { commands, type NoteView_Serialize } from "../bindings";

export type Note = NoteView_Serialize;

/** The longest body the core keeps, in bytes (`notes::MAX_BODY`). */
export const MAX_BODY = 64 * 1024;

export const [notes, setNotes] = createStore<{ byId: Record<string, Note>; loaded: boolean }>({
  byId: {},
  loaded: false,
});

export async function loadNotes() {
  const list = await commands.notesList();
  setNotes("byId", reconcile(Object.fromEntries(list.map((n) => [n.id, n]))));
  setNotes("loaded", true);
}

/** `notes-changed`: notes changed on your other devices, and ones deleted. */
export function applyNotesChanged(p: { notes?: Note[]; removed?: string[] }) {
  setNotes(
    "byId",
    produce((m) => {
      for (const n of p.notes ?? []) m[n.id] = n;
      for (const id of p.removed ?? []) delete m[id];
    }),
  );
}

/** Pinned first, then the most recently changed (as the core lists them). */
export function sortedNotes(): Note[] {
  return Object.values(notes.byId).sort(
    (a, b) =>
      Number(b.pinned) - Number(a.pinned) ||
      (b.updated_at ?? 0) - (a.updated_at ?? 0) ||
      a.id.localeCompare(b.id),
  );
}

/** Every typed word is in the title or the body. */
export function noteMatches(n: Note, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${n.title}\n${n.body}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

export type NoteFields = {
  title: string;
  body: string;
  pinned: boolean;
  checklist: boolean;
  color: string | null;
  /** The reminder: when (unix ms; null: none), and marked Done. */
  remindAt: number | null;
  remindDone: boolean;
};

/** Save a note (`id` null: a new one; a deleted one comes back). Throws
 *  the core's reason when it can't be kept. */
export async function saveNote(id: string | null, f: NoteFields): Promise<Note> {
  const r = await commands.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color, f.remindAt, f.remindDone);
  if (r.status === "error") throw new Error(r.error);
  setNotes("byId", r.data.id, r.data);
  return r.data;
}

export async function deleteNote(id: string): Promise<void> {
  const r = await commands.noteDelete(id);
  if (r.status === "error") throw new Error(r.error);
  setNotes(
    "byId",
    produce((m) => {
      delete m[id];
    }),
  );
}

/** Bring back a note just deleted, as it was (Undo). */
export function restoreNote(n: Note): Promise<Note> {
  return saveNote(n.id, {
    title: n.title,
    body: n.body,
    pinned: n.pinned,
    checklist: n.checklist,
    color: n.color,
    remindAt: n.remind_at,
    remindDone: n.remind_done,
  });
}

// --- reminders: every device with the note shows it at its time --------------

/** A reminder that hasn't been marked Done. */
export function pendingReminder(n: Note): boolean {
  return n.remind_at != null && !n.remind_done;
}

/** Notes with a reminder still to come (or due and not Done), soonest first. */
export function upcomingNotes(): Note[] {
  return Object.values(notes.byId)
    .filter(pendingReminder)
    .sort((a, b) => (a.remind_at ?? 0) - (b.remind_at ?? 0) || a.id.localeCompare(b.id));
}

/** Its time has come and it isn't Done. */
export function reminderDue(n: Note, now: number): boolean {
  return pendingReminder(n) && (n.remind_at ?? 0) <= now;
}

/** Set a note's reminder (`at` unix ms: a time, or a snooze), or take it
 *  off (null). Saved and sent to your other devices at once. */
export async function setReminder(id: string, at: number | null): Promise<Note> {
  const r = await commands.noteRemindAt(id, at);
  if (r.status === "error") throw new Error(r.error);
  setNotes("byId", r.data.id, r.data);
  return r.data;
}

/** Mark a note's reminder Done: it stops on all your devices. */
export async function reminderDone(id: string): Promise<Note> {
  const r = await commands.noteRemindDone(id);
  if (r.status === "error") throw new Error(r.error);
  setNotes("byId", r.data.id, r.data);
  return r.data;
}

const MINUTE = 60_000;

/** "Later today": 18:00 before 17:00, else three hours on (on the hour),
 *  while that's still today; null late in the evening. */
export function laterToday(now: Date): Date | null {
  const d = new Date(now);
  if (now.getHours() < 17) {
    d.setHours(18, 0, 0, 0);
    return d;
  }
  d.setMinutes(0, 0, 0);
  d.setHours(d.getHours() + 3);
  return d.getDate() === now.getDate() ? d : null;
}

/** Tomorrow at 9:00. */
export function tomorrowMorning(now: Date): Date {
  const d = new Date(now);
  d.setDate(d.getDate() + 1);
  d.setHours(9, 0, 0, 0);
  return d;
}

/** Next Monday at 9:00 (a week on, from a Monday). */
export function nextWeek(now: Date): Date {
  const d = new Date(now);
  const days = (8 - d.getDay()) % 7 || 7;
  d.setDate(d.getDate() + days);
  d.setHours(9, 0, 0, 0);
  return d;
}

export type Snooze = "10m" | "1h" | "tomorrow";

/** When a snooze ends. */
export function snoozeUntil(s: Snooze, now: Date): number {
  if (s === "10m") return now.getTime() + 10 * MINUTE;
  if (s === "1h") return now.getTime() + 60 * MINUTE;
  return tomorrowMorning(now).getTime();
}

const pad = (n: number) => String(n).padStart(2, "0");

/** A time as a date-and-time field's value ("YYYY-MM-DDTHH:MM", local). */
export function toLocalInput(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** A date-and-time field's value as unix ms (null: empty or not a time). */
export function fromLocalInput(v: string): number | null {
  if (!v) return null;
  const ms = new Date(v).getTime();
  return Number.isFinite(ms) ? ms : null;
}

/** "Thu 6 Nov, 09:00", with the year only when it isn't this year. */
export function reminderTime(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return d.toLocaleString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: sameYear ? undefined : "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// Reminders shown here (the backend's `note-reminder`): the window keeps
// them up with Open, Done and Snooze until one of those, or until the note
// is Done or given a new time (here or on another device).
export const [shownReminders, setShownReminders] = createStore<{ ids: string[] }>({ ids: [] });

/** `note-reminder`: a note's reminder came due here. */
export function onReminder(n: Note) {
  setNotes("byId", n.id, n);
  if (!shownReminders.ids.includes(n.id)) setShownReminders("ids", (ids) => [...ids, n.id]);
}

/** Take a reminder off the window. */
export function dismissReminder(id: string) {
  setShownReminders("ids", (ids) => ids.filter((x) => x !== id));
}

// --- checklists: a convention over plain text --------------------------------

export type Line = { box: boolean; checked: boolean; text: string };

const BOX = /^\[( |x|X)\] (.*)$/;

/** The body's lines; "[ ] " / "[x] " ones are boxes. */
export function checklistLines(body: string): Line[] {
  if (!body) return [];
  return body.split("\n").map((l) => {
    const m = BOX.exec(l);
    return m ? { box: true, checked: m[1] !== " ", text: m[2] } : { box: false, checked: false, text: l };
  });
}

/** The body with box line `i` ticked or unticked. */
export function toggleLine(body: string, i: number): string {
  const lines = body.split("\n");
  const m = BOX.exec(lines[i] ?? "");
  if (!m) return body;
  lines[i] = `[${m[1] === " " ? "x" : " "}] ${m[2]}`;
  return lines.join("\n");
}

/** The body with a new unticked item at the end. */
export function addItem(body: string, text: string): string {
  const item = `[ ] ${text.replace(/\s*\n\s*/g, " ").trim()}`;
  return body && !body.endsWith("\n") ? `${body}\n${item}` : `${body}${item}`;
}

/** "2 of 5 done" counts: [done, all] boxes. */
export function checklistCount(body: string): [number, number] {
  const boxes = checklistLines(body).filter((l) => l.box);
  return [boxes.filter((l) => l.checked).length, boxes.length];
}

/** How long a body is in bytes (the core's limit is in bytes). */
export function bodyBytes(body: string): number {
  return new TextEncoder().encode(body).length;
}
