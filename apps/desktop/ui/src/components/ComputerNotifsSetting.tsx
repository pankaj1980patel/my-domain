// "Show this computer's notifications on my phone" (computer_notifs.rs):
// off by default; once on, every app that shows a notification here is
// listed as it's seen, shared until muted. Linux only: Windows needs a
// packaged app for it, and macOS has no way for apps to read others'
// notifications; the tile says so there.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type ComputerNotifsState } from "../bindings";
import { peerName } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function ComputerNotifsSetting() {
  const [s, setS] = createSignal<ComputerNotifsState>();
  const [busy, setBusy] = createSignal(false);

  const refresh = async () => {
    try {
      setS(await commands.computerNotifsState());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  onMount(() => {
    refresh();
    // New apps as they're seen; the phones showing them change by themselves.
    const un = listen("computer-notifs-changed", () => refresh());
    const timer = setInterval(() => {
      if (s()?.on) refresh();
    }, 15_000);
    onCleanup(() => {
      clearInterval(timer);
      un.then((f) => f());
    });
  });

  const toggle = async (on: boolean) => {
    setBusy(true);
    try {
      setS(await commands.computerNotifsSetEnabled(on));
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const setShared = async (id: string, shared: boolean) => {
    try {
      setS(await commands.computerNotifsSetMuted(id, !shared));
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const unsupported = () =>
    t(s()?.os === "macos" ? "computerNotifs.macos" : s()?.os === "windows" ? "computerNotifs.windows" : "computerNotifs.unavailable");
  const phones = () => (s()?.phones ?? []).map((n) => peerName(n)).join(", ");

  return (
    <article class="tile" id="settings-computer-notifs">
      <h2>
        {t("computerNotifs.title")}
        <HelpLink topic="notifications" />
      </h2>
      <Show when={s()}>
        <Show when={s()?.supported} fallback={<p class="muted small">{unsupported()}</p>}>
          <label class="switch-row">
            <span>{t("computerNotifs.switch")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={s()?.on ?? false}
              disabled={busy()}
              onChange={(e) => toggle(e.currentTarget.checked)}
            />
          </label>
          <p class="muted small">{t("computerNotifs.hint")}</p>
          <Show when={s()?.on}>
            <Show when={s()?.error}>
              <p class="cn-error small" role="alert">
                {t("computerNotifs.error", { why: s()?.error ?? "" })}
              </p>
            </Show>
            <p class="muted small">
              {(s()?.phones.length ?? 0) > 0 ? t("computerNotifs.phones", { names: phones() }) : t("computerNotifs.noPhones")}
            </p>
            <h3 class="cn-head">{t("computerNotifs.apps")}</h3>
            <Show when={(s()?.apps.length ?? 0) > 0} fallback={<p class="muted small">{t("computerNotifs.noApps")}</p>}>
              <ul class="cn-apps">
                <For each={s()?.apps ?? []}>
                  {(a) => (
                    <li>
                      <label class="switch-row">
                        <span>{a.name || t("computerNotifs.unnamed")}</span>
                        <input
                          type="checkbox"
                          role="switch"
                          checked={!a.muted}
                          aria-label={t("computerNotifs.shareApp", { app: a.name || t("computerNotifs.unnamed") })}
                          onChange={(e) => setShared(a.id, e.currentTarget.checked)}
                        />
                      </label>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </Show>
        </Show>
      </Show>
    </article>
  );
}
