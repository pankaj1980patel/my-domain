// Transfers: every file going to or coming from your devices, in one place.
// Those under way first (live progress, speed and time left from the
// store's events), then the newest finished or failed ones (kept by the
// backend in transfer-history.json across restarts). Several files sent
// together (a folder, or files dropped at once) are one row with the
// overall progress; its files open underneath. Each row under way can be
// paused, resumed or cancelled; a failed one sent from here can be retried.

import { Icon } from "../components/Icon";
import { Tabs, panelProps } from "../components/Tabs";
import { MenuButton, type MenuItem } from "../components/Menu";
import { quickSendFile, quickSendFolder } from "../lib/quickActions";
import BrowseTile from "./device/BrowseTile";
import PhotosTile from "./device/PhotosTile";
import { SyncSetting } from "../components/SyncSetting";
import DeviceArt from "../components/DeviceArt";
import { peerIcon } from "../lib/core";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { commands, type TransferEntry_Serialize as Entry } from "../bindings";
import { cancelFile, pauseTransfer, peerName, retryTransfer, sendFile, state, unwrap } from "../lib/core";
import { fileFailReason, friendlyError, humanSize, timeAgo } from "../lib/words";
import { Empty, toast } from "../components/ui";
import { foundId, isFound } from "../lib/search";
import { t, tn } from "../i18n";
import { downloads } from "../lib/remoteDownload";

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
      .map((x) => `${x.id}:${x.state}:${x.paused ? 1 : 0}`)
      .join(",") +
    "|" +
    Object.values(state.batches)
      .map((b) => `${b.id}:${b.state}`)
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

  /** Rows of their own: a batch's files show inside it. */
  const top = createMemo(() => list().filter((e) => !e.batch));
  const filesOf = (id: string) => list().filter((e) => e.batch === id);
  const active = createMemo(() => top().filter((e) => e.state === "active"));
  const finished = createMemo(() => top().filter((e) => e.state !== "active"));

  type FilesTab = "transfers" | "mine" | "device" | "photos" | "sync";
  const [tab, setTab] = createSignal<FilesTab>("transfers");
  const received = createMemo(() => finished().filter((e) => e.incoming && e.state !== "failed"));
  // Devices whose folders or photos can be looked at from here.
  const browsable = () => state.peers.map((p) => p.node_id).filter((n) => (state.status[n]?.sharedFolders ?? 0) > 0);
  const withPhotos = () => state.peers.map((p) => p.node_id).filter((n) => !!state.status[n]?.actions.includes("gallery"));
  const [picked, setPicked] = createSignal<string | null>(null);
  const pickFor = () => {
    const list = tab() === "device" ? browsable() : withPhotos();
    const p = picked();
    return p && list.includes(p) ? p : list[0] ?? null;
  };
  const sendMenu = (): MenuItem[] =>
    state.peers.map((p) => ({ icon: "devices" as const, label: peerName(p.node_id), onSelect: () => quickSendFile(p.node_id) }));
  const folderMenu = (): MenuItem[] =>
    state.peers.map((p) => ({ icon: "devices" as const, label: peerName(p.node_id), onSelect: () => quickSendFolder(p.node_id) }));

  const clear = () =>
    commands
      .transferClearFinished()
      .then(refresh)
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <section class="page page-fit">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="transfers" size={26} tone="blue" />
        </span>
        <div class="grow">
          <h1>{t("files.pageTitle")}</h1>
          <p class="muted">{t("files.pageSub")}</p>
        </div>
        <MenuButton label={t("files.sendTo")} labelled triggerClass="btn btn-primary" items={sendMenu()}>
          <Icon name="send-file" />
          {t("files.sendTo")}
          <Icon name="chevron-down" size={16} />
        </MenuButton>
      </header>
      <Tabs
        label={t("files.pageTitle")}
        idBase="files"
        tabs={[
          { id: "transfers", label: t("transfers.title"), icon: "transfers", count: active().length },
          { id: "mine", label: t("files.mine"), icon: "download" },
          { id: "device", label: t("browse.title"), icon: "folder-open" },
          { id: "photos", label: t("photos.title"), icon: "image" },
          { id: "sync", label: t("sync.title"), icon: "folder-sync" },
        ]}
        selected={tab()}
        onSelect={(id) => setTab(id as FilesTab)}
      />
      {/* Each tab scrolls in its own panel; the page stays put. */}
      <div class="page-scroll" {...panelProps("files", tab())}>
      <Show when={tab() === "transfers"}>
      {/* Drop files anywhere in the window (or choose them) to send. */}
      <div class="drop-zone">
        <Icon name="send-file" size={28} tone="blue" />
        <span>{t("files.dropHint")}</span>
        <div class="row">
          <MenuButton label={t("files.chooseFiles")} labelled triggerClass="btn btn-soft btn-sm" items={sendMenu()}>
            {t("files.chooseFiles")}
          </MenuButton>
          <MenuButton label={t("files.chooseFolder")} labelled triggerClass="btn btn-soft btn-sm" items={folderMenu()}>
            {t("files.chooseFolder")}
          </MenuButton>
          <Show when={finished().length > 0}>
            <button class="btn btn-ghost btn-sm" onClick={clear}>
              {t("transfers.clear")}
            </button>
          </Show>
        </div>
      </div>
      <Show
        when={top().length > 0}
        fallback={
          <Empty icon="transfers" title={t("transfers.emptyTitle")}>
            {t("transfers.emptyBody")}
          </Empty>
        }
      >
        <Show when={active().length > 0}>
          <h2 class="xfer-section">{t("transfers.active")}</h2>
          <ul class="xfer-list">
            <For each={active()}>{(e) => <ActiveRow e={e} clock={clock()} files={filesOf(e.id)} />}</For>
          </ul>
        </Show>
        <Show when={finished().length > 0}>
          <h2 class="xfer-section">{t("transfers.recent")}</h2>
          <ul class="xfer-list">
            <For each={finished()}>{(e) => <FinishedRow e={e} clock={clock()} files={filesOf(e.id)} onChange={refresh} />}</For>
          </ul>
        </Show>
      </Show>
      </Show>
      {/* What came in, newest first, each with its own Show / Open. */}
      <Show when={tab() === "mine"}>
        <Show when={received().length} fallback={<p class="muted small">{t("files.noneReceived")}</p>}>
          <ul class="xfer-list">
            <For each={received()}>{(e) => <FinishedRow e={e} clock={clock()} files={filesOf(e.id)} onChange={refresh} />}</For>
          </ul>
        </Show>
      </Show>
      <Show when={tab() === "device" || tab() === "photos"}>
        <DevicePicker
          devices={tab() === "device" ? browsable() : withPhotos()}
          chosen={pickFor()}
          onPick={setPicked}
          empty={tab() === "device" ? t("files.noBrowse") : t("files.noPhotos")}
        />
        <Show when={pickFor()} keyed>
          {(n) => (tab() === "device" ? <BrowseTile node={n} /> : <PhotosTile node={n} />)}
        </Show>
      </Show>
      <Show when={tab() === "sync"}>
        <SyncSetting />
      </Show>
      </div>
    </section>
  );
}

