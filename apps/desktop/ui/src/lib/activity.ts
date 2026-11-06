// Activity: the timeline's rows in words (activity.rs builds and pages them).
// Each row is one sentence, a meta line (size, how it went) and where a tap
// takes you.

import type { ActivityItem_Serialize as ActivityItem } from "../bindings";
import { t, type Key } from "../i18n";
import { humanSize } from "./words";

export const ACTIVITY_KINDS = ["files", "clipboard", "messages", "calls", "notifications", "connections", "other"] as const;
export type ActivityKind = (typeof ACTIVITY_KINDS)[number];

export const ACTIVITY_KIND_LABEL: Record<ActivityKind, Key> = {
  files: "activity.kind.files",
  clipboard: "activity.kind.clipboard",
  messages: "activity.kind.messages",
  calls: "activity.kind.calls",
  notifications: "activity.kind.notifications",
  connections: "activity.kind.connections",
  other: "activity.kind.other",
};

const ICON: Record<string, string> = {
  files: "📄",
  clipboard: "📋",
  messages: "💬",
  calls: "📞",
  notifications: "🔔",
  connections: "🔗",
  other: "•",
};

export function activityIcon(a: ActivityItem): string {
  if (a.action === "note") return "📝";
  if (a.action === "timer") return "⏱";
  if (a.action === "read_later") return "🔖";
  if (a.action === "key_changed") return "⚠";
  if (a.action === "printed" || a.action === "print_failed") return "🖨";
  return ICON[a.kind] ?? "•";
}

