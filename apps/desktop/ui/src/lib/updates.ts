// Automatic updates (src-tauri/src/updates.rs): the status the Rust side
// keeps, kept current from its "update-status" event, shared by the banner
// and Settings → Updates.

import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type UpdateStatus_Serialize as UpdateStatus } from "../bindings";

export type { UpdateStatus };

const [status, setStatus] = createSignal<UpdateStatus>();
export { status as updateStatus };

let watching = false;
/** Start following the status (once; later calls do nothing). */
export function watchUpdates() {
  if (watching) return;
  watching = true;
  void listen<UpdateStatus>("update-status", (e) => setStatus(e.payload));
  commands.updateStatus().then(setStatus, () => {});
}

export async function checkNow() {
  setStatus(await commands.updateCheck());
}

/** Download if needed, install, restart. Returns only on failure (the
 *  error), as the app restarts otherwise. */
export async function installUpdate(): Promise<string | null> {
  try {
    const r = await commands.updateInstall();
    return r.status === "error" ? r.error : null;
  } catch (e) {
    return String(e);
  }
}

export const later = () => commands.updateLater();

export async function setAutoInstall(on: boolean) {
  setStatus(await commands.setUpdateAuto(on));
}

/** "12.3": a download's size in MB for the progress line. */
export function megabytes(bytes: number): string {
  return (bytes / 1_000_000).toFixed(1);
}

/** 0-100, or null while the total isn't known. */
export function percent(s: UpdateStatus): number | null {
  return s.total ? Math.min(100, Math.round(((s.downloaded ?? 0) / s.total) * 100)) : null;
}
