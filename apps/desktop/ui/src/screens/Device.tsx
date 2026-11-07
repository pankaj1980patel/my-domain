// One device: its status, then a tile per feature.

import DeviceArt, { frameOf } from "../components/DeviceArt";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import {
  cancelFile,
  chatKeep,
  chatPage,
  chatTick,
  clearChat,
  clearClipHistory,
  connect,
  disconnect,
  copyText,
  loadClipHistory,
  mutedApps,
  notificationAction,
  notifTick,
  phoneNotifications,
  phoneNotifPrefs,
  setMutedApps,
  setPhoneNotifPrefs,
  openFile,
  openUrlOn,
  peerModel,
  peerIcon,
  peerName,
  pickAndSendFile,
  pickAndSendFolder,
  pullClipboard,
  reconnect,
  requestApps,
  requestStatus,
  revealFile,
  ringDevice,
  runAction,
  sendMessage,
  setClipSync,
  shareNotification,
  setState,
  state,
  toggleApp,
  RADIOS,
  type ChatMessage,
  type DeviceStatus,
  type NotifGroup,
  type NotifPrefs,
  type Radio,
  type WifiNetwork,
  type Transfer,
  unwrap,
} from "../lib/core";
import { fileFailReason, friendlyError, humanSize, presenceLabel, presenceTone, progressLabel, routeOf, timeAgo } from "../lib/words";
import { Gate, linkStateOf, RouteChip, toast } from "../components/ui";
import { DeviceLabelForm } from "../components/DeviceLabel";
import KeyChangeBanner from "../components/KeyChangeBanner";
import { deviceProblems } from "../lib/health";
import ScreenSettingsRow from "./device/ScreenSettingsRow";
import { t, tn, type Key } from "../i18n";
import { MenuButton, type MenuItem } from "../components/Menu";
import { Tabs, panelProps } from "../components/Tabs";
import { Icon, type IconName, type Tone } from "../components/Icon";
import { quickGetClipboard, quickRing, quickSendClipboard, quickSendFile, quickSendFolder } from "../lib/quickActions";
import Overview, { DeviceFacts } from "./device/Overview";
import { osName } from "../lib/deviceInfo";
import { howLabel } from "../lib/health";
import { focusSoon, rememberFocus, trapTab } from "../lib/a11y";
import PhotosTile from "./device/PhotosTile";
import BrowseTile from "./device/BrowseTile";
import SyncTile from "./device/SyncTile";
import DropToSend from "./device/DropToSend";
import InfoTile from "./device/InfoTile";
import ScheduleTile from "./device/ScheduleTile";
import CommandsTile from "./device/CommandsTile";
import LostModeTile from "./device/LostModeTile";
import NotificationHistory from "./device/NotificationHistory";
import { foundId, foundTarget, isFound } from "../lib/search";
import ClipHistoryTile from "./device/ClipHistoryTile";
import PhoneAudioTile from "./device/PhoneAudioTile";
import ComputerAudioTile from "./device/ComputerAudioTile";
import CameraTile from "./device/CameraTile";
import InsertTile from "./device/InsertTile";
import NowPlayingTile from "./device/NowPlayingTile";
import WakeButton from "./device/WakeButton";
import ConnectionTile from "./device/ConnectionTile";
import { NearbyBadge } from "../components/NearbyBadge";
import { moveOnto, moveStep, pageKind, type PageKind, type TileDef } from "../lib/pageLayout";
import type { PageLayoutView } from "../bindings";
import { useVerify } from "../components/VerifyDevice";
import { isVerified } from "../lib/verify";
import { RemoveDeviceConfirm } from "../components/RemovedDevices";
import { askConfirm } from "../components/Confirm";
import { PermissionsTile } from "../components/PeerPermissions";

/** The page's tabs, in order. A tile lives in one (its `tab`); Overview is
 *  the device's dashboard (device/Overview.tsx). A tab with nothing this
 *  device offers isn't shown. */
const TAB_IDS = ["overview", "messages", "files", "remote", "media", "tools", "settings"] as const;
type DeviceTab = (typeof TAB_IDS)[number];
const TAB_ICON: Record<DeviceTab, IconName> = {
  overview: "home",
  messages: "chat",
  files: "files",
  remote: "remote",
  media: "media",
  tools: "tools",
  settings: "settings",
};
const TAB_LABEL: Record<DeviceTab, Key> = {
  overview: "device.tab.overview",
  messages: "chat.title",
  files: "files.title",
  remote: "device.tab.remote",
  media: "device.tab.media",
  tools: "device.tab.tools",
  settings: "nav.settings",
};

/** The tab last open on each device's page (this computer only). */
const TAB_STORE = "md.deviceTab.";
function savedTab(node: string): DeviceTab | null {
  try {
    const v = localStorage.getItem(TAB_STORE + node);
    return (TAB_IDS as readonly string[]).includes(v ?? "") ? (v as DeviceTab) : null;
  } catch {
    return null;
  }
}
function saveTab(node: string, tab: DeviceTab) {
  try {
    localStorage.setItem(TAB_STORE + node, tab);
  } catch {
    /* no storage: it opens on Overview next time */
  }
}

/** Every tile a device's page can show, in default order. A new tile is one
 *  entry here (its `id` is kept in saved layouts: never rename one); a saved
 *  layout that predates it shows it at this place. */
