// The app's view of the core: one reactive store, kept current by core
// events, plus thin wrappers over the generated commands (bindings.ts).
//
// Screens read `state` and call the functions here; they never talk to Tauri
// directly.

import { createSignal } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands, type ClipPage_Serialize, type DeviceLabel_Serialize, type HistoryPage_Serialize, type Identity_Serialize, type Peer_Serialize, type Quality, type TransportKind } from "../bindings";
import { t } from "../i18n";
import { startRing, stopRing } from "./ring";
import { niceName } from "./words";
import { toast } from "../components/ui";

export type Peer = Peer_Serialize;
/** What you call a device here: nickname, icon, colour (this computer only). */
export type DeviceLabel = DeviceLabel_Serialize;
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

/** A button on a phone notification; `reply` ones take text. */
export type NotifAction = { title: string; reply: boolean };

/** A notification a device shared from one of its subscribed apps. */
export type PhoneNotification = {
  id: number;
  /** The device it came from (node id). */
  from: string;
  app?: string;
  title: string;
  body: string;
  /** Its key on the device; without one it can't be acted on. */
  key?: string;
  actions: NotifAction[];
  ts: number;
  /** A button was pressed or a reply sent from here. */
  handled?: boolean;
};

/** What a device reported about itself (answer to requestStatus). */
/** A clipboard value in the history; `from` is the device (none: this one). */
export type ClipEntry = { text: string; from?: string; ts: number };

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
  /** Radios it reported, on or off: wifi, bluetooth, airplane, data, hotspot. */
  radios: Partial<Record<Radio, boolean>>;
  /** Radios it switches with no tap on it (Accessibility or Shizuku). */
  radioDirect: Radio[];
  /** Its Wi-Fi network now, if connected. */
  wifiSsid?: string;
  /** Nearby Wi-Fi networks from its last scan (none until asked). */
  wifiNetworks?: WifiNetwork[];
  /** What stops it scanning: "permission" or "location". */
  wifiScanNeeds?: string;
  /** It lets your computers see where it is (its own switch). */
  locationSharing: boolean;
  /** Its Accessibility helper is on (phone keys, no-tap radios). */
  helper?: boolean;
  /** Screen settings (ScreenSettings on Android): brightness 0-100, auto
   *  brightness, screen timeout in seconds, auto-rotate. */
  brightness?: number;
  autoBrightness?: boolean;
  screenTimeoutS?: number;
  autoRotate?: boolean;
  /** It may change those (Android's "Modify system settings"). */
  writeSettings?: boolean;
  /** Lost mode is on there (its message shows on its lock screen). */
  lost?: boolean;
  /** It's on Do Not Disturb (phones; absent: it didn't say). */
  dnd?: boolean;
  /** Where it last said it was (after a Locate). */
  location?: { lat: number; lon: number; acc: number; at: number };
  /** How many folders it lets you browse (absent: it can't be browsed). */
  sharedFolders?: number;
  /** Free and total bytes of its disk and memory. */
  storage?: Space;
  ram?: Space;
  /** Percent of its processor in use lately, 0-100 (computers only). */
  cpu?: number;
  /** Seconds since it started, when it answered (at `ts`). */
  uptimeS?: number;
  /** Commands its owner lets your devices run there (the `run` action). */
  commands?: { id: string; name: string }[];
  /** How the last command run there went. */
  lastRun?: LastRun;
  ts: number;
};

/** A command run on a computer: exit code (none if stopped), the end of its
 *  output, or why it was refused; `at` in unix seconds. */
export type LastRun = { id: string; name: string; code?: number; output: string; timedOut: boolean; error?: string; at: number };

function commandsOf(v: unknown): DeviceStatus["commands"] {
  if (!Array.isArray(v)) return undefined;
  return v
    .filter((c): c is { id: string; name: string } => !!c && typeof c.id === "string" && typeof c.name === "string")
    .map((c) => ({ id: c.id, name: c.name }));
}

function lastRunOf(v: unknown): LastRun | undefined {
  const r = v as Record<string, unknown> | undefined;
  if (!r || typeof r.id !== "string" || typeof r.at !== "number") return undefined;
  return {
    id: r.id,
    name: typeof r.name === "string" ? r.name : "",
    code: typeof r.code === "number" ? r.code : undefined,
    output: typeof r.output === "string" ? r.output : "",
    timedOut: r.timed_out === true,
    error: typeof r.error === "string" ? r.error : undefined,
    at: r.at,
  };
}

