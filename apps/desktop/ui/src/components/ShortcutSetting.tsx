// Settings → "Keyboard shortcuts": keys you choose, pressed in any app, that
// make my-domain do something (send what you copied to your phone, ring or
// lock a device, open the command palette…). The list, and the form that
// adds or changes one, with a key recorder. They're kept and registered with
// the OS in Rust (src-tauri/src/shortcuts.rs), which also checks the keys.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  commands,
  type Automation_Serialize as Automation,
  type Binding_Serialize as Binding,
  type ShortcutList_Serialize as ShortcutList,
  type ShortcutView_Serialize as ShortcutView,
} from "../bindings";
import { peerName, state, unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";
import { focusSoon } from "../lib/a11y";

/** What a shortcut can do, what it needs, and how the form names it. */
type Needs = "nothing" | "device" | "deviceOrPhone" | "automation";
const ACTIONS: { id: string; needs: Needs; label: Key }[] = [
  { id: "send_clipboard", needs: "deviceOrPhone", label: "shortcuts.action.sendClipboard" },
  { id: "ring", needs: "device", label: "shortcuts.action.ring" },
  { id: "lock", needs: "device", label: "shortcuts.action.lock" },
  { id: "locate", needs: "device", label: "shortcuts.action.locate" },
  { id: "send_file", needs: "device", label: "shortcuts.action.sendFile" },
  { id: "palette", needs: "nothing", label: "shortcuts.action.palette" },
  { id: "window", needs: "nothing", label: "shortcuts.action.window" },
  { id: "phone_audio", needs: "device", label: "shortcuts.action.phoneAudio" },
  { id: "mute", needs: "device", label: "shortcuts.action.mute" },
  { id: "media", needs: "device", label: "shortcuts.action.media" },
  { id: "wake", needs: "device", label: "shortcuts.action.wake" },
  { id: "insert_photo", needs: "deviceOrPhone", label: "shortcuts.action.insertPhoto" },
  { id: "insert_scan", needs: "deviceOrPhone", label: "shortcuts.action.insertScan" },
  { id: "automation", needs: "automation", label: "shortcuts.action.automation" },
];

/** How a row says what a binding does. */
const DOES: Record<string, Key> = {
  send_clipboard: "shortcuts.does.sendClipboard",
  ring: "shortcuts.does.ring",
  lock: "shortcuts.does.lock",
  locate: "shortcuts.does.locate",
  send_file: "shortcuts.does.sendFile",
  palette: "shortcuts.does.palette",
  window: "shortcuts.does.window",
  phone_audio: "shortcuts.does.phoneAudio",
  mute: "shortcuts.does.mute",
  media: "shortcuts.does.media",
  wake: "shortcuts.does.wake",
  insert_photo: "shortcuts.does.insertPhoto",
  insert_scan: "shortcuts.does.insertScan",
  automation: "shortcuts.does.automation",
};

const needsOf = (action: string): Needs => ACTIONS.find((a) => a.id === action)?.needs ?? "nothing";

const MODIFIER_KEYS = new Set(["Meta", "Control", "Alt", "Shift", "OS", "AltGraph", "CapsLock", "Fn"]);

/** The keys of a keydown, written the way Rust reads them ("Cmd+Shift+Y").
 *  `key` is null while only modifiers are held. */
export function keysOf(
  e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "key" | "code">,
): { mods: string[]; key: string | null } {
  const mods: string[] = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (MODIFIER_KEYS.has(e.key) || !e.code) return { mods, key: null };
  // KeyY -> Y, Digit5 -> 5; the rest (F5, Space, ArrowUp, Comma…) as is.
  const key = e.code.replace(/^Key([A-Z])$/, "$1").replace(/^Digit(\d)$/, "$1");
  return { mods, key };
}

/** Held modifiers as people read them, while recording. */
function modsLabel(mods: string[], mac: boolean): string {
  if (mac) {
    const sym: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Cmd: "⌘" };
    return ["Ctrl", "Alt", "Shift", "Cmd"].filter((m) => mods.includes(m)).map((m) => sym[m]).join("");
  }
  const name: Record<string, string> = { Ctrl: "Ctrl", Alt: "Alt", Shift: "Shift", Cmd: "Win" };
  return ["Ctrl", "Alt", "Shift", "Cmd"].filter((m) => mods.includes(m)).map((m) => name[m]).join("+");
}

