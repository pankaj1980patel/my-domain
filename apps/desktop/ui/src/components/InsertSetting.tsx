// "Insert from phone" settings (insert_capture.rs): "Insert where I am"
// pastes a photo or scan from the phone into the app in front as soon as it
// arrives (off by default). It uses the keyboard permission the phone's
// touchpad already needs (Accessibility on a Mac), never asked for here.

import { createSignal, onMount, Show } from "solid-js";
import { commands, type InsertSettings_Serialize as InsertState } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function InsertSetting() {
  const [s, setS] = createSignal<InsertState>();

  onMount(async () => {
    try {
      setS(await commands.insertSettings());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  });

  const toggle = async (on: boolean) => {
    try {
      await commands.setInsertPaste(on);
      setS(await commands.insertSettings());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  return (
    <article class="tile" id="settings-insert">
      <h2>
        {t("insert.settingsTitle")}
        <HelpLink topic="screen" />
      </h2>
      <p class="muted small">{t("insert.settingsHint")}</p>
      <label class="switch-row">
        <span>{t("insert.pasteSwitch")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={s()?.paste ?? false}
          disabled={!s()}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
      </label>
      <Show when={s()?.paste && s()?.blocked}>
        <p class="muted small">
          {s()?.blocked === "accessibility" ? t("insert.pasteNeedsAccessibility") : t("insert.pasteNoDisplay")}
        </p>
      </Show>
    </article>
  );
}
