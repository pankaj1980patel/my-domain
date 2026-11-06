// Verifying a device's security key: this computer's own fingerprint (shown
// in Settings and under Add a device), and per device the fingerprints, the
// safety number and a QR code to compare. "Verified" is kept by the core
// with the key it was for: trusting a new key clears it.

import { createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { commands, type KeyVerification } from "../bindings";
import { unwrap } from "./core";

export type { KeyVerification };

const [ownFp, setOwnFp] = createSignal("");
let asked = false;
/** This computer's own short fingerprint ("" until read). */
export function ownFingerprint(): string {
  if (!asked) {
    asked = true;
    commands
      .ownFingerprint()
      .then(setOwnFp)
      .catch(() => (asked = false));
  }
  return ownFp();
}

const [verified, setVerifiedMap] = createStore<Record<string, boolean>>({});

/** What to compare with `node` (none until it has connected with a key). */
export async function loadVerification(node: string): Promise<KeyVerification | null> {
  const v = await commands.keyVerification(node);
  setVerifiedMap(node, !!v?.verified);
  return v;
}

/** Whether `node` is verified, as last read (`loadVerification`). */
export function isVerified(node: string): boolean {
  return !!verified[node];
}

/** Mark `node` verified (you compared and they match), or take it back. */
export async function setVerified(node: string, on: boolean): Promise<void> {
  try {
    await unwrap(commands.setDeviceVerified(node, on));
  } finally {
    await loadVerification(node).catch(() => {});
  }
}

/** The safety number in three lines of four groups, as both devices show it. */
export function safetyLines(n: string): string[] {
  const groups = n.split(" ").filter(Boolean);
  const lines: string[] = [];
  for (let i = 0; i < groups.length; i += 4) lines.push(groups.slice(i, i + 4).join(" "));
  return lines;
}
