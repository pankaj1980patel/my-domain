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
      {/* A disabled control can't take focus, so its tooltip never shows
          from the keyboard: the reason is also read out next to it. */}
      <Show when={props.reason}>
        <span class="sr-only">{t("a11y.unavailable", { reason: props.reason ?? "" })}</span>
      </Show>
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

/** Two live regions, always on the page so screen readers hear what's added:
 *  errors interrupt (alert), everything else waits its turn (status). */
export function Toasts() {
  const of = (error: boolean) => toasts().filter((x) => (x.tone === "error") === error);
  return (
    <div class="toasts">
      <div class="toast-region" role="alert" aria-live="assertive" aria-relevant="additions">
        <For each={of(true)}>{(x) => <div class={`toast toast-${x.tone}`}>{x.text}</div>}</For>
      </div>
      <div class="toast-region" role="status" aria-live="polite" aria-relevant="additions">
        <For each={of(false)}>{(x) => <div class={`toast toast-${x.tone}`}>{x.text}</div>}</For>
      </div>
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
    // The name is the card's button (Tab, Enter and Space reach it); a click
    // anywhere else on the card opens it too.
    <article class="device-card" onClick={props.onOpen}>
      <DeviceAvatar node={props.peer.node_id} />
      <div class="device-main">
        <h2 class="device-card-name">
          <button
            type="button"
            class="device-open"
            aria-describedby={`device-card-state-${props.peer.node_id}`}
            onClick={(e) => {
              e.stopPropagation();
              props.onOpen();
            }}
          >
            {peerName(props.peer.node_id)}
          </button>
        </h2>
        <span class="device-card-state" id={`device-card-state-${props.peer.node_id}`}>
          <StatusPill node={props.peer.node_id} />
          <Show when={unread() > 0}>
            <span class="sr-only">, {tn("device.unread", unread())}</span>
          </Show>
        </span>
      </div>
      <Show when={unread() > 0}>
        <span class="badge" aria-hidden="true">
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
      <h2 class="empty-title">{props.title}</h2>

      <div class="empty-body">{props.children}</div>
    </div>
  );
}
