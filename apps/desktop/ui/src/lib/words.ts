// Everything the user reads about connections and errors, in plain words.
// No protocol names (QUIC, relay, NAT) on screen: those stay on the
// Developer screen.

import type { Quality, TransportKind } from "../bindings";
import { t } from "../i18n";

export type LinkState = "direct" | "relay" | "connecting" | "offline" | "failed";

export function linkState(link: TransportKind | undefined, progress: string | undefined): LinkState {
  if (link === "relay") return "relay";
  if (link) return "direct";
  if (progress === "failed") return "failed";
  if (progress && progress !== "connected") return "connecting";
  return "offline";
}

export function linkLabel(s: LinkState): string {
  switch (s) {
    case "direct":
      return t("link.direct");
    case "relay":
      return t("link.relay");
    case "connecting":
      return t("link.connecting");
    case "failed":
      return t("link.failed");
    default:
      return t("link.offline");
  }
}

/** Link quality in plain words (the core's rating; no milliseconds here). */
export function qualityLabel(q: Quality | undefined): string | null {
  switch (q) {
    case "excellent":
      return t("quality.excellent");
    case "good":
      return t("quality.good");
    case "weak":
      return t("quality.weak");
    default:
      return null;
  }
}

/** One line under the status while the connect ladder runs. */
export function progressLabel(stage: string | undefined): string | null {
  switch (stage) {
    case "quic":
      return t("progress.quic");
    case "signaled_quic":
      return t("progress.signaledQuic");
    case "relay":
      return t("progress.relay");
    case "punching":
    case "webrtc":
      return t("progress.punching");
    case "failed":
      return t("progress.failed");
    default:
      return null;
  }
}

/** Turn a core error into something a person can act on. */
export function friendlyError(e: unknown): string {
  const m = (e instanceof Error ? e.message : String(e)).toLowerCase();
  if (m.includes("not connected yet")) return t("error.notConnectedYet");
  if (m.includes("connection closed")) return t("error.connectionClosed");
  if (m.includes("encryption key")) return t("error.noKey");
  if (m.includes("peer not found")) return t("error.peerNotFound");
  if (m.includes("not logged in")) return t("error.signedOut");
  if (m.includes("invalid credentials") || m.includes("401")) return t("error.badCredentials");
  if (m.includes("no clipboard response")) return t("error.noClipboard");
  if (m.includes("only http and https")) return t("error.badLink");
  if (m.includes("timed out") || m.includes("timeout")) return t("error.timeout");
  if (m.includes("dns") || m.includes("connection refused") || m.includes("network"))
    return t("error.network");
  return e instanceof Error ? e.message : String(e);
}

/** First letter of the device's name, for its avatar. */
export function initial(name: string): string {
  return (niceName(name).match(/[A-Za-z0-9]/)?.[0] ?? "?").toUpperCase();
}

/** "MacBook-Pro.local" -> "MacBook Pro"; keeps names readable. */
export function niceName(name: string): string {
  return name.replace(/\.local$/i, "").replace(/[-_]+/g, " ").trim() || name;
}

export function timeAgo(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 10) return t("time.justNow");
  if (s < 60) return t("time.secondsAgo", { count: s });
  const m = Math.round(s / 60);
  if (m < 60) return t("time.minutesAgo", { count: m });
  const h = Math.round(m / 60);
  if (h < 24) return t("time.hoursAgo", { count: h });
  return new Date(ts).toLocaleDateString();
}
