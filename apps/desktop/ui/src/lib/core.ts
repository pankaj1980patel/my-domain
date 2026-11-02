// The app's view of the core: one reactive store, kept current by core
// events, plus thin wrappers over the generated commands (bindings.ts).
//
// Screens read `state` and call the functions here; they never talk to Tauri
// directly.

import { createStore, produce } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands, type Identity_Serialize, type Peer_Serialize, type Quality, type TransportKind } from "../bindings";
import { t } from "../i18n";
import { startRing, stopRing } from "./ring";
import { niceName } from "./words";
import { toast } from "../components/ui";

export type Peer = Peer_Serialize;
export type Identity = Identity_Serialize;

export type ChatMessage = {
  id: number;
  node: string;
  dir: "in" | "out";
  text: string;
  ts: number;
  via?: string;
  /** Message id shared with the core; the delivery receipt carries it back. */
  mid?: string;
  /** Sent (or queued) but the device hasn't confirmed receipt yet. */
  pending?: boolean;
  /** Set when the receipt arrives. */
  delivered?: boolean;
};

export type ActivityItem = {
  id: number;
  ts: number;
  icon: string;
  text: string;
  node?: string;
  tone: "info" | "ok" | "warn" | "error";
};

export type AppEntry = { pkg: string; label: string };

/** What a device reported about itself (answer to requestStatus). */
export type DeviceStatus = {
  battery?: number;
  charging?: boolean;
  platform?: string;
  /** Actions it takes (flashlight, volume, mute, media, ringer) and their state. */
  actions: string[];
  torch?: boolean;
  volume?: number;
  muted?: boolean;
  ringer?: string;
  ts: number;
};

/** A file going to or coming from a device. `waiting`: offered, the device
 *  hasn't started taking it yet (or isn't reachable). */
export type TransferState = "waiting" | "sending" | "receiving" | "sent" | "saved" | "failed";
export type Transfer = {
  id: string;
  /** The other device. */
  node: string;
  name: string;
  done: number;
  total: number;
  incoming: boolean;
  state: TransferState;
  /** Why it failed (the core's words; see fileFailReason). */
  reason?: string;
  /** Where a received file was saved. */
  path?: string;
  ts: number;
};

/** Raw event log for the Developer screen. */
export type DevEvent = { ts: number; name: string; payload: unknown };

type State = {
  me: Identity | null;
  account: { username: string; server: string };
  peers: Peer[];
  /** node_id -> live link transport */
  links: Record<string, TransportKind>;
  /** node_id -> latest connect-ladder stage */
  progress: Record<string, string>;
  /** node_id -> measured link quality (live links only) */
  quality: Record<string, Quality>;
  messages: ChatMessage[];
  activity: ActivityItem[];
  apps: Record<string, { apps: AppEntry[]; subscribed: string[] }>;
  clipSync: boolean;
  /** node_id -> latest reported battery/charging */
  status: Record<string, DeviceStatus>;
  /** Set while another device is ringing this one (find my device). */
  ringing: { from: string } | null;
  /** transfer id -> file transfer (both ways) */
  transfers: Record<string, Transfer>;
  devLog: DevEvent[];
};

export const [state, setState] = createStore<State>({
  me: null,
  account: { username: "", server: "" },
  peers: [],
  links: {},
  progress: {},
  quality: {},
  messages: [],
  activity: [],
  apps: {},
  clipSync: false,
  status: {},
  ringing: null,
  transfers: {},
  devLog: [],
});

let seq = 0;
const now = () => Date.now();

/** Typed command results -> value, or a thrown Error with the core's message. */
export async function unwrap<T>(
  r: Promise<{ status: "ok"; data: T } | { status: "error"; error: string }>,
): Promise<T> {
  const x = await r;
  if (x.status === "error") throw new Error(x.error);
  return x.data;
}

export function peerName(node: string | undefined): string {
  if (!node) return t("activity.someDevice");
  return state.peers.find((p) => p.node_id === node)?.name ?? t("activity.someDevice");
}

export function addActivity(item: Omit<ActivityItem, "id" | "ts">) {
  setState(
    "activity",
    produce((list) => {
      list.unshift({ ...item, id: ++seq, ts: now() });
      if (list.length > 300) list.length = 300;
    }),
  );
}

