// "Check connection": the core's report on one device's link, and its
// speed test. Reports are kept per device (the Developer screen shows the
// last one as JSON); the speed test's progress arrives as `speed-test`
// events (core.ts hands them to `onSpeedEvent`).

import { createStore, produce } from "solid-js/store";
import { commands, type Diagnosis_Serialize, type RttStats_Serialize, type TransportKind } from "../bindings";

export type Diagnosis = Diagnosis_Serialize;

/** What a speed test measured (the core's `SpeedResult`). */
export type SpeedResult = {
  bytes: number;
  upload_mbps: number;
  download_mbps: number;
  upload_ms: number;
  download_ms: number;
  rtt: RttStats_Serialize;
  transport: TransportKind;
};

async function unwrap<T>(r: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>): Promise<T> {
  const x = await r;
  if (x.status === "error") throw new Error(x.error);
  return x.data;
}

/** One running or finished speed test, as its events said. */
export type SpeedRun = {
  id: string;
  phase: "ping" | "upload" | "download" | "done" | "failed" | "cancelled";
  done: number;
  total: number;
  result?: SpeedResult | null;
  error?: string | null;
};

const [diag, setDiag] = createStore<{
  reports: Record<string, Diagnosis>;
  /** When each report was made (ms since the epoch). */
  at: Record<string, number>;
  speed: Record<string, SpeedRun>;
}>({ reports: {}, at: {}, speed: {} });

export { diag };

/** Ask the core for `node`'s report (takes a few seconds) and keep it. */
export async function diagnose(node: string): Promise<Diagnosis> {
  const r = await unwrap(commands.diagnose(node));
  setDiag("reports", node, r);
  setDiag("at", node, Date.now());
  return r;
}

/** Bytes each way: less over the relay or Bluetooth (slow, and the relay
 *  carries everyone's traffic). */
export function speedBytes(transport: string | undefined): number {
  return transport === "relay" || transport === "ble" ? 2 * 1024 * 1024 : 8 * 1024 * 1024;
}

export async function startSpeedTest(node: string, bytes: number): Promise<void> {
  const id = await unwrap(commands.speedTest(node, bytes));
  // Its first events may have come already.
  if (diag.speed[node]?.id !== id) setDiag("speed", node, { id, phase: "ping", done: 0, total: bytes });
}

export async function cancelSpeedTest(node: string): Promise<void> {
  const run = diag.speed[node];
  if (run) await commands.cancelSpeedTest(run.id);
}

export function speedRunning(node: string): boolean {
  const p = diag.speed[node]?.phase;
  return p === "ping" || p === "upload" || p === "download";
}

/** A `speed-test` event. A test still running for the device wins over
 *  any other. */
export function onSpeedEvent(e: SpeedRun & { node_id: string }) {
  const cur = diag.speed[e.node_id];
  if (cur && cur.id !== e.id && speedRunning(e.node_id)) return;
  setDiag(
    "speed",
    produce((m) => {
      m[e.node_id] = {
        id: e.id,
        phase: e.phase,
        done: e.done,
        total: e.total,
        result: e.result ?? null,
        error: e.error ?? null,
      };
    }),
  );
}
