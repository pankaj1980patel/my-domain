// Calls on this computer (calls.rs): the phone's call state, kept current by
// the call-* events, and the computer's audio devices for the pickers (refreshed on
// `calls-audio-devices` when one comes or goes).

import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type AudioDevices_Serialize as AudioDevices,
  type AudioTarget_Serialize as AudioTarget,
  type CallsStatus_Serialize as CallsStatus,
} from "../bindings";
import { friendlyError } from "./words";
import { toast } from "../components/ui";

export type { AudioDevices, AudioTarget, CallsStatus };

const [status, setStatus] = createSignal<CallsStatus>();
const [devices, setDevices] = createSignal<AudioDevices>({
  outputs: [],
  inputs: [],
  default_output: null,
  default_input: null,
});

export { status as callStatus, devices as audioDevices };

export async function refreshCalls() {
  try {
    setStatus(await commands.callsStatus());
  } catch {
    // Not ready yet: the next event refreshes it.
  }
}

export async function refreshAudioDevices() {
  try {
    setDevices(await commands.callsAudioDevices());
  } catch {
    // Keep the last list.
  }
}

export function setCallStatus(s: CallsStatus) {
  setStatus(s);
}

let started = false;
/** Start following calls (once). */
export function watchCalls() {
  if (started) return;
  started = true;
  refreshCalls();
  for (const ev of ["call-incoming", "call-active", "call-ended", "call-state"]) {
    listen(ev, () => refreshCalls());
  }
  listen("calls-audio-devices", () => refreshAudioDevices());
}

/** Run a call command; a failure becomes an error toast. */
export async function callDo(r: Promise<{ status: "ok"; data: null } | { status: "error"; error: string }>) {
  const x = await r;
  if (x.status === "error") toast(friendlyError(new Error(x.error)), "error");
  refreshCalls();
}

/** mm:ss (or h:mm:ss) since `since` (unix ms). */
export function duration(since: number, now: number): string {
  const s = Math.max(0, Math.floor((now - since) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

export const KEYPAD = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"];
