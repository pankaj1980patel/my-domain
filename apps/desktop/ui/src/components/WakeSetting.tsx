// "Let my other devices wake this computer" (wol.rs): off by default. On,
// this computer's status lists its network cards so your phone and other
// computers can wake it from sleep. Shows how to set the computer itself
// to wake (per OS), whether a Mac's "Wake for network access" is on, and
// the cards it shares.

import { createSignal, onMount, Show } from "solid-js";
import { commands, type WakeSetting_Serialize as WakeState } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function WakeSetting() {
  const [s, setS] = createSignal<WakeState>();
  const [busy, setBusy] = createSignal(false);

  onMount(async () => {
    try {
      setS(await commands.wakeSetting());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  });

  const toggle = async (on: boolean) => {
    setBusy(true);
    try {
      setS(await commands.setWakeSetting(on));
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const osHint = (): Key => (s()?.os === "macos" ? "wake.hintMac" : s()?.os === "windows" ? "wake.hintWindows" : "wake.hintLinux");
  const cards = () =>
    (s()?.ifaces ?? []).map((c) => t(c.wired ? "wake.wired" : "wake.wifi", { iface: c.iface })).join(", ");
  const wifiOnly = () => {
    const list = s()?.ifaces ?? [];
    return list.length > 0 && list.every((c) => !c.wired);
  };

  return (
    <article class="tile" id="settings-wake">
      <h2>
        {t("wake.title")}
        <HelpLink topic="wake" />
      </h2>
      <label class="switch-row">
        <span>{t("wake.switch")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={s()?.on ?? false}
          disabled={busy() || !s()}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
      </label>
      <p class="muted small">{t("wake.switchHint")}</p>
      <p class="muted small">{t(osHint())}</p>
      <Show when={s()?.womp != null}>
        <p class={s()?.womp ? "wake-ok small" : "wake-warn small"} role={s()?.womp ? undefined : "alert"}>
          {t(s()?.womp ? "wake.wompOn" : "wake.wompOff")}
        </p>
      </Show>
      <Show when={s()?.on}>
        <p class="muted small">
          {(s()?.ifaces.length ?? 0) > 0 ? t("wake.cards", { cards: cards() }) : t("wake.noCards")}
        </p>
        <Show when={wifiOnly()}>
          <p class="wake-warn small">{t("wake.wifiOnly")}</p>
        </Show>
      </Show>
    </article>
  );
}