export type Space = { free: number; total: number };

function spaceOf(v: unknown): Space | undefined {
  const s = v as { free?: unknown; total?: unknown } | undefined;
  if (typeof s?.free !== "number" || typeof s?.total !== "number" || s.total <= 0) return undefined;
  return { free: Math.min(s.free, s.total), total: s.total };
}

export type WifiNetwork = { ssid: string; signal: number; secure: boolean; saved: boolean; current: boolean };

function wifiNetworksOf(v: unknown): WifiNetwork[] | undefined {
  if (!Array.isArray(v)) return undefined;
  return v
    .filter((n): n is Record<string, unknown> => !!n && typeof n === "object" && typeof n.ssid === "string")
    .map((n) => ({
      ssid: n.ssid as string,
      signal: typeof n.signal === "number" ? n.signal : 0,
      secure: n.secure === true,
      saved: n.saved === true,
      current: n.current === true,
    }));
}

function locationOf(v: unknown): DeviceStatus["location"] {
  const l = v as Record<string, unknown> | undefined;
  if (!l || typeof l.lat !== "number" || typeof l.lon !== "number") return undefined;
  return {
    lat: l.lat,
    lon: l.lon,
    acc: typeof l.acc === "number" ? l.acc : 0,
    at: typeof l.at === "number" ? l.at : 0,
  };
}

export const RADIOS = ["wifi", "bluetooth", "data", "hotspot", "location", "airplane"] as const;
export type Radio = (typeof RADIOS)[number];

/** The radios in a device's status (Wi-Fi is an object, mobile data is
 *  `mobile_data`). */
function radiosOf(status: Record<string, unknown>): Partial<Record<Radio, boolean>> {
  const wifi = status?.wifi as { on?: unknown } | undefined;
  const raw: Record<Radio, unknown> = {
    wifi: wifi?.on,
    bluetooth: status?.bluetooth,
    data: status?.mobile_data,
    hotspot: status?.hotspot,
    location: status?.location_on,
    airplane: status?.airplane,
  };
  const out: Partial<Record<Radio, boolean>> = {};
  for (const r of RADIOS) if (typeof raw[r] === "boolean") out[r] = raw[r] as boolean;
  return out;
}

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
  /** Measured speed in bytes a second (smoothed), while it moves. */
  rate?: number;
  /** When `done` last moved (ms), for `rate`. */
  movedAt?: number;
  /** It carried on from a partial file: the bytes it resumed at. */
  resumedFrom?: number;
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
  /** node_id -> recent notifications from it, newest first (memory only). */
  notifications: Record<string, PhoneNotification[]>;
  clipSync: boolean;
  /** Recent clipboard values from all your devices, newest first. */
  clipHistory: ClipEntry[];
  /** node_id -> latest reported battery/charging */
  status: Record<string, DeviceStatus>;
  /** Set while another device is ringing this one (find my device). */
  ringing: { from: string } | null;
  /** Your phones on Do Not Disturb that keep this computer's pop-ups held
   *  ("Stay quiet while my phone is on Do Not Disturb"); empty: none. */
  quietPhones: string[];
  /** transfer id -> file transfer (both ways) */
  transfers: Record<string, Transfer>;
  /** node_id -> the nickname, icon and colour you gave it here. */
  labels: Record<string, DeviceLabel>;
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
  notifications: {},
  clipSync: false,
  clipHistory: [],
  status: {},
  ringing: null,
  quietPhones: [],
  transfers: {},
  labels: {},
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

/** The name to show for a device: the nickname you gave it here, else its
 *  own name tidied (niceName). Every screen names devices through this. */
export function peerName(node: string | undefined): string {
  if (!node) return t("activity.someDevice");
  const nick = state.labels[node]?.nickname;
  if (nick) return nick;
  const own = state.peers.find((p) => p.node_id === node)?.name;
  return own ? niceName(own) : t("activity.someDevice");
}

