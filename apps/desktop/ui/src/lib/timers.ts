// Timers: countdown timers shared between your devices (the core's
// `features::timers`, `timers.rs`). The store keeps every timer shown, by
// id: loaded once, changed by what's done here and by `timers-changed`
// events (the whole list, after a change on another device). Each timer
// counts down here from its end time; nothing ticks over the network.
//
// A timer that ends and rings on this computer comes as `timer-ring` (the
// backend brought the window up and posted a notification): it's kept in
// `ringing` until it's stopped here or on any device, given more time, or
// cancelled.

import { createStore, reconcile } from "solid-js/store";
import { commands, type TimerView } from "../bindings";

export type Timer = TimerView;

export const [timers, setTimers] = createStore<{ byId: Record<string, Timer>; loaded: boolean }>({
  byId: {},
  loaded: false,
});

/** Timers ringing on this computer now (ids). */
export const [ringing, setRinging] = createStore<{ ids: string[] }>({ ids: [] });

const byId = (list: Timer[]) => Object.fromEntries(list.map((x) => [x.id, x]));

export async function loadTimers() {
  const list = await commands.timersList();
  setTimers("byId", reconcile(byId(list)));
  setTimers("loaded", true);
}

/** `timers-changed`: every timer shown, after a change on another device. */
export function applyTimersChanged(p: { timers?: Timer[] }) {
  setTimers("byId", reconcile(byId(p.timers ?? [])));
}

/** `timer-ring`: a timer ended and rings here. */
export function onTimerRing(x: Timer) {
  setTimers("byId", x.id, x);
  if (!ringing.ids.includes(x.id)) setRinging("ids", (ids) => [...ids, x.id]);
}

/** Stop ringing here (the timer itself stays until it's stopped). */
export function quietTimer(id: string) {
  setRinging("ids", (ids) => ids.filter((x) => x !== id));
}

const n = (v: number | null | undefined) => v ?? 0;

/** It ended (and hasn't been stopped): ringing where it rings. */
export function hasEnded(x: Timer, now: number): boolean {
  return x.state === "running" && n(x.ends_at) <= now;
}

/** What's left at `now`, in ms. */
export function remaining(x: Timer, now: number): number {
  if (x.state === "paused") return n(x.remaining_ms);
  return Math.max(0, n(x.ends_at) - now);
}

/** How far along it is, 0..1. */
export function progress(x: Timer, now: number): number {
  const total = n(x.duration_ms);
  if (total <= 0) return 1;
  return Math.min(1, Math.max(0, 1 - remaining(x, now) / total));
}

/** Running ones by when they end (ended first), then paused ones, oldest
 *  first (as the core lists them). */
export function sortedTimers(): Timer[] {
  const rank = (x: Timer) => (x.state === "running" ? 0 : 1);
  return Object.values(timers.byId).sort(
    (a, b) => rank(a) - rank(b) || n(a.ends_at) - n(b.ends_at) || n(a.created_at) - n(b.created_at) || a.id.localeCompare(b.id),
  );
}

/** "1:05:03", "4:32", "0:07": hours only when there are some; seconds
 *  rounded up, as a timer shows them. */
export function clock(ms: number): string {
  const total = Math.ceil(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor(total / 60) % 60;
  const s = total % 60;
  const pad = (v: number) => String(v).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** Hours, minutes and seconds typed in as ms (null: nothing, or over a
 *  week). */
export function durationOf(h: string, m: string, s: string): number | null {
  const num = (v: string) => {
    const x = Number(v.trim() || "0");
    return Number.isFinite(x) && x >= 0 ? x : NaN;
  };
  const ms = (num(h) * 3600 + num(m) * 60 + num(s)) * 1000;
  if (!Number.isFinite(ms) || ms < 1000 || ms > MAX_MS) return null;
  return Math.round(ms);
}

/** The longest timer the core takes (a week). */
export const MAX_MS = 7 * 24 * 3600 * 1000;

/** The quick ones, in minutes. */
export const PRESETS = [1, 3, 5, 10, 15, 30, 60];

// (Not core.ts's unwrap: core.ts loads this file.)
async function unwrap<T>(r: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>): Promise<T> {
  const x = await r;
  if (x.status === "error") throw new Error(x.error);
  return x.data;
}

const keep = (x: Timer) => {
  setTimers("byId", x.id, x);
  return x;
};

/** Start a timer: it shows on your other devices and rings on `ringOn`. */
export async function startTimer(label: string, ms: number, ringOn: string[]): Promise<Timer> {
  return keep(await unwrap(commands.timerStart(label, ms, ringOn)));
}

export type TimerAction = "pause" | "resume" | "cancel" | "dismiss" | "restart";

/** Pause, resume, cancel, stop (an ended one) or restart: on every device. */
export async function timerAction(id: string, action: TimerAction): Promise<void> {
  const x = await unwrap(commands.timerAction(id, action));
  if (action === "cancel" || action === "dismiss") {
    setTimers("byId", id, undefined!);
    quietTimer(id);
  } else {
    keep(x);
    if (action === "restart") quietTimer(id);
  }
}

/** Add time (an ended one starts again for that long). */
export async function addTime(id: string, ms: number): Promise<void> {
  keep(await unwrap(commands.timerAdd(id, ms)));
  quietTimer(id);
}

/** Ring on these devices when it ends. */
export async function setRing(id: string, ringOn: string[]): Promise<void> {
  keep(await unwrap(commands.timerSetRing(id, ringOn)));
}
