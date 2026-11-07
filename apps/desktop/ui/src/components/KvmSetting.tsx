// Settings → "Share mouse and keyboard" (backend kvm.rs, mdcore
// features::kvm): this computer in the middle, a slot on each side; drag
// your other computers into the slot for the side they sit on (or pick the
// side from the list: the same thing without a mouse). The other computer
// learns where this one sits, so both agree. Then the switches: share
// this computer's mouse and keyboard, which computers may control this one
// (a switch on each, off until allowed), the
// clipboard following the pointer, Cmd ↔ Ctrl, and the hotkey that brings
// everything back.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type KvmSettings_Serialize as Settings, type KvmComputer } from "../bindings";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";
import { HelpLink } from "../help/HelpLink";

const SIDES = ["top", "left", "right", "bottom"] as const;
type SideName = (typeof SIDES)[number];
const HOTKEYS = ["ctrl+alt+esc", "ctrl+alt+f12", "scroll_lock"];
const DRAG_TYPE = "application/x-mydomain-computer";

export function KvmSetting() {
  const [s, setS] = createSignal<Settings>();
  const [over, setOver] = createSignal<SideName | null>(null);
  const load = () => commands.kvmSettings().then(setS, (e) => toast(friendlyError(e), "error"));
  onMount(() => {
    load();
    const subs = [listen("kvm-layout", load), listen("kvm-changed", load), listen("peers-updated", load)];
    onCleanup(() => subs.forEach((p) => p.then((un) => un())));
  });

  const apply = async (p: Promise<Settings | { status: "ok"; data: Settings } | { status: "error"; error: string }>) => {
    try {
      const r = await p;
      if ("status" in r) {
        if (r.status === "error") throw r.error;
        setS(r.data);
      } else {
        setS(r);
      }
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const place = (node: string, side: SideName | null) => apply(commands.kvmPlace(node, side));
  const at = (side: SideName) => s()?.layout.find((n) => n.side === side);
  const sideOf = (node: string) => s()?.layout.find((n) => n.node_id === node)?.side ?? "";
  const computer = (node: string): KvmComputer | undefined => s()?.computers.find((c) => c.node_id === node);
  const nameOf = (node: string) => niceName(computer(node)?.name ?? node);
  const status = () => {
    const st = s()?.state;
    if (!st || st.role === "idle") return null;
    const name = niceName(st.name ?? "");
    return st.role === "controlling" ? t("kvm.controlling", { name }) : t("kvm.controlled", { name });
  };

  const drop = (side: SideName, e: DragEvent) => {
    e.preventDefault();
    setOver(null);
    const node = e.dataTransfer?.getData(DRAG_TYPE);
    if (node) place(node, side);
  };

  return (
    <article class="tile" id="settings-kvm">
      <h2>
        {t("kvm.title")} <HelpLink topic="share-mouse-keyboard" />
      </h2>
      <Show when={s()} fallback={<p class="muted small">…</p>}>
        {(cur) => (
          <>
            <Show when={cur().supported} fallback={<p class="muted">{t("kvm.unsupported")}</p>}>
              <p class="muted small">{t("kvm.intro")}</p>
              <Show when={cur().blocked}>
                {(why) => <p class="kvm-blocked small" role="note">{t(`kvm.blocked.${why()}` as Key)}</p>}
              </Show>
              <Show when={status()}>
                {(line) => (
                  <div class="kvm-status" role="status">
                    <span>{line()}</span>
                    <button class="btn btn-ghost btn-sm" onClick={() => commands.kvmStop()}>
                      {cur().state.role === "controlled" ? t("kvm.takeBack") : t("kvm.stop")}
                    </button>
                  </div>
                )}
              </Show>

              <div class="kvm-grid" aria-label={t("kvm.layout")}>
                <For each={SIDES}>
                  {(side) => (
                    <div
                      class={`kvm-slot kvm-${side}`}
                      classList={{ "kvm-over": over() === side, "kvm-filled": !!at(side) }}
                      onDragOver={(e) => {
                        e.preventDefault();
                        setOver(side);
                      }}
                      onDragLeave={() => setOver(null)}
                      onDrop={(e) => drop(side, e)}
                    >
                      <span class="kvm-side muted small">{t(`kvm.side.${side}` as Key)}</span>
                      <Show when={at(side)} fallback={<span class="muted small">{t("kvm.dropHere")}</span>}>
                        {(n) => (
                          <span
                            class="kvm-chip"
                            draggable={true}
                            onDragStart={(e) => e.dataTransfer?.setData(DRAG_TYPE, n().node_id)}
                          >
                            {nameOf(n().node_id)}
                            <button
                              class="kvm-x"
                              aria-label={t("kvm.remove", { name: nameOf(n().node_id) })}
                              title={t("kvm.remove", { name: nameOf(n().node_id) })}
                              onClick={() => place(n().node_id, null)}
                            >
                              ×
                            </button>
                          </span>
                        )}
                      </Show>
                    </div>
                  )}
                </For>
                <div class="kvm-me">{t("kvm.thisComputer")}</div>
              </div>

              <h3 class="small">{t("kvm.computers")}</h3>
              <Show when={cur().computers.length > 0} fallback={<p class="muted small">{t("kvm.noComputers")}</p>}>
                <ul class="kvm-list">
                  <For each={cur().computers}>
                    {(c) => (
                      <li class="row small">
                        <span
                          class="kvm-chip grow"
                          draggable={true}
                          onDragStart={(e) => e.dataTransfer?.setData(DRAG_TYPE, c.node_id)}
                        >
                          {niceName(c.name)}
                          <Show when={!c.linked}>
                            <span class="muted"> · {t("kvm.notLinked")}</span>
                          </Show>
                        </span>
                        <label class="row small" title={t("kvm.allowed", { name: niceName(c.name) })}>
                          <input
                            type="checkbox"
                            role="switch"
                            aria-label={t("kvm.allowed", { name: niceName(c.name) })}
                            checked={c.allowed}
                            onChange={(e) => apply(commands.kvmSetAllowed(c.node_id, e.currentTarget.checked))}
                          />
                        </label>
                        <select
                          aria-label={t("kvm.place", { name: niceName(c.name) })}
                          onChange={(e) => place(c.node_id, (e.currentTarget.value || null) as SideName | null)}
                        >
                          <option value="" selected={sideOf(c.node_id) === ""}>
                            {t("kvm.notPlaced")}
                          </option>
                          <For each={SIDES}>
                            {(side) => (
                              <option value={side} selected={sideOf(c.node_id) === side}>
                                {t(`kvm.side.${side}` as Key)}
                              </option>
                            )}
                          </For>
                        </select>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>

              <label class="switch-row">
                <span>{t("kvm.enabled")}</span>
                <input
                  type="checkbox"
                  role="switch"
                  checked={cur().enabled}
                  onChange={(e) => apply(commands.kvmSetEnabled(e.currentTarget.checked))}
                />
              </label>
              <p class="muted small">{t("kvm.allowedHint")}</p>
              <label class="switch-row">
                <span>{t("kvm.clip")}</span>
                <input
                  type="checkbox"
                  role="switch"
                  checked={cur().clip_follows}
                  onChange={(e) => apply(commands.kvmSetClip(e.currentTarget.checked))}
                />
              </label>
              <p class="muted small">{t("kvm.clipHint")}</p>
              <label class="switch-row">
                <span>{t("kvm.swap")}</span>
                <input
                  type="checkbox"
                  role="switch"
                  checked={cur().swap_mods}
                  onChange={(e) => apply(commands.kvmSetSwap(e.currentTarget.checked))}
                />
              </label>
              <p class="muted small">{t("kvm.swapHint")}</p>
              <label class="row small">
                <span class="grow">{t("kvm.hotkey")}</span>
                <select aria-label={t("kvm.hotkey")} onChange={(e) => apply(commands.kvmSetHotkey(e.currentTarget.value))}>
                  <For each={HOTKEYS}>
                    {(h) => (
                      <option value={h} selected={cur().hotkey === h}>
                        {t(`kvm.hotkey.${h}` as Key)}
                      </option>
                    )}
                  </For>
                </select>
              </label>
              <p class="muted small">{t("kvm.hotkeyHint")}</p>
            </Show>
          </>
        )}
      </Show>
    </article>
  );
}