/** The icon and colour you gave a device here, if any. */
export function peerLabel(node: string): DeviceLabel | undefined {
  return state.labels[node];
}

/** Give a device a nickname, an icon and a colour (blank: none). */
export async function setDeviceLabel(node: string, nickname: string, icon: string | null, color: string | null) {
  const labels = await unwrap(commands.setDeviceLabel(node, nickname.trim() || null, icon, color));
  setState("labels", reconcile(labels));
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

const MAX_NOTIFICATIONS = 30;

/** Keep a shared notification in its device's list (newest first, capped).
 *  The same key again (an update on the phone) replaces the older one. */
function addNotification(n: { from: string; title: string; body: string; app?: string | null; key?: string | null; actions?: unknown }) {
  const node = state.peers.find((p) => p.node_id === n.from || p.name === n.from)?.node_id ?? n.from;
  if (!node) return;
  const actions: NotifAction[] = Array.isArray(n.actions)
    ? n.actions
        .filter((a): a is { title: unknown; reply?: unknown } => !!a && typeof a === "object" && typeof (a as { title?: unknown }).title === "string")
        .map((a) => ({ title: a.title as string, reply: a.reply === true }))
    : [];
  const item: PhoneNotification = {
    id: ++seq,
    from: node,
    app: n.app ?? undefined,
    title: n.title ?? "",
    body: n.body ?? "",
    key: n.key ?? undefined,
    actions,
    ts: now(),
  };
  setState("notifications", node, (list = []) =>
    [item, ...list.filter((x) => !item.key || x.key !== item.key)].slice(0, MAX_NOTIFICATIONS),
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
    on<{ from: string; title: string; body: string; app?: string | null; key?: string | null; actions?: unknown }>(
      "notification-event",
      (n) => {
        addActivity({ icon: "🔔", tone: "info", text: n.body ? t("activity.notificationWithBody", { title: n.title, body: n.body }) : t("activity.notification", { title: n.title }) });
        addNotification(n);
        setHistoryTick((x) => x + 1);
      },
    ),
    // Gone on the phone (the backend already marked it in the history).
    on<{ from: string; key: string }>("notification-removed-event", ({ from, key }) => {
      const node = state.peers.find((p) => p.node_id === from || p.name === from)?.node_id ?? from;
      if (node && key && state.notifications[node]) {
        setState("notifications", node, (list = []) => list.filter((x) => x.key !== key));
      }
      setHistoryTick((x) => x + 1);
    }),
    on<{ from: string; caller: string; state: string }>("call-notification-event", (c) =>
      addActivity({ icon: "📞", tone: "info", text: t("activity.call", { caller: c.caller, state: c.state }) }),
    ),
    on<{ from: string; code: string; app?: string | null }>("code-copied", ({ from, app }) => {
      const device = peerName(from);
      addActivity({ icon: "", tone: "ok", node: from, text: t("activity.codeCopied", { device }) });
      toast(app ? t("code.copiedFrom", { app, device }) : t("code.copied", { device }), "ok");
    }),
    on<null>("clip-history-changed", () => setClipTick((x) => x + 1)),
    on<{ entries: ClipEntry[] }>("clipboard-history", ({ entries }) => setState("clipHistory", entries ?? [])),
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
        radios: radiosOf(status),
        wifiSsid: typeof (status?.wifi as { ssid?: unknown })?.ssid === "string" ? (status.wifi as { ssid: string }).ssid : undefined,
        wifiNetworks: wifiNetworksOf(status?.wifi_networks),
        wifiScanNeeds: typeof status?.wifi_scan_needs === "string" ? status.wifi_scan_needs : undefined,
        locationSharing: status?.location_sharing === true,
        helper: typeof status?.helper === "boolean" ? status.helper : undefined,
        location: locationOf(status?.location),
        lost: status?.lost === true,
        dnd: typeof status?.dnd === "boolean" ? status.dnd : undefined,
        brightness: typeof status?.brightness === "number" ? Math.round(status.brightness) : undefined,
        autoBrightness: typeof status?.auto_brightness === "boolean" ? status.auto_brightness : undefined,
        screenTimeoutS: typeof status?.screen_timeout_s === "number" ? Math.round(status.screen_timeout_s) : undefined,
        autoRotate: typeof status?.auto_rotate === "boolean" ? status.auto_rotate : undefined,
        writeSettings: typeof status?.write_settings === "boolean" ? status.write_settings : undefined,
        sharedFolders: typeof status?.shared_folders === "number" ? status.shared_folders : undefined,
        storage: spaceOf(status?.storage),
        ram: spaceOf(status?.ram),
        cpu: typeof status?.cpu === "number" ? Math.min(100, Math.max(0, Math.round(status.cpu))) : undefined,
        uptimeS: typeof status?.uptime_s === "number" ? Math.round(status.uptime_s) : undefined,
        commands: commandsOf(status?.commands),
        lastRun: lastRunOf(status?.last_run),
        radioDirect: Array.isArray(status?.radio_direct)
          ? RADIOS.filter((r) => (status.radio_direct as unknown[]).includes(r))
          : [],
        ts: now(),
      }),
    ),
    // Pop-ups held while a phone is on Do Not Disturb (the sidebar says so).
    on<string[]>("phone-quiet", (phones) => setState("quietPhones", Array.isArray(phones) ? phones : [])),
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
    on<{ id: string; node_id: string; name: string; done: number; total: number; incoming: boolean; resumed?: boolean }>(
      "file-progress",
      ({ id, node_id, name, done, total, incoming, resumed }) => {
        const cur = state.transfers[id];
        if (cur && (cur.state === "sent" || cur.state === "saved" || cur.state === "failed")) return;
        // Speed: smoothed over the moves seen (not counting a resumed start).
        const at = now();
        let rate = cur?.rate;
        if (cur?.movedAt && done > cur.done && at > cur.movedAt) {
          const inst = ((done - cur.done) * 1000) / (at - cur.movedAt);
          rate = rate === undefined ? inst : rate * 0.7 + inst * 0.3;
        }
        const movedAt = !cur || done !== cur.done ? at : (cur.movedAt ?? at);
        const resumedFrom = cur?.resumedFrom ?? (resumed ? done : undefined);
        setState("transfers", id, {
          rate,
          movedAt,
          resumedFrom,
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
      const device = peerName(from);
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
  const [me, peers, info, clip, labels] = await Promise.all([
    commands.getIdentity(),
    commands.getPeers(),
    commands.sessionInfo(),
    commands.clipboardSyncEnabled(),
    commands.getDeviceLabels(),
  ]);
  setState("labels", reconcile(labels));
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
  commands.phoneQuiet().then((p) => setState("quietPhones", p), () => {});
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

/** Close the link now; it comes back when either side sends something. */
export async function disconnect(node: string) {
  await unwrap(commands.disconnect(node));
}

/** Close the link and connect afresh (for a link that's up but stuck). */
export async function reconnect(node: string) {
  setState("progress", node, "quic");
  await unwrap(commands.reconnect(node));
}

/** Forget the chat with a device, on this computer only. */
export function clearChat(node: string) {
  setState("messages", (list) => list.filter((m) => m.node !== node));
}

export async function setClipSync(on: boolean) {
  await (on ? commands.enableClipboardSync() : commands.disableClipboardSync());
  setState("clipSync", on);
}

export async function loadClipHistory() {
  try {
    setState("clipHistory", JSON.parse(await commands.clipboardHistory()) as ClipEntry[]);
  } catch {
    setState("clipHistory", []);
  }
}

export async function clearClipHistory() {
  await commands.clearClipboardHistory();
}

/** Copy a history entry back onto this computer's clipboard. */
export async function copyText(text: string) {
  await unwrap(commands.copyToClipboard(text));
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

/** Press a shared notification's button (`index`), reply to it (`text`), or
 *  dismiss it (-1: also drops it from the list). Pressing or replying marks
 *  it handled. Returns the transport, or "queued". */
export async function notificationAction(n: PhoneNotification, index: number, text?: string): Promise<string> {
  const drop = () => setState("notifications", n.from, (list = []) => list.filter((x) => x.id !== n.id));
  if (!n.key) {
    // Nothing to tell the device; dismissing just forgets it here.
    if (index < 0) {
      drop();
      return "local";
    }
    throw new Error(t("notifications.cantAct"));
  }
  const via = await unwrap(commands.notificationAction(n.from, n.key, index, text ?? null));
  if (index < 0) {
    drop();
    setHistoryTick((x) => x + 1);
  } else setState("notifications", n.from, (x) => x.id === n.id, "handled", true);
  return via;
}

// --- notification history (kept by the backend, notif_history.rs) ---

export type HistoryPage = HistoryPage_Serialize;
export type HistoryEntry = HistoryPage["entries"][number];

/** Bumped when the history may have changed (a notification came, one was
 *  dismissed), so the history view reloads. */
const [historyTick, setHistoryTick] = createSignal(0);
export { historyTick };

/** A device's kept notifications: the first `limit` matching `query`
 *  (title, text or app), from `app` only when given. */
export function notificationHistory(node: string, query: string, app: string | null, limit: number): Promise<HistoryPage> {
  return commands.notificationHistory(node, query, app, limit);
}

export async function clearNotificationHistory(node: string) {
  await commands.clearNotificationHistory(node);
  setHistoryTick((x) => x + 1);
}

export function mutedApps(): Promise<string[]> {
  return commands.mutedApps();
}

/** Replace the muted list (returned tidied). A newly muted app's
 *  notifications also leave the lists here. */
export async function setMutedApps(apps: string[]): Promise<string[]> {
  const muted = await commands.setMutedApps(apps);
  const isMuted = (app?: string) => !!app && muted.some((m) => m.toLowerCase() === app.trim().toLowerCase());
  for (const node of Object.keys(state.notifications)) {
    setState("notifications", node, (list = []) => list.filter((n) => !isMuted(n.app)));
  }
  return muted;
}

// --- clipboard history (kept by the backend, clip_history.rs) ---

export type ClipPage = ClipPage_Serialize;
export type ClipRecord = ClipPage["entries"][number];

/** Bumped when the clipboard history changed (`clip-history-changed`), so
 *  the device's clipboard history tile reloads. */
const [clipTick, setClipTick] = createSignal(0);
export { clipTick };

/** A device's kept clips, sent and received: the first `limit` matching
 *  `query` (every word in the text). */
export function clipHistory(node: string, query: string, limit: number): Promise<ClipPage> {
  return commands.clipHistory(node, query, limit);
}

export async function clearClipHistoryOf(node: string) {
  await commands.clearClipHistory(node);
}

/** Send a kept clip to a device's clipboard again. Returns the transport,
 *  or "queued". */
export async function sendClipAgain(node: string, text: string): Promise<string> {
  return unwrap(commands.sendText(node, text, "clipboard"));
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

/** Run one of this computer's own actions (`localActions`) now. */
export async function runLocalAction(name: string, args: Record<string, unknown> = {}): Promise<string> {
  return unwrap(commands.runLocalAction(name, JSON.stringify(args)));
}

/** Send what's copied on this computer (text, else a picture) to a device's
 *  clipboard. Returns "sent", or "queued" (goes once it's reachable). */
export async function sendClipboardTo(node: string): Promise<string> {
  return unwrap(commands.sendClipboard(node));
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

/** Send a whole folder (a path on this computer); the device recreates it
 *  under its downloads. Each file inside shows up as its own transfer as
 *  it starts; returns how many files there are. */
export async function sendFolder(node: string, path: string): Promise<number> {
  return unwrap(commands.sendFolder(node, path));
}

/** Choose a file in the system dialog and send it; false if none was chosen. */
export async function pickAndSendFile(node: string): Promise<boolean> {
  const path = await commands.pickFile();
  if (!path) return false;
  await sendFile(node, path);
  return true;
}

/** Choose a folder in the system dialog and send everything in it; how many
 *  files went, 0 if none was chosen. */
export async function pickAndSendFolder(node: string): Promise<number> {
  const path = await commands.pickFolder();
  if (!path) return 0;
  return sendFolder(node, path);
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
/** The tray's "Open" on a device: call `cb` with its node id. Resolves to an
 *  unsubscribe function. */
export function onOpenDevice(cb: (node: string) => void): Promise<() => void> {
  return listen<{ node_id: string }>("open-device", (e) => cb(e.payload.node_id));
}

export function dismissRing() {
  stopRing();
  setState("ringing", null);
}

export async function signOut() {
  await commands.logout();
  location.reload();
}