/** "via Wi-Fi Direct" / "via phone hotspot". */
function viaText(via: string): string {
  return via === "hotspot" ? t("transfers.viaHotspot") : t("transfers.viaWifiDirect");
}

/** ↑ sent / ↓ received, then the name and the device. */
function Head(props: { e: Entry }) {
  const device = () => peerName(props.e.node_id);
  const name = () =>
    props.e.kind && props.e.kind !== "folder" && (props.e.count ?? 0) > 1
      ? `${props.e.name} · ${tn("transfers.batchFiles", props.e.count ?? 0)}`
      : props.e.name;
  return (
    <div class="xfer-head">
      <span
        class={`xfer-dir ${props.e.incoming ? "in" : "out"}`}
        role="img"
        aria-label={props.e.incoming ? t("transfers.received") : t("transfers.sent")}
      >
        {props.e.kind === "folder" ? "▤" : props.e.incoming ? "↓" : "↑"}
      </span>
      <div class="xfer-what">
        <strong class="xfer-name" title={name()}>
          {name()}
        </strong>
        <span class="muted small">
          {props.e.link
            ? props.e.incoming
              ? t("transfers.dl.here", { host: props.e.host ?? "", device: device() })
              : t("transfers.dl.there", { host: props.e.host ?? "", device: device() })
            : props.e.incoming
              ? t("transfers.from", { device: device() })
              : t("transfers.to", { device: device() })}
        </span>
      </div>
    </div>
  );
}

