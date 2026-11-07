// Everything the user reads about connections and errors, in plain words.
// No protocol names (QUIC, relay, NAT) on screen: those stay on the
// Developer screen.

import type { Quality, TransportKind } from "../bindings";
import { t, language } from "../i18n";

/** `waking`: a push went to wake it; `asleep`: it didn't answer one, or
 *  closed its link to save battery (it wakes when something needs it). */
export type LinkState = "direct" | "relay" | "connecting" | "waking" | "asleep" | "offline" | "failed";

export function linkState(link: TransportKind | undefined, progress: string | undefined): LinkState {
  if (link === "relay") return "relay";
  if (link) return "direct";
  if (progress === "failed") return "failed";
  if (progress === "waking") return "waking";
  if (progress === "asleep") return "asleep";
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
    case "waking":
      return t("link.waking");
    case "asleep":
      return t("link.asleep");
    case "failed":
      return t("link.failed");
    default:
      return t("link.offline");
  }
}

/** Is it reachable now (directly or through the relay)? */
export function isLive(s: LinkState): boolean {
  return s === "direct" || s === "relay";
}

/** Presence, one word for every screen: Online (green), Asleep, Offline
 *  (grey; red only when the last try failed). How it's linked is the route
 *  (routeOf), shown beside it. */
export function presenceLabel(s: LinkState): string {
  if (isLive(s)) return t("dash.online");
  if (s === "asleep") return t("link.asleep");
  if (s === "connecting") return t("link.connecting");
  if (s === "waking") return t("link.waking");
  return t("dash.offline");
}
export function presenceTone(s: LinkState): "green" | "red" | "muted" {
  return isLive(s) ? "green" : s === "failed" ? "red" : "muted";
}

export type Route = { icon: "wifi" | "globe" | "server"; label: string; relay: boolean };
/** The route a live link takes: Local network (wifi), Direct (globe) or
 *  Relay (server; the only one tinted amber). `how` is the map's word for
 *  the link ("lan", "punched"...), when known. Null when not live. */
export function routeOf(s: LinkState, how?: string): Route | null {
  if (s === "relay") return { icon: "server", label: t("route.relay"), relay: true };
  if (s !== "direct") return null;
  return how === "lan" ? { icon: "wifi", label: t("route.lan"), relay: false } : { icon: "globe", label: t("route.direct"), relay: false };
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
    case "waking":
      return t("progress.waking");
    case "asleep":
      return t("progress.asleep");
    default:
      return null;
  }
}

/** Turn a core error into something a person can act on. */
export function friendlyError(e: unknown): string {
  const m = (e instanceof Error ? e.message : String(e)).toLowerCase();
  // Adding a device (mdcore join / registry pair_*).
  if (m.includes("update the server to use qr join")) return t("error.updateServer");
  if (m.includes("expired or was already used")) return t("error.joinCodeUsed");
  if (m.includes("too many tries")) return t("error.tooManyTries");
  if (m.includes("join code is damaged") || m.includes("isn't a my-domain join code")) return t("error.badJoinLink");
  if (m.includes("not connected yet")) return t("error.notConnectedYet");
  if (m.includes("connection closed")) return t("error.connectionClosed");
  if (m.includes("encryption key")) return t("error.noKey");
  if (m.includes("peer not found")) return t("error.peerNotFound");
  if (m.includes("not logged in")) return t("error.signedOut");
  if (m.includes("incorrect password") || m === "unauthorized") return t("error.wrongPassword");
  if (m.includes("update the server to list sessions")) return t("account.sessionsOldServer");
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

/** When, one way everywhere: under a day "2 h ago", then "4 Nov" this
 *  year, else "4 Nov 2025" (in the app's language). */
export function timeAgo(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 10) return t("time.justNow");
  if (s < 60) return t("time.secondsAgo", { count: s });
  const m = Math.round(s / 60);
  if (m < 60) return t("time.minutesAgo", { count: m });
  const h = Math.round(m / 60);
  if (h < 24) return t("time.hoursAgo", { count: h });
  return shortDate(ts);
}

/** "4 Nov" this year, "4 Nov 2025" otherwise. */
export function shortDate(ts: number): string {
  const d = new Date(ts);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(language(), { day: "numeric", month: "short", year: sameYear ? undefined : "numeric" }).format(d);
}

/** A clock time: "3:08 PM" (no leading zero), or "15:08" where that's the way. */
export function clockTime(ts: number): string {
  return new Intl.DateTimeFormat(language(), { hour: "numeric", minute: "2-digit" }).format(new Date(ts));
}

/** A date and a time: "4 Nov, 3:08 PM". */
export function dateAndTime(ts: number): string {
  return `${shortDate(ts)}, ${clockTime(ts)}`;
}

/** 1536 -> "1.5 KB": whole bytes, one decimal below 100 of a larger unit. */
export function humanSize(bytes: number): string {
  if (bytes < 1024) return t("size.b", { n: bytes });
  const units = ["size.kb", "size.mb", "size.gb", "size.tb"] as const;
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return t(units[i], { n: v >= 100 ? Math.round(v) : Math.round(v * 10) / 10 });
}

/** The core's reason a transfer stopped, in the user's words. */
export function fileFailReason(reason: string | undefined, incoming: boolean): string {
  const r = (reason ?? "").toLowerCase();
  if (r === "cancelled") return t("files.reason.cancelled");
  if (r.includes("cancelled on the other device")) return t("files.reason.cancelledThere");
  if (r.includes("damaged")) return t("files.reason.damaged");
  if (r.includes("can't receive files")) return incoming ? t("files.reason.noFolderHere") : t("files.reason.cantReceive");
  if (r.includes("downloads folder") || r.includes("couldn't save"))
    return incoming ? t("files.reason.saveFailedHere") : t("files.reason.saveFailedThere");
  if (r.includes("can't read")) return t("files.reason.cantRead");
  if (r.includes("wasn't reachable")) return t("files.reason.unreachable");
  return reason || t("files.reason.unknown");
}
