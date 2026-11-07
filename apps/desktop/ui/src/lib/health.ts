// The health check (Settings → Health, the sidebar badge, a device's page)
// and first-run setup. The core rates each line (features::health); this
// keeps the last report, runs it again now and then, and puts each line
// into words.

import { createSignal } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type HealthCheck_Serialize,
  type HealthReport_Serialize,
  type Onboarding_Serialize,
  type Step,
} from "../bindings";
import { peerName, unwrap } from "./core";
import { humanSize } from "./words";
import { t, type Key } from "../i18n";
import { en } from "../i18n/en";

export type HealthReport = HealthReport_Serialize;
export type HealthCheck = HealthCheck_Serialize;
export type Onboarding = Onboarding_Serialize;
export type { Step };

/** The feature groups setup offers, in order (features::onboarding::GROUPS). */
export const GROUPS = ["clipboard", "notifications", "files", "media", "find"] as const;
/** The steps, in order. */
export const STEPS: Step[] = ["name", "devices", "features", "permissions", "connection"];

const [report, setReport] = createSignal<HealthReport | null>(null);
const [running, setRunning] = createSignal(false);
const [ranAt, setRanAt] = createSignal(0);
export { report, running, ranAt };

/** Run the check. `server` asks the server too (a few seconds). */
export async function runHealth(server = true): Promise<HealthReport | null> {
  setRunning(true);
  try {
    const r = await unwrap(commands.healthCheck(server));
    // A quick run (no server) keeps the last server line.
    const prev = report();
    if (!server && prev) {
      const s = prev.checks.find((c) => c.id === "server");
      if (s && !r.checks.some((c) => c.id === "server")) r.checks = [s, ...r.checks];
      r.bad = r.checks.filter((c) => c.level === "bad").length;
      r.warn = r.checks.filter((c) => c.level === "warn").length;
    }
    setReport(r);
    setRanAt(Date.now());
    return r;
  } catch {
    return null;
  } finally {
    setRunning(false);
  }
}

/** Red lines now (the sidebar and Home badge). */
export const badCount = () => report()?.bad ?? 0;

/** The amber and red lines about one device (its page's banner). */
export function deviceProblems(node: string): HealthCheck[] {
  return report()?.checks.filter((c) => c.node_id === node && (c.level === "bad" || c.level === "warn")) ?? [];
}

/** Keep the report fresh while the app runs: at once, quickly when links
 *  change, and with the server every 10 minutes. */
export function watchHealth(): () => void {
  let soon: ReturnType<typeof setTimeout> | undefined;
  const quick = () => {
    if (soon) return;
    soon = setTimeout(() => {
      soon = undefined;
      runHealth(false);
    }, 2000);
  };
  runHealth(true);
  const timer = setInterval(() => runHealth(true), 10 * 60 * 1000);
  const offs = ["peer-connected", "peer-disconnected", "peers-updated", "key-changed", "links-map-changed"].map((n) =>
    listen(n, quick),
  );
  // Back from System Settings or a prompt: permissions may have changed.
  window.addEventListener("focus", quick);
  return () => {
    window.removeEventListener("focus", quick);
    clearInterval(timer);
    if (soon) clearTimeout(soon);
    for (const off of offs) off.then((f) => f(), () => {});
  };
}

// --- words ---------------------------------------------------------------------

const LINK_REASON: Record<string, Key> = {
  push_failed: "conn.reason.pushFailed",
  no_answer: "conn.reason.noAnswer",
  relay_no_answer: "conn.reason.relayNoAnswer",
  relay_unreachable: "conn.reason.relayUnreachable",
  no_path: "conn.reason.noPath",
  dropped: "conn.reason.dropped",
  key_changed: "conn.reason.keyChanged",
};

const HOW: Record<string, Key> = {
  lan: "map.how.lan",
  direct: "map.how.direct",
  punched: "map.how.punched",
  webrtc: "map.how.webrtc",
  wifi_direct: "map.how.wifi_direct",
  hotspot: "map.how.hotspot",
  relay: "map.how.relay",
  ble: "map.how.ble",
};

/** How a link is carried, in words. */
export function howLabel(how: string): string {
  return HOW[how] ? t(HOW[how]) : t("map.how.none");
}

function has(key: string): key is Key {
  return key in en;
}

/** A feature flag or group in words. */
export function featureLabel(f: string): string {
  const k = `health.feature.${f}`;
  if (has(k)) return t(k);
  const g = `setup.group.${f}`;
  return has(g) ? t(g) : f;
}

/** A permission's name. */
export function permissionLabel(id: string): string {
  const k = `health.perm.${id}`;
  return has(k) ? t(k) : id;
}

