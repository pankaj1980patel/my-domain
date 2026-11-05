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
};

/** Save a note (`id` null: a new one; a deleted one comes back). Throws
 *  the core's reason when it can't be kept. */
export async function saveNote(id: string | null, f: NoteFields): Promise<Note> {
  const r = await commands.noteSave(id, f.title, f.body, f.pinned, f.checklist, f.color);
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
  return saveNote(n.id, { title: n.title, body: n.body, pinned: n.pinned, checklist: n.checklist, color: n.color });
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
