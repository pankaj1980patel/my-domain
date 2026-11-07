// App shell: sign in first, then a sidebar with Devices / Notes / Timers / Read later / Activity /
// Transfers / Search / Settings / Help. A device opens its detail page; the Developer screen
// is hidden.

import { createEffect, createSignal, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { dismissRing, onOpenDevice, onQuickOpen, peerName, resume, state } from "./lib/core";
import { unlockAudio } from "./lib/ring";
import { niceName } from "./lib/words";
import { toast, Toasts } from "./components/ui";
import CommandPalette, { type Place } from "./components/CommandPalette";
import CallOverlay from "./components/CallOverlay";
import SendToPicker from "./components/SendToPicker";
import DropAnywhere from "./components/DropAnywhere";
import UpdateBanner from "./components/UpdateBanner";
import { DownloadAsk, DownloadOnPicker } from "./components/DownloadOn";
import { initRemoteDownloads } from "./lib/remoteDownload";
import { logoSrc, Tagline, Wordmark } from "./components/Brand";
import SignIn from "./screens/SignIn";
import Home from "./screens/Home";
import Device from "./screens/Device";
import Activity from "./screens/Activity";
import Transfers from "./screens/Transfers";
import Settings from "./screens/Settings";
import Developer from "./screens/Developer";
import Help from "./screens/Help";
import Search from "./screens/Search";
import { revealFound, type Found } from "./lib/search";
import Notes, { type NoteRequest } from "./screens/Notes";
import NoteReminders from "./components/NoteReminders";
import Timers from "./screens/Timers";
import TimerAlarm from "./components/TimerAlarm";
import { timers } from "./lib/timers";
import { notes } from "./lib/notes";
import ReadLater from "./screens/ReadLater";
import LinksMap from "./screens/LinksMap";
import Health, { type HealthNav } from "./screens/Health";
import Setup from "./screens/Setup";
import { badCount, loadSetup, watchHealth } from "./lib/health";
import { readLater, unreadCount } from "./lib/readLater";
import { setHelpOpener } from "./help/HelpLink";
import { t, tn, type Key } from "./i18n";
import { rememberFocus } from "./lib/a11y";
import { commands } from "./bindings";
import { HotspotAsk } from "./components/HotspotAsk";
import { PrintAsk } from "./components/PrintAsk";
import { PermissionAsk, PermissionNotice } from "./components/PeerPermissions";

type View =
  | { name: "home" }
  | { name: "device"; node: string }
  | { name: "notes"; request?: NoteRequest }
  | { name: "timers" }
  | { name: "readLater" }
  | { name: "linksMap" }
  | { name: "health" }
  | { name: "setup" }
  | { name: "activity" }
  | { name: "transfers" }
  | { name: "settings" }
  | { name: "developer" }
  | { name: "help"; topic?: string }
  | { name: "search"; query?: string };

export default function App() {
  // null while checking for a saved session, so sign-in doesn't flash.
  const [signedIn, setSignedIn] = createSignal<boolean | null>(null);
  onMount(() => {
    unlockAudio();
    resume().then(setSignedIn, () => setSignedIn(false));
  });
  const [view, setView] = createSignal<View>({ name: "home" });
  let main: HTMLElement | undefined;
  const go = (v: View) => setView(v);
  // A new screen: the window title names it, and focus moves to its content
  // when it was in the sidebar or got lost with the old screen (keyboard and
  // screen reader users land on the new page, not back at the top).
  const titles: Record<View["name"], Key> = {
    home: "nav.devices",
    device: "nav.devices",
    notes: "nav.notes",
    timers: "nav.timers",
    readLater: "nav.readLater",
    linksMap: "map.title",
    health: "health.title",
    setup: "setup.title",
    activity: "nav.activity",
    transfers: "nav.transfers",
    settings: "nav.settings",
    developer: "nav.settings",
    help: "nav.help",
    search: "nav.search",
  };
  createEffect(
    on(
      () => view().name,
      (name) => {
        document.title = `${t(titles[name])} · ${t("app.name")}`;
        const at = document.activeElement;
        if (!at || at === document.body || at.closest(".sidebar") || !at.isConnected) {
          requestAnimationFrame(() => {
            const now = document.activeElement;
            if (!now || now === document.body || now.closest(".sidebar")) main?.focus({ preventScroll: true });
          });
        }
      },
      { defer: true },
    ),
  );
  const skip = (e: MouseEvent) => {
    e.preventDefault();
    main?.focus();
  };
  // The command palette's places; a Settings section scrolls into view once
  // the page is drawn.
  const goPlace = (place: Place, section?: string) => {
    // Help's "section" is a topic.
    if (place === "help") return go({ name: "help", topic: section });
    go({ name: place });
    if (!section) return;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => document.getElementById(section)?.scrollIntoView({ block: "start" })),
    );
  };
  // Health check and setup: where their buttons go.
  const healthNav: HealthNav = {
    onDevice: (node) => go({ name: "device", node }),
    onMap: () => go({ name: "linksMap" }),
    onSettings: (section) => goPlace("settings", section),
    onHelp: (topic) => go({ name: "help", topic }),
  };
  // The health check runs while signed in (the sidebar's badge).
  createEffect(
    on(signedIn, (yes) => {
      if (!yes) return;
      const stop = watchHealth();
      onCleanup(stop);
    }),
  );
  /** Signed in or joined just now: first-run setup, unless done here before. */
  const signedInNow = () => {
    setSignedIn(true);
    loadSetup().then((s) => !s.finished && go({ name: "setup" }), () => {});
  };
  // The map's "Connection details": the device's page, at its Connection tile.
  const openConnection = (node: string) => {
    go({ name: "device", node });
    requestAnimationFrame(() =>
      requestAnimationFrame(() => document.querySelector(".conn-tile")?.scrollIntoView({ block: "start" })),
    );
  };
  // A search result opens where it lives: a device's page with that
  // message, clip, notification or folder brought into view and marked;
  // Notes with that note open; Transfers with that file; the Settings
  // section; the Help topic.
  const openFound = (f: Found) => {
    const missing = () => toast(t("search.gone"));
    switch (f.kind) {
      case "devices":
        return go({ name: "device", node: f.id });
      case "notes":
        if (notes.loaded && !notes.byId[f.id]) return missing();
        return go({ name: "notes", request: { id: f.id, at: Date.now() } });
      case "read_later":
        if (readLater.loaded && !readLater.byId[f.id]) return missing();
        go({ name: "readLater" });
        return revealFound({ kind: f.kind, id: f.id }, missing);
      case "messages":
      case "clipboard":
      case "notifications":
      case "folders":
        if (!f.node) return;
        go({ name: "device", node: f.node });
        return revealFound({ kind: f.kind, id: f.id, node: f.node }, missing);
      case "transfers":
        go({ name: "transfers" });
        return revealFound({ kind: f.kind, id: f.id, node: f.node }, missing);
      case "automations":
      case "schedules":
      case "shortcuts":
        return goPlace("settings", `settings-${f.kind}`);
      case "settings":
        return goPlace("settings", `settings-${f.id}`);
      case "help":
        return go({ name: "help", topic: f.id });
    }
  };
  // A note (or a new one, `id` null) from the command palette.
  const openNote = (id: string | null) => go({ name: "notes", request: { id, at: Date.now() } });
  // A "?" next to a setting opens its Help topic.
  setHelpOpener((topic) => go({ name: "help", topic }));
  // The tray's "Open" on a device jumps to that device's page.
  onMount(() => {
    const off = onOpenDevice((node) => go({ name: "device", node }));
    onCleanup(() => off.then((f) => f()));
  });
  // Links downloaded on a device (either way), and the "Download on…" picker.
  onMount(() => {
    const off = initRemoteDownloads();
    onCleanup(() => off.then((f) => f()).catch(() => {}));
  });
  // The quick panel's footer: Settings, Timers, or Search with what was typed there
  // ("Open my-domain" just brings the window forward as it is).
  onMount(() => {
    const off = onQuickOpen((place, query) => {
      if (place === "settings") go({ name: "settings" });
      else if (place === "timers") go({ name: "timers" });
      else if (place === "search") go({ name: "search", query: query ?? undefined });
    });
    onCleanup(() => off.then((f) => f()));
  });
  const is = (n: View["name"]) => view().name === n || (n === "home" && view().name === "device");
  const connected = () => state.peers.filter((p) => state.links[p.node_id]).length;
  // Running timers (ended ones too, until stopped).
  const activeTimers = () => Object.values(timers.byId).filter((x) => x.state === "running").length;
  const activeTransfers = () =>
    Object.values(state.transfers).filter((x) => x.state === "waiting" || x.state === "sending" || x.state === "receiving").length;

  return (
    <>
      <Show when={signedIn() !== null}>
        <Show when={signedIn()} fallback={<SignIn onDone={signedInNow} />}>
        <a class="skip-link" href="#main" onClick={skip}>
          {t("a11y.skip")}
        </a>
        <div class="shell">
          <nav class="sidebar" aria-label={t("nav.label")}>
            <div class="brand">
              <img src={logoSrc()} alt="" />
              <span class="brand-text">
                <Wordmark />
                <Tagline />
              </span>
            </div>
            <ul class="nav-list">
              <li>
                <button
                  class={`nav-item ${is("home") || is("linksMap") ? "active" : ""}`}
                  aria-current={is("home") ? "page" : undefined}
                  onClick={() => go({ name: "home" })}
                >
                  {t("nav.devices")}
                  <Show when={connected() > 0}>
                    <span class="nav-count" aria-hidden="true">
                      {connected()}
                    </span>
                    <span class="sr-only">, {tn("a11y.devicesConnected", connected())}</span>
                  </Show>
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("notes") ? "active" : ""}`}
                  aria-current={is("notes") ? "page" : undefined}
                  onClick={() => go({ name: "notes" })}
                >
                  {t("nav.notes")}
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("timers") ? "active" : ""}`}
                  aria-current={is("timers") ? "page" : undefined}
                  onClick={() => go({ name: "timers" })}
                >
                  {t("nav.timers")}
                  <Show when={activeTimers() > 0}>
                    <span class="nav-count" aria-hidden="true">
                      {activeTimers()}
                    </span>
                    <span class="sr-only">, {tn("a11y.timersRunning", activeTimers())}</span>
                  </Show>
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("readLater") ? "active" : ""}`}
                  aria-current={is("readLater") ? "page" : undefined}
                  onClick={() => go({ name: "readLater" })}
                >
                  {t("nav.readLater")}
                  <Show when={unreadCount() > 0}>
                    <span class="nav-count" aria-hidden="true">
                      {unreadCount()}
                    </span>
                    <span class="sr-only">, {tn("a11y.unreadLinks", unreadCount())}</span>
                  </Show>
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("activity") ? "active" : ""}`}
                  aria-current={is("activity") ? "page" : undefined}
                  onClick={() => go({ name: "activity" })}
                >
                  {t("nav.activity")}
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("transfers") ? "active" : ""}`}
                  aria-current={is("transfers") ? "page" : undefined}
                  onClick={() => go({ name: "transfers" })}
                >
                  {t("nav.transfers")}
                  <Show when={activeTransfers() > 0}>
                    <span class="nav-count" aria-hidden="true">
                      {activeTransfers()}
                    </span>
                    <span class="sr-only">, {tn("a11y.transfersActive", activeTransfers())}</span>
                  </Show>
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("search") ? "active" : ""}`}
                  aria-current={is("search") ? "page" : undefined}
                  onClick={() => go({ name: "search" })}
                >
                  {t("nav.search")}
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("settings") || is("developer") || is("health") || is("setup") ? "active" : ""}`}
                  aria-current={is("settings") || is("developer") || is("health") || is("setup") ? "page" : undefined}
                  onClick={() => go({ name: "settings" })}
                >
                  {t("nav.settings")}
                </button>
              </li>
              <li>
                <button
                  class={`nav-item ${is("help") ? "active" : ""}`}
                  aria-current={is("help") ? "page" : undefined}
                  onClick={() => go({ name: "help" })}
                >
                  {t("nav.help")}
                </button>
              </li>
            </ul>
            {/* The health check found something red. */}
            <Show when={badCount() > 0}>
              <button class="health-badge small" onClick={() => go({ name: "health" })}>
                <span class="health-badge-dot" aria-hidden="true" />
                {t("health.badge")}
              </button>
            </Show>
            {/* Pop-ups held while a phone is on Do Not Disturb. */}
            <Show when={state.quietPhones.length > 0}>
              <div class="quiet-badge small" role="status" title={t("quiet.hint")}>
                <span class="quiet-dot" aria-hidden="true" />
                {t("quiet.on")}
              </div>
            </Show>
            {/* This computer's sound playing on a phone: always in sight,
                with its Stop, while it lasts. */}
            <Show when={(state.computerAudio?.playing_to.length ?? 0) > 0}>
              <div class="sound-badge small" role="status">
                <span class="sound-dot" aria-hidden="true" />
                <span class="sound-badge-text">
                  {t("computerAudio.badge", { names: (state.computerAudio?.playing_to ?? []).map(peerName).join(", ") })}
                </span>
                <button
                  class="btn btn-ghost btn-sm"
                  title={t("computerAudio.badgeStop")}
                  aria-label={t("computerAudio.badgeStop")}
                  onClick={() => commands.computerAudioStop(null)}
                >
                  {t("computerAudio.stop")}
                </button>
              </div>
            </Show>
            <div class="sidebar-foot muted small">{niceName(state.me?.name ?? "")}</div>
          </nav>
          <main class="content" id="main" tabIndex={-1} ref={main}>
            <UpdateBanner />
            <Switch>
              <Match when={view().name === "home"}>
                <Home
                  onOpen={(node) => go({ name: "device", node })}
                  onMap={() => go({ name: "linksMap" })}
                  onHealth={() => go({ name: "health" })}
                  onSetup={() => go({ name: "setup" })}
                />
              </Match>
              <Match when={view().name === "health"}>
                <Health {...healthNav} />
              </Match>
              <Match when={view().name === "setup"}>
                <Setup onClose={() => go({ name: "home" })} nav={healthNav} />
              </Match>
              <Match when={view().name === "linksMap"}>
                <LinksMap onDevice={openConnection} />
              </Match>
              <Match when={view().name === "device" && view()}>
                {(v) => (
                  <Device
                    node={(v() as { node: string }).node}
                    onBack={() => go({ name: "home" })}
                    onHealth={() => go({ name: "health" })}
                  />
                )}
              </Match>
              <Match when={view().name === "notes" && view()}>
                {(v) => <Notes request={(v() as { request?: NoteRequest }).request} />}
              </Match>
              <Match when={view().name === "timers"}>
                <Timers />
              </Match>
              <Match when={view().name === "readLater"}>
                <ReadLater />
              </Match>
              <Match when={view().name === "activity"}>
                <Activity
                  onOpen={(x) =>
                    x.kind === "timers"
                      ? go({ name: "timers" })
                      : openFound({ kind: x.kind, id: x.id, node: x.node, title: "", detail: "", score: 0 })
                  }
                />
              </Match>
              <Match when={view().name === "transfers"}>
                <Transfers />
              </Match>
              <Match when={view().name === "settings"}>
                <Settings
                  onDeveloper={() => go({ name: "developer" })}
                  onHealth={() => go({ name: "health" })}
                  onSetup={() => go({ name: "setup" })}
                />
              </Match>
              <Match when={view().name === "help" && view()}>
                {(v) => <Help topic={(v() as { topic?: string }).topic} onTopic={(topic) => go({ name: "help", topic })} />}
              </Match>
              <Match when={view().name === "search" && view()}>
                {(v) => <Search query={(v() as { query?: string }).query} onOpen={openFound} />}
              </Match>
              <Match when={view().name === "developer"}>
                <Developer onBack={() => go({ name: "settings" })} />
              </Match>
            </Switch>
          </main>
        </div>
        <CommandPalette
          onDevice={(node) => go({ name: "device", node })}
          onPlace={goPlace}
          onFound={openFound}
          onSearch={(query) => go({ name: "search", query })}
          onNote={openNote}
        />
        <CallOverlay />
        <SendToPicker />
        <DropAnywhere enabled={() => view().name !== "device"} />
        <DownloadOnPicker />
        <DownloadAsk />
        <HotspotAsk />
        <PrintAsk />
        <PermissionAsk />
        <PermissionNotice />
        <NoteReminders onOpen={(id) => openNote(id)} />
        <TimerAlarm onOpen={() => go({ name: "timers" })} />
        </Show>
      </Show>
      <Show when={state.ringing}>{(r) => <RingBanner from={r().from} />}</Show>
      <Toasts />
    </>
  );
}

/** Another device is looking for this one: any click silences it. Stop takes
 *  focus while it shows; focus goes back where it was after. */
function RingBanner(props: { from: string }) {
  let stop: HTMLButtonElement | undefined;
  let root: HTMLDivElement | undefined;
  const restore = rememberFocus();
  onMount(() => requestAnimationFrame(() => stop?.focus()));
  onCleanup(() => {
    const at = document.activeElement;
    if (!at || at === document.body || root?.contains(at)) restore();
  });
  return (
    <div
      ref={root}
      class="ring-banner"
      role="alertdialog"
      aria-modal="false"
      aria-labelledby="ring-banner-title"
      onClick={dismissRing}
      onKeyDown={(e) => e.key === "Escape" && dismissRing()}
    >
      <span class="ring-pulse" aria-hidden="true" />
      <strong id="ring-banner-title">{t("ring.title", { name: peerName(props.from) })}</strong>
      <button class="btn btn-primary" ref={stop} onClick={dismissRing}>
        {t("ring.stop")}
      </button>
    </div>
  );
}
