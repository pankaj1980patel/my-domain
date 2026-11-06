// The quick panel: what the menu-bar / tray icon opens (quick_panel.rs), in
// its own small window. Your devices with their link and battery and a few
// buttons each, the last clips and transfers, and a way into the main
// window (Open, Settings, Search with what you type).
//
// It reads the same store as the app (lib/core.ts), filled in its own
// window: startEvents in panel mode (no ring sound or toasts: the main
// window has those), and loadPanel each time it's shown.
//
// Keyboard: Up / Down move between rows, Left / Right between a row's
// buttons, Enter presses, Esc closes, typing goes to the search field.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type ClipRecord_Serialize as Clip, type OtpView, type TransferEntry_Serialize as Entry } from "../bindings";
import { clipTick, copyText, copyVerificationCode, loadPanel, peerName, startEvents, state, unwrap } from "../lib/core";
import { linkLabel } from "../lib/words";
import { linkStateOf } from "../components/ui";
import { DeviceAvatar } from "../components/DeviceLabel";
import { logoSrc } from "../components/Brand";
import { trapTab } from "../lib/a11y";
import { t } from "../i18n";

const isMac = /Mac/i.test(navigator.platform || navigator.userAgent);
/** How many clips and transfers "Recent" shows. */
const RECENT = 3;

const hide = () => commands.quickPanelHide();
/** A transfer's latest moment: when it ended, else when it started. */
const when = (e: Entry) => e.ended ?? e.started ?? 0;

