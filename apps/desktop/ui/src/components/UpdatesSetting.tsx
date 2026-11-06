// Settings → Updates: this version, "Check now", "Install updates
// automatically" (off by default), the channel (stable only) and a quiet
// status line: when it last looked, or why it couldn't.

import { createSignal, Show } from "solid-js";
import { checkNow, installUpdate, setAutoInstall, updateStatus, watchUpdates, type UpdateStatus } from "../lib/updates";
import { toast } from "./ui";
import { t } from "../i18n";

function line(s: UpdateStatus): string {
  if (s.disabled === "dev") return t("updates.offDev");
  if (s.disabled) return t("updates.offUnsigned");
  if (s.phase === "checking") return t("updates.checking");
  if (s.phase === "downloading") return t("updates.downloading");
  if (s.phase === "installing") return t("updates.installing");
  if (s.phase === "ready") return t("updates.ready", { version: s.available?.version ?? "" });
  if (s.error === "network") return t("updates.errNetwork");
  if (s.error === "signature") return t("updates.errSignature");
  if (s.error === "install") return t("updates.installFailed");
  if (s.error) return t("updates.errServer");
  if (s.available) return t("updates.available", { version: s.available.version });
  if (s.last_check) return t("updates.upToDate", { time: new Date(s.last_check * 1000).toLocaleString() });
  return t("updates.notYet");
}

export function UpdatesSetting() {
  watchUpdates();
  const [checking, setChecking] = createSignal(false);
  const check = async () => {
    setChecking(true);
    try {
      await checkNow();
    } finally {
      setChecking(false);
    }
  };
  return (
    <article class="tile" id="settings-updates">
      <h2>{t("updates.title")}</h2>
      <Show when={updateStatus()}>
        {(st) => (
          <>
            <p class="small">{t("updates.current", { version: st().current })}</p>
            <p class="muted small" role="status">
              {line(st())}
            </p>
            <div class="row">
              <button
                class="btn btn-soft btn-sm"
                disabled={!!st().disabled || checking() || st().phase !== "idle"}
                onClick={check}
              >
                {t("updates.checkNow")}
              </button>
              <Show when={st().available && st().phase !== "installing"}>
                <button
                  class="btn btn-ghost btn-sm"
                  disabled={st().phase === "downloading"}
                  onClick={async () => {
                    if (await installUpdate()) toast(t("updates.installFailed"), "error");
                  }}
                >
                  {st().phase === "ready" ? t("updates.restartNow") : t("updates.installRestart")}
                </button>
              </Show>
            </div>
            <label class="switch-row">
              <span>{t("updates.auto")}</span>
              <input
                type="checkbox"
                role="switch"
                checked={st().auto_install}
                disabled={!!st().disabled}
                onChange={(e) => setAutoInstall(e.currentTarget.checked)}
              />
            </label>
            <p class="muted small">{t("updates.autoHint")}</p>
            <p class="muted small">{t("updates.channel")}</p>
          </>
        )}
      </Show>
    </article>
  );
}