/** A batch's files, opened under its row. */
function BatchFiles(props: { files: Entry[]; name: string }) {
  const pct = (f: Entry) => {
    const live = state.transfers[f.id];
    const done = live?.done ?? f.done ?? 0;
    const total = live?.total || f.total || 0;
    return total ? Math.floor((done / total) * 100) : 0;
  };
  const status = (f: Entry) => {
    const live = state.transfers[f.id]?.state;
    if (f.state === "failed" || live === "failed") return fileFailReason(f.reason ?? undefined, f.incoming);
    if (f.state === "done" || live === "sent" || live === "saved") return humanSize(f.total ?? 0);
    return t("a11y.percent", { pct: pct(f) });
  };
  return (
    <ul class="xfer-files" aria-label={t("transfers.filesOf", { name: props.name })}>
      <For each={props.files}>
        {(f) => (
          <li classList={{ "text-error": f.state === "failed" }}>
            <span class="xfer-name" title={f.name}>
              {f.name}
            </span>
            <span class="muted small">{status(f)}</span>
          </li>
        )}
      </For>
    </ul>
  );
}

function ActiveRow(props: { e: Entry; clock: number; files: Entry[] }) {
  const [open, setOpen] = createSignal(false);
  const isBatch = () => !!props.e.kind;
  // The store has the live numbers; the list has them as of its fetch.
  const live = () => (isBatch() ? state.batches[props.e.id] : state.transfers[props.e.id]);
  const done = () => live()?.done ?? props.e.done ?? 0;
  const total = () => live()?.total || props.e.total || 0;
  const pct = () => (total() ? Math.min(100, (done() / total()) * 100) : 0);
  const paused = () => {
    const b = state.batches[props.e.id];
    if (b) return b.state === "paused";
    return state.transfers[props.e.id]?.paused ?? props.e.paused ?? false;
  };
  const rate = () => {
    const x = live();
    if (paused() || !x?.rate || !x.movedAt || props.clock - x.movedAt > STALE_MS) return 0;
    return x.rate;
  };
  const resumedFrom = () => props.e.resumed_from ?? state.transfers[props.e.id]?.resumedFrom;
  /** A link downloaded on a device: where it is before it moves. */
  const stage = () => (props.e.link ? (downloads[props.e.id]?.state ?? props.e.stage ?? null) : null);
  const line = () => {
    const device = peerName(props.e.node_id);
    switch (stage()) {
      case "requested":
        return t("transfers.dl.requested", { device });
      case "checking":
        return t("transfers.dl.checking");
      case "asking":
        return props.e.incoming ? t("transfers.dl.askingHere") : t("transfers.dl.askingThere", { device });
      case "queued":
        return t("transfers.dl.queued");
    }
    if (!total()) return t("transfers.waiting", { device: peerName(props.e.node_id) });
    const parts: string[] = [];
    if (isBatch()) {
      const b = state.batches[props.e.id];
      parts.push(t("transfers.batchProgress", { done: b?.files_done ?? props.e.files_done ?? 0, count: b?.count ?? props.e.count ?? 0 }));
    }
    parts.push(t("transfers.progress", { done: humanSize(done()), total: humanSize(total()) }));
    if (paused()) {
      parts.push(t("transfers.paused"));
      return parts.join(" · ");
    }
    const r = rate();
    if (r > 0) {
      parts.push(t("transfers.speed", { size: humanSize(Math.round(r)) }));
      parts.push(t("transfers.eta", { time: duration((total() - done()) / r) }));
    }
    return parts.join(" · ");
  };
  const cancel = () => cancelFile(props.e.id).catch((err) => toast(friendlyError(err), "error"));
  const togglePause = () => pauseTransfer(props.e.id, !paused()).catch((err) => toast(friendlyError(err), "error"));
  return (
    <li class="xfer" id={foundId("transfers", props.e.id)} tabIndex={-1} classList={{ "found-target": isFound("transfers", props.e.id) }}>
      <div class="xfer-top">
        <Head e={props.e} />
        <div class="row xfer-actions">
          <Show when={isBatch() && props.files.length > 0}>
            <button class="btn btn-ghost btn-sm" aria-expanded={open()} onClick={() => setOpen(!open())}>
              {open() ? t("transfers.hideFiles") : t("transfers.showFiles")}
            </button>
          </Show>
          <Show when={!props.e.link}>
            <button
              class="btn btn-ghost btn-sm"
              onClick={togglePause}
              aria-label={paused() ? t("transfers.resumeLabel", { name: props.e.name }) : t("transfers.pauseLabel", { name: props.e.name })}
            >
              {paused() ? t("transfers.resume") : t("transfers.pause")}
            </button>
          </Show>
          <button class="btn btn-ghost btn-sm" onClick={cancel} aria-label={t("transfers.cancelLabel", { name: props.e.name })}>
            {t("transfers.cancel")}
          </button>
        </div>
      </div>
      <div
        class="progress"
        role="progressbar"
        aria-label={t("a11y.transferProgress", { name: props.e.name })}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct())}
        aria-valuetext={total() ? `${t("a11y.percent", { pct: Math.round(pct()) })}, ${line()}` : line()}
      >
        <span style={{ width: `${pct()}%` }} />
      </div>
      <p class="muted small xfer-line">
        <span>{line()}</span>
        <Show when={!isBatch() && resumedFrom() !== undefined && resumedFrom() !== null && total() > 0}>
          <span class="xfer-resumed">{t("transfers.resumed", { pct: Math.floor(((resumedFrom() ?? 0) / total()) * 100) })}</span>
        </Show>
        <Show when={props.e.via || state.wifiDirect[props.e.node_id]}>
          {(via) => <span class="xfer-resumed">{viaText(via())}</span>}
        </Show>
      </p>
      <Show when={open()}>
        <BatchFiles files={props.files} name={props.e.name} />
      </Show>
    </li>
  );
}

