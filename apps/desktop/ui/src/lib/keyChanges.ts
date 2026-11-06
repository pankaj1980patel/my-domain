// Devices whose security key changed (reinstalled, data cleared, or someone
// else using their id): the core refuses their direct connections until you
// trust the new key. Read at start and again when the core says a key
// changed (`key-changed`, once per new key) or after you decide.

import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type KeyChange } from "../bindings";
import { unwrap } from "./core";

const [keyChanges, setKeyChanges] = createSignal<KeyChange[]>([]);
export { keyChanges };
export type { KeyChange };

/** Read the list again. */
export async function loadKeyChanges(): Promise<void> {
  setKeyChanges(await commands.keyChanges());
}

let watching = false;
/** Keep the list fresh (once per window). */
export function watchKeyChanges(): void {
  if (watching) return;
  watching = true;
  loadKeyChanges().catch(() => {});
  listen("key-changed", () => loadKeyChanges().catch(() => {})).catch(() => {});
}

/** The change recorded for `node`, if any (dismissed ones too). */
export function keyChangeOf(node: string): KeyChange | undefined {
  return keyChanges().find((c) => c.node_id === node);
}

/** Trust `node`'s new key: the one whose fingerprint was shown. */
export async function trustNewKey(node: string, newFingerprint: string): Promise<void> {
  try {
    await unwrap(commands.trustNewKey(node, newFingerprint));
  } finally {
    await loadKeyChanges().catch(() => {});
  }
}

/** "Not now": keep refusing it, stop asking. */
export async function dismissKeyChange(node: string): Promise<void> {
  await commands.dismissKeyChange(node);
  await loadKeyChanges().catch(() => {});
}
