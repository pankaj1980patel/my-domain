// Read later: links saved to read later, the same on all your devices (the
// core's `features::read_later`, `read_later.rs`). The store keeps every
// link by id: loaded once, changed by what's saved here, by the tray's
// "Save copied link" (`read-later-saved`), and by `read-later-changed`
// events (links changed on your other devices; nothing saved here comes
// back that way). Nothing is fetched from the web: a link's title is the
// one given, else its host.

import { createStore, produce, reconcile } from "solid-js/store";
import { commands, type ReadLaterView } from "../bindings";

export type ReadLaterItem = ReadLaterView;

export const [readLater, setReadLater] = createStore<{ byId: Record<string, ReadLaterItem>; loaded: boolean }>({
  byId: {},
  loaded: false,
});

export async function loadReadLater() {
  const list = await commands.readLaterList();
  setReadLater("byId", reconcile(Object.fromEntries(list.map((i) => [i.id, i]))));
  setReadLater("loaded", true);
}

/** `read-later-changed`: links changed on your other devices, and ones
 *  deleted there. */
export function applyReadLaterChanged(p: { items?: ReadLaterItem[]; removed?: string[] }) {
  setReadLater(
    "byId",
    produce((m) => {
      for (const i of p.items ?? []) m[i.id] = i;
      for (const id of p.removed ?? []) delete m[id];
    }),
  );
}

/** One link saved or changed here. */
export function applySaved(i: ReadLaterItem) {
  setReadLater("byId", i.id, i);
}

/** The most recently saved first (as the core lists them). */
export function sortedReadLater(): ReadLaterItem[] {
  return Object.values(readLater.byId).sort((a, b) => (b.saved_at ?? 0) - (a.saved_at ?? 0) || a.id.localeCompare(b.id));
}

export function unreadCount(): number {
  return Object.values(readLater.byId).filter((i) => !i.read).length;
}

/** Every typed word is in the title, the host or the link. */
export function readLaterMatches(i: ReadLaterItem, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${i.title}\n${i.host}\n${i.url}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

const check = <T>(r: { status: "ok"; data: T } | { status: "error"; error: string }): T => {
  if (r.status === "error") throw new Error(r.error);
  return r.data;
};

/** Save a link (`text`: the link, or text with one in it; `title` blank:
 *  its host). Throws the core's reason when it isn't a web link. */
export async function saveLink(text: string, title?: string): Promise<ReadLaterItem> {
  const saved = check(await commands.readLaterSave(text, title?.trim() ? title : null));
  applySaved(saved);
  return saved;
}

/** Save the link copied on this computer. */
export async function saveCopiedLink(): Promise<ReadLaterItem> {
  const saved = check(await commands.readLaterSaveClipboard());
  applySaved(saved);
  return saved;
}

export async function markRead(id: string, read: boolean): Promise<ReadLaterItem> {
  const saved = check(await commands.readLaterMark(id, read));
  applySaved(saved);
  return saved;
}

/** Open it in the browser (and mark it read). */
export async function openLink(id: string): Promise<ReadLaterItem> {
  const saved = check(await commands.readLaterOpen(id));
  applySaved(saved);
  return saved;
}

export async function deleteLink(id: string): Promise<void> {
  check(await commands.readLaterDelete(id));
  setReadLater(
    "byId",
    produce((m) => {
      delete m[id];
    }),
  );
}

/** Bring back a link just deleted (Undo): saved again, with its title and
 *  read mark. */
export async function restoreLink(i: ReadLaterItem): Promise<ReadLaterItem> {
  const back = await saveLink(i.url, i.title);
  return i.read ? markRead(back.id, true) : back;
}
