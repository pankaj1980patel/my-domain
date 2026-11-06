// "Show my phone's battery in the menu bar" (tray_battery.rs, off by
// default): the phone's battery next to the icon (macOS, Linux) or in its
// tooltip (Windows), a red dot on the icon when it's low, and which phone
// when there are several.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type TrayBatterySetting as Setting } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function TrayBatterySetting() {
  const [s, setS] = createSignal<Setting>();
  onMount(() => commands.trayBatterySetting().then(setS, (e) => toast(friendlyError(e), "error")));

  const save = async (patch: Partial<Pick<Setting, "on" | "badge" | "phone">>) => {
    const cur = s();
    if (!cur) return;
    const next = { ...cur, ...patch };
    try {
      await commands.setTrayBattery(next.on, next.badge, next.phone);
      setS(next);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  return (
    <div class="tray-battery">
      <label class="switch-row">
        <span>{t("trayBattery.switch")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={s()?.on ?? false}
          disabled={!s()}
          onChange={(e) => save({ on: e.currentTarget.checked })}
        />
      </label>
      <p class="muted small">{t("trayBattery.hint")}</p>
      <Show when={s()?.on}>
        <div class="tray-battery-more">
          <Show when={(s()?.phones.length ?? 0) > 1}>
            <label class="row small">
              <span class="muted">{t("trayBattery.phone")}</span>
              <select class="grow" onChange={(e) => save({ phone: e.currentTarget.value })}>
                <option value="" selected={!s()?.phone}>
                  {t("trayBattery.anyPhone")}
                </option>
                <For each={s()?.phones ?? []}>
                  {(p) => (
                    <option value={p.node} selected={p.node === s()?.phone}>
                      {p.name}
                    </option>
                  )}
                </For>
              </select>
            </label>
          </Show>
          <Show when={!s()?.phones.length}>
            <p class="muted small">{t("trayBattery.noPhone")}</p>
          </Show>
          <label class="switch-row">
            <span>{t("trayBattery.badge")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={s()?.badge ?? true}
              onChange={(e) => save({ badge: e.currentTarget.checked })}
            />
          </label>
          <p class="muted small">{t("trayBattery.badgeHint")}</p>
        </div>
      </Show>
    </div>
  );
}
