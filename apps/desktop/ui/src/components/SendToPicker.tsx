// "Send to…": files and folders sent here from Finder's Services menu,
// Explorer's Send to, a Linux file manager or `my-domain --send` (see
// src-tauri/src/send_to.rs). Lists your paired devices (the one chosen last
// time first, then the connected ones) and sends everything to the one you
// pick: files one by one, folders whole.

import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { listen } from "@tauri-apps/api/event";
import { commands, type SendItem } from "../bindings";
import { peerName, state } from "../lib/core";
import { friendlyError } from "../lib/words";
import { DeviceAvatar } from "./DeviceLabel";
import { StatusPill, toast } from "./ui";
import { t, tn } from "../i18n";
import { rememberFocus, trapTab } from "../lib/a11y";

/** Shown by name; the rest are counted. */
const NAMED = 3;

export default function SendToPicker() {
  const [items, setItems] = createSignal<SendItem[]>([]);
  const [last, setLast] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let dialog: HTMLDivElement | undefined;
  /** Where focus was before it opened; it goes back there on close. */
  let restore: (() => void) | undefined;

  /** Pick up what arrived; more while it's open are added to the list. */
  const check = async () => {
    const req = await commands.takeSendRequest();
    if (!req) return;
    if (!items().length) restore = rememberFocus();
    setLast(req.last_device);
    setItems((cur) => [...cur, ...req.items.filter((i) => !cur.some((c) => c.path === i.path))]);
    requestAnimationFrame(() => dialog?.querySelector<HTMLButtonElement>(".send-to-device")?.focus());
  };
  onMount(() => {
    check();
    const off = listen("send-request", () => check());
    onCleanup(() => off.then((f) => f()));
  });

  const close = () => {
    setItems([]);
    restore?.();
    restore = undefined;
  };

  /** The chosen-last device, then the connected ones, then by name. */
  const devices = createMemo(() =>
    [...state.peers].sort((a, b) => {
      const rank = (n: string) => (n === last() ? 0 : state.links[n] ? 1 : 2);
      return rank(a.node_id) - rank(b.node_id) || peerName(a.node_id).localeCompare(peerName(b.node_id));
    }),
  );

  const what = () => {
    const list = items();
    const names = list.slice(0, NAMED).map((i) => (i.folder ? t("sendTo.folderName", { name: i.name }) : i.name));
    const more = list.length - NAMED;
    return more > 0 ? t("sendTo.andMore", { names: names.join(", "), count: more }) : names.join(", ");
  };

  const send = async (node: string) => {
    if (busy()) return;
    setBusy(true);
    try {
      const out = await commands.sendPaths(node, items().map((i) => i.path));
      const name = peerName(node);
      if (out.files) toast(tn("files.started", out.files, { name }), "ok");
      for (const f of out.failed) toast(t("sendTo.failed", { what: f }), "error");
      if (!out.files && !out.failed.length) toast(t("sendTo.empty", { name }), "info");
      close();
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={items().length > 0}>
      <Portal>
        <div class="send-to-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
          <div
            class="send-to"
            ref={dialog}
            role="dialog"
            aria-modal="true"
            aria-labelledby="send-to-title"
            aria-describedby="send-to-what"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                close();
              } else trapTab(e, dialog);

            }}
          >
            <h2 id="send-to-title">{t("sendTo.title")}</h2>
            <p id="send-to-what" class="muted small send-to-what" title={items().map((i) => i.path).join("\n")}>
              {what()}
            </p>
            <ul class="send-to-list" aria-label={t("sendTo.devices")}>
              <For each={devices()} fallback={<li class="muted small">{t("sendTo.noDevices")}</li>}>
                {(p) => (
                  <li>
                    <button class="send-to-device" type="button" disabled={busy()} onClick={() => send(p.node_id)}>
                      <DeviceAvatar node={p.node_id} />
                      <span class="send-to-name">{peerName(p.node_id)}</span>
                      <Show when={p.node_id === last()}>
                        <span class="send-to-tag small">{t("sendTo.last")}</span>
                      </Show>
                      {/* Only Bluetooth reaches it (no network, no server): a
                          big file still goes, the offer for Wi-Fi Direct or
                          the phone's hotspot goes over Bluetooth. */}
                      <Show when={state.links[p.node_id] === "ble"}>
                        <span class="send-to-tag small">{t("sendTo.nearbyOffline")}</span>
                      </Show>
                      <StatusPill node={p.node_id} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
            <p class="muted small">{t("sendTo.hint")}</p>
            <div class="row send-to-actions">
              <button class="btn btn-ghost" type="button" onClick={close}>
                {t("settings.cancel")}
              </button>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}
