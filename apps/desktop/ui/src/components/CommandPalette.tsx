// The command palette: Cmd+K (Ctrl+K elsewhere) anywhere in the window
// opens a search box over everything you can do. For each paired device:
// open its page, ring it, send it the clipboard or a file, and the actions it
// lists in its status (the device page's buttons); this computer's own
// actions; your notes ("New note", and each note by its title); Read later
// ("Save copied link to Read later", and each unread link to open); and the
// app's places (Settings and its sections, Help and each of its topics).
//
// Actions that need more from you (a number to call, a lost-mode message)
// open the device's page instead of running blind. Ones that can't run now
// show greyed with the reason. The last few run come first.
//
// Below the actions, what you typed is also searched across what this
// computer keeps (lib/search.ts): messages, notes, clipboard, notifications, files,
// synced folders, Help and more, a few per group, with the words marked.
// Enter on one opens it where it lives; the last option opens the Search
// screen with every result.

import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type LocalActions } from "../bindings";
import {
  askInsert,
  peerName,
  pickAndSendFile,
  pickAndSendFolder,
  pullClipboard,
  requestStatus,
  ringDevice,
  runAction,
  runLocalAction,
  sendClipboardTo,
  state,
  unwrap,
  RADIOS,
  type DeviceStatus,
  type Radio,
} from "../lib/core";
import { DEVICE_PRESETS, LOCAL_PRESETS, offeredPresets, type Preset } from "../lib/presets";
import { sortedNotes } from "../lib/notes";
import { openLink, saveCopiedLink, sortedReadLater } from "../lib/readLater";
import { copiedLink, openDownloadPicker, requestDownload } from "../lib/remoteDownload";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { TOPICS } from "../help/topics";
import { KIND_LABEL, queryWords, searchAll, type Found, type FoundGroup } from "../lib/search";
import { FoundText } from "./Found";
import { t, tn, type Key } from "../i18n";

export type Place = "home" | "linksMap" | "notes" | "timers" | "readLater" | "activity" | "transfers" | "settings" | "help";

type Cmd = {
  /** Stable across openings: the recents list keeps these. */
  id: string;
  label: string;
  /** Where it acts: a device's name, "This computer" or "Go to". */
  group: string;
  /** Why it can't run now (shown greyed with this). */
  disabled?: string;
  run: () => unknown;
};

const RECENT_KEY = "palette.recent";
const MAX_RECENT = 8;
const MAX_SHOWN = 60;
/** Search results per group in the palette (the Search screen has more). */
const FOUND_PER_GROUP = 3;
/** How long typing must pause before searching. */
const SEARCH_DELAY = 120;

/** One row of the list: an action, a search result, or "see all". */
type Entry = { type: "cmd"; cmd: Cmd } | { type: "hit"; hit: Found } | { type: "all" };

/** The palette action a result would repeat (devices, notes, Help,
 *  Settings). */
function sameAs(f: Found): string | null {
  if (f.kind === "devices") return `open:${f.id}`;
  if (f.kind === "notes") return `note.${f.id}`;
  if (f.kind === "read_later") return `readLater.${f.id}`;
  if (f.kind === "help") return `place.help.${f.id}`;
  if (f.kind === "settings") return `place.settings.${f.id}`;
  return null;
}

function loadRecent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string").slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}

function saveRecent(list: string[]) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list.slice(0, MAX_RECENT)));
  } catch {
    // Storage full or off: recents just aren't kept.
  }
}

/** Every typed word is in the label or the group (device name). */
export function matches(c: { label: string; group: string }, query: string): boolean {
  const hay = `${c.label} ${c.group}`.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((w) => hay.includes(w));
}

const isMac = () => /mac/i.test(navigator.platform || navigator.userAgent);

/** Report how a sent action went: on its way, or waiting for the device. */
const report = (what: string, name: string) => (via: string) =>
  toast(via === "queued" ? t("palette.queued", { what, name }) : t("palette.sent", { what, name }), "ok");

const fail = (e: unknown) => toast(friendlyError(e), "error");