function logDev(name: string, payload: unknown) {
  setState(
    "devLog",
    produce((list) => {
      list.unshift({ ts: now(), name, payload });
      if (list.length > 500) list.length = 500;
    }),
  );
}

// --- events ---------------------------------------------------------------

type IncomingMessage = { from: string; node_id?: string; ip: string; protocol: string; text: string; ts: number; ok: boolean; id?: string | null };

let listening = false;
let qualityTimer: ReturnType<typeof setInterval> | undefined;

/** Re-read every live link's quality (RTT + loss, rated by the core). */
async function pollQuality() {
  const stats = await commands.linkStats().catch(() => []);
  setState("quality", Object.fromEntries(stats.map((s) => [s.node_id, s.quality])));
}

/** Subscribe to the core's events once; the store updates itself after. */
export async function startEvents() {
  if (listening) return;
  listening = true;
  pollQuality();
  qualityTimer ??= setInterval(pollQuality, 5000);

  const on = <T>(name: string, f: (p: T) => void) =>
    listen<T>(name, (e) => {
      logDev(name, e.payload);
      f(e.payload);
    });

  await Promise.all([
    on<Peer[]>("peers-updated", (peers) => setState("peers", peers)),
    on<{ node_id: string; transport: TransportKind }>("peer-connected", ({ node_id, transport }) => {
      const was = state.links[node_id];
      setState("links", node_id, transport);
      setState("progress", node_id, "connected");
      if (!was) addActivity({ icon: "🔗", tone: "ok", node: node_id, text: t("activity.connected", { name: peerName(node_id) }) });
    }),
    // Matched on the id alone: a hand-entered device acks under its real id.
    on<{ node_id: string; id: string }>("message-delivered", ({ id }) =>
      setState("messages", (m) => m.mid === id, { pending: false, delivered: true }),
    ),
    on<string>("peer-disconnected", (node_id) => {
      setState("links", produce((m) => delete m[node_id]));
      addActivity({ icon: "⛓️‍💥", tone: "info", node: node_id, text: t("activity.disconnected", { name: peerName(node_id) }) });
    }),
    on<{ node_id: string; stage: string; detail?: string | null }>("connect-progress", ({ node_id, stage }) =>
      setState("progress", node_id, stage),
    ),
    on<IncomingMessage>("message-received", (m) => {
      if (!m.ok) {
        addActivity({
          icon: "🔒",
          tone: "error",
          node: m.node_id,
          text: t("activity.unreadable"),
        });
        return;
      }
      const node = m.node_id ?? state.peers.find((p) => p.ip === m.ip)?.node_id ?? "";
      setState(
        "messages",
        produce((list) => list.push({ id: ++seq, node, dir: "in", text: m.text, ts: now(), via: m.protocol })),
      );
      addActivity({ icon: "💬", tone: "info", node, text: t("activity.message", { name: peerName(node), text: m.text }) });
    }),
    on<{ from: string; title: string; body: string; app?: string | null }>("notification-event", (n) =>
      addActivity({ icon: "🔔", tone: "info", text: n.body ? t("activity.notificationWithBody", { title: n.title, body: n.body }) : t("activity.notification", { title: n.title }) }),
    ),
    on<{ from: string; caller: string; state: string }>("call-notification-event", (c) =>
      addActivity({ icon: "📞", tone: "info", text: t("activity.call", { caller: c.caller, state: c.state }) }),
    ),
    on<{ from: string; action: string }>("clipboard-event", (c) =>
      addActivity({ icon: "📋", tone: "info", text: c.action === "pushed" ? t("activity.clipboardReceived") : t("activity.clipboardSynced") }),
    ),
    on<{ from: string; apps: AppEntry[]; subscribed: string[] }>("apps-list-event", (a) => {
      const node = state.peers.find((p) => p.name === a.from || p.node_id === a.from)?.node_id ?? a.from;
      setState("apps", node, { apps: a.apps ?? [], subscribed: a.subscribed ?? [] });
    }),
    on<{ from: string; status: Record<string, unknown> }>("device-status-event", ({ from, status }) =>
      setState("status", from, {
        battery: typeof status?.battery === "number" ? Math.round(status.battery) : undefined,
        charging: typeof status?.charging === "boolean" ? status.charging : undefined,
        platform: typeof status?.platform === "string" ? status.platform : undefined,
        actions: Array.isArray(status?.actions) ? status.actions.filter((a): a is string => typeof a === "string") : [],
        torch: typeof status?.torch === "boolean" ? status.torch : undefined,
        volume: typeof status?.volume === "number" ? Math.round(status.volume) : undefined,
        muted: typeof status?.muted === "boolean" ? status.muted : undefined,
        ringer: typeof status?.ringer === "string" ? status.ringer : undefined,
        ts: now(),
      }),
    ),
    // The backend brings the window forward and shows the OS notification;
    // the sound and the banner are here.
    on<{ from: string }>("ring-event", ({ from }) => {
      setState("ringing", { from });
      startRing(() => setState("ringing", null));
      addActivity({ icon: "", tone: "warn", node: from, text: t("activity.ringing", { name: peerName(from) }) });
    }),
    // The backend opens it in the browser.
    on<{ from: string; url: string }>("open-url-event", ({ from, url }) =>
      addActivity({ icon: "", tone: "info", node: from, text: t("activity.openedLink", { name: peerName(from), url }) }),
    ),
    // Files. A transfer may be first seen by any of these (a fast progress
    // event can beat sendFile's reply), so each one creates it if missing.
    on<{ id: string; node_id: string; name: string; done: number; total: number; incoming: boolean }>(
      "file-progress",
      ({ id, node_id, name, done, total, incoming }) => {
        const cur = state.transfers[id];
        if (cur && (cur.state === "sent" || cur.state === "saved" || cur.state === "failed")) return;
        setState("transfers", id, {
          id,
          node: node_id,
          name,
          done,
          total,
          incoming,
          state: incoming ? "receiving" : "sending",
          ts: cur?.ts ?? now(),
        });
      },
    ),
    on<{ id: string; from: string; name: string; path: string; size: number }>("file-received", ({ id, from, name, path, size }) => {
      setState("transfers", id, {
        id,
        node: from,
        name,
        done: size,
        total: size,
        incoming: true,
        state: "saved",
        path,
        ts: state.transfers[id]?.ts ?? now(),
      });
      const device = niceName(peerName(from));
      addActivity({ icon: "", tone: "ok", node: from, text: t("activity.fileSaved", { name, device }) });
      toast(t("files.savedToast", { name, device }), "ok");
    }),
    on<{ id: string; node_id: string; name: string }>("file-sent", ({ id, node_id, name }) => {
      const cur = state.transfers[id];
      setState("transfers", id, {
        id,
        node: node_id,
        name,
        done: cur?.total ?? 0,
        total: cur?.total ?? 0,
        incoming: false,
        state: "sent",
        ts: cur?.ts ?? now(),
      });
    }),
    on<{ id: string; node_id: string; name: string; incoming: boolean; reason: string }>(
      "file-failed",
      ({ id, node_id, name, incoming, reason }) => {
        const cur = state.transfers[id];
        setState("transfers", id, {
          id,
          node: node_id,
          name,
          done: cur?.done ?? 0,
          total: cur?.total ?? 0,
          incoming,
          state: "failed",
          reason,
          ts: cur?.ts ?? now(),
        });
      },
    ),
    on<unknown>("fcm-event", () => {}),
  ]);
}

