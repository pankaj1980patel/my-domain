// Small shared pieces: toasts, the status pill, the device card.

import { createSignal, For, Show, type JSX } from "solid-js";
import { peerIcon, peerName, state, type Peer } from "../lib/core";
import { DeviceIcon } from "./DeviceIcon";
import { linkLabel, linkState, qualityLabel, type LinkState } from "../lib/words";
import type { Quality } from "../bindings";
import { t, tn } from "../i18n";
import { Icon, type IconName } from "./Icon";
import { quickRing, quickSendClipboard, quickSendFile } from "../lib/quickActions";

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

/** A button on a toast (e.g. Undo): pressing it runs `run` and closes it. */
type ToastAction = { label: string; run: () => void };
type Toast = { id: number; text: string; tone: "ok" | "error" | "info"; action?: ToastAction };
const [toasts, setToasts] = createSignal<Toast[]>([]);
let toastSeq = 0;

const dropToast = (id: number) => setToasts((t) => t.filter((x) => x.id !== id));

export function toast(text: string, tone: Toast["tone"] = "info", action?: ToastAction) {
  const id = ++toastSeq;
  setToasts((t) => [...t, { id, text, tone, action }]);
  // One with a button stays longer: time to reach it.
  setTimeout(() => dropToast(id), action ? 8000 : tone === "error" ? 6000 : 3500);
}

function ToastView(props: { toast: Toast }) {
  return (
    <div class={`toast toast-${props.toast.tone}`} classList={{ "toast-with-action": !!props.toast.action }}>
      <span>{props.toast.text}</span>
      <Show when={props.toast.action}>
        {(a) => (
          <button
            type="button"
            class="toast-action"
            onClick={() => {
              dropToast(props.toast.id);
              a().run();
            }}
          >
            {a().label}
          </button>
        )}
      </Show>
    </div>
  );
}

/** Two live regions, always on the page so screen readers hear what's added:
 *  errors interrupt (alert), everything else waits its turn (status). */
export function Toasts() {
  const of = (error: boolean) => toasts().filter((x) => (x.tone === "error") === error);
  return (
    <div class="toasts">
      <div class="toast-region" role="alert" aria-live="assertive" aria-relevant="additions">
        <For each={of(true)}>{(x) => <ToastView toast={x} />}</For>
      </div>
      <div class="toast-region" role="status" aria-live="polite" aria-relevant="additions">
        <For each={of(false)}>{(x) => <ToastView toast={x} />}</For>
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
      <Icon name={`link-${s()}` as IconName} size={15} class="pill-icon" />
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
  const node = () => props.peer.node_id;
  const s = () => linkStateOf(node());
  const unread = () => state.messages.filter((m) => m.node === node() && m.dir === "in").length;
  const name = () => peerName(node());
  /** A quick action's button: it runs here, without opening the page. */
  const quick = (icon: IconName, label: string, full: string, run: () => unknown) => (
    <button
      type="button"
      class="quick-btn"
      aria-label={full}
      title={full}
      onClick={(e) => {
        e.stopPropagation();
        run();
      }}
    >
      <Icon name={icon} />
      <span>{label}</span>
    </button>
  );
  return (
    // The name is the card's button (Tab, Enter and Space reach it); a click
    // anywhere else on the card opens it too.
    <article class="device-card" data-drop-node={node()} onClick={props.onOpen}>
      <div class="device-card-top">
        {/* The device's art (its wallpaper when known: DeviceArt drops in
            here); its icon until then. */}
        <div class="device-art device-art-sm" aria-hidden="true">
          <DeviceIcon icon={peerIcon(node())} size={30} />
        </div>
        <div class="device-main">
          <h2 class="device-card-name">
            <button
              type="button"
              class="device-open"
              aria-describedby={`device-card-state-${node()}`}
              onClick={(e) => {
                e.stopPropagation();
                props.onOpen();
              }}
            >
              {name()}
            </button>
          </h2>
          <span class="device-card-state" id={`device-card-state-${node()}`}>
            <StatusPill node={node()} />
            <Show when={state.status[node()]?.battery !== undefined}>
              <span class="chip chip-sm">
                <Icon name="battery" size={13} />
                {state.status[node()]!.battery}%
              </span>
            </Show>
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
        <Show when={s() === "offline" || s() === "asleep" || s() === "failed"}>
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
      </div>
      {/* One click, no page: what's done most (each queues for a device
          that isn't reachable now). */}
      <div class="device-quick" role="group" aria-label={t("quick.label", { name: name() })}>
        {quick("send-file", t("quick.file"), t("palette.sendFile", { name: name() }), () => quickSendFile(node()))}
        {quick("send-clip", t("quick.clipboard"), t("palette.sendClipboard", { name: name() }), () =>
          quickSendClipboard(node()),
        )}
        {quick("ring", t("control.ring"), t("palette.ring", { name: name() }), () => quickRing(node()))}
      </div>
    </article>
  );
}

export function Empty(props: { icon: IconName; title: string; children?: JSX.Element }) {
  return (
    <div class="empty">
      <div class="empty-icon">
        <Icon name={props.icon} size={44} />
      </div>
      <h2 class="empty-title">{props.title}</h2>

      <div class="empty-body">{props.children}</div>
    </div>
  );
}
