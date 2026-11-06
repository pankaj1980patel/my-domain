// Files and folders dragged from Finder / Explorer onto the window (except a
// device's page, which has its own drop, DropToSend): dropped on a device
// card, they go to that device; anywhere else, to your only device, or a
// picker asks which one when there are several. Several files or a folder
// go as one batch (one row in Transfers).

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { peerName, sendPaths, state } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { DeviceAvatar } from "./DeviceLabel";
import { t, tn } from "../i18n";
import { rememberFocus, trapTab } from "../lib/a11y";

/** The device card under the pointer (`data-drop-node`), if any. */
function cardAt(x: number, y: number): HTMLElement | null {
  const ratio = window.devicePixelRatio || 1;
  const el = document.elementFromPoint(x / ratio, y / ratio);
  return (el?.closest("[data-drop-node]") as HTMLElement | null) ?? null;
}

export default function DropAnywhere(props: { enabled: () => boolean }) {
  const [over, setOver] = createSignal(false);
  const [pending, setPending] = createSignal<string[]>([]);
  let lit: HTMLElement | null = null;
  let restore: (() => void) | undefined;
  let dialog: HTMLDivElement | undefined;

  const light = (el: HTMLElement | null) => {
    if (lit === el) return;
    lit?.classList.remove("drop-target");
    el?.classList.add("drop-target");
    lit = el;
  };

  const send = async (node: string, paths: string[]) => {
    try {
      const n = await sendPaths(node, paths);
      toast(tn("files.started", n, { name: peerName(node) }), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const close = () => {
    setPending([]);
    restore?.();
    restore = undefined;
  };

  const choose = (paths: string[]) => {
    const peers = state.peers;
    if (!peers.length) return toast(t("drop.noDevices"), "error");
    if (peers.length === 1) return void send(peers[0].node_id, paths);
    restore = rememberFocus();
    setPending(paths);
    requestAnimationFrame(() => dialog?.querySelector<HTMLButtonElement>("button")?.focus());
  };

  let gone = false;
  let unlisten: (() => void) | undefined;
  onCleanup(() => {
    gone = true;
    unlisten?.();
    light(null);
  });
  onMount(async () => {
    const stop = await getCurrentWebview().onDragDropEvent((ev) => {
      if (!props.enabled()) {
        setOver(false);
        light(null);
        return;
      }
      const p = ev.payload;
      if (p.type === "enter" || p.type === "over") {
        setOver(true);
        light(cardAt(p.position.x, p.position.y));
      } else if (p.type === "leave") {
        setOver(false);
        light(null);
      } else if (p.type === "drop") {
        setOver(false);
        const card = cardAt(p.position.x, p.position.y);
        light(null);
        if (!p.paths.length) return;
        const node = card?.dataset.dropNode;
        if (node) void send(node, p.paths);
        else choose(p.paths);
      }
    });
    if (gone) stop();
    else unlisten = stop;
  });

  return (
    <>
      <Show when={over()}>
        <Portal>
          <div class="drop-overlay" role="status" aria-live="polite" style={{ background: "transparent" }}>
            <div class="drop-card">
              <strong>{t("drop.anywhere")}</strong>
              <span class="muted small">{t("drop.anywhereHint")}</span>
            </div>
          </div>
        </Portal>
      </Show>
      <Show when={pending().length > 0}>
        <Portal>
          <div class="drop-overlay" style={{ "pointer-events": "auto" }} onClick={(e) => e.target === e.currentTarget && close()}>
            <div
              class="drop-card drop-pick"
              role="dialog"
              aria-modal="true"
              aria-labelledby="drop-pick-title"
              ref={dialog}
              onKeyDown={(e) => {
                if (e.key === "Escape") close();
                else if (dialog) trapTab(e, dialog);
              }}
            >
              <strong id="drop-pick-title">{t("drop.pickTitle")}</strong>
              <span class="muted small">{tn("drop.pickBody", pending().length)}</span>
              <ul class="drop-pick-list">
                <For each={state.peers}>
                  {(p) => (
                    <li>
                      <button
                        class="btn btn-soft"
                        onClick={() => {
                          const paths = pending();
                          close();
                          void send(p.node_id, paths);
                        }}
                      >
                        <DeviceAvatar node={p.node_id} />
                        <span>{peerName(p.node_id)}</span>
                      </button>
                    </li>
                  )}
                </For>
              </ul>
              <button class="btn btn-ghost btn-sm" onClick={close}>
                {t("drop.cancel")}
              </button>
            </div>
          </div>
        </Portal>
      </Show>
    </>
  );
}