// --- actions ----------------------------------------------------------------

/** Load everything the home screen shows. */
export async function loadAll() {
  const [me, peers, info, clip] = await Promise.all([
    commands.getIdentity(),
    commands.getPeers(),
    commands.sessionInfo(),
    commands.clipboardSyncEnabled(),
  ]);
  setState({
    me,
    peers,
    account: { username: info.username ?? "", server: info.server_url ?? "" },
    clipSync: clip,
  });
}

export async function refreshDevices() {
  await unwrap(commands.refreshFromServer());
  setState("peers", await commands.getPeers());
}

export async function signIn(server: string, user: string, pass: string, key: string, create: boolean) {
  await unwrap(create ? commands.authRegister(server, user, pass) : commands.authLogin(server, user, pass));
  await unwrap(commands.setEncryptionKey(key));
  await startEvents();
  await refreshDevices().catch(() => {});
  await loadAll();
  // Let the other devices know how to reach us (background).
  commands.reportFirewall();
}

/** Started signed in (the session came back from the OS keychain)? Then
 *  load the app without the sign-in screen. */
export async function resume(): Promise<boolean> {
  if (!(await commands.isReady())) return false;
  await startEvents();
  await loadAll();
  refreshDevices().catch(() => {});
  return true;
}

export async function sendMessage(node: string, text: string): Promise<string> {
  // Pending until the device acks it ("message-delivered"); the core resends
  // until then. The bubble goes in first so a fast receipt can't beat it.
  const mid = crypto.randomUUID();
  const id = ++seq;
  setState("messages", produce((list) => list.push({ id, node, dir: "out", text, ts: now(), mid, pending: true })));
  try {
    const via = await unwrap(commands.sendMessage(node, text, mid));
    setState("messages", (m) => m.id === id, "via", via);
    return via;
  } catch (e) {
    setState("messages", (list) => list.filter((m) => m.id !== id));
    throw e;
  }
}

