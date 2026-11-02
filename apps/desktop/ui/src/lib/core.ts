// The app's view of the core: one reactive store, kept current by core
// events, plus thin wrappers over the generated commands (bindings.ts).
//
// Screens read `state` and call the functions here; they never talk to Tauri
// directly.

import { createStore, produce } from "solid-js/store";
import { listen } from "@tauri-apps/api/event";
import { commands, type Identity_Serialize, type Peer_Serialize, type TransportKind } from "../bindings";

export type Peer = Peer_Serialize;
export type Identity = Identity_Serialize;

export type ChatMessage = {
  id: number;
  node: string;
  dir: "in" | "out";
  text: string;
  ts: number;
  via?: string;
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
  messages: ChatMessage[];
  activity: ActivityItem[];
  apps: Record<string, { apps: AppEntry[]; subscribed: string[] }>;
  clipSync: boolean;
  devLog: DevEvent[];
};

export const [state, setState] = createStore<State>({
  me: null,
  account: { username: "", server: "" },
  peers: [],
  links: {},
  progress: {},
  messages: [],
  activity: [],
  apps: {},
  clipSync: false,
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
  if (!node) return "a device";
  return state.peers.find((p) => p.node_id === node)?.name ?? "a device";
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

type IncomingMessage = { from: string; node_id?: string; ip: string; protocol: string; text: string; ts: number; ok: boolean };

let listening = false;

/** Subscribe to the core's events once; the store updates itself after. */
export async function startEvents() {
  if (listening) return;
  listening = true;

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
      if (!was) addActivity({ icon: "🔗", tone: "ok", node: node_id, text: `Connected to ${peerName(node_id)}` });
    }),
    on<string>("peer-disconnected", (node_id) => {
      setState("links", produce((m) => delete m[node_id]));
      addActivity({ icon: "⛓️‍💥", tone: "info", node: node_id, text: `${peerName(node_id)} disconnected` });
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
          text: "A message couldn't be read: that device uses a different key",
        });
        return;
      }
      const node = m.node_id ?? state.peers.find((p) => p.ip === m.ip)?.node_id ?? "";
      setState(
        "messages",
        produce((list) => list.push({ id: ++seq, node, dir: "in", text: m.text, ts: now(), via: m.protocol })),
      );
      addActivity({ icon: "💬", tone: "info", node, text: `${peerName(node)}: ${m.text}` });
    }),
    on<{ from: string; title: string; body: string; app?: string | null }>("notification-event", (n) =>
      addActivity({ icon: "🔔", tone: "info", text: `${n.title}${n.body ? ` — ${n.body}` : ""}` }),
    ),
    on<{ from: string; caller: string; state: string }>("call-notification-event", (c) =>
      addActivity({ icon: "📞", tone: "info", text: `Call from ${c.caller} (${c.state})` }),
    ),
    on<{ from: string; action: string }>("clipboard-event", (c) =>
      addActivity({ icon: "📋", tone: "info", text: c.action === "pushed" ? "Clipboard received" : "Clipboard synced" }),
    ),
    on<{ from: string; apps: AppEntry[]; subscribed: string[] }>("apps-list-event", (a) => {
      const node = state.peers.find((p) => p.name === a.from || p.node_id === a.from)?.node_id ?? a.from;
      setState("apps", node, { apps: a.apps ?? [], subscribed: a.subscribed ?? [] });
    }),
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

export async function sendMessage(node: string, text: string): Promise<string> {
  const via = await unwrap(commands.sendMessage(node, text));
  setState("messages", produce((list) => list.push({ id: ++seq, node, dir: "out", text, ts: now(), via })));
  return via;
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
  addActivity({ icon: "📋", tone: "ok", node, text: `Copied ${peerName(node)}'s clipboard` });
  return text;
}

export async function shareNotification(title: string, body: string) {
  await commands.shareNotification(title, body, "my-domain");
  addActivity({ icon: "🔔", tone: "ok", text: `Sent “${title}” to your devices` });
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

export async function signOut() {
  await commands.logout();
  location.reload();
}
