// "Version X is available": a non-blocking banner over the content while a
// newer version is found and not put off with "Later". What's new shows the
// release notes; Install and restart downloads (with progress), installs and
// starts the new version. With "Install updates automatically" on, it says
// the update is ready and installs when the app quits.

import { createSignal, Show } from "solid-js";
import { installUpdate, later, megabytes, percent, updateStatus, watchUpdates } from "../lib/updates";
import { toast } from "./ui";
import { t } from "../i18n";

export default function UpdateBanner() {
  watchUpdates();
  const [notes, setNotes] = createSignal(false);
  const shown = () => {
    const st = updateStatus();
    return st && !st.disabled && st.available && !st.later ? st : undefined;
  };
  const busy = () => updateStatus()?.phase === "downloading" || updateStatus()?.phase === "installing";
  const install = async () => {
    if (await installUpdate()) toast(t("updates.installFailed"), "error");
  };
  return (
    <Show when={shown()}>
      {(st) => (
        <div class="update-banner" role="status">
          <div class="update-banner-line">
            <strong>{t("updates.available", { version: st().available?.version ?? "" })}</strong>
            <Show when={st().phase === "ready"}>
              <span class="muted small">{t("updates.readyOnQuit")}</span>
            </Show>
          </div>
          <Show when={st().phase === "downloading"}>
            <div class="update-progress small">
              <progress max="100" value={percent(st()) ?? undefined} aria-label={t("updates.downloading")} />
              <span class="muted">
                {percent(st()) === null
                  ? t("updates.downloadingMb", { mb: megabytes(st().downloaded ?? 0) })
                  : t("updates.downloadingPct", { pct: String(percent(st())) })}
              </span>
            </div>
          </Show>
          <Show when={st().phase === "installing"}>
            <p class="small muted">{t("updates.installing")}</p>
          </Show>
          <Show when={notes()}>
            <div class="update-notes small">{st().available?.notes || t("updates.noNotes")}</div>
          </Show>
          <div class="row">
            <button class="btn btn-ghost btn-sm" aria-expanded={notes()} onClick={() => setNotes(!notes())}>
              {t("updates.whatsNew")}
            </button>
            <button class="btn btn-soft btn-sm" disabled={busy()} onClick={install}>
              {st().phase === "ready" ? t("updates.restartNow") : t("updates.installRestart")}
            </button>
            <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => later()}>
              {t("updates.later")}
            </button>
          </div>
        </div>
      )}
    </Show>
  );
}