export default function QuickPanel() {
  const [ready, setReady] = createSignal<boolean | null>(null);
  const [clips, setClips] = createSignal<Clip[]>([]);
  const [transfers, setTransfers] = createSignal<Entry[]>([]);
  const [query, setQuery] = createSignal("");
  const [copied, setCopied] = createSignal<number | null>(null);
  // The latest verification code still valid, and when it goes (ms).
  const [otp, setOtp] = createSignal<{ v: OtpView; until: number } | null>(null);
  const [tick, setTick] = createSignal(Date.now());
  let root: HTMLDivElement | undefined;
  let search: HTMLInputElement | undefined;

  const loadOtp = () =>
    commands.verificationCodes().then(
      (list) => setOtp(list[0] ? { v: list[0], until: Date.now() + list[0].expires_in_ms } : null),
      () => setOtp(null),
    );
  const loadClips = () => commands.quickPanelClips(RECENT).then(setClips, () => {});
  const loadTransfers = () =>
    commands.transferList().then(
      (list) => setTransfers([...list].sort((a, b) => when(b) - when(a)).slice(0, RECENT)),
      () => {},
    );

  /** Read everything again and put focus on the first row. */
  const refresh = async () => {
    setCopied(null);
    const ok = await commands.isReady().catch(() => false);
    setReady(ok);
    if (ok) {
      await startEvents({ panel: true });
      await loadPanel().catch(() => {});
      loadClips();
      loadTransfers();
      loadOtp();
    }
    requestAnimationFrame(() => (rows()[0] ?? search)?.focus());
  };

  onMount(() => {
    refresh();
    // Shown again from the icon: fresh data, and focus back at the top.
    const off = listen("quick-panel-shown", () => refresh());
    const offCode = listen("verification-code", () => loadOtp());
    const offGone = listen("verification-code-gone", () => loadOtp());
    // The seconds left, and the code gone once it expires.
    const every = window.setInterval(() => {
      setTick(Date.now());
      const o = otp();
      if (o && Date.now() >= o.until) setOtp(null);
    }, 1000);
    onCleanup(() => {
      off.then((f) => f());
      offCode.then((f) => f());
      offGone.then((f) => f());
      clearInterval(every);
    });
  });

  // Clips and transfers that change while it's open.
  createEffect(on(clipTick, () => ready() && loadClips(), { defer: true }));
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
        timer = window.setTimeout(loadTransfers, 250);
      },
      { defer: true },
    ),
  );
  onCleanup(() => clearTimeout(timer));

  // Connected ones first, then by name (as on Devices).
  const devices = createMemo(() =>
    [...state.peers].sort((a, b) => {
      const la = state.links[a.node_id] ? 0 : 1;
      const lb = state.links[b.node_id] ? 0 : 1;
      return la - lb || peerName(a.node_id).localeCompare(peerName(b.node_id));
    }),
  );

  const act = (node: string, action: string) => commands.quickPanelAction(node, action).catch(() => {});

  const copyClip = async (c: Clip) => {
    try {
      await copyText(c.text);
      setCopied(c.id);
      setTimeout(hide, 600);
    } catch {
      setCopied(null);
    }
  };
  const reveal = (e: Entry) => {
    unwrap(commands.transferReveal(e.id)).then(hide, () => {});
  };
  const openMain = (place: string, q: string | null = null) => commands.quickPanelOpenMain(place, q);

  /** The rows arrows move between: each one's first focusable element. */
  const rows = () =>
    Array.from(root?.querySelectorAll<HTMLElement>("[data-qp-row]") ?? [])
      .map((row) => row.querySelector<HTMLElement>("button:not([disabled]), input"))
      .filter((el): el is HTMLElement => !!el);

  const onKey = (e: KeyboardEvent) => {
    trapTab(e, root);
    if (e.key === "Escape") {
      e.preventDefault();
      hide();
      return;
    }
    const at = document.activeElement as HTMLElement | null;
    const row = at?.closest<HTMLElement>("[data-qp-row]");
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const list = rows();
      const i = list.findIndex((el) => el.closest("[data-qp-row]") === row);
      const next = e.key === "ArrowDown" ? (i < 0 ? 0 : Math.min(i + 1, list.length - 1)) : Math.max(i - 1, 0);
      if (list[next]) {
        e.preventDefault();
        list[next].focus();
        list[next].scrollIntoView({ block: "nearest" });
      }
      return;
    }
    if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && row && at?.tagName !== "INPUT") {
      const buttons = Array.from(row.querySelectorAll<HTMLElement>("button:not([disabled])"));
      const i = buttons.indexOf(at!);
      const next = buttons[i + (e.key === "ArrowRight" ? 1 : -1)];
      if (next) {
        e.preventDefault();
        next.focus();
      }
      return;
    }
    // Typing anywhere goes into the search field.
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey && at !== search && e.key !== " ") search?.focus();
  };

  return (
    <div class="qp" ref={root} role="dialog" aria-modal="true" aria-labelledby="qp-title" onKeyDown={onKey}>
      <header class="qp-head">
        <img src={logoSrc()} alt="" class="qp-logo" />
        <h1 id="qp-title" class="qp-title">
          {t("app.name")}
          <span class="sr-only"> · {t("panel.title")}</span>
        </h1>
        <button type="button" class="qp-close" aria-label={t("panel.close")} title={t("panel.close")} onClick={hide}>
          <span aria-hidden="true">×</span>
        </button>
      </header>

      <Show when={state.quietPhones.length > 0}>
        <div class="quiet-badge small qp-quiet" role="status">
          <span class="quiet-dot" aria-hidden="true" />
          {t("panel.quiet")}
        </div>
      </Show>

      <div class="qp-body">
        <Show when={ready() !== false} fallback={<p class="muted small qp-empty">{t("panel.signedOut")}</p>}>
          <Show when={otp()}>
            {(o) => {
              const from = () => o().v.issuer || o().v.app || peerName(o().v.from);
              const left = () => Math.max(0, Math.ceil((o().until - tick()) / 1000));
              return (
                <section class="qp-section" aria-labelledby="qp-otp">
                  <h2 id="qp-otp" class="qp-h">
                    {t("otp.panelTitle")}
                  </h2>
                  <ul class="qp-list">
                    <li data-qp-row>
                      <button
                        type="button"
                        class="qp-item"
                        aria-label={t("otp.panelCopy", { code: o().v.code, from: from() })}
                        onClick={() => void copyVerificationCode(o().v.id).then((ok) => ok && setTimeout(hide, 600))}
                      >
                        <span class="qp-item-text">
                          <strong>{o().v.code}</strong> · {from()}
                        </span>
                        <span class="qp-item-meta muted small">
                          {t("otp.copy")} · {t("otp.left", { n: left() })}
                        </span>
                      </button>
                    </li>
                  </ul>
                </section>
              );
            }}
          </Show>
          <section class="qp-section" aria-labelledby="qp-devices">
            <h2 id="qp-devices" class="qp-h">
              {t("panel.devices")}
            </h2>
            <Show when={devices().length > 0} fallback={<p class="muted small qp-empty">{t("panel.noDevices")}</p>}>
              <ul class="qp-list">
                <For each={devices()}>{(p) => <DeviceRow node={p.node_id} act={act} />}</For>
              </ul>
            </Show>
          </section>

          <section class="qp-section" aria-labelledby="qp-recent">
            <h2 id="qp-recent" class="qp-h">
              {t("panel.recent")}
            </h2>
            <h3 class="qp-sub muted small" id="qp-clips">
              {t("panel.clips")}
            </h3>
            <Show when={clips().length > 0} fallback={<p class="muted small qp-empty">{t("panel.noClips")}</p>}>
              <ul class="qp-list" aria-labelledby="qp-clips">
                <For each={clips()}>
                  {(c) => {
                    const preview = () => c.text.replace(/\s+/g, " ").trim();
                    const who = () =>
                      c.direction === "in"
                        ? c.from_node && t("panel.clipFrom", { name: peerName(c.from_node) })
                        : c.to_node && t("panel.clipTo", { name: peerName(c.to_node) });
                    return (
                      <li data-qp-row>
                        <button
                          type="button"
                          class="qp-item"
                          aria-label={t("panel.clipCopy", { text: preview().slice(0, 80) })}
                          onClick={() => copyClip(c)}
                        >
                          <span class="qp-item-text">{preview()}</span>
                          <span class="qp-item-meta muted small">{copied() === c.id ? t("panel.copied") : who()}</span>
                        </button>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </Show>
            <h3 class="qp-sub muted small" id="qp-transfers">
              {t("panel.transfers")}
            </h3>
            <Show when={transfers().length > 0} fallback={<p class="muted small qp-empty">{t("panel.noTransfers")}</p>}>
              <ul class="qp-list" aria-labelledby="qp-transfers">
                <For each={transfers()}>
                  {(e) => (
                    <li data-qp-row>
                      <button
                        type="button"
                        class="qp-item"
                        disabled={!e.exists}
                        aria-label={t(isMac ? "panel.revealMac" : "panel.reveal", { name: e.name })}
                        onClick={() => reveal(e)}
                      >
                        <span class="qp-item-text">
                          <span aria-hidden="true">{e.incoming ? "↓ " : "↑ "}</span>
                          {e.name}
                        </span>
                        <span class="qp-item-meta muted small">
                          {e.state === "active"
                            ? t("panel.moving")
                            : e.state === "failed"
                              ? t("panel.failed")
                              : t(e.incoming ? "transfers.from" : "transfers.to", { device: peerName(e.node_id) })}
                        </span>
                      </button>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </section>
        </Show>
      </div>

      <footer class="qp-foot">
        <form
          class="qp-search"
          data-qp-row
          role="search"
          onSubmit={(e) => {
            e.preventDefault();
            openMain("search", query().trim() || null);
          }}
        >
          <input
            ref={search}
            type="search"
            placeholder={t("panel.search")}
            aria-label={t("panel.searchLabel")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
        </form>
        <div class="qp-foot-row" data-qp-row>
          <button type="button" class="btn btn-primary btn-sm" onClick={() => openMain("home")}>
            {t("panel.openApp")}
          </button>
          <button type="button" class="btn btn-ghost btn-sm" onClick={() => openMain("timers")}>
            {t("nav.timers")}
          </button>
          <button type="button" class="btn btn-ghost btn-sm" onClick={() => openMain("settings")}>
            {t("panel.settings")}
          </button>
        </div>
      </footer>
    </div>
  );
}

/** One device: avatar, name, link and battery, and its quick buttons. */
function DeviceRow(props: { node: string; act: (node: string, action: string) => void }) {
  const name = () => peerName(props.node);
  const st = () => state.status[props.node];
  const link = () => linkStateOf(props.node);
  const phone = () => st()?.platform === "android";
  const computer = () => !!st()?.platform && !phone();
  const battery = () => {
    const s = st();
    if (s?.battery === undefined) return null;
    return t(s.charging ? "panel.batteryCharging" : "panel.battery", { n: s.battery });
  };
  // A phone that said location sharing is off; a computer that can't lock.
  const locateOff = () => phone() && st()?.locationSharing === false;
  const lockOff = () => computer() && !st()?.actions.includes("lock");
  return (
    <li class="qp-device" data-qp-row>
      <DeviceAvatar node={props.node} />
      <div class="qp-device-main">
        <span class="qp-device-name">{name()}</span>
        <span class="qp-device-state small">
          <span class={`pill pill-${link()}`}>
            <span class="dot" aria-hidden="true" />
            {linkLabel(link())}
          </span>
          <Show when={battery()}>{(b) => <span class="muted">{b()}</span>}</Show>
        </span>
      </div>
      <div class="qp-actions" role="group" aria-label={t("panel.deviceActions", { name: name() })}>
        <button type="button" class="qp-act" aria-label={t("panel.ringLabel", { name: name() })} title={t("panel.ringLabel", { name: name() })} onClick={() => props.act(props.node, "ring")}>
          {t("panel.ring")}
        </button>
        <button type="button" class="qp-act" aria-label={t("panel.clipLabel", { name: name() })} title={t("panel.clipLabel", { name: name() })} onClick={() => props.act(props.node, "clipboard")}>
          {t("panel.clip")}
        </button>
        <button type="button" class="qp-act" aria-label={t("panel.fileLabel", { name: name() })} title={t("panel.fileLabel", { name: name() })} onClick={() => props.act(props.node, "send_file")}>
          {t("panel.file")}
        </button>
        <Show when={phone()}>
          <button
            type="button"
            class="qp-act"
            disabled={locateOff()}
            aria-label={locateOff() ? t("panel.locateOff", { name: name() }) : t("panel.locateLabel", { name: name() })}
            title={locateOff() ? t("panel.locateOff", { name: name() }) : t("panel.locateLabel", { name: name() })}
            onClick={() => props.act(props.node, "locate")}
          >
            {t("panel.locate")}
          </button>
        </Show>
        <Show when={computer()}>
          <button
            type="button"
            class="qp-act"
            disabled={lockOff()}
            aria-label={lockOff() ? t("panel.lockOff", { name: name() }) : t("panel.lockLabel", { name: name() })}
            title={lockOff() ? t("panel.lockOff", { name: name() }) : t("panel.lockLabel", { name: name() })}
            onClick={() => props.act(props.node, "lock")}
          >
            {t("panel.lock")}
          </button>
        </Show>
      </div>
    </li>
  );
}
