// Removing a device for good (`mdcore::features::revoke`): "Remove device"
// on a device's page asks first, saying it's permanent for that install
// (your other devices refuse it too, even with the passphrase); Settings
// lists the removed devices, each with "Allow again".

import { createSignal, For, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { listen } from "@tauri-apps/api/event";
import { commands, type RemovedDevice } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { rememberFocus, trapTab } from "../lib/a11y";
import { toast } from "./ui";
import { t } from "../i18n";

const [removed, setRemoved] = createSignal<RemovedDevice[]>([]);

async function loadRemoved(): Promise<void> {
  setRemoved(await commands.removedDevices());
}

let watching = false;
function watchRemoved(): void {
  if (watching) return;
  watching = true;
  loadRemoved().catch(() => {});
  listen("revocations-changed", () => loadRemoved().catch(() => {})).catch(() => {});
}

/** "Remove device" on a device's page (asks first). */
export function RemoveDeviceButton(props: { node: string; name: string; onRemoved: () => void }) {
  const [asking, setAsking] = createSignal(false);
  return (
    <>
      <button class="btn btn-ghost btn-sm" title={t("removeDevice.hint")} onClick={() => setAsking(true)}>
        {t("removeDevice.button")}
      </button>
      <Show when={asking()}>
        <RemoveDeviceConfirm node={props.node} name={props.name} onClose={() => setAsking(false)} onRemoved={props.onRemoved} />
      </Show>
    </>
  );
}

/** "Remove device?", Cancel first (the device page's "⋯" menu opens it). */
export function RemoveDeviceConfirm(props: { node: string; name: string; onClose: () => void; onRemoved: () => void }) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  const remove = () => {
    const name = props.name;
    unwrap(commands.removeDevice(props.node))
      .then(() => {
        toast(t("removeDevice.done", { name }), "ok");
        loadRemoved().catch(() => {});
        props.onClose();
        props.onRemoved();
      })
      .catch((e) => {
        toast(friendlyError(e), "error");
        close();
      });
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview key-confirm"
          ref={(el) => {
            dialog = el;
            // Cancel first: removing is never the default.
            queueMicrotask(() => el.querySelector<HTMLElement>("button")?.focus());
          }}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="remove-device-title"
          aria-describedby="remove-device-body"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, dialog);
          }}
        >
          <h2 id="remove-device-title">{t("removeDevice.title", { name: props.name })}</h2>
          <p id="remove-device-body" class="small">
            {t("removeDevice.body", { name: props.name })}
          </p>
          <p class="small muted">{t("removeDevice.passphrase")}</p>
          <div class="row">
            <button class="btn btn-ghost btn-sm" onClick={close}>
              {t("keyChange.cancel")}
            </button>
            <span class="grow" />
            <button class="btn btn-primary btn-sm" onClick={remove}>
              {t("removeDevice.button")}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}

/** Settings: the removed devices, each with "Allow again". */
export function RemovedDevicesSetting() {
  onMount(watchRemoved);
  const allow = (d: RemovedDevice) =>
    unwrap(commands.allowDeviceAgain(d.node_id))
      .then(() => toast(t("removedDevices.allowed", { name: d.name }), "ok"))
      .catch((e) => toast(friendlyError(e), "error"))
      .finally(() => loadRemoved().catch(() => {}));
  return (
    <article class="tile" id="settings-removed-devices">
      <h2>{t("removedDevices.title")}</h2>
      <Show when={removed().length > 0} fallback={<p class="muted small">{t("removedDevices.none")}</p>}>
        <p class="muted small">{t("removedDevices.hint")}</p>
        <div class="removed-list">
          <For each={removed()}>
            {(d) => (
              <div class="row removed-row">
                <div class="grow">
                  <div>{d.name}</div>
                  <Show when={d.fingerprint}>
                    <div class="small mono muted">{d.fingerprint}</div>
                  </Show>
                  <div class="small muted">
                    {d.by_me ? t("removedDevices.byMe") : t("removedDevices.by", { name: d.by_name })}
                  </div>
                </div>
                <button class="btn btn-soft btn-sm" onClick={() => allow(d)}>
                  {t("removedDevices.allow")}
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
    </article>
  );
}
