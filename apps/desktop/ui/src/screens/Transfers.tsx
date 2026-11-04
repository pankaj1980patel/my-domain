// Transfers: every file going to or coming from your devices, in one place.
// Those under way first (live progress, speed and time left from the
// store's file-progress events), then the newest finished or failed ones
// (kept by the backend in transfer-history.json across restarts).

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { commands, type TransferEntry_Serialize as Entry } from "../bindings";
import { cancelFile, peerName, sendFile, state, unwrap } from "../lib/core";
import { fileFailReason, friendlyError, humanSize, timeAgo } from "../lib/words";
import { Empty, toast } from "../components/ui";
import { t } from "../i18n";

const isMac = /Mac/i.test(navigator.platform || navigator.userAgent);
/** Speed older than this (no bytes moved) isn't shown. */
const STALE_MS = 5000;

/** 75 -> "1 min" style, for time left. */
function duration(secs: number): string {
  const s = Math.max(1, Math.round(secs));
  if (s < 60) return t("transfers.secs", { n: s });
  const m = Math.round(s / 60);
  if (m < 60) return t("transfers.mins", { n: m });
  return t("transfers.hours", { h: Math.floor(m / 60), m: m % 60 });
}

export default function Transfers() {
  const [list, setList] = createSignal<Entry[]>([]);
  const refresh = () => commands.transferList().then(setList, () => {});
  onMount(refresh);

  // A transfer starting or ending changes the list: fetch it again (once,
  // for a burst such as a folder). Progress alone doesn't.
  const shape = () =>
    Object.values(state.transfers)
      .map((x) => `${x.id}:${x.state}`)
      .join(",");
  let timer: number | undefined;
  createEffect(
    on(
      shape,
      () => {
        clearTimeout(timer);
        timer = window.setTimeout(refresh, 250);
      },
      { defer: true },
    ),
  );

  // A clock for "speed is stale" and the finished rows' "2 min ago".
  const [clock, setClock] = createSignal(Date.now());
  const tick = window.setInterval(() => setClock(Date.now()), 1000);
  onCleanup(() => {
    clearInterval(tick);
    clearTimeout(timer);
  });

  const active = createMemo(() => list().filter((e) => e.state === "active"));
  const finished = createMemo(() => list().filter((e) => e.state !== "active"));

  const clear = () =>
    commands
      .transferClearFinished()
      .then(refresh)
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>{t("transfers.title")}</h1>
          <p class="muted">{t("transfers.subtitle")}</p>
        </div>
        <Show when={finished().length > 0}>
          <button class="btn btn-ghost btn-sm" onClick={clear}>
            {t("transfers.clear")}
          </button>
        </Show>
      </header>
      <Show
        when={list().length > 0}
        fallback={
          <Empty icon="" title={t("transfers.emptyTitle")}>
            {t("transfers.emptyBody")}
          </Empty>
        }
      >
        <Show when={active().length > 0}>
          <h2 class="xfer-section">{t("transfers.active")}</h2>
          <ul class="xfer-list">
            <For each={active()}>{(e) => <ActiveRow e={e} clock={clock()} />}</For>
          </ul>
        </Show>
        <Show when={finished().length > 0}>
          <h2 class="xfer-section">{t("transfers.recent")}</h2>
          <ul class="xfer-list">
            <For each={finished()}>{(e) => <FinishedRow e={e} clock={clock()} onChange={refresh} />}</For>
          </ul>
        </Show>
      </Show>
    </section>
  );
}

/** ↑ sent / ↓ received, then the name and the device. */
function Head(props: { e: Entry }) {
  const device = () => peerName(props.e.node_id);
  return (
    <div class="xfer-head">
      <span
        class={`xfer-dir ${props.e.incoming ? "in" : "out"}`}
        role="img"
        aria-label={props.e.incoming ? t("transfers.received") : t("transfers.sent")}
      >
        {props.e.incoming ? "↓" : "↑"}
      </span>
      <div class="xfer-what">
        <strong class="xfer-name" title={props.e.name}>
          {props.e.name}
        </strong>
        <span class="muted small">
          {props.e.incoming ? t("transfers.from", { device: device() }) : t("transfers.to", { device: device() })}
        </span>
      </div>
    </div>
  );
}

