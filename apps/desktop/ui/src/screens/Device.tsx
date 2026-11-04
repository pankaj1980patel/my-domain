// One device: its status, then a tile per feature.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import {
  cancelFile,
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
  state,
  toggleApp,
  RADIOS,
  type DeviceStatus,
  type PhoneNotification,
  type Radio,
  type WifiNetwork,
  type Transfer,
  unwrap,
} from "../lib/core";
import { fileFailReason, friendlyError, humanSize, initial, niceName, progressLabel } from "../lib/words";
import { Gate, linkStateOf, StatusPill, toast } from "../components/ui";
import ScreenSettingsRow from "./device/ScreenSettingsRow";
import { t, tn } from "../i18n";
import PhotosTile from "./device/PhotosTile";
import BrowseTile from "./device/BrowseTile";
import DropToSend from "./device/DropToSend";
import InfoTile from "./device/InfoTile";

export default function Device(props: { node: string; onBack: () => void }) {
  const peer = createMemo(() => state.peers.find((p) => p.node_id === props.node));
  const s = () => linkStateOf(props.node);
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
            <div class="device-avatar lg" aria-hidden="true">
              {initial(p().name)}
            </div>
            <div class="device-main">
              <h1>{niceName(p().name)}</h1>
              <StatusPill node={props.node} />
              <Show when={s() === "connecting" || s() === "failed"}>
                <p class="muted small">{progressLabel(state.progress[props.node])}</p>
              </Show>
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

          <div class="tiles">
            <ChatTile node={props.node} />
            <NotificationsTile node={props.node} />
            <ClipboardTile node={props.node} />
            <FilesTile node={props.node} />
            <DropToSend node={props.node} />
            <BrowseTile node={props.node} />
            <ControlTile node={props.node} />
            <InfoTile node={props.node} />
            <Show when={state.status[props.node]?.actions.includes("screen")}>
              <ScreenTile node={props.node} />
            </Show>
            <Show when={state.status[props.node]?.actions.includes("gallery")}>
              <PhotosTile node={props.node} />
            </Show>
            <NotifyTile />
            <AppsTile node={props.node} />
          </div>
        </section>
      )}
    </Show>
  );
}

