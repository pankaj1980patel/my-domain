// One device: its status, then a tile per feature.

import { createMemo, createSignal, For, Show } from "solid-js";
import {
  connect,
  pullClipboard,
  requestApps,
  sendMessage,
  setClipSync,
  shareNotification,
  state,
  toggleApp,
} from "../lib/core";
import { friendlyError, initial, niceName, progressLabel } from "../lib/words";
import { linkStateOf, StatusPill, toast } from "../components/ui";

export default function Device(props: { node: string; onBack: () => void }) {
  const peer = createMemo(() => state.peers.find((p) => p.node_id === props.node));
  const s = () => linkStateOf(props.node);

  return (
    <Show
      when={peer()}
      fallback={
        <section class="page">
          <button class="link" onClick={props.onBack}>
            ← Your devices
          </button>
          <p class="muted">This device isn't in your list any more.</p>
        </section>
      }
    >
      {(p) => (
        <section class="page">
          <button class="link back" onClick={props.onBack}>
            ← Your devices
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
                Connect
              </button>
            </Show>
          </header>

          <div class="tiles">
            <ChatTile node={props.node} />
            <ClipboardTile node={props.node} />
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
      <h2> Messages
      </h2>
      <div class="thread" ref={list}>
        <Show when={thread().length > 0} fallback={<p class="muted small center">Say hi. Messages go straight to this device, encrypted.</p>}>
          <For each={thread()}>
            {(m) => (
              <div class={`bubble bubble-${m.dir}`} classList={{ "bubble-pending": !!m.pending }}>
                <span>{m.text}</span>
                <Show when={m.pending}>
                  <small class="bubble-note">{m.via === "queued" ? "Sends when connected" : "Sending…"}</small>
                </Show>
                <Show when={m.delivered && m.id === lastDelivered()}>
                  <small class="bubble-note">Delivered</small>
                </Show>
              </div>
            )}
          </For>
        </Show>
      </div>
      <form class="composer" onSubmit={send}>
        <input
          placeholder="Write a message"
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
          aria-label="Message"
        />
        <button class="btn btn-primary" type="submit" disabled={busy() || !text().trim()}>
          Send
        </button>
      </form>
    </article>
  );
}

function ClipboardTile(props: { node: string }) {
  const [busy, setBusy] = createSignal(false);
  return (
    <article class="tile">
      <h2> Clipboard
      </h2>
      <p class="muted small">Copy something there, paste it here.</p>
      <button
        class="btn btn-soft btn-block"
        disabled={busy()}
        onClick={async () => {
          setBusy(true);
          try {
            await pullClipboard(props.node);
            toast("Copied to your clipboard", "ok");
          } catch (e) {
            toast(friendlyError(e), "error");
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy() ? "Getting it…" : "Get its clipboard"}
      </button>
      <label class="switch-row">
        <span>Keep clipboards in sync</span>
        <input
          type="checkbox"
          role="switch"
          checked={state.clipSync}
          onChange={(e) => setClipSync(e.currentTarget.checked)}
        />
      </label>
    </article>
  );
}

function NotifyTile() {
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  return (
    <article class="tile">
      <h2> Send a notification
      </h2>
      <form
        class="stack"
        onSubmit={async (e) => {
          e.preventDefault();
          if (!title().trim()) return;
          await shareNotification(title().trim(), body().trim());
          setTitle("");
          setBody("");
          toast("Sent to your devices", "ok");
        }}
      >
        <input placeholder="Title" value={title()} onInput={(e) => setTitle(e.currentTarget.value)} aria-label="Title" />
        <input placeholder="Message (optional)" value={body()} onInput={(e) => setBody(e.currentTarget.value)} aria-label="Message" />
        <button class="btn btn-soft btn-block" type="submit" disabled={!title().trim()}>
          Send to all my devices
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
      <h2> App notifications
      </h2>
      <p class="muted small">Choose which of its apps can show notifications here.</p>
      <Show
        when={data()}
        fallback={
          <button class="btn btn-soft" onClick={() => requestApps(props.node).catch((e) => toast(friendlyError(e), "error"))}>
            Show its apps
          </button>
        }
      >
        <input placeholder="Search apps" value={q()} onInput={(e) => setQ(e.currentTarget.value)} aria-label="Search apps" />
        <ul class="app-list">
          <For each={shown()} fallback={<li class="muted small">No apps to show.</li>}>
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