export async function connect(node: string) {
  setState("progress", node, "quic");
  await unwrap(commands.connect(node));
}

export async function setClipSync(on: boolean) {
  await (on ? commands.enableClipboardSync() : commands.disableClipboardSync());
  setState("clipSync", on);
}

export async function pullClipboard(node: string): Promise<string> {
  const text = await unwrap(commands.getClipboard(node));
  addActivity({ icon: "📋", tone: "ok", node, text: t("activity.clipboardCopied", { name: peerName(node) }) });
  return text;
}

export async function shareNotification(title: string, body: string) {
  await commands.shareNotification(title, body, "my-domain");
  addActivity({ icon: "🔔", tone: "ok", text: t("activity.notificationSent", { title }) });
}

export async function requestApps(node: string) {
  await unwrap(commands.requestApps(node));
}

export async function toggleApp(node: string, pkg: string) {
  const cur = state.apps[node]?.subscribed ?? [];
  const next = cur.includes(pkg) ? cur.filter((p) => p !== pkg) : [...cur, pkg];
  await unwrap(commands.subscribeApps(node, JSON.stringify(next)));
  setState("apps", node, "subscribed", next);
}

/** Ask a device for its battery; the answer lands in `state.status`. */
export async function requestStatus(node: string) {
  await unwrap(commands.requestStatus(node));
}

/** Returns the transport, or "queued" (rings once the device is reachable). */
/** Run an action the device listed in its status, then ask for its status
 *  again so the buttons show the new state. */
export async function runAction(node: string, name: string, args: Record<string, unknown> = {}): Promise<string> {
  const via = await unwrap(commands.runAction(node, name, JSON.stringify(args)));
  setTimeout(() => requestStatus(node).catch(() => {}), 400);
  return via;
}

export async function ringDevice(node: string): Promise<string> {
  return unwrap(commands.ringDevice(node));
}

/** Returns the transport, or "queued" (opens once the device is reachable). */
export async function openUrlOn(node: string, url: string): Promise<string> {
  const via = await unwrap(commands.openUrlOn(node, url));
  addActivity({ icon: "", tone: "ok", node, text: t("activity.sentLink", { name: peerName(node), url }) });
  return via;
}

/** Send a file (a path on this computer). It shows as `waiting` until the
 *  device starts taking it; returns the transfer id. */
export async function sendFile(node: string, path: string): Promise<string> {
  const id = await unwrap(commands.sendFile(node, path));
  if (!state.transfers[id]) {
    const name = path.split(/[\\/]/).pop() || path;
    setState("transfers", id, { id, node, name, done: 0, total: 0, incoming: false, state: "waiting", ts: now() });
  }
  return id;
}

/** Choose a file in the system dialog and send it; false if none was chosen. */
export async function pickAndSendFile(node: string): Promise<boolean> {
  const path = await commands.pickFile();
  if (!path) return false;
  await sendFile(node, path);
  return true;
}

/** Stop a transfer; the core answers with file-failed ("cancelled"). */
export async function cancelFile(id: string) {
  await commands.cancelFile(id);
}

/** Show a received file in Finder / Explorer. */
export async function revealFile(path: string) {
  await unwrap(commands.revealFile(path));
}

/** Open a received file with its default app. */
export async function openFile(path: string) {
  await unwrap(commands.openFile(path));
}

/** Silence this device's ring. */
export function dismissRing() {
  stopRing();
  setState("ringing", null);
}

export async function signOut() {
  await commands.logout();
  location.reload();
}