/** A timer's length, from ms: "5 min", "1 h 30 min", "45 s". */
function length(ms: number): string {
  const s = Math.round(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const parts: string[] = [];
  if (h) parts.push(t("activity.hours", { n: h }));
  if (m) parts.push(t("activity.minutes", { n: m }));
  if (!h && !m) parts.push(t("activity.seconds", { n: s % 60 }));
  return parts.join(" ");
}

/** What the row says, with `device` the other device's name. */
export function activityText(a: ActivityItem, device: string): string {
  const name = a.subject;
  const on = a.detail === "on";
  switch (a.action) {
    case "file_sent":
      return t("activity.row.fileSent", { name, device });
    case "file_received":
      return t("activity.row.fileReceived", { name, device });
    case "file_send_failed":
      return t("activity.row.fileSendFailed", { name, device });
    case "file_receive_failed":
      return t("activity.row.fileReceiveFailed", { name, device });
    case "file_sending":
      return t("activity.row.fileSending", { name, device });
    case "file_receiving":
      return t("activity.row.fileReceiving", { name, device });
    case "clip_in":
      return t("activity.row.clipIn", { device, text: a.subject });
    case "clip_out":
      return t("activity.row.clipOut", { device, text: a.subject });
    case "msg_in":
      return t("activity.row.msgIn", { device, text: a.subject });
    case "msg_out":
      return t("activity.row.msgOut", { device, text: a.subject });
    case "msg_failed":
      return t("activity.row.msgFailed", { device, text: a.subject });
    case "notification":
      return a.subject || a.detail;
    case "call": {
      const caller = a.subject || t("activity.unknownCaller");
      if (a.detail === "ringing") return t("activity.row.callRinging", { caller });
      if (a.detail === "missed") return t("activity.row.callMissed", { caller });
      if (a.detail === "ended") return t("activity.row.callEnded", { caller });
      return t("activity.call", { caller, state: a.detail });
    }
    case "link_up":
      return t("activity.connected", { name: device });
    case "link_down":
      return t("activity.disconnected", { name: device });
    case "key_changed":
      return t("activity.row.keyChanged", { device: device || a.subject });
    case "wake":
      return t("activity.row.wake", { device });
    case "note":
      return t("activity.row.note", { title: a.subject || t("activity.untitled") });
    case "read_later":
      return t("activity.row.readLater", { title: a.subject });
    case "timer":
      return a.subject
        ? t("activity.row.timer", { title: a.subject, length: length(Number(a.detail) || 0) })
        : t("activity.row.timerNoLabel", { length: length(Number(a.detail) || 0) });
    case "ring":
      return t("activity.ringing", { name: device });
    case "open_url":
      return t("activity.openedLink", { name: device, url: a.subject });
    case "screen_watch":
      return t(on ? "activity.row.screenWatchOn" : "activity.row.screenWatchOff", { device });
    case "screen_offer":
      return t(on ? "activity.row.screenOfferOn" : "activity.row.screenOfferOff", { device });
    case "audio_offer":
      return t(on ? "activity.row.audioOfferOn" : "activity.row.audioOfferOff", { device });
    case "audio_listen":
      return t(on ? "activity.row.audioListenOn" : "activity.row.audioListenOff", { device });
    case "printed":
      return t("activity.row.printed", { name, device, printer: a.detail });
    case "print_failed":
      return t("activity.row.printFailed", { name, device });
    case "schedule": {
      const what = a.subject;
      if (a.detail === "queued") return t("activity.row.scheduleQueued", { what, device });
      if (a.detail === "woken") return t("activity.row.scheduleWoken", { what, device });
      if (a.detail === "failed") return t("activity.row.scheduleFailed", { what, device });
      return t("activity.row.scheduleDone", { what, device });
    }
    default:
      return a.subject || a.action;
  }
}

/** The bits after the sentence: size, how it went, the app. */
export function activityMeta(a: ActivityItem): string[] {
  const out: string[] = [];
  if (a.bytes) out.push(humanSize(a.bytes));
  if (a.via === "wifi_direct") out.push(t("transfers.viaWifiDirect"));
  else if (a.via === "hotspot") out.push(t("transfers.viaHotspot"));
  if (a.action === "link_up" && a.detail === "relay") out.push(t("activity.viaRelay"));
  if (a.action === "link_up" && a.detail === "ble") out.push(t("activity.viaBluetooth"));
  if (a.action === "notification" && a.subject && a.detail) out.push(a.detail);
  if (a.action === "read_later" && a.detail) out.push(a.detail);
  if (a.action === "print_failed" && a.detail) out.push(a.detail);
  return out;
}

/** Whether a row failed (shown in the danger colour). */
export function activityFailed(a: ActivityItem): boolean {
  return a.action.endsWith("_failed") || (a.action === "schedule" && a.detail === "failed") || a.action === "key_changed";
}

/** The day a row falls on, as a heading: Today, Yesterday, or the date. */
export function dayLabel(ts: number, now = Date.now()): string {
  const d = new Date(ts);
  const today = new Date(now);
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  if (ts >= start) return t("activity.today");
  if (ts >= start - 24 * 3600 * 1000) return t("activity.yesterday");
  const sameYear = d.getFullYear() === today.getFullYear();
  return d.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long", year: sameYear ? undefined : "numeric" });
}

/** Rows grouped by day, in order (each group's rows newest first). */
export function byDay(items: ActivityItem[], now = Date.now()): { day: string; items: ActivityItem[] }[] {
  const out: { day: string; items: ActivityItem[] }[] = [];
  for (const a of items) {
    const day = dayLabel(a.ts ?? 0, now);
    const last = out[out.length - 1];
    if (last && last.day === day) last.items.push(a);
    else out.push({ day, items: [a] });
  }
  return out;
}

/** Where a row opens: one of Search's kinds (App's openFound), Timers, or nowhere. */
export type ActivityTarget =
  | { kind: "transfers" | "clipboard" | "messages" | "notifications" | "notes" | "read_later" | "devices"; id: string; node?: string }
  | { kind: "timers" }
  | null;

export function activityTarget(a: ActivityItem): ActivityTarget {
  const node = a.node ?? undefined;
  switch (a.kind) {
    case "files":
      return { kind: "transfers", id: a.target, node };
    case "clipboard":
      return node ? { kind: "clipboard", id: a.target, node } : null;
    case "messages":
      return node ? { kind: "messages", id: a.target, node } : null;
    case "notifications":
      return node ? { kind: "notifications", id: a.target, node } : null;
  }
  if (a.action === "note") return { kind: "notes", id: a.target };
  if (a.action === "read_later") return { kind: "read_later", id: a.target };
  if (a.action === "timer") return { kind: "timers" };
  return node ? { kind: "devices", id: node } : null;
}

/** A row's time of day ("14:05"). */
export function clock(ts: number): string {
  return new Date(ts).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
