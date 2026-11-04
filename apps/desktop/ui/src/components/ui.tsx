// Small shared pieces: toasts, the status pill, the device card.

import { createSignal, For, Show, type JSX } from "solid-js";
import { peerName, state, type Peer } from "../lib/core";
import { DeviceAvatar } from "./DeviceLabel";
import { linkLabel, linkState, qualityLabel, type LinkState } from "../lib/words";
import type { Quality } from "../bindings";
import { t, tn } from "../i18n";

/** Wraps a control that may be unavailable: greyed out, with [reason] as
 *  the hover tooltip (a disabled button shows none on its own in WebKit). */
export function Gate(props: { reason?: string; children: JSX.Element }) {
  return (
    <span class="gate" classList={{ "gate-off": !!props.reason }} title={props.reason}>
      {props.children}
    </span>
  );
}

// --- toasts -------------------------------------------------------------------

type Toast = { id: number; text: string; tone: "ok" | "error" | "info" };
const [toasts, setToasts] = createSignal<Toast[]>([]);
let toastSeq = 0;

export function toast(text: string, tone: Toast["tone"] = "info") {
  const id = ++toastSeq;
  setToasts((t) => [...t, { id, text, tone }]);
  setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), tone === "error" ? 6000 : 3500);
}

export function Toasts() {
  return (
    <div class="toasts" role="status" aria-live="polite">
      <For each={toasts()}>{(t) => <div class={`toast toast-${t.tone}`}>{t.text}</div>}</For>
    </div>
  );
}

// --- status -------------------------------------------------------------------

export function linkStateOf(node: string): LinkState {
  return linkState(state.links[node], state.progress[node]);
}

export function StatusPill(props: { node: string }) {
  const s = () => linkStateOf(props.node);
  const q = () => (s() === "direct" || s() === "relay" ? state.quality[props.node] : undefined);
  return (
    <span class={`pill pill-${s()}`}>
      <span class="dot" aria-hidden="true" />
      {linkLabel(s())}
      <Show when={q() && q() !== "unknown"}>
        <SignalBars quality={q()!} />
      </Show>
    </span>
  );
}

/** Three bars, filled by link quality; the label is for screen readers and
 *  the hover tooltip. */
export function SignalBars(props: { quality: Quality }) {
  const filled = () => ({ excellent: 3, good: 2, weak: 1, unknown: 0 })[props.quality];
  const label = () => qualityLabel(props.quality) ?? "";
  return (
    <span class={`bars bars-${props.quality}`} role="img" aria-label={label()} title={label()}>
      <For each={[1, 2, 3]}>{(i) => <span class={i <= filled() ? "on" : ""} />}</For>
    </span>
  );
}

// --- device card ----------------------------------------------------------------

export function DeviceCard(props: { peer: Peer; onOpen: () => void; onConnect: () => void }) {
  const s = () => linkStateOf(props.peer.node_id);
  const unread = () => state.messages.filter((m) => m.node === props.peer.node_id && m.dir === "in").length;
  return (
    <article class="device-card" onClick={props.onOpen} tabIndex={0} onKeyDown={(e) => e.key === "Enter" && props.onOpen()}>
      <DeviceAvatar node={props.peer.node_id} />
      <div class="device-main">
        <h3>{peerName(props.peer.node_id)}</h3>
        <StatusPill node={props.peer.node_id} />
      </div>
      <Show when={unread() > 0}>
        <span class="badge" aria-label={tn("device.unread", unread())}>
          {unread()}
        </span>
      </Show>
      <Show when={s() === "offline" || s() === "failed"}>
        <button
          class="btn btn-soft btn-sm"
          onClick={(e) => {
            e.stopPropagation();
            props.onConnect();
          }}
        >
          {t("device.connect")}
        </button>
      </Show>
    </article>
  );
}

export function Empty(props: { icon: string; title: string; children?: JSX.Element }) {
  return (
    <div class="empty">
      <div class="empty-icon" aria-hidden="true">
        {props.icon}
      </div>
      <h3>{props.title}</h3>
      <div class="empty-body">{props.children}</div>
    </div>
  );
}