/** The commands for one paired device. */
function deviceCommands(node: string, name: string, onDevice: (node: string) => void): Cmd[] {
  const live = !!state.links[node];
  const st: DeviceStatus | undefined = state.status[node];
  const has = (a: string) => !!st?.actions.includes(a);
  const out: Cmd[] = [];
  const add = (id: string, label: string, run: () => unknown, disabled?: string) =>
    out.push({ id: `${id}:${node}`, label, group: name, run, disabled });
  /** An action sent the way the device page's buttons send it. */
  const act = (id: string, label: string, action: string, args: Record<string, unknown>, disabled?: string) =>
    add(id, label, () => runAction(node, action, args).then(report(label, name), fail), disabled);
  /** Needs more from you: opens the device's page. */
  const page = (id: string, label: string) => add(id, label, () => onDevice(node));

  add("open", t("palette.open", { name }), () => onDevice(node));
  add("ring", t("palette.ring", { name }), () =>
    ringDevice(node).then(
      (via) => toast(via === "queued" ? t("control.ringQueued", { name }) : t("control.ringing", { name }), "ok"),
      fail,
    ),
  );
  add("clip", t("palette.sendClipboard", { name }), () =>
    sendClipboardTo(node).then(
      // "cancelled": it looked like a password and you said not to send it.
      (via) =>
        via !== "cancelled" &&
        toast(via === "queued" ? t("palette.clipQueued", { name }) : t("palette.clipSent", { name }), "ok"),
      fail,
    ),
  );
  // Asked of the device and answered at once: only over a live link.
  add(
    "getclip",
    t("palette.getClipboard", { name }),
    () => pullClipboard(node).then(() => toast(t("clipboard.copied"), "ok"), fail),
    live ? undefined : t("palette.offline", { name }),
  );
  // The copied link, downloaded by the device itself (only one that can).
  if (st?.remoteDownload) {
    add("download", t("palette.downloadOn", { name }), () =>
      copiedLink().then((url) =>
        url
          ? requestDownload(node, url).then(() => toast(t("download.asked", { device: name }), "ok"), fail)
          : openDownloadPicker(null),
      ),
    );
  }
  add("file", t("palette.sendFile", { name }), () =>
    pickAndSendFile(node).then((sent) => sent && toast(tn("files.started", 1, { name }), "ok"), fail),
  );
  add("folder", t("palette.sendFolder", { name }), () =>
    pickAndSendFolder(node).then((sent) => sent && toast(tn("files.started", sent, { name }), "ok"), fail),
  );
  page("link", t("palette.openLink"));

  if (!st) return out;

  // The presets schedules use, where the device lists them (Ring is above).
  const presets = offeredPresets(DEVICE_PRESETS, st.actions).filter((p) => p.action !== "ring");
  const preset = (p: Preset) => act(`preset.${p.id}`, t(p.label), p.action, p.args);
  for (const p of presets) if (p.id !== "torch_off" && p.id !== "media") preset(p);

  if (has("flashlight")) {
    if (st.torch) act("torch", t("schedule.preset.torchOff"), "flashlight", { on: false });
    else act("torch", t("palette.torchOn"), "flashlight", { on: true });
  }
  if (has("volume")) {
    act("louder", t("palette.louder"), "volume", { step: 1 });
    act("quieter", t("palette.quieter"), "volume", { step: -1 });
  }
  if (has("mute")) {
    if (st.muted) act("mute", t("palette.unmute"), "mute", { on: false });
    else act("mute", t("palette.mute"), "mute", { on: true });
  }
  if (has("media")) {
    act("media", t("schedule.preset.media"), "media", { key: "play_pause" });
    act("media.next", t("palette.next"), "media", { key: "next" });
    act("media.previous", t("palette.previous"), "media", { key: "previous" });
  }
  if (has("locate")) {
    const off = st.locationSharing ? undefined : t("locate.offOn", { name });
    add("locate", t("palette.locate"), () => runAction(node, "locate", {}).then(() => toast(t("locate.asking")), fail), off);
  }
  if (has("radio")) {
    for (const r of RADIOS.filter((r) => st.radios[r] !== undefined)) {
      const radio = t(`control.radio.${r}` as Key);
      const on = !st.radios[r];
      const direct = st.radioDirect.includes(r as Radio);
      add(`radio.${r}`, t(on ? "palette.radioOn" : "palette.radioOff", { radio }), () =>
        runAction(node, "radio", { what: r, on }).then(
          () => toast(t(direct ? "control.radioSwitching" : "control.radioAsked", { name, radio })),
          fail,
        ),
      );
    }
  }
  if (has("key")) {
    const off = st.helper === false ? t("control.keysOff", { name }) : undefined;
    const keys = [
      ["back", "control.key.back"],
      ["home", "control.key.home"],
      ["recents", "control.key.recents"],
      ["notifications", "control.key.notifications"],
      ["quick_settings", "control.key.quickSettings"],
      ["lock", "control.key.lock"],
    ] as const;
    for (const [key, label] of keys) {
      const what = t("palette.key", { key: t(label) });
      act(`key.${key}`, what, "key", { key }, off);
    }
  }
  if (has("run")) {
    for (const c of st.commands ?? []) {
      add(`run.${c.id}`, t("palette.run", { what: c.name }), () =>
        runAction(node, "run", { id: c.id }).then(
          (via) => toast(via === "queued" ? t("run.queued", { name }) : t("run.asked", { name, what: c.name })),
          fail,
        ),
      );
    }
  }
  // These take something from you: the device's page has the form.
  if (has("dial")) page("dial", t("palette.dial"));
  if (has("lost")) page("lost", t("palette.lost"));
  if (has("screen")) page("screen", t("palette.screen"));
  if (st?.cameraView && st.platform === "android") page("camera", t("palette.camera"));
  if (st?.insertCapture && st.platform === "android") {
    add("insert-photo", t("palette.insertPhoto", { name }), () => askInsert(node, "photo"));
    add("insert-scan", t("palette.insertScan", { name }), () => askInsert(node, "scan"));
  }
  if (has("screen_settings")) page("screen_settings", t("palette.screenSettings"));
  if (has("gallery")) page("gallery", t("palette.photos"));
  if (has("wifi")) page("wifi", t("palette.wifi"));
  if (has("launch")) page("launch", t("palette.apps"));
  return out;
}