function FinishedRow(props: { e: Entry; clock: number; files: Entry[]; onChange: () => void }) {
  const [open, setOpen] = createSignal(false);
  const when = () => {
    void props.clock;
    return timeAgo(props.e.ended ?? props.e.started ?? Date.now());
  };
  const size = () => humanSize(props.e.total ?? 0);
  const line = () => {
    const e = props.e;
    if (e.state === "done" && e.link) {
      return e.incoming
        ? t("transfers.dl.doneHere", { size: size(), when: when() })
        : t("transfers.dl.doneThere", { device: peerName(e.node_id), size: size(), when: when() });
    }
    if (e.state === "done") {
      const line = e.incoming ? t("transfers.doneSaved", { size: size(), when: when() }) : t("transfers.doneSent", { size: size(), when: when() });
      const counted = e.kind ? `${tn("transfers.batchFiles", e.count ?? 0)} · ${line}` : line;
      return e.via ? `${counted} · ${viaText(e.via)}` : counted;
    }
    const failed = t("transfers.failed", { reason: fileFailReason(e.reason ?? undefined, e.incoming), when: when() });
    return e.kind && (e.files_failed ?? 0) > 0 ? `${tn("transfers.batchFailed", e.files_failed ?? 0)} · ${failed}` : failed;
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
  /** A failed one sent (or asked for) from here: send it again. */
  const canRetry = () => props.e.state === "failed" && !props.e.incoming;
  const retry = async () => {
    try {
      await retryTransfer(props.e.id);
      toast(t("transfers.retrying", { name: props.e.name }), "ok");
      props.onChange();
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };
  return (
    <li class="xfer" id={foundId("transfers", props.e.id)} tabIndex={-1} classList={{ "found-target": isFound("transfers", props.e.id) }}>
      <div class="xfer-top">
        <Head e={props.e} />
        <div class="row xfer-actions">
          <Show when={props.e.kind && props.files.length > 0}>
            <button class="btn btn-ghost btn-sm" aria-expanded={open()} onClick={() => setOpen(!open())}>
              {open() ? t("transfers.hideFiles") : t("transfers.showFiles")}
            </button>
          </Show>
          <Show when={props.e.path && props.e.exists}>
            <button class="btn btn-ghost btn-sm" onClick={reveal}>
              {isMac ? t("transfers.showMac") : t("transfers.show")}
            </button>
          </Show>
          <Show when={canRetry()}>
            <button class="btn btn-soft btn-sm" onClick={retry} aria-label={t("transfers.retryLabel", { name: props.e.name })}>
              {t("transfers.retry")}
            </button>
          </Show>
          <Show when={!canRetry() && !props.e.kind && !props.e.incoming && props.e.path && props.e.exists}>
            <button class="btn btn-soft btn-sm" onClick={again}>
              {t("transfers.sendAgain")}
            </button>
          </Show>
        </div>
      </div>
      <p class="muted small" classList={{ "text-error": props.e.state === "failed" }}>
        {line()}
      </p>
      <Show when={open()}>
        <BatchFiles files={props.files} name={props.e.name} />
      </Show>
    </li>
  );
}

/** Which device: one chip each (with its picture). */
function DevicePicker(props: { devices: string[]; chosen: string | null; onPick: (n: string) => void; empty: string }) {
  return (
    <Show when={props.devices.length} fallback={<p class="muted small">{props.empty}</p>}>
      <div class="home-filters" role="group">
        <For each={props.devices}>
          {(n) => (
            <button class="filter-chip device-chip" aria-pressed={props.chosen === n} onClick={() => props.onPick(n)}>
              <DeviceArt node={n} icon={peerIcon(n)} size={20} />
              {peerName(n)}
            </button>
          )}
        </For>
      </div>
    </Show>
  );
}
