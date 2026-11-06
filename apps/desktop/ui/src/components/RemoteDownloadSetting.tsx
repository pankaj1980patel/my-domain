// Settings → "Downloads from my devices" (remote_download.rs): what this
// computer does when another of your devices asks it to download a link.
// Ask the first time for each device (the default), always, or never;
// links to the local network; the biggest file; and the devices allowed
// without asking (forgotten together).

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type DownloadSettingsView as Settings } from "../bindings";
import { peerName, unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";
import { HelpLink } from "../help/HelpLink";

const POLICIES = ["ask", "always", "never"] as const;
/** The size limits offered, in MB (0: no limit). */
const SIZES = [100, 500, 1024, 4096, 0];

export function RemoteDownloadSetting() {
  const [s, setS] = createSignal<Settings>();
  onMount(() => commands.remoteDownloadSettings().then(setS, (e) => toast(friendlyError(e), "error")));

  const change = async (patch: Partial<Settings>) => {
    const cur = s();
    if (!cur) return;
    const next = { ...cur, ...patch };
    try {
      await unwrap(commands.setRemoteDownloadSettings(next));
      setS(next);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const sizeLabel = (mb: number) =>
    mb === 0 ? t("download.noLimit") : mb >= 1024 ? t("download.gb", { n: mb / 1024 }) : t("download.mb", { n: mb });

  return (
    <article class="tile" id="settings-downloads">
      <h2>
        {t("download.settingsTitle")} <HelpLink topic="files" />
      </h2>
      <Show when={s()} fallback={<p class="muted small">…</p>}>
        {(cur) => (
          <>
            <p class="muted small">{t("download.settingsIntro")}</p>
            <label class="row small">
              <span class="grow">{t("download.policy")}</span>
              <select aria-label={t("download.policy")} onChange={(e) => change({ policy: e.currentTarget.value })}>
                <For each={POLICIES}>
                  {(p) => (
                    <option value={p} selected={cur().policy === p}>
                      {t(`download.policy.${p}` as Key)}
                    </option>
                  )}
                </For>
              </select>
            </label>
            <label class="row small">
              <span class="grow">{t("download.maxSize")}</span>
              <select aria-label={t("download.maxSize")} onChange={(e) => change({ max_mb: Number(e.currentTarget.value) })}>
                <For each={SIZES.includes(cur().max_mb) ? SIZES : [...SIZES, cur().max_mb]}>
                  {(mb) => (
                    <option value={mb} selected={cur().max_mb === mb}>
                      {sizeLabel(mb)}
                    </option>
                  )}
                </For>
              </select>
            </label>
            <label class="switch-row">
              <span>{t("download.allowLan")}</span>
              <input type="checkbox" role="switch" checked={cur().allow_lan} onChange={(e) => change({ allow_lan: e.currentTarget.checked })} />
            </label>
            <p class="muted small">{t("download.allowLanHint")}</p>
            <Show when={cur().allowed.length > 0}>
              <div class="row small">
                <span class="grow">
                  {t("download.allowedList", { devices: cur().allowed.map((n) => peerName(n)).join(", ") })}
                </span>
                <button class="btn btn-ghost btn-sm" type="button" onClick={() => change({ allowed: [] })}>
                  {t("download.forget")}
                </button>
              </div>
            </Show>
            <p class="muted small">{t("download.settingsNote")}</p>
          </>
        )}
      </Show>
    </article>
  );
}