/** "1.2 s" / "350 ms". */
function ms(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)} s` : `${n} ms`;
}

/** The line's title. */
export function checkTitle(c: HealthCheck): string {
  const name = c.node_id ? peerName(c.node_id) : "";
  switch (c.id) {
    case "server":
      return t(c.reason === "ok" ? "health.server.ok" : "health.server.unreachable");
    case "push":
      return t(`health.push.${c.reason}` as Key);
    case "disk":
      return t(`health.disk.${c.reason}` as Key, { free: humanSize(Number(c.detail ?? 0) * 1024 * 1024) });
    case "permission":
      return permissionLabel(c.detail ?? "");
    case "key":
      return t("health.key.changed", { name });
    case "link":
      if (c.reason === "direct") return t("health.link.direct", { name, how: howLabel(c.detail ?? "") });
      return t(`health.link.${c.reason}` as Key, { name });
    case "version":
      if (c.reason === "older" && !c.detail) return t("health.version.olderUnknown", { name });
      return t(`health.version.${c.reason}` as Key, { name, version: c.detail ?? "" });
    case "allowed":
      return t("health.allowed", { name });
    default:
      return c.id;
  }
}

/** What to know, under the title (empty: nothing more). */
export function checkDetail(c: HealthCheck): string {
  const name = c.node_id ? peerName(c.node_id) : "";
  switch (c.id) {
    case "server":
      return c.reason === "ok" ? "" : t("health.server.unreachableHint");
    case "push":
      return c.reason === "working" ? "" : t(`health.push.${c.reason}Hint` as Key);
    case "disk":
      return c.reason === "ok" ? "" : t("health.disk.hint");
    case "permission": {
      const why = c.features.length ? t("health.perm.for", { features: c.features.map(featureLabel).join(", ") }) : "";
      const own = `health.perm.${c.detail}.${c.reason}`;
      const state = has(own) ? t(own) : t(`health.perm.${c.reason}` as Key);
      return why ? `${state} · ${why}` : state;
    }
    case "key":
      return t("health.key.hint");
    case "link": {
      const parts: string[] = [];
      if (c.reason === "bad_key") parts.push(t("health.link.badKeyHint"));
      if (c.detail && c.reason !== "direct") {
        // "internet:symmetric_nat" (a relayed link) or a connection-check reason.
        const [path, why] = c.detail.includes(":") ? c.detail.split(":", 2) : ["", c.detail];
        const k = path ? `map.why.${path}.${why}` : "";
        const words = k && has(k) ? t(k as Key, { name }) : LINK_REASON[why] ? t(LINK_REASON[why], { name }) : "";
        if (words) parts.push(t("health.link.why", { why: words }));
      }
      if (c.reconnect_ms != null) parts.push(t("health.link.reconnect", { time: ms(c.reconnect_ms) }));
      return parts.join(" · ");
    }
    case "version":
      if (c.reason === "older" && c.features.length)
        return t("health.version.olderHint", { features: c.features.map(featureLabel).join(", ") });
      if (c.reason === "newer") return t("health.version.newerHint");
      return "";
    case "allowed":
      return c.features.map((f) => (has(`perm.name.${f}`) ? t(`perm.name.${f}` as Key) : f)).join(", ");
    default:
      return "";
  }
}

/** The fix button's words (empty: no button). */
export function fixLabel(c: HealthCheck): string {
  // Local Network has no prompt to ask for, only an action that brings it up.
  if (c.fix === "allow_permission" && c.detail === "local_network") return t("health.fix.ask_now");
  return c.fix ? t(`health.fix.${c.fix}` as Key) : "";
}

/** The level in words (screen readers; the dot shows it by colour). */
export function levelLabel(c: HealthCheck): string {
  return t(`health.level.${c.level}` as Key);
}

// --- setup ---------------------------------------------------------------------

const [setup, setSetup] = createSignal<Onboarding | null>(null);
export { setup };

export async function loadSetup(): Promise<Onboarding> {
  const s = await commands.onboardingState();
  setSetup(s);
  return s;
}

export async function setupAct(action: string, step: Step | null = null): Promise<Onboarding> {
  const s = await unwrap(commands.onboardingAct(action, step));
  setSetup(s);
  return s;
}

export async function setupFeatures(features: string[]): Promise<Onboarding> {
  const s = await commands.setOnboardingFeatures(features);
  setSetup(s);
  // Which permissions count changed.
  runHealth(false);
  return s;
}

/** The step's number, 1-5. */
export function stepNumber(s: { step: Step }): number {
  const i = STEPS.indexOf(s.step);
  return i < 0 ? STEPS.length : i + 1;
}
