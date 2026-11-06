// One device: its status, then a tile per feature.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
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
  notificationAction,
  openFile,
  openUrlOn,
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
  type PhoneNotification,
  type Radio,
  type WifiNetwork,
  type Transfer,
  unwrap,
} from "../lib/core";
import { fileFailReason, friendlyError, humanSize, progressLabel } from "../lib/words";
import { Gate, linkStateOf, StatusPill, toast } from "../components/ui";
import { DeviceAvatar, DeviceLabelForm } from "../components/DeviceLabel";
import KeyChangeBanner from "../components/KeyChangeBanner";
import ScreenSettingsRow from "./device/ScreenSettingsRow";
import { t, tn } from "../i18n";
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
import WakeButton from "./device/WakeButton";
import ConnectionTile from "./device/ConnectionTile";
import { moveOnto, moveStep, pageKind, type PageKind, type TileDef } from "../lib/pageLayout";
import type { PageLayoutView } from "../bindings";

/** Every tile a device's page can show, in default order. A new tile is one
 *  entry here (its `id` is kept in saved layouts: never rename one); a saved
 *  layout that predates it shows it at this place. */
const TILES: TileDef[] = [
  { id: "chat", title: () => t("chat.title"), wide: true, render: (n) => <ChatTile node={n} /> },
  { id: "notifications", title: () => t("notifications.title"), wide: true, render: (n) => <NotificationsTile node={n} /> },
  { id: "notification-history", title: () => t("history.title"), wide: true, render: (n) => <NotificationHistory node={n} /> },
  { id: "clipboard", title: () => t("clipboard.title"), render: (n) => <ClipboardTile node={n} /> },
  { id: "clipboard-history", title: () => t("clipHistory.title"), wide: true, render: (n) => <ClipHistoryTile node={n} /> },
  { id: "files", title: () => t("files.title"), render: (n) => <FilesTile node={n} /> },
  { id: "browse", title: () => t("browse.title"), wide: true, render: (n) => <BrowseTile node={n} /> },
  { id: "sync", title: () => t("sync.title"), render: (n) => <SyncTile node={n} /> },
  { id: "control", title: () => t("control.title"), render: (n) => <ControlTile node={n} /> },
  { id: "info", title: () => t("info.title"), render: (n) => <InfoTile node={n} /> },
  { id: "connection", title: () => t("conn.title"), wide: true, render: (n) => <ConnectionTile node={n} /> },
  { id: "schedules", title: () => t("schedule.title"), wide: true, render: (n) => <ScheduleTile node={n} /> },
  {
    id: "commands",
    title: () => t("run.title"),
    available: (n) => !!state.status[n]?.actions.includes("run"),
    render: (n) => <CommandsTile node={n} />,
  },
  {
    id: "screen",
    title: (n) => (state.status[n]?.platform === "android" ? t("phoneScreen.title") : t("screen.title")),
    available: (n) => !!state.status[n]?.actions.includes("screen"),
    render: (n) => <ScreenTile node={n} />,
  },
  {
    // A phone that can show its camera here (it says camera_view).
    id: "camera",
    title: () => t("camera.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.cameraView && state.status[n]?.platform === "android",
    render: (n) => <CameraTile node={n} />,
  },
  {
    id: "phone-audio",
    title: () => t("phoneAudio.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.audioSource && state.status[n]?.platform === "android",
    render: (n) => <PhoneAudioTile node={n} />,
  },
  {
    // A phone that plays this computer's sound (it says audio_sink).
    id: "computer-audio",
    title: () => t("computerAudio.title"),
    kinds: ["phone"],
    available: (n) => !!state.status[n]?.audioSink && state.status[n]?.platform === "android",
    render: (n) => <ComputerAudioTile node={n} />,
  },
  {
    id: "photos",
    title: () => t("photos.title"),
    wide: true,
    available: (n) => !!state.status[n]?.actions.includes("gallery"),
    render: (n) => <PhotosTile node={n} />,
  },
  { id: "lost-mode", title: () => t("lost.title"), render: (n) => <LostModeTile node={n} /> },
  { id: "notify", title: () => t("notify.title"), render: () => <NotifyTile /> },
  { id: "apps", title: () => t("apps.title"), wide: true, render: (n) => <AppsTile node={n} /> },
];

export default function Device(props: { node: string; onBack: () => void }) {
  const peer = createMemo(() => state.peers.find((p) => p.node_id === props.node));
  const s = () => linkStateOf(props.node);
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
        <section class="page">
          <button class="link back" onClick={props.onBack}>
            {t("device.back")}
          </button>
          <header class="device-head">
            <DeviceAvatar node={props.node} large />
            <div class="device-main">
              <div class="label-title">
                <h1>{peerName(p().node_id)}</h1>
                <button
                  class="btn btn-ghost btn-sm"
                  title={t("label.editHint")}
                  aria-expanded={editing()}
                  onClick={() => setEditing((v) => !v)}
                >
                  {t("label.edit")}
                </button>
                <button
                  class="btn btn-ghost btn-sm"
                  title={t("layout.editHint")}
                  aria-pressed={arranging()}
                  onClick={() => setArranging((v) => !v)}
                >
                  {arranging() ? t("layout.done") : t("layout.edit")}
                </button>
              </div>
              <StatusPill node={props.node} />
              <Show when={s() === "connecting" || s() === "failed"}>
                <p class="muted small">{progressLabel(state.progress[props.node])}</p>
              </Show>
              {/* Asleep? Wake it, if it said it may be woken. */}
              <WakeButton node={props.node} />
            </div>
            <Show when={s() === "offline" || s() === "failed"}>
              <button
                class="btn btn-primary"
                onClick={() => connect(props.node).catch((e) => toast(friendlyError(e), "error"))}
              >
                {t("device.connect")}
              </button>
            </Show>
            <Show when={s() === "direct" || s() === "relay"}>
              <div class="row">
                <button
                  class="btn btn-ghost btn-sm"
                  title={t("device.reconnectHint")}
                  onClick={() => reconnect(props.node).catch((e) => toast(friendlyError(e), "error"))}
                >
                  {t("device.reconnect")}
                </button>
                <button
                  class="btn btn-ghost btn-sm"
                  title={t("device.disconnectHint")}
                  onClick={() => disconnect(props.node).catch((e) => toast(friendlyError(e), "error"))}
                >
                  {t("device.disconnect")}
                </button>
              </div>
            </Show>
          </header>
          <KeyChangeBanner node={props.node} />
          <Show when={editing()}>
            <DeviceLabelForm node={props.node} onClose={() => setEditing(false)} />
          </Show>

          <DropToSend node={props.node} />
          <Show
            when={arranging()}
            fallback={
              <div class="tiles">
                <For each={shownTiles()}>
                  {(d) => <Show when={d.available?.(props.node) ?? true}>{d.render(props.node)}</Show>}
                </For>
                <Show when={layout() && shownTiles().length === 0}>
                  <p class="muted small">{t("layout.allHidden")}</p>
                </Show>
              </div>
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

function ChatTile(props: { node: string }) {
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
    <article class="tile tile-wide">
      <div class="tile-head">
        <h2>{t("chat.title")}</h2>
        <Show when={thread().length > 0}>
          <button class="btn btn-ghost btn-sm" title={t("chat.clearHint")} onClick={() => clearChat(props.node).catch(() => {})}>
            {t("chat.clear")}
          </button>
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

function NotificationsTile(props: { node: string }) {
  const list = () => state.notifications[props.node] ?? [];
  return (
    <article class="tile tile-wide">
      <h2>{t("notifications.title")}</h2>
      <Show when={list().length} fallback={<p class="muted small">{t("notifications.empty")}</p>}>
        <ul class="notif-list">
          <For each={list()}>{(n) => <NotificationRow n={n} />}</For>
        </ul>
        <p class="muted small">{t("notifications.hint")}</p>
      </Show>
    </article>
  );
}

function NotificationRow(props: { n: PhoneNotification }) {
  const [reply, setReply] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.n.from);
  const actionable = () => !!props.n.key && !props.n.handled;
  // Buttons keep their index in the device's list; the first `reply` one
  // becomes the reply box.
  const buttons = () => props.n.actions.map((a, i) => ({ ...a, i })).filter((a) => !a.reply);
  const replyIndex = () => props.n.actions.findIndex((a) => a.reply);

  const act = async (index: number, text?: string) => {
    setBusy(true);
    try {
      const via = await notificationAction(props.n, index, text);
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

  const send = (e: Event) => {
    e.preventDefault();
    const text = reply().trim();
    if (text) act(replyIndex(), text);
  };

  return (
    <li class="notif" classList={{ "notif-handled": !!props.n.handled }}>
      <div class="row">
        <span class="small muted grow notif-app">{props.n.app || name()}</span>
        <Show when={props.n.handled}>
          <span class="small muted">{t("notifications.handled")}</span>
        </Show>
        <button
          class="btn btn-ghost btn-sm"
          disabled={busy()}
          onClick={() => act(-1)}
          aria-label={t("notifications.dismissLabel", { title: props.n.title })}
        >
          {t("notifications.dismiss")}
        </button>
      </div>
      <strong class="notif-title">{props.n.title}</strong>
      <Show when={props.n.body}>
        <p class="notif-body small">{props.n.body}</p>
      </Show>
      <Show when={actionable() && buttons().length}>
        <div class="row">
          <For each={buttons()}>
            {(b) => (
              <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => act(b.i)}>
                {b.title}
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={actionable() && replyIndex() >= 0}>
        <form class="composer" onSubmit={send}>
          <input
            placeholder={props.n.actions[replyIndex()]?.title || t("notifications.replyPlaceholder")}
            value={reply()}
            onInput={(e) => setReply(e.currentTarget.value)}
            aria-label={t("notifications.replyLabel", { title: props.n.title })}
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
      <button
        class="btn btn-soft btn-block"
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
        {busy() ? t("clipboard.getting") : t("clipboard.get")}
      </button>
      <button
        class="btn btn-soft btn-block"
        onClick={async () => {
          const r = await commands.sendClipboardImage(props.node);
          if (r.status === "error") toast(friendlyError(r.error), "error");
          else toast(t("clipboard.pictureSent", { name: peerName(props.node) }), "ok");
        }}
      >
        {t("clipboard.sendPicture")}
      </button>
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
          <button class="btn btn-ghost btn-sm" onClick={() => clearClipHistory()}>
            {t("clipboard.clear")}
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
            {!st()?.helper
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

  return (
    <article class="tile">
      <h2>{t("control.title")}</h2>
      <p class="muted small">{t("control.hint")}</p>
      <Show when={status()?.actions.length}>
        <Controls node={props.node} st={status()!} />
      </Show>
      <button class="btn btn-soft btn-block" disabled={busy()} onClick={ring}>
        {t("control.ring")}
      </button>
      <form class="stack" onSubmit={send}>
        <span class="small">{t("control.openLink")}</span>
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
    </article>
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

function AppsTile(props: { node: string }) {
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
function Controls(props: { node: string; st: DeviceStatus }) {
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
      <Show when={has("flashlight")}>
        <button class="btn btn-soft btn-block" onClick={run("flashlight", { on: !props.st.torch })}>
          {props.st.torch ? t("control.torchOff") : t("control.torchOn")}
        </button>
      </Show>
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
      <Show when={has("dial")}>
        <DialRow node={props.node} />
      </Show>
      <Show when={has("radio")}>
        <RadioRow node={props.node} st={props.st} />
      </Show>
      <Show when={has("key")}>
        <KeyRow node={props.node} st={props.st} />
      </Show>
      <Show when={has("wifi")}>
        <WifiList node={props.node} st={props.st} />
      </Show>
      <Show when={has("locate")}>
        <LocateRow node={props.node} st={props.st} />
      </Show>
      <Show when={has("screen_settings")}>
        <ScreenSettingsRow node={props.node} st={props.st} />
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
    props.st.helper === false ? t("control.keysOff", { name: peerName(props.node) }) : undefined;
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
