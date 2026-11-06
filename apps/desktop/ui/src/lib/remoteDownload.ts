// "Download on <device>" (remote_download.rs): links downloaded on one of
// your devices at another's request, both ways. The backend's
// `remote-download` event brings each change (asked, waiting for an answer,
// queued, downloading, done...); progress itself comes with the transfers.
// The "Download on…" picker opens from the command palette, the tray
// (`download-link-pick`, with the copied link), Send to… and a device's
// commands.

import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import { unwrap } from "./core";

export type DownloadItem = {
  id: string;
  node_id: string;
  here: boolean;
  url: string;
  host: string;
  name: string;
  size: number;
  done: number;
  state: string;
  error?: string | null;
  path?: string | null;
  started_at: number;
};

const [items, setItems] = createStore<Record<string, DownloadItem>>({});
/** Every download either way this run, by id. */
export const downloads = items;

/** Downloads waiting for an answer here, the oldest first. */
export function waitingAsks(): DownloadItem[] {
  return Object.values(items)
    .filter((i) => i.here && i.state === "asking")
    .sort((a, b) => a.started_at - b.started_at);
}

const [pick, setPick] = createSignal<string | null>(null);
/** The link the "Download on…" picker is open for ("" = none yet), or null. */
export const pickingLink = pick;
export const openDownloadPicker = (url: string | null): void => void setPick(url ?? "");
export const closeDownloadPicker = (): void => void setPick(null);

/** The first web link in `text`, without punctuation right after it. */
export function linkIn(text: string): string | null {
  const m = text.match(/https?:\/\/[^\s<>"]+/i);
  if (!m) return null;
  let url = m[0].replace(/[.,;:!?'*]+$/, "");
  const count = (c: string) => url.split(c).length - 1;
  while (url.endsWith(")") && count(")") > count("(")) url = url.slice(0, -1);
  return url;
}

/** What's known now, then each change as it comes. */
export async function initRemoteDownloads(): Promise<() => void> {
  const offs: (() => void)[] = [];
  offs.push(await listen<{ item: DownloadItem }>("remote-download", (e) => setItems(e.payload.item.id, e.payload.item)));
  offs.push(await listen<{ url: string }>("download-link-pick", (e) => openDownloadPicker(e.payload.url)));
  for (const i of await commands.remoteDownloads().catch(() => [])) {
    if (!items[i.id]) setItems(i.id, { ...i, size: i.size ?? 0, done: i.done ?? 0, started_at: i.started_at ?? 0 });
  }
  return () => offs.forEach((f) => f());
}

/** The web link copied on this computer, if there is one. */
export async function copiedLink(): Promise<string | null> {
  try {
    return await unwrap(commands.remoteDownloadCopiedLink());
  } catch {
    return null;
  }
}

export const requestDownload = (node: string, text: string) => unwrap(commands.remoteDownloadRequest(node, text));
export const answerDownload = (id: string, allow: boolean, always: boolean) =>
  unwrap(commands.remoteDownloadAnswer(id, allow, always));
/** The devices (node ids) that download links. */
export const downloadTargets = () => commands.remoteDownloadTargets();
