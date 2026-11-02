// Everything the user reads about connections and errors, in plain words.
// No protocol names (QUIC, relay, NAT) on screen: those stay on the
// Developer screen.

import type { Quality, TransportKind } from "../bindings";

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
      return "Connected directly";
    case "relay":
      return "Connected through the internet";
    case "connecting":
      return "Connecting…";
    case "failed":
      return "Couldn't reach it";
    default:
      return "Not connected";
  }
}

/** Link quality in plain words (the core's rating; no milliseconds here). */
export function qualityLabel(q: Quality | undefined): string | null {
  switch (q) {
    case "excellent":
      return "Excellent connection";
    case "good":
      return "Good connection";
    case "weak":
      return "Weak connection";
    default:
      return null;
  }
}

/** One line under the status while the connect ladder runs. */
export function progressLabel(stage: string | undefined): string | null {
  switch (stage) {
    case "quic":
      return "Looking for it on your networks…";
    case "signaled_quic":
      return "Asking it to connect back…";
    case "relay":
      return "Connecting through the internet…";
    case "punching":
    case "webrtc":
      return "Finding a faster direct route…";
    case "failed":
      return "Make sure my-domain is open on it and it's online.";
    default:
      return null;
  }
}

/** Turn a core error into something a person can act on. */
export function friendlyError(e: unknown): string {
  const m = (e instanceof Error ? e.message : String(e)).toLowerCase();
  if (m.includes("not connected yet")) return "Not connected yet. Connecting now: try again in a moment.";
  if (m.includes("connection closed")) return "The connection dropped. Reconnecting: try again in a moment.";
  if (m.includes("encryption key")) return "Set your device key first (Settings).";
  if (m.includes("peer not found")) return "That device isn't in your list any more.";
  if (m.includes("not logged in")) return "You're signed out. Sign in again.";
  if (m.includes("invalid credentials") || m.includes("401")) return "Wrong username or password.";
  if (m.includes("no clipboard response")) return "That device didn't answer. Is my-domain open on it?";
  if (m.includes("timed out") || m.includes("timeout")) return "That took too long. Check your internet connection.";
  if (m.includes("dns") || m.includes("connection refused") || m.includes("network"))
    return "Can't reach the server. Check your internet connection.";
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
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return new Date(ts).toLocaleDateString();
}
