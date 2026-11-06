// Big files with a phone nearby that isn't on this computer's network:
// "Use Wi-Fi Direct for big files when nearby" (wifi_direct.rs, on by
// default; only where this computer can host a Wi-Fi Direct group:
// Windows), and "Use the phone’s hotspot for big files" (hotspot.rs: ask
// each time, always, never; anywhere with Wi-Fi).

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type Mode, type WifiDirectSetting as Setting } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";

const MODES: Mode[] = ["ask", "always", "never"];

export function WifiDirectSetting() {
  const [s, setS] = createSignal<Setting>();
  onMount(() => commands.wifiDirectSetting().then(setS, (e) => toast(friendlyError(e), "error")));

  const change = async (on: boolean) => {
    try {
      setS(await commands.setWifiDirect(on));
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const changeHotspot = async (mode: Mode) => {
    try {
      setS(await commands.setPhoneHotspot(mode));
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  return (
    <>
      <Show when={s()?.supported}>
        <label class="switch-row">
          <span>{t("settings.wifiDirect")}</span>
          <input type="checkbox" role="switch" checked={s()?.on ?? true} onChange={(e) => change(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.wifiDirectHint")}</p>
      </Show>
      <Show when={s()?.hotspot_supported}>
        <label class="row small">
          <span class="grow">{t("settings.phoneHotspot")}</span>
          <select aria-label={t("settings.phoneHotspot")} onChange={(e) => changeHotspot(e.currentTarget.value as Mode)}>
            <For each={MODES}>
              {(m) => (
                <option value={m} selected={s()?.hotspot === m}>
                  {t(`settings.phoneHotspot.${m}` as Key)}
                </option>
              )}
            </For>
          </select>
        </label>
        <p class="muted small">{t("settings.phoneHotspotHint")}</p>
      </Show>
    </>
  );
}
