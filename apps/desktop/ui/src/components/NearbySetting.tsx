// "Use nearby mode for media" (backend `nearby.rs`, mdcore
// `features::nearby`, on by default, in the settings backup): while audio,
// a screen or a camera streams with a device on this network or in
// Bluetooth range, move it to a direct local link and buffer less.

import { createSignal, onMount } from "solid-js";
import { commands } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function NearbySetting() {
  const [on, setOn] = createSignal<boolean>();
  onMount(() => commands.nearbyMedia().then(setOn, (e) => toast(friendlyError(e), "error")));
  const save = async (v: boolean) => {
    try {
      await commands.setNearbyMedia(v);
      setOn(v);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  return (
    <>
      <label class="switch-row">
        <span>{t("nearby.setting")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={on() ?? true}
          disabled={on() === undefined}
          onChange={(e) => save(e.currentTarget.checked)}
        />
      </label>
      <p class="muted small">{t("nearby.settingHint")}</p>
    </>
  );
}