function ChatTile(props: { node: string }) {
  const [text, setText] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const thread = () => state.messages.filter((m) => m.node === props.node);
  // Only the newest delivered message of ours gets the marker.
  const lastDelivered = () => thread().filter((m) => m.dir === "out" && m.delivered).pop()?.id;
  let list: HTMLDivElement | undefined;

  const send = async (e: Event) => {
    e.preventDefault();
    const t = text().trim();
    if (!t) return;
    setBusy(true);
    try {
      await sendMessage(props.node, t);
      setText("");
      queueMicrotask(() => list?.scrollTo({ top: list.scrollHeight }));
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
          <button class="btn btn-ghost btn-sm" title={t("chat.clearHint")} onClick={() => clearChat(props.node)}>
            {t("chat.clear")}
          </button>
        </Show>
      </div>
      <div class="thread" ref={list}>
        <Show when={thread().length > 0} fallback={<p class="muted small center">{t("chat.empty")}</p>}>
          <For each={thread()}>
            {(m) => (
              <div class={`bubble bubble-${m.dir}`} classList={{ "bubble-pending": !!m.pending }}>
                <span>{m.text}</span>
                <Show when={m.pending}>
                  <small class="bubble-note">{m.via === "queued" ? t("chat.queued") : t("chat.sending")}</small>
                </Show>
                <Show when={m.delivered && m.id === lastDelivered()}>
                  <small class="bubble-note">{t("chat.delivered")}</small>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
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
  const name = () => niceName(peerName(props.n.from));
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
          else toast(t("clipboard.pictureSent", { name: niceName(peerName(props.node)) }), "ok");
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
                  <span class="muted small">{e.from ? niceName(peerName(e.from)) : t("clipboard.fromHere")}</span>
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

/** See the device's screen: watched while "Show" is on (renewed before the
 *  25 s watch runs out); snapshots arrive as `screen-frame` events. A phone
 *  asks its owner first, every time. */
function ScreenTile(props: { node: string }) {
  const [watching, setWatching] = createSignal(false);
  const [frame, setFrame] = createSignal<string>();
  let renew: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void) | undefined;

  // A computer that takes input: clicks, scrolls and typing on the picture
  // go to it.
  const controls = () => state.status[props.node]?.actions.includes("input") ?? false;
  const send = (events: object[]) =>
    commands.sendInput(props.node, JSON.stringify(events)).then((r) => {
      if (r.status === "error") toast(friendlyError(r.error), "error");
    });
  const clickAt = (e: MouseEvent, button: "left" | "right") => {
    if (!controls()) return;
    e.preventDefault();
    const img = e.currentTarget as HTMLImageElement;
    img.focus();
    const spot = (v: number, size: number) => Math.max(0, Math.min(10000, Math.round((v / size) * 10000)));
    send([
      { kind: "point", x: spot(e.offsetX, img.clientWidth), y: spot(e.offsetY, img.clientHeight) },
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

  const watch = async (on: boolean) => {
    const r = await commands.screenWatch(props.node, on);
    if (r.status === "error") throw new Error(r.error);
  };
  const stop = () => {
    if (renew) clearInterval(renew);
    renew = undefined;
    unlisten?.();
    unlisten = undefined;
    if (watching()) watch(false).catch(() => {});
    setWatching(false);
    setFrame(undefined);
  };
  const start = async () => {
    try {
      unlisten = await listen<{ from: string; jpeg: string }>("screen-frame", (e) => {
        if (e.payload.from === props.node) setFrame(`data:image/jpeg;base64,${e.payload.jpeg}`);
      });
      await watch(true);
      setWatching(true);
      renew = setInterval(() => watch(true).catch(() => {}), 10_000);
    } catch (err) {
      stop();
      toast(friendlyError(err), "error");
    }
  };
  onCleanup(stop);

  return (
    <article class="tile">
      <h2>{t("screen.title")}</h2>
      <p class="muted small">{t("screen.hint")}</p>
      <Show when={watching() && frame()} fallback={<Show when={watching()}><p class="muted small">{t("screen.waiting")}</p></Show>}>
        <img
          class="screen-view"
          classList={{ "screen-control": controls() }}
          src={frame()}
          alt={t("screen.alt")}
          tabIndex={controls() ? 0 : undefined}
          onClick={(e) => clickAt(e, "left")}
          onContextMenu={(e) => clickAt(e, "right")}
          onWheel={wheel}
          onKeyDown={keyDown}
        />
        <Show when={controls()}>
          <p class="muted small">{t("screen.controlHint")}</p>
        </Show>
      </Show>
      <button class={watching() ? "btn btn-ghost btn-sm" : "btn btn-soft btn-block"} onClick={() => (watching() ? stop() : start())}>
        {watching() ? t("screen.stop") : t("screen.show")}
      </button>
    </article>
  );
}

function ControlTile(props: { node: string }) {
  const [url, setUrl] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const name = () => niceName(peerName(props.node));
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
  const name = () => niceName(peerName(props.node));
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
  const name = () => niceName(peerName(props.node));
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

  return (
    <article class="tile">
      <h2>{t("files.title")}</h2>
      <p class="muted small">{t("files.hint")}</p>
      <button class="btn btn-soft btn-block" disabled={busy()} onClick={pick}>
        {busy() ? t("files.choosing") : t("files.send")}
      </button>
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
        <div class="progress">
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
    const name = niceName(peerName(props.node));
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
  const name = () => niceName(peerName(props.node));
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
  const name = () => niceName(peerName(props.node));
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
                      {"▂▄▆█".slice(0, Math.max(1, n.signal))}
                      {n.secure ? " 🔒" : ""}
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
    props.st.helper === false ? t("control.keysOff", { name: niceName(peerName(props.node)) }) : undefined;
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
  const name = () => niceName(peerName(props.node));
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