function ActiveRow(props: { e: Entry; clock: number }) {
  // The store has the live numbers; the list has them as of its fetch.
  const live = () => state.transfers[props.e.id];
  const done = () => live()?.done ?? props.e.done ?? 0;
  const total = () => live()?.total || props.e.total || 0;
  const pct = () => (total() ? Math.min(100, (done() / total()) * 100) : 0);
  const rate = () => {
    const x = live();
    if (!x?.rate || !x.movedAt || props.clock - x.movedAt > STALE_MS) return 0;
    return x.rate;
  };
  const resumedFrom = () => props.e.resumed_from ?? live()?.resumedFrom;
  const line = () => {
    if (!total()) return t("transfers.waiting", { device: peerName(props.e.node_id) });
    const parts = [t("transfers.progress", { done: humanSize(done()), total: humanSize(total()) })];
    const r = rate();
    if (r > 0) {
      parts.push(t("transfers.speed", { size: humanSize(Math.round(r)) }));
      parts.push(t("transfers.eta", { time: duration((total() - done()) / r) }));
    }
    return parts.join(" · ");
  };
  const cancel = () => cancelFile(props.e.id).catch((err) => toast(friendlyError(err), "error"));
  return (
    <li class="xfer">
      <div class="xfer-top">
        <Head e={props.e} />
        <button class="btn btn-ghost btn-sm" onClick={cancel} aria-label={t("transfers.cancelLabel", { name: props.e.name })}>
          {t("transfers.cancel")}
        </button>
      </div>
      <div class="progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(pct())}>
        <span style={{ width: `${pct()}%` }} />
      </div>
      <p class="muted small xfer-line">
        <span>{line()}</span>
        <Show when={resumedFrom() !== undefined && resumedFrom() !== null && total() > 0}>
          <span class="xfer-resumed">{t("transfers.resumed", { pct: Math.floor(((resumedFrom() ?? 0) / total()) * 100) })}</span>
        </Show>
      </p>
    </li>
  );
}

function FinishedRow(props: { e: Entry; clock: number; onChange: () => void }) {
  const when = () => {
    void props.clock;
    return timeAgo(props.e.ended ?? props.e.started ?? Date.now());
  };
  const size = () => humanSize(props.e.total ?? 0);
  const line = () => {
    const e = props.e;
    if (e.state === "done") return e.incoming ? t("transfers.doneSaved", { size: size(), when: when() }) : t("transfers.doneSent", { size: size(), when: when() });
    return t("transfers.failed", { reason: fileFailReason(e.reason ?? undefined, e.incoming), when: when() });
  };
  const reveal = () => unwrap(commands.transferReveal(props.e.id)).catch((err) => toast(friendlyError(err), "error"));
  const again = async () => {
    const path = props.e.path;
    if (!path) return;
    try {
      await sendFile(props.e.node_id, path);
      toast(t("transfers.sendingAgain", { name: props.e.name, device: peerName(props.e.node_id) }), "ok");
      props.onChange();
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };
  return (
    <li class="xfer">
      <div class="xfer-top">
        <Head e={props.e} />
        <div class="row xfer-actions">
          <Show when={props.e.path && props.e.exists}>
            <button class="btn btn-ghost btn-sm" onClick={reveal}>
              {isMac ? t("transfers.showMac") : t("transfers.show")}
            </button>
          </Show>
          <Show when={!props.e.incoming && props.e.path && props.e.exists}>
            <button class="btn btn-soft btn-sm" onClick={again}>
              {t("transfers.sendAgain")}
            </button>
          </Show>
        </div>
      </div>
      <p class="muted small" classList={{ "text-error": props.e.state === "failed" }}>
        {line()}
      </p>
    </li>
  );
}