const blank = (): Binding => ({ id: "", action: "send_clipboard", node: "", automation: "", keys: "", enabled: true });

export function ShortcutSetting(props: { me?: string }) {
  const [list, setList] = createSignal<ShortcutList | null>(null);
  const [automations, setAutomations] = createSignal<Automation[]>([]);
  /** The binding being added or changed, if any. */
  const [draft, setDraft] = createSignal<Binding | null>(null);
  /** The draft's keys for people ("⇧⌘Y"). */
  const [label, setLabel] = createSignal("");
  const [recording, setRecording] = createSignal(false);
  /** What the recorder says (read out as it changes). */
  const [feedback, setFeedback] = createSignal("");
  const [feedbackBad, setFeedbackBad] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  let recorder: HTMLButtonElement | undefined;

  const mac = () => list()?.mac ?? /mac/i.test(navigator.platform || navigator.userAgent);
  const peers = () => state.peers.filter((p) => p.node_id !== props.me);

  onMount(async () => {
    setList(await commands.listShortcuts());
    commands.listAutomations().then(setAutomations, () => {});
  });
  // Never leave the shortcuts off.
  onCleanup(() => {
    if (recording()) commands.pauseShortcuts(false).catch(() => {});
  });

  const automationName = (id: string) => {
    const a = automations().find((x) => x.id === id);
    return a ? a.label || a.action : id;
  };

  /** What a binding does, for people. */
  const does = (b: Binding) => {
    if (b.action === "send_clipboard" && !b.node) return t("shortcuts.does.sendClipboardPhone");
    if (b.action === "insert_photo" && !b.node) return t("shortcuts.does.insertPhotoPhone");
    if (b.action === "insert_scan" && !b.node) return t("shortcuts.does.insertScanPhone");
    const key = DOES[b.action];
    if (!key) return b.action;
    return t(key, { name: b.node ? peerName(b.node) : "", what: automationName(b.automation) });
  };

  const patch = (p: Partial<Binding>) => setDraft((d) => (d ? { ...d, ...p } : d));

  // Keyboard: the form takes focus when it opens (its first field), and
  // "Add a shortcut" gets it back when the form closes.
  let addButton: HTMLButtonElement | undefined;
  let formEl: HTMLFormElement | undefined;
  const focusForm = () => focusSoon(() => formEl?.querySelector<HTMLElement>("select, button"));
  const startAdd = () => {
    setDraft(blank());
    setLabel("");
    setFeedback("");
    setFeedbackBad(false);
    focusForm();
  };
  const startEdit = (v: ShortcutView) => {
    setDraft({ ...v.binding });
    setLabel(v.label);
    setFeedback("");
    setFeedbackBad(false);
    focusForm();
  };
  const cancel = () => {
    stopRecording();
    setDraft(null);
    focusSoon(() => addButton);
  };

  const startRecording = () => {
    if (recording()) return;
    setRecording(true);
    setFeedbackBad(false);
    setFeedback(t("shortcuts.recordNow"));
    // So pressing a shortcut that's already set is seen here, not run.
    commands.pauseShortcuts(true).catch(() => {});
  };
  const stopRecording = () => {
    if (!recording()) return;
    setRecording(false);
    commands.pauseShortcuts(false).catch(() => {});
  };

  const onRecordKey = async (e: KeyboardEvent) => {
    if (!recording()) {
      // Enter / Space start it, like a click.
      return;
    }
    const plain = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
    // Tab still moves on (and stops recording through blur).
    if (e.key === "Tab" && plain) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && plain) {
      stopRecording();
      setFeedback(t("shortcuts.recordCancelled"));
      return;
    }
    const { mods, key } = keysOf(e);
    if (!key) {
      setFeedbackBad(false);
      setFeedback(mods.length ? t("shortcuts.recordHolding", { keys: modsLabel(mods, mac()) }) : t("shortcuts.recordNow"));
      return;
    }
    if (!mods.some((m) => m !== "Shift")) {
      setFeedbackBad(true);
      setFeedback(t("shortcuts.recordNeedsModifier"));
      return;
    }
    const keys = [...mods, key].join("+");
    try {
      const ok = await unwrap(commands.checkShortcutKeys(keys, draft()?.id ?? ""));
      patch({ keys: ok.keys });
      setLabel(ok.label);
      setFeedbackBad(false);
      setFeedback(t("shortcuts.recordGot", { keys: ok.label }));
      stopRecording();
    } catch (err) {
      setFeedbackBad(true);
      setFeedback(friendlyError(err));
    }
  };

  const save = async (b: Binding, quiet = false) => {
    setBusy(true);
    try {
      const next = await unwrap(commands.saveShortcut(b));
      setList(next);
      const mine = next.bindings.find((v) => v.binding.id === next.saved);
      if (mine?.problem && mine.binding.enabled) toast(mine.problem, "error");
      else if (!quiet) toast(t("shortcuts.saved"), "ok");
      return true;
    } catch (err) {
      toast(friendlyError(err), "error");
      return false;
    } finally {
      setBusy(false);
    }
  };

  const submit = async (e: Event) => {
    e.preventDefault();
    const d = draft();
    if (!d) return;
    if (!d.keys) {
      setFeedbackBad(true);
      setFeedback(t("shortcuts.recordFirst"));
      recorder?.focus();
      return;
    }
    const needs = needsOf(d.action);
    const b: Binding = {
      ...d,
      node: needs === "device" || needs === "deviceOrPhone" ? (needs === "device" ? d.node || peers()[0]?.node_id || "" : d.node) : "",
      automation: needs === "automation" ? d.automation || automations()[0]?.id || "" : "",
    };
    if (await save(b)) {
      stopRecording();
      setDraft(null);
      focusSoon(() => addButton);
    }
  };

  const remove = async (v: ShortcutView) => {
    setList(await commands.deleteShortcut(v.binding.id));
    toast(t("shortcuts.removed", { keys: v.label }), "ok");
  };

  const toggle = (v: ShortcutView, on: boolean) => save({ ...v.binding, enabled: on }, true);

  return (
    <article class="tile" id="settings-shortcuts">
      <h2>{t("shortcuts.title")}</h2>
      <p class="muted small">{t("shortcuts.hint")}</p>
      <Show when={list()}>
        {(l) => (
          <>
            <Show when={l().unavailable}>
              <p class="small shortcut-problem">{t("shortcuts.unavailable")}</p>
            </Show>
            <Show when={l().bindings.length} fallback={<p class="muted small">{t("shortcuts.empty")}</p>}>
              <ul class="shortcut-list">
                <For each={l().bindings}>
                  {(v) => (
                    <li class="shortcut-row" classList={{ "shortcut-off": !v.binding.enabled }}>
                      <div class="row">
                        <kbd class="shortcut-keys">{v.label}</kbd>
                        <span class="grow">{does(v.binding)}</span>
                        <input
                          type="checkbox"
                          role="switch"
                          checked={v.binding.enabled}
                          disabled={busy()}
                          aria-label={t("shortcuts.enabledLabel", { what: does(v.binding), keys: v.label })}
                          onChange={(e) => toggle(v, e.currentTarget.checked)}
                        />
                      </div>
                      <Show when={v.binding.enabled && v.problem}>
                        <p class="small shortcut-problem">{v.problem}</p>
                      </Show>
                      <div class="row shortcut-row-actions">
                        <button class="btn btn-ghost btn-sm" onClick={() => startEdit(v)}>
                          {t("shortcuts.edit")}
                        </button>
                        <button class="btn btn-ghost btn-sm" onClick={() => remove(v)}>
                          {t("shortcuts.remove")}
                        </button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </>
        )}
      </Show>

      <Show
        when={draft()}
        fallback={
          <button class="btn btn-ghost" ref={addButton} onClick={startAdd} disabled={list()?.unavailable}>
            {t("shortcuts.add")}
          </button>
        }
      >
        {(d) => (
          <form
            class="stack shortcut-form"
            ref={formEl}
            aria-labelledby="shortcut-form-title"
            onSubmit={submit}
            onKeyDown={(e) => {
              // Esc cancels, except while recording (there it stops recording).
              if (e.key === "Escape" && !recording()) {
                e.preventDefault();
                cancel();
              }
            }}
          >
            <span class="small" id="shortcut-form-title">
              {d().id ? t("shortcuts.editTitle") : t("shortcuts.addTitle")}
            </span>

            <label class="row small">
              <span class="muted">{t("shortcuts.actionLabel")}</span>
              <select class="grow" onChange={(e) => patch({ action: e.currentTarget.value })}>
                <For each={ACTIONS}>
                  {(a) => (
                    <option value={a.id} selected={a.id === d().action}>
                      {t(a.label)}
                    </option>
                  )}
                </For>
              </select>
            </label>
            <Show when={needsOf(d().action) === "device" || needsOf(d().action) === "deviceOrPhone"}>
              <label class="row small">
                <span class="muted">{t("shortcuts.deviceLabel")}</span>
                <select class="grow" onChange={(e) => patch({ node: e.currentTarget.value })}>
                  <Show when={needsOf(d().action) === "deviceOrPhone"}>
                    <option value="" selected={!d().node}>
                      {t("shortcuts.yourPhone")}
                    </option>
                  </Show>
                  <For each={peers()}>
                    {(p) => (
                      <option value={p.node_id} selected={p.node_id === d().node}>
                        {peerName(p.node_id)}
                      </option>
                    )}
                  </For>
                </select>
              </label>
              <Show when={needsOf(d().action) === "device" && !peers().length}>
                <p class="muted small">{t("shortcuts.noDevices")}</p>
              </Show>
            </Show>
            <Show when={needsOf(d().action) === "automation"}>
              <label class="row small">
                <span class="muted">{t("shortcuts.automationLabel")}</span>
                <select class="grow" onChange={(e) => patch({ automation: e.currentTarget.value })}>
                  <For each={automations()}>
                    {(a) => (
                      <option value={a.id} selected={a.id === d().automation}>
                        {a.label || a.action}
                      </option>
                    )}
                  </For>
                </select>
              </label>
              <Show when={!automations().length}>
                <p class="muted small">{t("shortcuts.noAutomations")}</p>
              </Show>
            </Show>
            <div class="row small">
              <span class="muted">{t("shortcuts.keysLabel")}</span>
              <button
                ref={recorder}
                type="button"
                class="btn btn-ghost shortcut-recorder grow"
                classList={{ "shortcut-recording": recording() }}
                aria-pressed={recording()}
                aria-describedby="shortcut-feedback"
                onClick={() => (recording() ? stopRecording() : startRecording())}
                on:keydown={onRecordKey}
                onBlur={stopRecording}
              >
                {recording() ? t("shortcuts.recording") : label() || t("shortcuts.record")}
              </button>
            </div>
            <p
              id="shortcut-feedback"
              class="small shortcut-feedback"
              classList={{ "shortcut-problem": feedbackBad(), muted: !feedbackBad() }}
              aria-live="polite"
              role="status"
            >
              {feedback() || t("shortcuts.recordHint")}
            </p>
            <div class="row">
              <button class="btn btn-primary" type="submit" disabled={busy() || recording()}>
                {t("settings.save")}
              </button>
              <button class="btn btn-ghost" type="button" onClick={cancel}>
                {t("settings.cancel")}
              </button>
            </div>
          </form>
        )}
      </Show>
    </article>
  );
}