export default function CommandPalette(props: {
  onDevice: (node: string) => void;
  onPlace: (place: Place, section?: string) => void;
  /** Open a search result where it lives. */
  onFound: (f: Found) => void;
  /** The Search screen, with this query. */
  onSearch: (query: string) => void;
  /** Open a note (`id` null: a new one). */
  onNote: (id: string | null) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [active, setActive] = createSignal(0);
  const [recent, setRecent] = createSignal<string[]>(loadRecent());
  const [local, setLocal] = createSignal<LocalActions>();
  const [found, setFound] = createSignal<FoundGroup[]>([]);
  let input: HTMLInputElement | undefined;
  let dialog: HTMLDivElement | undefined;
  let list: HTMLUListElement | undefined;
  let returnFocus: HTMLElement | null = null;

  const show = () => {
    returnFocus = document.activeElement as HTMLElement | null;
    setQuery("");
    setFound([]);
    setActive(0);
    setOpen(true);
    queueMicrotask(() => input?.focus());
    // This computer's actions change (a command switched on in Settings).
    unwrap(commands.localActions()).then(setLocal, () => {});
    // Fresh action lists from devices already linked (asking one that isn't
    // would wake it by push just for this).
    for (const p of state.peers) if (state.links[p.node_id]) requestStatus(p.node_id).catch(() => {});
  };
  const close = () => {
    setOpen(false);
    returnFocus?.focus?.();
    returnFocus = null;
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = isMac() ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
      if (mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (open()) close();
        else show();
      } else if (e.key === "Escape" && open()) {
        // Focus somehow left the box: Esc still closes it.
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
    // A keyboard shortcut (Settings) brings the window forward and opens it.
    const off = listen("open-palette", () => {
      if (!open()) show();
    });
    onCleanup(() => {
      off.then((f) => f(), () => {});
    });
  });

  const go = (place: Place, section?: string) => () => props.onPlace(place, section);
  const places = (): Cmd[] => {
    const group = t("palette.goTo");
    const section = (id: string, key: Key) => ({
      id: `place.settings.${id}`,
      label: t("palette.settingsSection", { section: t(key) }),
      group,
      run: go("settings", `settings-${id}`),
    });
    return [
      { id: "place.home", label: t("palette.devices"), group, run: go("home") },
      { id: "place.linksMap", label: t("palette.linksMap"), group, run: go("linksMap") },
      { id: "place.notes", label: t("palette.notes"), group, run: go("notes") },
      { id: "place.timers", label: t("palette.timers"), group, run: go("timers") },
      { id: "place.readLater", label: t("palette.readLater"), group, run: go("readLater") },
      { id: "place.activity", label: t("palette.activity"), group, run: go("activity") },
      { id: "place.transfers", label: t("palette.transfers"), group, run: go("transfers") },
      { id: "place.settings", label: t("palette.settings"), group, run: go("settings") },
      section("account", "settings.account"),
      section("device", "settings.thisDevice"),
      section("calls", "calls.title"),
      section("commands", "commands.title"),
      section("schedules", "schedule.local.title"),
      section("automations", "automation.title"),
      section("webhooks", "webhooks.title"),
      section("shortcuts", "shortcuts.title"),
      section("key", "settings.deviceKey"),
      section("appearance", "settings.appearance"),
      { id: "place.search", label: t("palette.search"), group, run: () => props.onSearch("") },
      { id: "place.help", label: t("palette.help"), group, run: go("help") },
      ...TOPICS.map((topic) => ({
        id: `place.help.${topic.id}`,
        label: t("palette.helpTopic", { topic: topic.title }),
        group,
        run: go("help", topic.id),
      })),
    ];
  };
  const localCommands = (): Cmd[] => {
    const me = local();
    if (!me) return [];
    const group = t("palette.here");
    return offeredPresets(LOCAL_PRESETS, me.actions).map((p) => {
      const what = t(p.label);
      return {
        id: `local.${p.id}`,
        label: what,
        group,
        run: () => runLocalAction(p.action, p.args).then(() => toast(t("palette.doneHere", { what }), "ok"), fail),
      };
    });
  };

  /** "New note", and each note by its title. */
  const noteCommands = (): Cmd[] => {
    const group = t("palette.notesGroup");
    return [
      { id: "note.new", label: t("palette.newNote"), group, run: () => props.onNote(null) },
      ...sortedNotes().map((n) => ({
        id: `note.${n.id}`,
        label: t("palette.note", { title: n.title || t("notes.untitled") }),
        group,
        run: () => props.onNote(n.id),
      })),
    ];
  };

  /** "Save copied link to Read later", and each unread link to open in
   *  the browser. */
  const readLaterCommands = (): Cmd[] => {
    const group = t("palette.readLaterGroup");
    return [
      {
        id: "readLater.saveCopied",
        label: t("palette.saveCopiedLink"),
        group,
        run: () => saveCopiedLink().then((i) => toast(t("readLater.saved", { title: i.title }), "ok"), fail),
      },
      // "Download on <device>": the copied link, on the device picked next.
      {
        id: "download.copied",
        label: t("palette.downloadCopiedLink"),
        group,
        run: () => copiedLink().then(openDownloadPicker),
      },
      ...sortedReadLater()
        .filter((i) => !i.read)
        .map((i) => ({
          id: `readLater.${i.id}`,
          label: t("palette.readLink", { title: i.title || i.host }),
          group,
          run: () => openLink(i.id).catch(fail),
        })),
    ];
  };

  const all = createMemo(() => {
    if (!open()) return [] as Cmd[];
    const devices = state.peers.flatMap((p) => deviceCommands(p.node_id, peerName(p.node_id), props.onDevice));
    return [...devices, ...localCommands(), ...noteCommands(), ...readLaterCommands(), ...places()];
  });

  /** Matches, the recent ones first (newest first), then in list order. */
  const shown = createMemo(() => {
    const q = query();
    const hits = all().filter((c) => matches(c, q));
    const rank = new Map(recent().map((id, i) => [id, i]));
    const first = hits.filter((c) => rank.has(c.id)).sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
    const rest = hits.filter((c) => !rank.has(c.id));
    return [...first, ...rest].slice(0, MAX_SHOWN);
  });

  // Search what this computer keeps, once typing pauses.
  let asked = 0;
  let timer: number | undefined;
  createEffect(
    on(query, (q) => {
      const mine = ++asked;
      clearTimeout(timer);
      if (!queryWords(q).length) {
        setFound([]);
        return;
      }
      timer = window.setTimeout(() => {
        searchAll(q, { limit: FOUND_PER_GROUP + 2 }).then(
          (groups) => mine === asked && open() && setFound(groups),
          () => {},
        );
      }, SEARCH_DELAY);
    }),
  );
  onCleanup(() => clearTimeout(timer));

  /** The search results not already in the actions, a few per group. */
  const groups = createMemo(() => {
    const ids = new Set(shown().map((c) => c.id));
    return found()
      .map((g) => ({ ...g, hits: g.hits.filter((h) => !ids.has(sameAs(h) ?? "")).slice(0, FOUND_PER_GROUP) }))
      .filter((g) => g.hits.length > 0);
  });
  const words = createMemo(() => queryWords(query()));

  /** Every row in order: actions, results by group, then "see all". */
  const entries = createMemo<Entry[]>(() => [
    ...shown().map((cmd): Entry => ({ type: "cmd", cmd })),
    ...groups().flatMap((g) => g.hits.map((hit): Entry => ({ type: "hit", hit }))),
    ...(words().length ? [{ type: "all" } as Entry] : []),
  ]);
  /** Where each group's rows start in `entries`. */
  const groupStart = (gi: number) => shown().length + groups().slice(0, gi).reduce((n, g) => n + g.hits.length, 0);
  const hitCount = () => groups().reduce((n, g) => n + g.hits.length, 0);
  const lastIndex = () => entries().length - 1;

  // A new search starts at the top.
  createEffect(on(query, () => setActive(0), { defer: true }));
  // Keep the highlighted one in view.
  createEffect(
    on(active, (i) => {
      if (!open()) return;
      list?.querySelector(`#palette-opt-${i}`)?.scrollIntoView({ block: "nearest" });
    }),
  );

  const optionId = (i: number) => `palette-opt-${i}`;
  const current = () => entries()[active()];

  const runCmd = (c: Cmd | undefined) => {
    if (!c) return;
    if (c.disabled) {
      toast(c.disabled);
      return;
    }
    const next = [c.id, ...recent().filter((x) => x !== c.id)].slice(0, MAX_RECENT);
    setRecent(next);
    saveRecent(next);
    close();
    try {
      const r = c.run();
      if (r instanceof Promise) r.catch(fail);
    } catch (e) {
      fail(e);
    }
  };

  const runEntry = (e: Entry | undefined) => {
    if (!e) return;
    if (e.type === "cmd") return runCmd(e.cmd);
    const q = query();
    close();
    if (e.type === "hit") props.onFound(e.hit);
    else props.onSearch(q);
  };

  const move = (by: number) => {
    const n = entries().length;
    if (!n) return;
    setActive((i) => (i + by + n) % n);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(-1);
        break;
      case "Home":
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          setActive(0);
        }
        break;
      case "End":
        if (e.ctrlKey || e.metaKey) {
          e.preventDefault();
          setActive(Math.max(0, lastIndex()));
        }
        break;
      case "Enter":
        e.preventDefault();
        runEntry(current());
        break;
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        close();
        break;
      case "Tab": {
        // Keep focus inside the dialog.
        const focusable = Array.from(
          dialog?.querySelectorAll<HTMLElement>('input, button, [href], [tabindex]:not([tabindex="-1"])') ?? [],
        ).filter((el) => !el.hasAttribute("disabled"));
        if (!focusable.length) {
          e.preventDefault();
          return;
        }
        const i = focusable.indexOf(document.activeElement as HTMLElement);
        const next = e.shiftKey ? (i <= 0 ? focusable.length - 1 : i - 1) : i === focusable.length - 1 ? 0 : i + 1;
        e.preventDefault();
        focusable[next].focus();
        break;
      }
    }
  };

  return (
    <Show when={open()}>
      <div class="palette-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="palette"
          ref={dialog}
          role="dialog"
          aria-modal="true"
          aria-label={t("palette.label")}
          onKeyDown={onKeyDown}
        >
          <input
            ref={input}
            class="palette-input"
            type="text"
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-autocomplete="list"
            aria-activedescendant={current() ? optionId(active()) : undefined}
            aria-label={t("palette.inputLabel")}
            placeholder={t("palette.placeholder")}
            autocomplete="off"
            spellcheck={false}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
          <ul class="palette-list" id="palette-list" role="listbox" aria-label={t("palette.results")} ref={list}>
            <Show when={!shown().length && !hitCount()}>
              <li class="palette-empty muted small" role="presentation">
                {t("palette.none", { query: query() })}
              </li>
            </Show>
            <Show when={shown().length > 0 && hitCount() > 0}>
              <li class="palette-heading small" role="presentation" aria-hidden="true">
                {t("palette.commands")}
              </li>
            </Show>
            <For each={shown()}>
              {(c, i) => (
                <li
                  id={optionId(i())}
                  class="palette-option"
                  classList={{ "palette-active": i() === active(), "palette-disabled": !!c.disabled }}
                  role="option"
                  aria-selected={i() === active()}
                  aria-disabled={c.disabled ? "true" : undefined}
                  title={c.disabled}
                  onMouseMove={() => i() !== active() && setActive(i())}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => runCmd(c)}
                >
                  <span class="palette-text">
                    <span class="palette-label">{c.label}</span>
                    <Show when={c.disabled}>
                      <span class="palette-reason small">{c.disabled}</span>
                    </Show>
                  </span>
                  <Show when={recent().includes(c.id)}>
                    <span class="palette-tag small">{t("palette.recent")}</span>
                  </Show>
                  <span class="palette-group small">{c.group}</span>
                </li>
              )}
            </For>
            <For each={groups()}>
              {(g, gi) => (
                <li role="group" class="palette-section" aria-labelledby={`palette-group-${g.kind}`}>
                  <div class="palette-heading small" id={`palette-group-${g.kind}`}>
                    {t(KIND_LABEL[g.kind])}
                  </div>
                  <ul class="palette-sublist" role="presentation">
                    <For each={g.hits}>
                      {(h, hi) => {
                        const i = () => groupStart(gi()) + hi();
                        return (
                          <li
                            id={optionId(i())}
                            class="palette-option palette-found"
                            classList={{ "palette-active": i() === active() }}
                            role="option"
                            aria-selected={i() === active()}
                            onMouseMove={() => i() !== active() && setActive(i())}
                            onMouseDown={(e) => e.preventDefault()}
                            onClick={() => runEntry({ type: "hit", hit: h })}
                          >
                            <FoundText f={h} words={words()} class="palette-text" />
                          </li>
                        );
                      }}
                    </For>
                  </ul>
                </li>
              )}
            </For>
            <Show when={words().length > 0}>
              <li
                id={optionId(lastIndex())}
                class="palette-option palette-all"
                classList={{ "palette-active": lastIndex() === active() }}
                role="option"
                aria-selected={lastIndex() === active()}
                onMouseMove={() => active() !== lastIndex() && setActive(lastIndex())}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => runEntry({ type: "all" })}
              >
                <span class="palette-text">
                  <span class="palette-label">{t("palette.allResults", { query: query().trim() })}</span>
                </span>
                <span class="palette-group small">{t("palette.searchScreen")}</span>
              </li>
            </Show>
          </ul>
          {/* How many match, said after typing (the list itself isn't live). */}
          <p class="sr-only" role="status">
            {hitCount()
              ? `${tn("a11y.paletteCount", shown().length)}, ${tn("a11y.foundCount", hitCount())}`
              : shown().length
                ? tn("a11y.paletteCount", shown().length)
                : t("palette.none", { query: query() })}
          </p>
          <p class="palette-hint muted small" aria-hidden="true">

            {t("palette.hint")}
          </p>
        </div>
      </div>
    </Show>
  );
}
