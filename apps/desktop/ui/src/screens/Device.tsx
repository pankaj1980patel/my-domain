// One device: its status, then a tile per feature.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  cancelFile,
  clearClipHistory,
  connect,
  copyText,
  loadClipHistory,
  openFile,
  openUrlOn,
  peerName,
  pickAndSendFile,
  pullClipboard,
  requestApps,
  requestStatus,
  revealFile,
  ringDevice,
  runAction,
  sendFile,
  sendMessage,
  setClipSync,
  shareNotification,
  state,
  toggleApp,
  type DeviceStatus,
  type Transfer,
} from "../lib/core";
import { fileFailReason, friendlyError, humanSize, initial, niceName, progressLabel } from "../lib/words";
import { linkStateOf, StatusPill, toast } from "../components/ui";
import { t, tn } from "../i18n";

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
          </header>

          <div class="tiles">
            <ChatTile node={props.node} />
            <ClipboardTile node={props.node} />
            <FilesTile node={props.node} />
            <ControlTile node={props.node} />
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
      <h2>{t("chat.title")}</h2>
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

function ControlTile(props: { node: string }) {
  const [url, setUrl] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const name = () => niceName(peerName(props.node));
  const status = () => state.status[props.node];
  const battery = () => {
    const st = status();
    if (!st) return undefined;
    if (st.battery === undefined) return t("control.noBattery");
    return st.charging ? t("control.batteryCharging", { level: st.battery }) : t("control.battery", { level: st.battery });
  };

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
      <Show when={battery()}>
        <p class="small">{battery()}</p>
      </Show>
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
              <li>
                <label class="switch-row">
                  <span>{a.label}</span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={data()!.subscribed.includes(a.pkg)}
                    onChange={() => toggleApp(props.node, a.pkg).catch((e) => toast(friendlyError(e), "error"))}
                  />
                </label>
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
  const [dropping, setDropping] = createSignal(false);
  const name = () => niceName(peerName(props.node));
  const list = () =>
    Object.values(state.transfers)
      .filter((x) => x.node === props.node)
      .sort((a, b) => b.ts - a.ts);

  const sendAll = async (paths: string[]) => {
    try {
      for (const p of paths) await sendFile(props.node, p);
      toast(tn("files.started", paths.length, { name: name() }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

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

  // Files dropped anywhere on the window go to this device.
  onMount(async () => {
    const unlisten = await getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload;
      if (p.type === "enter" || p.type === "over") setDropping(true);
      else if (p.type === "leave") setDropping(false);
      else if (p.type === "drop") {
        setDropping(false);
        if (p.paths.length) sendAll(p.paths);
      }
    });
    onCleanup(unlisten);
  });

  return (
    <article class="tile" classList={{ "tile-drop": dropping() }}>
      <h2>{t("files.title")}</h2>
      <p class="muted small">{dropping() ? t("files.dropHere", { name: name() }) : t("files.hint")}</p>
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
