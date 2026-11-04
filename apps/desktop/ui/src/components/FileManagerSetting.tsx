// Settings: "Send from the file manager". macOS lists "Send with my-domain"
// in Finder's Services menu by itself; on Windows and Linux a switch adds or
// removes the entries (Explorer's Send to; "Open with", Dolphin and
// Nautilus). See src-tauri/src/send_to.rs.

import { createSignal, onMount, Show } from "solid-js";
import { commands, type FileManagerMenu } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function FileManagerSetting() {
  const [s, setS] = createSignal<FileManagerMenu>();
  const [busy, setBusy] = createSignal(false);
  onMount(() => commands.fileManagerMenu().then(setS, () => {}));

  const toggle = async (on: boolean) => {
    setBusy(true);
    try {
      setS(await unwrap(commands.setFileManagerMenu(on)));
    } catch (err) {
      toast(friendlyError(err), "error");
      commands.fileManagerMenu().then(setS, () => {});
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="tile" id="settings-file-manager">
      <h2>{t("sendTo.settingTitle")}</h2>
      <Show when={s()}>
        {(cur) => (
          <Show
            when={cur().switchable}
            fallback={
              <Show when={cur().os === "macos"} fallback={<p class="muted small">{t("sendTo.settingOther")}</p>}>
                <p class="muted small">{t("sendTo.settingMac")}</p>
                <p class="muted small">{t("sendTo.settingMacMissing")}</p>
                <code class="file-manager-cmd">/System/Library/CoreServices/pbs -update</code>
              </Show>
            }
          >
            <label class="switch-row">
              <span>{cur().os === "windows" ? t("sendTo.settingWindows") : t("sendTo.settingLinux")}</span>
              <input
                type="checkbox"
                role="switch"
                checked={cur().on}
                disabled={busy()}
                onChange={(e) => toggle(e.currentTarget.checked)}
              />
            </label>
            <p class="muted small">{cur().os === "windows" ? t("sendTo.settingWindowsHint") : t("sendTo.settingLinuxHint")}</p>
          </Show>
        )}
      </Show>
      <p class="muted small">{t("sendTo.settingCli")}</p>
    </article>
  );
}