const TILES: TileDef[] = [
  { id: "chat", icon: "chat", tab: "messages", title: () => t("chat.title"), wide: true, render: (n) => <ChatTile node={n} /> },
  { id: "notifications", icon: "notifications", tab: "messages", title: () => t("notifications.title"), wide: true, render: (n) => <NotificationsTile node={n} /> },
  { id: "notification-history", icon: "history", tab: "messages", title: () => t("history.title"), wide: true, render: (n) => <NotificationHistory node={n} /> },
  { id: "clipboard", icon: "clipboard", tab: "files", title: () => t("clipboard.title"), render: (n) => <ClipboardTile node={n} /> },
  { id: "clipboard-history", icon: "history", tab: "files", title: () => t("clipHistory.title"), wide: true, render: (n) => <ClipHistoryTile node={n} /> },
  { id: "files", icon: "files", tab: "files", title: () => t("files.title"), render: (n) => <FilesTile node={n} /> },
  { id: "browse", icon: "folder-open", tab: "files", title: () => t("browse.title"), wide: true, render: (n) => <BrowseTile node={n} /> },
  { id: "sync", icon: "folder-sync", tab: "files", title: () => t("sync.title"), render: (n) => <SyncTile node={n} /> },
  { id: "control", icon: "locate", tab: "remote", title: () => t("control.title"), render: (n) => <ControlTile node={n} /> },
  { id: "info", icon: "info", tab: "tools", title: () => t("info.title"), render: (n) => <InfoTile node={n} /> },
  { id: "connection", icon: "connect", tab: "settings", title: () => t("conn.title"), wide: true, render: (n) => <ConnectionTile node={n} /> },
  { id: "schedules", icon: "calendar", tab: "tools", title: () => t("schedule.title"), wide: true, render: (n) => <ScheduleTile node={n} /> },
  {
    id: "commands",
    icon: "terminal",
    tab: "remote",
    title: () => t("run.title"),
    available: (n) => !!state.status[n]?.actions.includes("run"),
    render: (n) => <CommandsTile node={n} />,
  },
  {
    id: "screen",
    icon: "screen",
    tab: "remote",
    title: (n) => (state.status[n]?.platform === "android" ? t("phoneScreen.title") : t("screen.title")),
    available: (n) => !!state.status[n]?.actions.includes("screen"),
    render: (n) => <ScreenTile node={n} />,
  },
  {
    // A phone that can show its camera here (it says camera_view).
    id: "camera",
    icon: "camera",
    tab: "remote",
    title: () => t("camera.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.cameraView && state.status[n]?.platform === "android",
    render: (n) => <CameraTile node={n} />,
  },
  {
    // Insert from phone: a photo or a scan taken on the phone, into
    // Downloads and the clipboard here (it says insert_capture).
    id: "insert",
    icon: "scan",
    tab: "tools",
    title: () => t("insert.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.insertCapture && state.status[n]?.platform === "android",
    render: (n) => <InsertTile node={n} />,
  },
  {
    // What plays there, with play/pause, skip, seek and volume when its
    // app takes media commands (media_ctl); an older one just shows it.
    id: "now-playing",
    icon: "music",
    tab: "media",
    title: () => t("media.title"),
    available: (n) => !!state.status[n]?.mediaCtl || !!state.playing[n],
    render: (n) => <NowPlayingTile node={n} />,
  },
  {
    id: "phone-audio",
    icon: "mic",
    tab: "media",
    title: () => t("phoneAudio.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.audioSource && state.status[n]?.platform === "android",
    render: (n) => <PhoneAudioTile node={n} />,
  },
  {
    // A phone that plays this computer's sound (it says audio_sink).
    id: "computer-audio",
    icon: "speaker",
    tab: "media",
    title: () => t("computerAudio.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.audioSink && state.status[n]?.platform === "android",
    render: (n) => <ComputerAudioTile node={n} />,
  },
  {
    id: "photos",
    icon: "image",
    tab: "files",
    title: () => t("photos.title"),
    wide: true,
    available: (n) => !!state.status[n]?.actions.includes("gallery"),
    render: (n) => <PhotosTile node={n} />,
  },
  { id: "lost-mode", icon: "lock", tab: "tools", title: () => t("lost.title"), render: (n) => <LostModeTile node={n} /> },
  // What this device may do here: control this computer, see its screen,
  // take over its mouse and keyboard (all off until allowed).
  { id: "permissions", icon: "shield", tab: "settings", title: () => t("perm.title"), render: (n) => <PermissionsTile node={n} /> },
  { id: "notify", icon: "send", tab: "messages", title: () => t("notify.title"), render: () => <NotifyTile /> },
  { id: "apps", icon: "apps", tab: "messages", title: () => t("apps.title"), wide: true, render: (n) => <AppsTile node={n} /> },
];

export default function Device(props: {
  node: string;
  /** Open at this tab (a search result, the map's Connection details). */
  tab?: { id: string; tile?: string; at: number };
  onBack: () => void;
  onHealth?: () => void;
  onActivity?: () => void;
}) {
  const peer = createMemo(() => state.peers.find((p) => p.node_id === props.node));
  const s = () => linkStateOf(props.node);
  const live = () => s() === "direct" || s() === "relay";
  const [editing, setEditing] = createSignal(false);
  // Another device's page: close the form.
  createEffect(on(() => props.node, () => setEditing(false), { defer: true }));

  // The page's arrangement ("Edit page"): its tiles in the saved order,
  // for this kind of device or this device alone.
  const [arranging, setArranging] = createSignal(false);
  const [layout, setLayout] = createSignal<PageLayoutView | null>(null);
  const kind = createMemo(() => pageKind(props.node));
  const kindTiles = createMemo(() => TILES.filter((d) => !d.kinds || d.kinds.includes(kind())));
  createEffect(on(() => props.node, () => setArranging(false), { defer: true }));
  createEffect(
    on([() => props.node, kind, kindTiles], ([node, k, tiles]) => {
      setLayout(null);
      commands
        .getPageLayout(k, node, tiles.map((d) => d.id))
        .then((v) => node === props.node && k === kind() && setLayout(v))
        .catch(() => {});
    }),
  );
  // Until the saved layout is read: the default order.
  const shownTiles = createMemo(() => {
    const l = layout();
    const byId = new Map(kindTiles().map((d) => [d.id, d]));
    if (!l) return kindTiles();
    return l.order.filter((id) => !l.hidden.includes(id)).flatMap((id) => byId.get(id) ?? []);
  });

  // Tabs: the shown tiles, by tab ("Edit page" order and hiding still
  // apply inside each). A tab with nothing the device offers is left out.
  const inTab = (d: TileDef, tab: DeviceTab) => d.tab === tab;
  const offered = (d: TileDef) => d.available?.(props.node) ?? true;
  const tabs = createMemo(() =>
    TAB_IDS.filter((tab) => tab === "overview" || shownTiles().some((d) => inTab(d, tab) && offered(d))),
  );
  const [tab, setTab] = createSignal<DeviceTab>(savedTab(props.node) ?? "overview");
  createEffect(on(() => props.node, (node) => setTab(savedTab(node) ?? "overview"), { defer: true }));
  // Asked to open at a tab: that one (when the device has it).
  createEffect(
    on(
      () => props.tab,
      (ask) => {
        if (ask && (TAB_IDS as readonly string[]).includes(ask.id)) {
          setTab(ask.id as DeviceTab);
          if (ask.tile) setSub({ ...sub(), [ask.id]: ask.tile });
        }
      },
    ),
  );
  // The tab shown: the chosen one, else Overview when it went away.
  const current = () => (tabs().includes(tab()) ? tab() : "overview");
  const choose = (id: string) => {
    setTab(id as DeviceTab);
    saveTab(props.node, id as DeviceTab);
  };
  const tabTiles = () => shownTiles().filter((d) => inTab(d, current()));
  // A tab with more than two tiles shows one at a time, picked from a row
  // of its tiles' names (Overview shows all of its few). Nothing is a long
  // page.
  const [sub, setSub] = createSignal<Partial<Record<string, string>>>({});
  const offeredTiles = () => tabTiles().filter(offered);
  const split = () => current() !== "overview" && offeredTiles().length > 2;
  const subTile = () => {
    const list = offeredTiles();
    return list.find((d) => d.id === sub()[current()])?.id ?? list[0]?.id;
  };
  const unread = () => state.messages.filter((m) => m.node === props.node && m.dir === "in").length;
  const moving = () =>
    Object.values(state.transfers).filter(
      (x) => x.node === props.node && (x.state === "waiting" || x.state === "sending" || x.state === "receiving"),
    ).length;
  const tabDefs = () =>
    tabs().map((id) => ({
      id,
      label: t(TAB_LABEL[id]),
      icon: TAB_ICON[id],
      count: id === "messages" ? unread() : id === "files" ? moving() : undefined,
    }));

  // The "⋯" menu: less-used actions. Verify and Remove open their dialogs.
  const verify = useVerify(() => props.node);
  const [removing, setRemoving] = createSignal(false);
  const menu = (): MenuItem[] => [
    // Overview has its own Quick actions card: the header's "Actions" button
    // isn't shown there, its items are here.
    ...(current() === "overview" ? quickMenu().map((m) => ({ ...m, divider: false })) : []),
    {
      icon: "layout",
      divider: current() === "overview",
      label: arranging() ? t("layout.done") : t("layout.edit"),
      hint: t("layout.editHint"),
      onSelect: () => setArranging((v) => !v),
    },
    { icon: "verify", label: t("verify.button"), hint: t("verify.buttonHint"), onSelect: verify.show },
    { icon: "remove", label: t("removeDevice.button"), hint: t("removeDevice.hint"), danger: true, onSelect: () => setRemoving(true) },
  ];
  // The link control: Online / Offline, with Reconnect and Disconnect, or
  // Connect.
  const linkMenu = (): MenuItem[] =>
    live()
      ? [
          {
            icon: "reconnect",
            label: t("device.reconnect"),
            hint: t("device.reconnectHint"),
            onSelect: () => reconnect(props.node).catch((e) => toast(friendlyError(e), "error")),
          },
          {
            icon: "disconnect",
            label: t("device.disconnect"),
            hint: t("device.disconnectHint"),
            onSelect: () => disconnect(props.node).catch((e) => toast(friendlyError(e), "error")),
          },
        ]
      : [
          {
            icon: "connect",
            label: t("device.connect"),
            onSelect: () => connect(props.node).catch((e) => toast(friendlyError(e), "error")),
          },
        ];
  // "Quick actions": what's done most, one click each (they wait for a
  // device that isn't reachable; getting its clipboard needs a link).
  const quickMenu = (): MenuItem[] => [
    { icon: "send-file", label: t("quick.sendFile"), onSelect: () => quickSendFile(props.node) },
    { icon: "send-folder", label: t("files.sendFolder"), onSelect: () => quickSendFolder(props.node) },
    { icon: "send-clip", label: t("quick.sendClipboard"), onSelect: () => quickSendClipboard(props.node) },
    ...(live() ? [{ icon: "get-clip" as const, label: t("quick.getClipboard"), onSelect: () => quickGetClipboard(props.node) }] : []),
    { icon: "ring", label: t("control.ring"), divider: true, onSelect: () => quickRing(props.node) },
  ];
  const goTo = (tab: string, tile?: string) => {
    choose(tab);
    if (tile) setSub({ ...sub(), [tab]: tile });
  };
  // Last seen and how it's linked: from the connections map, read once
  // for this page (and again when the link changes).
  const [seen, setSeen] = createSignal<{ ago: number | null; how: string } | null>(null);
  createEffect(
    on([() => props.node, live], ([node]) => {
      commands
        .linksMap()
        .then((v) => {
          const d = v.map.devices.find((x) => x.node_id === node);
          const how = v.edges.find((e) => e.mine && e.b === node)?.how ?? "";
          if (node === props.node) setSeen(d ? { ago: d.last_seen_ago_s, how } : null);
        })
        .catch(() => {});
    }),
  );
  const battery = () => state.status[props.node]?.battery;
  const network = () => {
    const ssid = state.status[props.node]?.wifiSsid;
    if (ssid) return ssid;
    const how = seen()?.how;
    return live() && how && how !== "none" ? howLabel(how) : "";
  };
  // One word for its presence (the pill and the line say the same), and
  // the route beside it when live.
  const statusTone = () => presenceTone(s());
  const route = () => routeOf(s(), seen()?.how);
  const lastSeen = () => (!live() && seen()?.ago != null ? t("map.lastSeen", { ago: timeAgo(Date.now() - seen()!.ago! * 1000) }) : "");
  // Asleep / not reached: what it means, behind a "?" (not a paragraph).
  const why = () => (s() === "asleep" ? t("progress.asleep") : s() === "failed" ? progressLabel("failed") : null);
  // Can it be woken (Wake-on-LAN)? Then Wake leads while it's not live.
  const [canWake, setCanWake] = createSignal(false);
  createEffect(on(() => [props.node, state.status[props.node]] as const, ([n]) => commands.canWake(n).then(setCanWake, () => setCanWake(false))));
  const wakeLeads = () => canWake() && !live();
  const mockMenu = import.meta.env.MODE === "mock" && new URLSearchParams(location.search).get("menu") === "1";

  // Ask for its battery whenever this page opens (answer: device-status-event).
  // Asked only over a live link (and again when one comes up): asking a
  // device that isn't connected would wake it by push just to show this.
  createEffect(
    on(
      () => [props.node, !!state.links[props.node]] as const,
      ([node, linked]) => linked && requestStatus(node).catch(() => {}),
    ),
  );

  return (
    <Show
      when={peer()}
      fallback={
        <section class="page">
          <h1 class="sr-only">{t("nav.devices")}</h1>
          <button class="link" onClick={props.onBack}>
            {t("device.back")}
          </button>
          <p class="muted">{t("device.gone")}</p>

        </section>
      }
    >
      {(p) => (
        <section class="page device-page">
          <button class="link back" onClick={props.onBack}>
            <Icon name="arrow-left" size={16} />
            {t("device.back")}
          </button>
          {/* The hero: the device's picture, its name, how it's linked, a
              few facts; the link control, Quick actions and "more". */}
          <header class="device-hero">
            {/* The device as it looks: its frame with its wallpaper. */}
            <div class="device-art">
              <DeviceArt
                node={props.node}
                icon={peerIcon(props.node)}
                size={["laptop", "monitor"].includes(frameOf(peerIcon(props.node))) ? 104 : "large"}
                title={peerName(props.node)}
              />
            </div>
            <div class="hero-main">
              <div class="label-title">
                <h1>{peerName(p().node_id)}</h1>
                <button
                  class="btn btn-ghost btn-sm btn-icon hero-edit"
                  aria-label={t("label.editHint")}
                  title={t("label.editHint")}
                  aria-expanded={editing()}
                  onClick={() => setEditing((v) => !v)}
                >
                  <Icon name="edit" />
                </button>
              </div>
              {/* Presence (the same word as the pill), then the route when
                  live, or when it was last seen. */}
              <p class="hero-status">
                <span class={`hero-presence tone-${statusTone()}`}>
                  <span class="hero-dot" aria-hidden="true" />
                  {presenceLabel(s())}
                </span>
                <RouteChip route={route()} />
                <Show when={s() === "connecting" || s() === "waking"}>
                  <span class="muted small">{progressLabel(state.progress[props.node])}</span>
                </Show>
                <Show when={lastSeen()}>
                  <span class="muted small">· {lastSeen()}</span>
                </Show>
                <Show when={why()}>
                  {(w) => (
                    <span class="hint-icon" tabIndex={0} role="img" aria-label={w()} title={w()}>
                      <Icon name="info" size={15} />
                    </span>
                  )}
                </Show>
              </p>
              <Show when={osName(props.node) || peerModel(props.node)}>
                <p class="muted small hero-facts">{osName(props.node) || peerModel(props.node)}</p>
              </Show>
              <div class="hero-chips">
                <Show when={isVerified(props.node)}>
                  <span class="chip chip-green" title={t("verify.badgeHint")}>
                    <Icon name="verify" size={15} />
                    {t("verify.badge")}
                  </span>
                </Show>
                <Show when={network()}>
                  <span class="chip">
                    <Icon name="wifi" size={15} />
                    {network()}
                  </span>
                </Show>
                <Show when={battery() !== undefined}>
                  <span class="chip" classList={{ "chip-red": battery()! <= 15 }}>
                    <Icon name="battery" size={15} />
                    {battery()}%
                  </span>
                </Show>
              </div>
            </div>
            <div class="hero-side">
              {/* Asleep? Wake leads, if it said it may be woken. */}
              <WakeButton node={props.node} />
              <MenuButton
                label={presenceLabel(s())}
                labelled
                triggerClass={`btn btn-soft hero-link tone-${statusTone()}`}
                items={linkMenu()}
              >
                <span class="hero-dot" aria-hidden="true" />
                {presenceLabel(s())}
                <Icon name="chevron-down" size={16} />
              </MenuButton>
              {/* One primary: Actions on every tab but Overview (which has
                  its Quick actions card), unless Wake leads. */}
              <Show when={current() !== "overview"}>
                <MenuButton
                  label={t("device.actions")}
                  labelled
                  triggerClass={`btn hero-actions ${wakeLeads() ? "btn-soft" : "btn-primary"}`}
                  items={quickMenu()}
                >
                  <Icon name="quick" />
                  <span class="btn-label">{t("device.actions")}</span>
                  <Icon name="chevron-down" size={16} />
                </MenuButton>
              </Show>
              <MenuButton label={t("device.more", { name: peerName(props.node) })} items={menu()} startOpen={mockMenu}>
                <Icon name="more" />
              </MenuButton>
              <DeviceFacts node={props.node} />
            </div>
          </header>
          {verify.dialog()}
          <Show when={removing()}>
            <RemoveDeviceConfirm
              node={props.node}
              name={peerName(props.node)}
              onClose={() => setRemoving(false)}
              onRemoved={props.onBack}
            />
          </Show>
          <KeyChangeBanner node={props.node} />
          {/* The health check found something wrong with this device. */}
          <Show when={props.onHealth && deviceProblems(props.node).some((c) => c.level === "bad")}>
            <div class="health-banner" role="status">
              <span>{t("health.deviceBanner")}</span>
              <button class="btn btn-soft btn-sm" onClick={() => props.onHealth?.()}>
                {t("health.open")}
              </button>
            </div>
          </Show>
          <Show when={editing()}>
            <DeviceLabelForm node={props.node} onClose={() => setEditing(false)} />
          </Show>

          <DropToSend node={props.node} />
          <Show
            when={arranging()}
            fallback={
              <>
                <Tabs
                  label={t("device.tabsLabel", { name: peerName(props.node) })}
                  idBase="device"
                  tabs={tabDefs()}
                  selected={current()}
                  onSelect={choose}
                />
                <Show when={current() === "overview"}>
                  <div class="tab-panel" {...panelProps("device", "overview")}>
                    <Overview node={props.node} how={seen()?.how} onGo={goTo} onActivity={() => props.onActivity?.()} />
                  </div>
                </Show>
                <div class="tiles tab-panel" data-tab={current()} classList={{ "tiles-split": split() }} hidden={current() === "overview"} {...panelProps("device", current())}>
                  <Show when={split()}>
                    <ul class="sub-nav tile-wide" aria-label={t("device.tilesOf", { tab: t(TAB_LABEL[current()]) })}>
                      <For each={offeredTiles()}>
                        {(d) => (
                          <li>
                            <button
                              type="button"
                              class="sub-nav-item"
                              classList={{ active: subTile() === d.id }}
                              aria-current={subTile() === d.id ? "true" : undefined}
                              onClick={() => setSub({ ...sub(), [current()]: d.id })}
                            >
                              <Icon name={d.icon} />
                              {d.title(props.node)}
                            </button>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                  <For each={tabTiles()}>
                    {(d) => (
                      <Show when={offered(d)}>
                        {/* The tile with its icon before its title. */}
                        <div
                          class="icon-slot"
                          classList={{ "slot-wide": !!d.wide || split() }}
                          hidden={split() && subTile() !== d.id}
                        >
                          <Icon name={d.icon} class="slot-icon" />
                          {d.render(props.node)}
                        </div>
                      </Show>
                    )}
                  </For>
                  <Show when={layout() && shownTiles().length === 0}>
                    <p class="muted small">{t("layout.allHidden")}</p>
                  </Show>
                </div>
              </>
            }
          >
            <ArrangePage
              node={props.node}
              kind={kind()}
              tiles={kindTiles()}
              layout={layout()}
              onLayout={setLayout}
              onDone={() => setArranging(false)}
            />
          </Show>
        </section>
      )}
    </Show>
  );
}

/** "Edit page": each shown tile as a card to drag by its handle (or move
 *  with Move up / Move down, or the arrow keys on the handle), and hide;
 *  hidden tiles listed below to show again. Saved as it changes (a drag
 *  when it's dropped), for every device of this kind or this one alone. */
function ArrangePage(props: {
  node: string;
  kind: PageKind;
  tiles: TileDef[];
  layout: PageLayoutView | null;
  onLayout: (v: PageLayoutView) => void;
  onDone: () => void;
}) {
  const ids = () => props.tiles.map((d) => d.id);
  const view = (): PageLayoutView => props.layout ?? { order: ids(), hidden: [], scope: "default" };
  const byId = createMemo(() => new Map(props.tiles.map((d) => [d.id, d])));
  const title = (id: string) => byId().get(id)?.title(props.node) ?? id;
  const shownIds = createMemo(() => view().order.filter((id) => !view().hidden.includes(id)));
  const hiddenIds = createMemo(() => view().order.filter((id) => view().hidden.includes(id)));
  const onlyThis = () => view().scope === "device";
  const name = () => peerName(props.node);
  const [announce, setAnnounce] = createSignal("");
  const [dragging, setDragging] = createSignal<string | null>(null);
  let grid: HTMLOListElement | undefined;
  let hiddenList: HTMLUListElement | undefined;

  // Only the newest answer counts (several saves can be on their way).
  let asked = 0;
  const save = (only: boolean, l: { order: string[]; hidden: string[] } | null) => {
    const mine = ++asked;
    return commands
      .setPageLayout(props.kind, props.node, only, l, ids())
      .then((v) => mine === asked && props.onLayout(v))
      .catch((e) => toast(friendlyError(e), "error"));
  };
  /** Shown at once; saved unless a drag is still going. */
  const change = (order: string[], hidden: string[], commit = true) => {
    props.onLayout({ order, hidden, scope: view().scope });
    if (commit) save(onlyThis(), { order, hidden });
  };
  const focusIn = (id: string, sel: string) =>
    queueMicrotask(() => {
      const card = grid?.querySelector<HTMLElement>(`[data-tile="${CSS.escape(id)}"]`);
      const el = card?.querySelector<HTMLButtonElement>(sel);
      (el && !el.disabled ? el : card?.querySelector<HTMLElement>(".arrange-handle"))?.focus();
    });
  const sayWhere = (id: string) => {
    const shown = shownIds();
    setAnnounce(t("layout.movedTo", { tile: title(id), n: shown.indexOf(id) + 1, total: shown.length }));
  };

  const step = (id: string, s: -1 | 1, refocus: string) => {
    const v = view();
    const order = moveStep(v.order, v.hidden, id, s);
    if (order === v.order) return;
    change(order, v.hidden);
    sayWhere(id);
    focusIn(id, refocus);
  };
  const hide = (id: string) => {
    const v = view();
    const next = shownIds()[shownIds().indexOf(id) + 1] ?? shownIds()[shownIds().indexOf(id) - 1];
    change(v.order, [...v.hidden, id]);
    setAnnounce(t("layout.hiddenNow", { tile: title(id) }));
    if (next) focusIn(next, ".arrange-hide");
    else queueMicrotask(() => hiddenList?.querySelector<HTMLElement>("button")?.focus());
  };
  const show = (id: string) => {
    const v = view();
    change(
      v.order,
      v.hidden.filter((x) => x !== id),
    );
    setAnnounce(t("layout.shownNow", { tile: title(id) }));
    focusIn(id, ".arrange-handle");
  };

  // Dragging: the handle keeps the pointer; the card under it takes the
  // dragged tile's place as it passes. Saved when dropped.
  const down = (id: string) => (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setDragging(id);
  };
  const drag = (e: PointerEvent) => {
    const id = dragging();
    if (!id) return;
    const card = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-tile]");
    const over = card?.dataset.tile;
    if (!over || over === id) return;
    const v = view();
    change(moveOnto(v.order, id, over), v.hidden, false);
  };
  const drop = () => {
    const id = dragging();
    if (!id) return;
    setDragging(null);
    const v = view();
    save(onlyThis(), { order: v.order, hidden: v.hidden });
    sayWhere(id);
  };
  const handleKey = (id: string) => (e: KeyboardEvent) => {
    const s = e.key === "ArrowUp" || e.key === "ArrowLeft" ? -1 : e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : 0;
    if (!s) return;
    e.preventDefault();
    step(id, s, ".arrange-handle");
  };

  const setOnlyThis = (on: boolean) => {
    const v = view();
    // On: this device keeps what it shows now as its own. Off: it follows
    // its kind's layout again.
    save(true, on ? { order: v.order, hidden: v.hidden } : null);
  };
  const reset = () => {
    save(onlyThis(), null).then(() => setAnnounce(t("layout.resetDone")));
  };
  const allOfKind = () => (props.kind === "phone" ? t("layout.allPhones") : t("layout.allComputers"));

  return (
    <div class="arrange">
      <div class="arrange-bar">
        <h2>{t("layout.title")}</h2>
        <p class="muted small">{t("layout.hint")}</p>
        <label class="switch-row">
          <span>{t("layout.onlyThis")}</span>
          <input type="checkbox" role="switch" checked={onlyThis()} onChange={(e) => setOnlyThis(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{onlyThis() ? t("layout.onlyThisOn", { name: name() }) : allOfKind()}</p>
        <div class="row">
          <button
            class="btn btn-ghost btn-sm"
            title={onlyThis() ? t("layout.resetDeviceHint", { name: name() }) : t("layout.resetHint")}
            disabled={view().scope === "default"}
            onClick={reset}
          >
            {t("layout.reset")}
          </button>
          <span class="grow" />
          <button class="btn btn-primary btn-sm" onClick={props.onDone}>
            {t("layout.done")}
          </button>
        </div>
      </div>
      <ol class="tiles arrange-grid" ref={grid} aria-label={t("layout.listLabel", { name: name() })}>
        <For each={shownIds()}>
          {(id) => {
            const d = () => byId().get(id);
            const at = () => shownIds().indexOf(id);
            return (
              <li
                class="tile arrange-card"
                classList={{ "tile-wide": !!d()?.wide, "arrange-dragging": dragging() === id }}
                data-tile={id}
              >
                <div class="row">
                  <button
                    class="arrange-handle"
                    aria-label={t("layout.handle", { tile: title(id) })}
                    title={t("layout.handleHint")}
                    onPointerDown={down(id)}
                    onPointerMove={drag}
                    onPointerUp={drop}
                    onPointerCancel={drop}
                    onKeyDown={handleKey(id)}
                  >
                    <svg width="14" height="20" viewBox="0 0 14 20" aria-hidden="true">
                      <For each={[3, 10, 17]}>
                        {(y) => (
                          <>
                            <circle cx="4" cy={y} r="1.6" fill="currentColor" />
                            <circle cx="10" cy={y} r="1.6" fill="currentColor" />
                          </>
                        )}
                      </For>
                    </svg>
                  </button>
                  <strong class="grow">{title(id)}</strong>
                </div>
                <Show when={!(d()?.available?.(props.node) ?? true)}>
                  <p class="muted small">{t("layout.notNow", { name: name() })}</p>
                </Show>
                <div class="row">
                  <button
                    class="btn btn-ghost btn-sm arrange-up"
                    disabled={at() === 0}
                    aria-label={t("layout.upLabel", { tile: title(id) })}
                    onClick={() => step(id, -1, ".arrange-up")}
                  >
                    {t("layout.up")}
                  </button>
                  <button
                    class="btn btn-ghost btn-sm arrange-down"
                    disabled={at() === shownIds().length - 1}
                    aria-label={t("layout.downLabel", { tile: title(id) })}
                    onClick={() => step(id, 1, ".arrange-down")}
                  >
                    {t("layout.down")}
                  </button>
                  <span class="grow" />
                  <button
                    class="btn btn-ghost btn-sm arrange-hide"
                    aria-label={t("layout.hideLabel", { tile: title(id) })}
                    onClick={() => hide(id)}
                  >
                    {t("layout.hide")}
                  </button>
                </div>
              </li>
            );
          }}
        </For>
      </ol>
      <Show when={hiddenIds().length}>
        <div class="arrange-hidden">
          <span class="small">{t("layout.hidden")}</span>
          <ul ref={hiddenList}>
            <For each={hiddenIds()}>
              {(id) => (
                <li>
                  <span>{title(id)}</span>
                  <button
                    class="btn btn-ghost btn-sm"
                    aria-label={t("layout.showLabel", { tile: title(id) })}
                    onClick={() => show(id)}
                  >
                    {t("layout.show")}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
      <p class="sr-only" role="status" aria-live="polite">
        {announce()}
      </p>
    </div>
  );
}

/** Saved messages loaded at first, and per step back. */
const CHAT_PAGE = 100;

export function ChatTile(props: { node: string }) {
  const [text, setText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  // Saved messages (chat_history.rs) from before this session's own,
  // oldest first; more of them as the chat is scrolled up.
  const [saved, setSaved] = createSignal<ChatMessage[]>([]);
  const [more, setMore] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  let asked = 0;
  let firstPage: Promise<void> = Promise.resolve();
  const live = () => state.messages.filter((m) => m.node === props.node);
  // This session's bubbles carry the live state; a saved copy of one is left out.
  const thread = createMemo(() => {
    const mine = live();
    const ids = new Set(mine.map((m) => m.mid).filter(Boolean));
    const older = saved().filter((m) => !ids.has(m.mid));
    return [...older, ...mine].sort((a, b) => a.ts - b.ts);
  });
  // Only the newest delivered message of ours gets the marker.
  const lastDelivered = () => thread().filter((m) => m.dir === "out" && m.delivered).pop()?.id;
  let list: HTMLDivElement | undefined;
  const toBottom = () => queueMicrotask(() => list?.scrollTo({ top: list.scrollHeight }));

  // The newest saved page, for this device (again after a clear or the
  // setting changing).
  createEffect(
    on([() => props.node, chatTick, chatKeep], ([node, , keep]) => {
      const mine = ++asked;
      setSaved([]);
      setMore(false);
      if (!keep) return;
      firstPage = chatPage(node, null, CHAT_PAGE)
        .then((p) => {
          if (mine !== asked) return;
          setSaved(p.messages);
          setMore(p.more);
          toBottom();
        })
        .catch(() => {});
    }),
  );

  /** One step further back; keeps what's on screen where it was. */
  const loadOlder = async () => {
    const first = saved()[0]?.mid;
    if (loading() || !more() || !first) return false;
    setLoading(true);
    const mine = asked;
    try {
      const p = await chatPage(props.node, first, CHAT_PAGE);
      if (mine !== asked) return false;
      const height = list?.scrollHeight ?? 0;
      setSaved((s) => [...p.messages, ...s]);
      setMore(p.more);
      queueMicrotask(() => {
        if (list) list.scrollTop += list.scrollHeight - height;
      });
      return p.messages.length > 0;
    } catch {
      return false;
    } finally {
      setLoading(false);
    }
  };
  const onScroll = () => {
    if (list && list.scrollTop < 40) loadOlder();
  };
  // A message opened from a search: step back until it's in the chat.
  createEffect(
    on(foundTarget, async (x) => {
      if (x?.kind !== "messages" || x.node !== props.node) return;
      await firstPage;
      while (!thread().some((m) => String(m.mid ?? m.id) === x.id)) {
        if (!(await loadOlder())) break;
      }
    }),
  );

  const send = async (e: Event) => {
    e.preventDefault();
    const t = text().trim();
    if (!t) return;
    setBusy(true);
    try {
      await sendMessage(props.node, t);
      setText("");
      toBottom();
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="tile tile-wide chat-tile">
      <div class="tile-head">
        {/* Named by the tab or sub-tab around it: kept for screen readers. */}
        <h2>{t("chat.title")}</h2>
        {/* Clear is destructive: in the card's "⋯", asked first. */}
        <Show when={thread().length > 0}>
          <MenuButton
            label={t("chat.more", { name: peerName(props.node) })}
            items={[
              {
                icon: "remove",
                label: t("chat.clear"),
                hint: t("chat.clearHint"),
                danger: true,
                onSelect: async () => {
                  if (await askConfirm({ title: t("chat.clearTitle"), body: t("chat.clearHint"), action: t("chat.clear") }))
                    clearChat(props.node).catch(() => {});
                },
              },
            ]}
          >
            <Icon name="more" />
          </MenuButton>
        </Show>
      </div>
      <div class="thread" ref={list} onScroll={onScroll} role="log" aria-label={t("a11y.chatLog", { name: peerName(props.node) })} tabIndex={0}>
        <Show when={more()}>
          <button class="btn btn-ghost btn-sm chat-older" disabled={loading()} onClick={() => loadOlder()}>
            {loading() ? t("chat.loadingOlder") : t("chat.older")}
          </button>
        </Show>
        <Show when={thread().length > 0} fallback={<p class="muted small center">{t("chat.empty")}</p>}>
          <For each={thread()}>
            {(m) => (
              <div
                id={foundId("messages", m.mid ?? m.id)}
                tabIndex={-1}
                class={`bubble bubble-${m.dir}`}
                classList={{
                  "bubble-pending": !!m.pending,
                  "bubble-failed": !!m.failed,
                  "found-target": isFound("messages", m.mid ?? m.id),
                }}
              >
                <span>{m.text}</span>
                <Show when={m.pending}>
                  <small class="bubble-note">{m.via === "queued" ? t("chat.queued") : t("chat.sending")}</small>
                </Show>
                <Show when={m.failed}>
                  <small class="bubble-note">{t("chat.failed")}</small>
                </Show>
                <Show when={m.delivered && m.id === lastDelivered()}>
                  <small class="bubble-note">{t("chat.delivered")}</small>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
      <Show when={chatKeep()}>
        <p class="muted small chat-kept">{t("chat.kept")}</p>
      </Show>
      <form class="composer" onSubmit={send}>
        <input
          placeholder={t("chat.placeholder")}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          aria-label={t("chat.label")}
        />
        <button class="btn btn-primary" type="submit" disabled={busy() || !text().trim()}>
          {t("chat.send")}
        </button>
      </form>
    </article>
  );
}

export function NotificationsTile(props: { node: string }) {
  const [groups, setGroups] = createSignal<NotifGroup[]>([]);
  const [prefs, setPrefs] = createSignal<NotifPrefs>({ private_mode: true, only_locked: false });
  const load = () => phoneNotifications(props.node).then(setGroups, () => {});
  createEffect(on(() => [props.node, notifTick(props.node)], load));
  onMount(() => phoneNotifPrefs().then(setPrefs, () => {}));
  const change = async (next: NotifPrefs) => {
    try {
      await setPhoneNotifPrefs(next);
      setPrefs(next);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  return (
    <article class="tile tile-wide">
      <h2>{t("notifications.title")}</h2>
      <Show when={groups().length} fallback={<p class="muted small">{t("notifications.empty")}</p>}>
        <ul class="notif-list">
          <For each={groups()}>{(g) => <NotificationGroup g={g} />}</For>
        </ul>
        <p class="muted small">{t("notifications.hint")}</p>
      </Show>
      <label class="switch-row">
        <span>{t("notifications.privateMode")}</span>
        <input type="checkbox" role="switch" checked={prefs().private_mode} onChange={(e) => change({ ...prefs(), private_mode: e.currentTarget.checked })} />
      </label>
      <p class="muted small">{t("notifications.privateModeHint")}</p>
      <label class="switch-row">
        <span>{t("notifications.onlyLocked")}</span>
        <input type="checkbox" role="switch" checked={prefs().only_locked} onChange={(e) => change({ ...prefs(), only_locked: e.currentTarget.checked })} />
      </label>
      <p class="muted small">{t("notifications.onlyLockedHint")}</p>
    </article>
  );
}

/** One conversation (or one notification): its app and sender, the last
 *  messages, any picture, its buttons and a reply box for the newest. */
function NotificationGroup(props: { g: NotifGroup }) {
  const [reply, setReply] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.g.from);
  const newest = () => props.g.items[0];
  const actionable = () => !!newest()?.key && !newest()?.handled;
  const appName = () => props.g.app || name();
  const title = () => (props.g.private ? t("notifications.private", { app: appName() }) : props.g.title || appName());
  // The conversation: the newest one's messages, else each one's text.
  const lines = () => {
    const n = newest();
    if (!n) return [];
    if (n.messages.length) return n.messages.map((m) => ({ who: m.sender, text: m.text }));
    return props.g.items
      .slice()
      .reverse()
      .filter((i) => i.body)
      .map((i) => ({ who: props.g.items.length > 1 || props.g.group_chat ? i.sender ?? "" : "", text: i.body }));
  };

  const act = async (index: number, text?: string) => {
    const n = newest();
    if (!n) return;
    setBusy(true);
    try {
      const via = await notificationAction(n, index, text);
      const queued = via === "queued";
      if (text !== undefined) {
        setReply("");
        toast(queued ? t("notifications.replyQueued", { name: name() }) : t("notifications.replied"), "ok");
      } else if (index >= 0) {
        toast(queued ? t("notifications.doneQueued", { name: name() }) : t("notifications.done"), "ok");
      }
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  // Dismiss every notification in it (on the phone too).
  const dismissAll = async () => {
    setBusy(true);
    try {
      for (const n of props.g.items) await notificationAction(n, -1);
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const mute = async () => {
    try {
      const app = props.g.app;
      if (!app) return;
      await setMutedApps([...(await mutedApps()), app]);
      toast(t("history.muted", { app }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const send = (e: Event) => {
    e.preventDefault();
    const text = reply().trim();
    const r = newest()?.reply;
    if (text && r) act(r.index, text);
  };

  return (
    <li class="notif" classList={{ "notif-handled": !!newest()?.handled }}>
      <div class="row">
        <Show when={props.g.icon} fallback={<span class="notif-icon"><Icon name="notifications" /></span>}>
          <img class="notif-icon" src={props.g.icon!} alt="" />
        </Show>
        <span class="small muted grow notif-app">{appName()}</span>
        <Show when={newest()?.handled}>
          <span class="small muted">{t("notifications.handled")}</span>
        </Show>
        <Show when={props.g.app}>
          <button class="btn btn-ghost btn-sm" onClick={mute} aria-label={t("history.muteLabel", { app: props.g.app })}>
            {t("history.mute")}
          </button>
        </Show>
        <button
          class="btn btn-ghost btn-sm"
          disabled={busy()}
          onClick={dismissAll}
          aria-label={t("notifications.dismissLabel", { title: title() })}
        >
          {t("notifications.dismiss")}
        </button>
      </div>
      <div class="row">
        <Show when={props.g.avatar}>
          <img class="notif-avatar" src={props.g.avatar!} alt="" />
        </Show>
        <strong class="notif-title grow">{title()}</strong>
        <Show when={props.g.items.length > 1}>
          <span class="small muted">{tn("notifications.count", props.g.items.length)}</span>
        </Show>
      </div>
      <Show when={!props.g.private}>
        <For each={lines()}>
          {(l) => (
            <p class="notif-body small">
              <Show when={l.who}>
                <strong>{l.who}: </strong>
              </Show>
              {l.text}
            </p>
          )}
        </For>
        <Show when={newest()?.picture}>
          <img class="notif-picture" src={newest()!.picture!} alt={t("notifications.pictureLabel")} />
        </Show>
      </Show>
      <Show when={actionable() && newest()!.buttons.length}>
        <div class="row">
          <For each={newest()!.buttons}>
            {(b) => (
              <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => act(b.index)}>
                {b.title}
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={actionable() && newest()!.reply}>
        <form class="composer" onSubmit={send}>
          <input
            placeholder={newest()!.reply!.title || t("notifications.replyPlaceholder")}
            value={reply()}
            onInput={(e) => setReply(e.currentTarget.value)}
            aria-label={t("notifications.replyLabel", { title: title() })}
          />
          <button class="btn btn-soft" type="submit" disabled={busy() || !reply().trim()}>
            {t("notifications.reply")}
          </button>
        </form>
      </Show>
    </li>
  );
}

function ClipboardTile(props: { node: string }) {
  const [busy, setBusy] = createSignal(false);
  onMount(() => loadClipHistory());
  return (
    <article class="tile">
      <h2>{t("clipboard.title")}</h2>
      <p class="muted small">{t("clipboard.hint")}</p>
      {/* Two inline actions, each with its icon. */}
      <div class="row">
      <button
        class="btn btn-soft btn-sm"
        disabled={busy()}
        onClick={async () => {
          setBusy(true);
          try {
            await pullClipboard(props.node);
            toast(t("clipboard.copied"), "ok");
          } catch (e) {
            toast(friendlyError(e), "error");
          } finally {
            setBusy(false);
          }
        }}
      >
        <Icon name="get-clip" size={16} />
        {busy() ? t("clipboard.getting") : t("clipboard.get")}
      </button>
      <button
        class="btn btn-soft btn-sm"
        onClick={async () => {
          const r = await commands.sendClipboardImage(props.node);
          if (r.status === "error") toast(friendlyError(r.error), "error");
          else toast(t("clipboard.pictureSent", { name: peerName(props.node) }), "ok");
        }}
      >
        <Icon name="image-up" size={16} />
        {t("clipboard.sendPicture")}
      </button>
      </div>
      <label class="switch-row">
        <span>{t("clipboard.sync")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={state.clipSync}
          onChange={(e) => setClipSync(e.currentTarget.checked)}
        />
      </label>
      <Show when={state.clipHistory.length}>
        <div class="row">
          <span class="small grow">{t("clipboard.recent")}</span>
          <button
            class="btn btn-ghost btn-sm btn-icon btn-danger-ghost"
            aria-label={t("clipboard.clearLabel")}
            title={t("clipboard.clearLabel")}
            onClick={async () => {
              if (await askConfirm({ title: t("clipboard.clearTitle"), action: t("clipboard.clear") })) clearClipHistory();
            }}
          >
            <Icon name="remove" size={16} />
          </button>
        </div>
        <ul class="clip-history">
          <For each={state.clipHistory.slice(0, 8)}>
            {(e) => (
              <li class="clip-entry">
                <div class="grow clip-text">
                  <span class="clip-value">{e.text}</span>
                  <span class="muted small">{e.from ? peerName(e.from) : t("clipboard.fromHere")}</span>
                </div>
                <button
                  class="btn btn-ghost btn-sm"
                  aria-label={t("clipboard.copyLabel")}
                  onClick={() =>
                    copyText(e.text)
                      .then(() => toast(t("clipboard.copied"), "ok"))
                      .catch((err) => toast(friendlyError(err), "error"))
                  }
                >
                  {t("clipboard.copy")}
                </button>
              </li>
            )}
          </For>
        </ul>
        <p class="muted small">{t("clipboard.recentHint")}</p>
      </Show>
    </article>
  );
}

/** See the device's screen: watched while it shows (renewed before the 25 s
 *  watch runs out); snapshots arrive as `screen-frame` events and are
 *  drawn on a canvas (never kept). A phone asks its owner first, every time
 *  (Android's own prompt), or offers its screen itself (`screen-offer`:
 *  then this starts by itself). On a computer that takes input, clicks,
 *  scrolls and typing go to it; on a phone, with Control on here and
 *  "Allow control" on at the phone, presses become taps and swipes. */
function ScreenTile(props: { node: string }) {
  const [watching, setWatching] = createSignal(false);
  const [hasFrame, setHasFrame] = createSignal(false);
  const [full, setFull] = createSignal(false);
  const [control, setControl] = createSignal(false);
  let canvas: HTMLCanvasElement | undefined;
  // Full screen covers the page: Tab stays inside it, the Exit button takes
  // focus, and focus goes back to "Full screen" (or Show/Stop) after.
  let stage: HTMLDivElement | undefined;
  let exitFullButton: HTMLButtonElement | undefined;
  let toggleButton: HTMLButtonElement | undefined;
  let restoreFull: (() => void) | undefined;
  const enterFull = () => {
    restoreFull = rememberFocus();
    setFull(true);
    focusSoon(() => exitFullButton);
  };
  const exitFull = () => {
    setFull(false);
    restoreFull?.();
    restoreFull = undefined;
  };
  let renew: ReturnType<typeof setInterval> | undefined;
  let poll: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void)[] = [];

  const st = () => state.status[props.node];
  const isPhone = () => st()?.platform === "android";
  // A computer that takes input: clicks, scrolls and typing on the picture
  // go to it.
  const controls = () => !isPhone() && (st()?.actions.includes("input") ?? false);
  // A phone that takes taps: its helper is on and its owner allows control
  // from here (while it shares); and Control is on here.
  const phoneCanControl = () => isPhone() && !!st()?.screenSend && !!st()?.helper && !!st()?.screenControl;
  const phoneControl = () => phoneCanControl() && control();
  createEffect(() => {
    if (!phoneCanControl()) setControl(false);
  });

  const send = (events: object[]) =>
    commands.sendInput(props.node, JSON.stringify(events)).then((r) => {
      if (r.status === "error") toast(friendlyError(r.error), "error");
    });
  const spot = (v: number, size: number) => Math.max(0, Math.min(10000, Math.round((v / Math.max(1, size)) * 10000)));
  const clickAt = (e: MouseEvent, button: "left" | "right") => {
    if (!controls()) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLCanvasElement;
    el.focus();
    send([
      { kind: "point", x: spot(e.offsetX, el.clientWidth), y: spot(e.offsetY, el.clientHeight) },
      { kind: "click", button },
    ]);
  };
  const wheel = (e: WheelEvent) => {
    if (!controls()) return;
    e.preventDefault();
    send([{ kind: "scroll", dx: Math.round(e.deltaX), dy: Math.round(e.deltaY) }]);
  };
  const keys: Record<string, string> = {
    Enter: "enter", Backspace: "backspace", Escape: "esc",
    ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down",
  };
  const keyDown = (e: KeyboardEvent) => {
    if (!controls() || e.metaKey || e.ctrlKey) return;
    const key = keys[e.key];
    if (key) send([{ kind: "key", key }]);
    else if (e.key.length === 1) send([{ kind: "text", text: e.key }]);
    else return;
    e.preventDefault();
  };

  // A phone: where a press went down; it becomes a tap, long press or
  // swipe when it comes up (the backend decides which).
  let press: { x: number; y: number; at: number } | undefined;
  const pressDown = (e: PointerEvent) => {
    if (!phoneControl() || e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLCanvasElement;
    el.setPointerCapture(e.pointerId);
    press = { x: spot(e.offsetX, el.clientWidth), y: spot(e.offsetY, el.clientHeight), at: performance.now() };
  };
  const pressUp = (e: PointerEvent) => {
    const p = press;
    press = undefined;
    if (!p || !phoneControl()) return;
    const el = e.currentTarget as HTMLCanvasElement;
    const ms = Math.round(performance.now() - p.at);
    commands
      .phoneTouch(props.node, p.x, p.y, spot(e.offsetX, el.clientWidth), spot(e.offsetY, el.clientHeight), ms)
      .then((r) => {
        if (r.status === "error") toast(friendlyError(r.error), "error");
      });
  };
  const phoneKey = (key: "back" | "home" | "recents") =>
    runAction(props.node, "key", { key }).catch((err) => toast(friendlyError(err), "error"));

  // Draw each snapshot as it comes; a newer one replaces it before it's
  // drawn if pictures arrive faster than they decode.
  let latest = 0;
  const draw = (jpeg: string) => {
    const n = ++latest;
    const img = new Image();
    img.onload = () => {
      if (n !== latest || !canvas) return;
      if (canvas.width !== img.naturalWidth || canvas.height !== img.naturalHeight) {
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
      }
      canvas.getContext("2d")?.drawImage(img, 0, 0);
      setHasFrame(true);
    };
    img.src = `data:image/jpeg;base64,${jpeg}`;
  };

  const watch = async (on: boolean) => {
    const r = await commands.screenWatch(props.node, on);
    if (r.status === "error") throw new Error(r.error);
  };
  const stop = () => {
    if (renew) clearInterval(renew);
    renew = undefined;
    if (poll) clearInterval(poll);
    poll = undefined;
    unlisten.forEach((f) => f());
    unlisten = [];
    if (watching()) watch(false).catch(() => {});
    const wasFull = full();
    setWatching(false);
    setHasFrame(false);
    setFull(false);
    restoreFull = undefined;
    if (wasFull) focusSoon(() => toggleButton);
    setControl(false);
    latest++;
  };
  const start = async () => {
    if (watching()) return;
    try {
      unlisten.push(
        await listen<{ from: string; jpeg: string }>("screen-frame", (e) => {
          if (e.payload.from === props.node) draw(e.payload.jpeg);
        }),
        // The phone stopped sharing (its Stop, its screen went off, ...).
        await listen<{ from: string; on: boolean }>("screen-offer", (e) => {
          if (e.payload.from === props.node && !e.payload.on && watching()) {
            stop();
            toast(t("phoneScreen.stopped", { name: peerName(props.node) }), "info");
          }
        }),
      );
      await watch(true);
      setWatching(true);
      renew = setInterval(() => watch(true).catch(() => {}), 10_000);
      // A phone's status says whether its owner allows control now (it
      // isn't pushed): ask again every few seconds while it shows.
      if (isPhone()) {
        requestStatus(props.node).catch(() => {});
        poll = setInterval(() => requestStatus(props.node).catch(() => {}), 4_000);
      }
    } catch (err) {
      stop();
      toast(friendlyError(err), "error");
    }
  };
  // The phone offered its screen (now, or just before this page opened):
  // watch it, once per offer.
  createEffect(() => {
    const at = state.screenOffers[props.node];
    if (!at) return;
    setState("screenOffers", props.node, undefined!);
    if (Date.now() - at < 60_000) start();
  });
  const leaveFull = (e: KeyboardEvent) => {
    if (e.key === "Escape" && full()) exitFull();
  };
  window.addEventListener("keydown", leaveFull);
  onCleanup(() => {
    window.removeEventListener("keydown", leaveFull);
    stop();
  });

  const title = () => (isPhone() ? t("phoneScreen.title") : t("screen.title"));
  return (
    <article class="tile">
      <h2>{title()}</h2>
      <p class="muted small">{isPhone() ? t("phoneScreen.hint") : t("screen.hint")}</p>
      <Show when={watching()}>
        <NearbyBadge node={props.node} />
        <div
          class="screen-stage"
          classList={{ "screen-stage-full": full() }}
          ref={stage}
          role={full() ? "dialog" : undefined}
          aria-modal={full() ? "true" : undefined}
          aria-label={full() ? title() : undefined}
          onKeyDown={(e) => full() && trapTab(e, stage)}
        >
          <Show when={full()}>
            <div class="screen-stage-bar">
              <strong>{title()}</strong>
              <span class="grow" />
              <button class="btn btn-ghost btn-sm" ref={exitFullButton} onClick={exitFull}>
                {t("phoneScreen.exitFull")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={stop}>
                {t("screen.stop")}
              </button>
            </div>
          </Show>
          <Show when={!hasFrame()}>
            <p class="muted small">{isPhone() ? t("phoneScreen.waiting") : t("screen.waiting")}</p>
          </Show>
          <canvas
            ref={canvas}
            class="screen-view"
            classList={{ "screen-control": controls() || phoneControl(), "screen-hidden": !hasFrame() }}
            // Taking input, it passes keys through to the other computer
            // (application); otherwise it's a picture with a name.
            role={controls() ? "application" : "img"}
            aria-label={t(controls() ? "a11y.screenOf" : "a11y.screenView", { name: peerName(props.node) })}
            aria-describedby={controls() ? `screen-hint-${props.node}` : undefined}
            tabIndex={controls() ? 0 : undefined}
            onClick={(e) => clickAt(e, "left")}
            onContextMenu={(e) => clickAt(e, "right")}
            onWheel={wheel}
            onKeyDown={keyDown}
            onPointerDown={pressDown}
            onPointerUp={pressUp}
            onPointerCancel={() => (press = undefined)}
          />
          <Show when={phoneControl()}>
            <div class="row screen-keys">
              <button class="btn btn-ghost btn-sm" onClick={() => phoneKey("back")}>
                {t("phoneScreen.back")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => phoneKey("home")}>
                {t("phoneScreen.home")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => phoneKey("recents")}>
                {t("phoneScreen.recents")}
              </button>
            </div>
          </Show>
        </div>
        <Show when={controls()}>
          <p class="muted small" id={`screen-hint-${props.node}`}>
            {t("screen.controlHint")}
          </p>
        </Show>
        <Show when={isPhone() && st()?.screenSend}>
          <label class="switch-row">
            <span>{t("phoneScreen.control")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={control()}
              disabled={!phoneCanControl()}
              onChange={(e) => setControl(e.currentTarget.checked)}
            />
          </label>
          <p class="muted small">
            {st()?.secureMode
              ? t("control.secureMode", { name: peerName(props.node) })
              : !st()?.helper
              ? t("phoneScreen.controlNeedsHelper")
              : !st()?.screenControl
                ? t("phoneScreen.controlNeedsAllow")
                : control()
                  ? t("phoneScreen.controlOn")
                  : t("phoneScreen.controlOff")}
          </p>
        </Show>
      </Show>
      <div class="row">
        <Show when={watching()}>
          <button class="btn btn-ghost btn-sm" onClick={enterFull}>
            {t("phoneScreen.full")}
          </button>
        </Show>
        <button
          ref={toggleButton}
          class={watching() ? "btn btn-ghost btn-sm" : "btn btn-soft btn-block"}
          onClick={() => (watching() ? stop() : start())}
        >
          {watching() ? t("screen.stop") : isPhone() ? t("phoneScreen.ask") : t("screen.show")}
        </button>
      </div>
    </article>
  );
}

function ControlTile(props: { node: string }) {
  const [url, setUrl] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const status = () => state.status[props.node];

  const ring = async () => {
    setBusy(true);
    try {
      const via = await ringDevice(props.node);
      toast(via === "queued" ? t("control.ringQueued", { name: name() }) : t("control.ringing", { name: name() }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const send = async (e: Event) => {
    e.preventDefault();
    let link = url().trim();
    if (!link) return;
    // "example.com" means the web page.
    if (!/^[a-z][a-z0-9+.-]*:/i.test(link)) link = `https://${link}`;
    setBusy(true);
    try {
      const via = await openUrlOn(props.node, link);
      toast(via === "queued" ? t("control.linkQueued", { name: name() }) : t("control.linkSent", { name: name() }), "ok");
      setUrl("");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  const st = () => status();
  const has = (x: string) => !!st()?.actions.includes(x);
  type Part = "find" | "sound" | "net";
  const parts = (): { id: Part; icon: IconName; label: string }[] => [
    { id: "find", icon: "find", label: t("control.part.find") },
    ...(has("volume") || has("mute") || has("media") || has("ringer") || has("flashlight") || has("screen_settings") || has("key")
      ? [{ id: "sound" as const, icon: "speaker" as const, label: t("control.part.sound") }]
      : []),
    ...(has("radio") || has("wifi") ? [{ id: "net" as const, icon: "wifi" as const, label: t("control.group.radios") }] : []),
  ];
  const [part, setPart] = createSignal<Part>("find");
  return (
    // Find & control as small cards, one job each.
    <div class="control-groups-wrap tile-wide">
      {/* In parts, so the tab fits the window: find and call, sound and
          screen, Wi-Fi and radios. */}
      <Show when={parts().length > 1}>
        <div class="home-filters" role="group" aria-label={t("control.title")}>
          <For each={parts()}>
            {(x) => (
              <button class="filter-chip" aria-pressed={part() === x.id} onClick={() => setPart(x.id)}>
                <Icon name={x.icon} size={16} />
                {x.label}
              </button>
            )}
          </For>
        </div>
      </Show>
    <div class="control-groups tile-wide">
      <Show when={part() === "find"}>
      <Group icon="find" tone="red" title={t("control.group.find")}>
        <p class="muted small">{t("control.hint")}</p>
        <button class="btn btn-soft btn-sm" disabled={busy()} onClick={ring}>
          <Icon name="ring" />
          {t("control.ring")}
        </button>
        <Show when={has("locate") && st()}>{(x) => <LocateRow node={props.node} st={x()} />}</Show>
      </Group>
      </Show>
      <Show when={part() === "sound"}>
      <Show when={st() && (has("volume") || has("mute") || has("media") || has("ringer"))}>
        <Group icon="speaker" tone="green" title={t("control.group.sound")}>
          <SoundControls node={props.node} st={st()!} />
        </Group>
      </Show>
      </Show>
      <Show when={part() === "find"}>
      <Show when={has("dial")}>
        <Group icon="call" tone="green" title={t("control.dial")}>
          <DialRow node={props.node} />
        </Group>
      </Show>
      </Show>
      <Show when={part() === "net"}>
      <Show when={st() && has("radio")}>
        <Group icon="bluetooth" tone="blue" title={t("control.group.radios")}>
          <RadioRow node={props.node} st={st()!} />
        </Group>
      </Show>
      <Show when={st() && has("wifi")}>
        <Group icon="wifi" tone="blue" title={t("control.radio.wifi")}>
          <WifiList node={props.node} st={st()!} />
        </Group>
      </Show>
      </Show>
      <Show when={part() === "sound"}>
      <Show when={st() && (has("flashlight") || has("screen_settings"))}>
        <Group icon="screen" tone="amber" title={t("control.group.display")}>
          <Show when={has("flashlight")}>
            <button
              class="btn btn-soft btn-sm"
              onClick={() =>
                runAction(props.node, "flashlight", { on: !st()!.torch }).catch((e) => toast(friendlyError(e), "error"))
              }
            >
              {st()!.torch ? t("control.torchOff") : t("control.torchOn")}
            </button>
          </Show>
          <Show when={has("screen_settings")}>
            <ScreenSettingsRow node={props.node} st={st()!} />
          </Show>
        </Group>
      </Show>
      <Show when={has("key") && st()}>
        {(x) => (
          <Group icon="keyboard" tone="violet" title={t("control.group.keys")}>
            <KeyRow node={props.node} st={x()} />
          </Group>
        )}
      </Show>
      </Show>
      <Show when={part() === "find"}>
      <Group icon="connect" tone="cyan" title={t("control.openLink")}>
        <form class="stack" onSubmit={send}>
          <div class="composer">
            <input
              type="text"
              inputMode="url"
              placeholder={t("control.linkPlaceholder")}
              value={url()}
              onInput={(e) => setUrl(e.currentTarget.value)}
              aria-label={t("control.linkLabel")}
            />
            <button class="btn btn-soft" type="submit" disabled={busy() || !url().trim()}>
              {t("control.send")}
            </button>
          </div>
        </form>
      </Group>
      </Show>
    </div>
    </div>
  );
}

/** One card of Find & control: an icon, a title, its controls. */
function Group(props: { icon: IconName; tone: Tone; title: string; children: JSX.Element }) {
  return (
    <section class="tile control-group">
      <div class="dash-head">
        <Icon name={props.icon} tone={props.tone} />
        <h3>{props.title}</h3>
      </div>
      {props.children}
    </section>
  );
}

function NotifyTile() {
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  return (
    <article class="tile">
      <h2>{t("notify.title")}</h2>
      <form
        class="stack"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!title().trim()) return;
          await shareNotification(title().trim(), body().trim());
          setTitle("");
          setBody("");
          toast(t("notify.sent"), "ok");
        }}
      >
        <input placeholder={t("notify.titleField")} value={title()} onInput={(e) => setTitle(e.currentTarget.value)} aria-label={t("notify.titleField")} />
        <input placeholder={t("notify.bodyField")} value={body()} onInput={(e) => setBody(e.currentTarget.value)} aria-label={t("notify.bodyLabel")} />
        <button class="btn btn-soft btn-block" type="submit" disabled={!title().trim()}>
          {t("notify.send")}
        </button>
      </form>
    </article>
  );
}

export function AppsTile(props: { node: string }) {
  const [q, setQ] = createSignal("");
  const data = () => state.apps[props.node];
  const name = () => peerName(props.node);
  // Opening an app needs the device to say it can (`launch` in its status).
  const canLaunch = () => !!state.status[props.node]?.actions.includes("launch");
  const shown = () =>
    (data()?.apps ?? []).filter((a) => a.label.toLowerCase().includes(q().toLowerCase())).slice(0, 50);
  return (
    <article class="tile tile-wide">
      <h2>{t("apps.title")}</h2>
      <p class="muted small">{t("apps.hint")}</p>
      <Show
        when={data()}
        fallback={
          <button class="btn btn-soft" onClick={() => requestApps(props.node).catch((e) => toast(friendlyError(e), "error"))}>
            {t("apps.show")}
          </button>
        }
      >
        <input placeholder={t("apps.search")} value={q()} onInput={(e) => setQ(e.currentTarget.value)} aria-label={t("apps.search")} />
        <ul class="app-list">
          <For each={shown()} fallback={<li class="muted small">{t("apps.none")}</li>}>
            {(a) => (
              <li class="app-row">
                <label class="switch-row grow">
                  <span>{a.label}</span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={data()!.subscribed.includes(a.pkg)}
                    onChange={() => toggleApp(props.node, a.pkg).catch((e) => toast(friendlyError(e), "error"))}
                    aria-label={t("apps.notifyLabel", { app: a.label })}
                  />
                </label>
                <Show when={canLaunch()}>
                  <button
                    class="btn btn-ghost btn-sm"
                    onClick={() =>
                      runAction(props.node, "launch", { pkg: a.pkg })
                        .then((via) =>
                          toast(
                            via === "queued"
                              ? t("apps.openQueued", { app: a.label, name: name() })
                              : t("apps.opening", { app: a.label, name: name() }),
                            "ok",
                          ),
                        )
                        .catch((e) => toast(friendlyError(e), "error"))
                    }
                  >
                    {t("apps.open")}
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </article>
  );
}

function FilesTile(props: { node: string }) {
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const list = () =>
    Object.values(state.transfers)
      .filter((x) => x.node === props.node)
      .sort((a, b) => b.ts - a.ts);

  const pick = async () => {
    setBusy(true);
    try {
      if (await pickAndSendFile(props.node)) toast(tn("files.started", 1, { name: name() }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const pickFolder = async () => {
    setBusy(true);
    try {
      const sent = await pickAndSendFolder(props.node);
      if (sent) toast(tn("files.started", sent, { name: name() }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="tile">
      <h2>{t("files.title")}</h2>
      <p class="muted small">{t("files.hint")}</p>
      <div class="row">
        <button class="btn btn-soft grow" disabled={busy()} onClick={pick}>
          {busy() ? t("files.choosing") : t("files.send")}
        </button>
        <button class="btn btn-soft grow" disabled={busy()} onClick={pickFolder}>
          {t("files.sendFolder")}
        </button>
      </div>
      <Show when={list().length} fallback={<p class="muted small">{t("files.empty")}</p>}>
        <ul class="transfers">
          <For each={list()}>{(x) => <TransferRow x={x} device={name()} />}</For>
        </ul>
      </Show>
    </article>
  );
}

function TransferRow(props: { x: Transfer; device: string }) {
  const active = () => ["waiting", "sending", "receiving"].includes(props.x.state);
  const pct = () => (props.x.total ? Math.min(100, (props.x.done / props.x.total) * 100) : 0);
  const line = () => {
    const x = props.x;
    const done = humanSize(x.done);
    const total = humanSize(x.total);
    switch (x.state) {
      case "waiting":
        return t("files.waiting", { name: props.device });
      case "sending":
        return t("files.sending", { done, total });
      case "receiving":
        return t("files.receiving", { done, total });
      case "sent":
        return t("files.sent", { size: total });
      case "saved":
        return t("files.saved", { size: total });
      default:
        return t("files.failed", { reason: fileFailReason(x.reason, x.incoming) });
    }
  };
  const run = (f: () => Promise<void>) => () => f().catch((e) => toast(friendlyError(e), "error"));
  return (
    <li class="transfer">
      <div class="transfer-head">
        <strong class="transfer-name">{props.x.name}</strong>
        <Show when={active()}>
          <button
            class="btn btn-ghost btn-sm"
            onClick={run(() => cancelFile(props.x.id))}
            aria-label={t("files.cancelLabel", { name: props.x.name })}
          >
            {t("files.cancel")}
          </button>
        </Show>
      </div>
      <Show when={active()}>
        <div
          class="progress"
          role="progressbar"
          aria-label={t("a11y.transferProgress", { name: props.x.name })}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(pct())}
          aria-valuetext={line()}
        >
          <span style={{ width: `${pct()}%` }} />
        </div>
      </Show>
      <p class="muted small" classList={{ "text-error": props.x.state === "failed" }}>
        {line()}
      </p>
      <Show when={props.x.state === "saved" && props.x.path}>
        <div class="row">
          <button class="btn btn-ghost btn-sm" onClick={run(() => openFile(props.x.path!))}>
            {t("files.open")}
          </button>
          <button class="btn btn-ghost btn-sm" onClick={run(() => revealFile(props.x.path!))}>
            {t("files.show")}
          </button>
        </div>
      </Show>
    </li>
  );
}

/** Buttons for the actions a device listed in its status. */
/** Volume, mute, play/pause and the ringer mode. */
function SoundControls(props: { node: string; st: DeviceStatus }) {
  const has = (a: string) => props.st.actions.includes(a);
  const run = (name: string, args: Record<string, unknown>) => () =>
    runAction(props.node, name, args).catch((e) => toast(friendlyError(e), "error"));
  const ringers = [
    ["normal", "control.ringerNormal"],
    ["vibrate", "control.ringerVibrate"],
    ["silent", "control.ringerSilent"],
  ] as const;
  return (
    <div class="stack">
      <Show when={has("volume") || has("mute")}>
        <div class="row">
          <span class="small muted grow">
            {props.st.volume !== undefined ? t("control.volumeLevel", { level: props.st.volume }) : t("control.volume")}
          </span>
          <Show when={has("volume")}>
            <button class="btn btn-ghost btn-sm" onClick={run("volume", { step: -1 })}>
              {t("control.quieter")}
            </button>
            <button class="btn btn-ghost btn-sm" onClick={run("volume", { step: 1 })}>
              {t("control.louder")}
            </button>
          </Show>
          <Show when={has("mute")}>
            <button class="btn btn-ghost btn-sm" onClick={run("mute", { on: !props.st.muted })}>
              {props.st.muted ? t("control.unmute") : t("control.mute")}
            </button>
          </Show>
        </div>
      </Show>
      <Show when={has("media")}>
        <div class="row">
          <button class="btn btn-ghost btn-sm grow" onClick={run("media", { key: "previous" })}>
            {t("control.previous")}
          </button>
          <button class="btn btn-ghost btn-sm grow" onClick={run("media", { key: "play_pause" })}>
            {t("control.playPause")}
          </button>
          <button class="btn btn-ghost btn-sm grow" onClick={run("media", { key: "next" })}>
            {t("control.next")}
          </button>
        </div>
      </Show>
      <Show when={has("ringer")}>
        <div class="row" role="group" aria-label={t("control.ringerLabel")}>
          <For each={ringers}>
            {([mode, label]) => (
              <button
                class="btn btn-sm grow"
                classList={{ "btn-soft": props.st.ringer === mode, "btn-ghost": props.st.ringer !== mode }}
                aria-pressed={props.st.ringer === mode}
                onClick={run("ringer", { mode })}
              >
                {t(label)}
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** Wi-Fi, Bluetooth, mobile data, hotspot, airplane mode: what the phone
 *  reported, and a tap to switch one. Android lets an app switch few of
 *  them itself, so the phone usually opens the switch for you to tap. */
function RadioRow(props: { node: string; st: DeviceStatus }) {
  const label = (r: Radio) => t(`control.radio.${r}` as const);
  const flip = (r: Radio) => () => {
    const on = !props.st.radios[r];
    const direct = props.st.radioDirect.includes(r);
    const name = peerName(props.node);
    runAction(props.node, "radio", { what: r, on })
      .then(() => toast(t(direct ? "control.radioSwitching" : "control.radioAsked", { name, radio: label(r) })))
      .catch((e) => toast(friendlyError(e), "error"));
  };
  return (
    <div class="stack">
      <div class="row" role="group" aria-label={t("control.radioLabel")}>
        <For each={RADIOS.filter((r) => props.st.radios[r] !== undefined)}>
          {(r) => (
            <button
              class="btn btn-sm"
              classList={{ "btn-soft": props.st.radios[r], "btn-ghost": !props.st.radios[r] }}
              aria-pressed={props.st.radios[r]}
              onClick={flip(r)}
            >
              {label(r)}
            </button>
          )}
        </For>
      </div>
      <p class="muted small">
        {props.st.radioDirect.length > 0 ? t("control.radioDirectHint") : t("control.radioPanelHint")}
      </p>
    </div>
  );
}

/** Where the phone is (its owner allows it on the phone, and sees a
 *  notification each time). */
function LocateRow(props: { node: string; st: DeviceStatus }) {
  const loc = () => props.st.location;
  const name = () => peerName(props.node);
  const off = () => (props.st.locationSharing ? undefined : t("locate.offOn", { name: name() }));
  const ago = () => {
    const l = loc();
    if (!l) return "";
    const mins = Math.max(0, Math.round((Date.now() / 1000 - l.at) / 60));
    return mins < 1 ? t("locate.justNow") : t("locate.minsAgo", { mins });
  };
  const locate = () =>
    runAction(props.node, "locate", {})
      .then(() => toast(t("locate.asking")))
      .catch((e) => toast(friendlyError(e), "error"));
  return (
    <div class="row">
      <span class="small muted grow">
        <Show when={loc()} fallback={t("locate.title")}>
          {(l) => t("locate.where", { lat: l().lat.toFixed(5), lon: l().lon.toFixed(5), acc: Math.round(l().acc), ago: ago() })}
        </Show>
      </span>
      <Gate reason={off()}>
        <button class="btn btn-ghost btn-sm" disabled={!!off()} onClick={locate}>
          {t("locate.locate")}
        </button>
      </Gate>
      <Show when={loc()}>
        {(l) => (
          <button
            class="btn btn-ghost btn-sm"
            onClick={async () => {
              const r = await commands.openMap(l().lat, l().lon);
              if (r.status === "error") toast(friendlyError(r.error), "error");
            }}
          >
            {t("locate.map")}
          </button>
        )}
      </Show>
    </div>
  );
}

/** The phone's nearby Wi-Fi networks: scan, join one (a password for a
 *  new secured network), leave or rejoin the current one. */
function WifiList(props: { node: string; st: DeviceStatus }) {
  const [asking, setAsking] = createSignal<string | null>(null);
  const [password, setPassword] = createSignal("");
  // This computer's own Wi-Fi, to hand to the phone (its password is read
  // and sent in Rust: it never comes to this page).
  const [mine, setMine] = createSignal<string | null>(null);
  onMount(() => commands.currentWifiName().then(setMine, () => {}));
  const name = () => peerName(props.node);
  const shareMine = () =>
    unwrap(commands.shareWifiTo(props.node)).then(
      (ssid) => toast(t("wifi.shareSent", { ssid, name: name() }), "ok"),
      (e) => toast(friendlyError(e), "error"),
    );
  const wifi = (args: Record<string, unknown>) =>
    runAction(props.node, "wifi", args).catch((e) => toast(friendlyError(e), "error"));
  const join = (n: WifiNetwork) => {
    if (n.secure && !n.saved) {
      setPassword("");
      setAsking(n.ssid);
    } else {
      wifi({ op: "connect", ssid: n.ssid });
      toast(t("wifi.joining", { ssid: n.ssid }));
    }
  };
  const sendPassword = (e: Event) => {
    e.preventDefault();
    const ssid = asking();
    if (!ssid || password().length < 8) return;
    wifi({ op: "connect", ssid, password: password() });
    toast(t("wifi.joining", { ssid }));
    setAsking(null);
  };
  return (
    <div class="stack">
      <div class="row">
        <span class="small muted grow">
          {props.st.wifiSsid ? t("wifi.on", { ssid: props.st.wifiSsid }) : t("wifi.title")}
        </span>
        <button class="btn btn-ghost btn-sm" onClick={() => wifi({ op: "scan" })}>
          {props.st.wifiNetworks ? t("wifi.rescan") : t("wifi.scan")}
        </button>
        <Show when={props.st.wifiSsid}>
          <button class="btn btn-ghost btn-sm" onClick={() => wifi({ op: "disconnect" })}>
            {t("wifi.disconnect")}
          </button>
        </Show>
        <Show when={!props.st.wifiSsid && props.st.radios.wifi}>
          <button class="btn btn-ghost btn-sm" onClick={() => wifi({ op: "reconnect" })}>
            {t("wifi.reconnect")}
          </button>
        </Show>
      </div>
      <Show when={mine()}>
        {(ssid) => (
          <button class="btn btn-soft btn-sm" title={t("wifi.shareHint", { name: name() })} onClick={shareMine}>
            {t("wifi.share", { ssid: ssid(), name: name() })}
          </button>
        )}
      </Show>
      <Show when={props.st.wifiScanNeeds}>
        <p class="muted small">
          {props.st.wifiScanNeeds === "location" ? t("wifi.needsLocation") : t("wifi.needsPermission")}
        </p>
      </Show>
      <Show when={props.st.wifiNetworks}>
        {(nets) => (
          <ul class="wifi-list">
            <For each={nets()} fallback={<li class="muted small">{t("wifi.none")}</li>}>
              {(n) => (
                <li class="row">
                  <span class="grow">
                    {n.ssid}
                    <span class="muted small">
                      {" "}
                      {/* Bars and a padlock to see; words to hear. */}
                      <span aria-hidden="true">
                        {"▂▄▆█".slice(0, Math.max(1, n.signal))}
                        {n.secure ? " 🔒" : ""}
                      </span>
                      <span class="sr-only">
                        {t("a11y.wifiSignal", { n: Math.max(1, Math.min(4, n.signal)) })}
                        {n.secure ? `, ${t("a11y.wifiSecure")}` : ""}
                      </span>
                      {n.current ? ` · ${t("wifi.connected")}` : n.saved ? ` · ${t("wifi.saved")}` : ""}
                    </span>
                  </span>

                  <Show when={!n.current}>
                    <button class="btn btn-ghost btn-sm" onClick={() => join(n)}>
                      {t("wifi.join")}
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        )}
      </Show>
      <Show when={asking()}>
        {(ssid) => (
          <form class="row" onSubmit={sendPassword}>
            <input
              class="grow"
              type="password"
              placeholder={t("wifi.password", { ssid: ssid() })}
              aria-label={t("wifi.password", { ssid: ssid() })}
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
            />
            <button class="btn btn-soft btn-sm" type="submit" disabled={password().length < 8}>
              {t("wifi.join")}
            </button>
            <button class="btn btn-ghost btn-sm" type="button" onClick={() => setAsking(null)}>
              {t("wifi.cancel")}
            </button>
          </form>
        )}
      </Show>
    </div>
  );
}

/** Back, Home, recent apps, notifications, Quick Settings, Lock on the
 *  phone (its Accessibility service presses them). */
function KeyRow(props: { node: string; st: DeviceStatus }) {
  const off = () =>
    props.st.secureMode
      ? t("control.secureMode", { name: peerName(props.node) })
      : props.st.helper === false
        ? t("control.keysOff", { name: peerName(props.node) })
        : undefined;
  const keys = [
    ["back", "control.key.back"],
    ["home", "control.key.home"],
    ["recents", "control.key.recents"],
    ["notifications", "control.key.notifications"],
    ["quick_settings", "control.key.quickSettings"],
    ["lock", "control.key.lock"],
  ] as const;
  const press = (key: string) => () =>
    runAction(props.node, "key", { key }).catch((e) => toast(friendlyError(e), "error"));
  return (
    <>
      <div class="row" role="group" aria-label={t("control.keyLabel")}>
        <For each={keys}>
          {([key, label]) => (
            <Gate reason={off()}>
              <button class="btn btn-ghost btn-sm" disabled={!!off()} onClick={press(key)}>
                {t(label)}
              </button>
            </Gate>
          )}
        </For>
      </div>
      {/* The phone helper is off on purpose (banking apps): say so. */}
      <Show when={props.st.secureMode}>
        <p class="muted small">{t("control.helperSecure")}</p>
      </Show>
    </>
  );
}

/** Type a number; the phone opens its dialer with it (it doesn't call). */
function DialRow(props: { node: string }) {
  const [number, setNumber] = createSignal("");
  const name = () => peerName(props.node);
  const send = (e: Event) => {
    e.preventDefault();
    const n = number().replace(/[^0-9+*#]/g, "");
    if (!n) return;
    runAction(props.node, "dial", { number: n })
      .then((via) => {
        toast(via === "queued" ? t("control.dialQueued", { name: name() }) : t("control.dialSent", { name: name() }), "ok");
        setNumber("");
      })
      .catch((err) => toast(friendlyError(err), "error"));
  };
  return (
    <form class="composer" onSubmit={send}>
      <input
        type="tel"
        placeholder={t("control.dialPlaceholder")}
        value={number()}
        onInput={(e) => setNumber(e.currentTarget.value)}
        aria-label={t("control.dialLabel")}
      />
      <button class="btn btn-soft" type="submit" disabled={!number().trim()}>
        {t("control.dial")}
      </button>
    </form>
  );
}
