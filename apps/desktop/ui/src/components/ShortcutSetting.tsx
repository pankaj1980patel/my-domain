// Settings: the keyboard shortcut that sends what you copied to your phone
// (the shortcut itself is registered in Rust, src-tauri/src/shortcut.rs).

import { createSignal, onMount, Show } from "solid-js";
import { commands, type ShortcutSettings_Serialize } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function ShortcutSetting() {
  const [s, setS] = createSignal<ShortcutSettings_Serialize | null>(null);
  const [editing, setEditing] = createSignal(false);
  const [keys, setKeys] = createSignal("");
  onMount(async () => setS(await commands.sendShortcut()));

  const toggle = async (on: boolean) => setS(await commands.setSendShortcut(on));
  const save = async (e: Event) => {
    e.preventDefault();
    try {
      setS(await unwrap(commands.setSendShortcutKeys(keys())));
      setEditing(false);
      toast(t("settings.shortcutSaved"), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };

  return (
    <Show when={s()}>
      {(cur) => (
        <>
          <label class="switch-row">
            <span>{t("settings.shortcut")}</span>
            <input type="checkbox" role="switch" checked={cur().on} onChange={(e) => toggle(e.currentTarget.checked)} />
          </label>
          <p class="muted small">{t("settings.shortcutHint", { keys: cur().label })}</p>
          <Show when={cur().on && cur().problem}>
            <p class="small">{cur().problem}</p>
          </Show>
          <Show
            when={editing()}
            fallback={
              <button
                class="btn btn-ghost"
                onClick={() => {
                  setKeys(cur().keys);
                  setEditing(true);
                }}
              >
                {t("settings.shortcutChange")}
              </button>
            }
          >
            <form class="stack" onSubmit={save}>
              <input
                type="text"
                placeholder="CmdOrCtrl+Shift+Y"
                aria-label={t("settings.shortcutKeys")}
                value={keys()}
                onInput={(e) => setKeys(e.currentTarget.value)}
              />
              <p class="muted small">{t("settings.shortcutKeysHint")}</p>
              <div class="row">
                <button class="btn btn-primary" type="submit" disabled={!keys().trim()}>
                  {t("settings.save")}
                </button>
                <button class="btn btn-ghost" type="button" onClick={() => setEditing(false)}>
                  {t("settings.cancel")}
                </button>
              </div>
            </form>
          </Show>
        </>
      )}
    </Show>
  );
}
