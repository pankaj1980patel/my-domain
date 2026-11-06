// "Back up settings" / "Restore settings…" (settings_backup.rs): saves this
// app's settings to a JSON file, and restores one after showing what would
// change, a box per section (sections naming devices that aren't yours here
// start unticked). The theme and language live here (localStorage), so the
// UI passes them in and applies them back; the rest is written by the
// backend, and most of it takes effect after a restart.

import { createSignal, For, Show } from "solid-js";
import { commands, type RestorePreview_Serialize as RestorePreview, type RestoreResult } from "../bindings";
import { friendlyError } from "../lib/words";
import { theme, setTheme, type ThemeId } from "../lib/theme";
import { languagePref, saveLanguagePref, type LangPref } from "../i18n";
import { toast } from "./ui";
import { t, tn, type Key } from "../i18n";
import { HelpLink } from "../help/HelpLink";
import { focusSoon } from "../lib/a11y";

const SECTION_KEYS: Record<string, Key> = {
  appearance: "backup.section.appearance",
  preferences: "backup.section.preferences",
  sharing: "backup.section.sharing",
  automations: "backup.section.automations",
  schedules: "backup.section.schedules",
  commands: "backup.section.commands",
  shortcuts: "backup.section.shortcuts",
  webhooks: "backup.section.webhooks",
  device_labels: "backup.section.deviceLabels",
  page_layout: "backup.section.pageLayout",
  sync_folders: "backup.section.syncFolders",
};

const appearanceNow = () => ({ theme: theme(), language: languagePref() });

export function SettingsBackup() {
  const [busy, setBusy] = createSignal(false);
  const [preview, setPreview] = createSignal<RestorePreview>();
  const [picked, setPicked] = createSignal<Set<string>>(new Set());
  const [result, setResult] = createSignal<RestoreResult>();
  let restoreButton: HTMLButtonElement | undefined;
  let previewHeading: HTMLHeadingElement | undefined;

  const save = async () => {
    setBusy(true);
    try {
      const r = await commands.settingsBackupSave(appearanceNow());
      if (r.status === "error") throw r.error;
      if (r.data) toast(t("backup.saved"), "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const open = async () => {
    setBusy(true);
    setResult(undefined);
    try {
      const r = await commands.settingsRestoreOpen(appearanceNow());
      if (r.status === "error") throw r.error;
      if (!r.data) return;
      setPreview(r.data);
      setPicked(new Set(r.data.sections.filter((s) => s.on && s.changed).map((s) => s.id)));
      focusSoon(() => previewHeading);
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const toggle = (id: string, on: boolean) => {
    const next = new Set(picked());
    if (on) next.add(id);
    else next.delete(id);
    setPicked(next);
  };

  const cancel = () => {
    setPreview(undefined);
    focusSoon(() => restoreButton);
  };

  const apply = async () => {
    const p = preview();
    if (!p) return;
    const ids = [...picked()];
    setBusy(true);
    try {
      const r = await commands.settingsRestoreApply(ids);
      if (r.status === "error") throw r.error;
      let reload = false;
      if (ids.includes("appearance") && p.appearance) {
        if (p.appearance.theme) setTheme(p.appearance.theme as ThemeId);
        const lang = p.appearance.language as LangPref | undefined;
        if (lang && lang !== languagePref()) {
          saveLanguagePref(lang);
          reload = true;
        }
      }
      setPreview(undefined);
      setResult(r.data);
      if (r.data.failed.length === 0) toast(t("backup.restored"), "ok");
      // The language shows after a reload; a restart reloads anyway.
      if (reload && !r.data.restart) location.reload();
      focusSoon(() => restoreButton);
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const when = (secs: number | null) => (secs ? new Date(secs * 1000).toLocaleString() : "");
  const platform = (p: string): string => (p === "macos" ? "macOS" : p === "windows" ? "Windows" : "Linux");

  return (
    <article class="tile" id="settings-backup">
      <h2>
        {t("backup.title")}
        <HelpLink topic="privacy" />
      </h2>
      <p class="muted small">{t("backup.hint")}</p>
      <div class="row backup-actions">
        <button type="button" class="btn btn-primary" disabled={busy()} onClick={save}>
          {t("backup.save")}
        </button>
        <button type="button" class="btn btn-ghost" ref={restoreButton} disabled={busy()} onClick={open}>
          {t("backup.restore")}
        </button>
      </div>
      <p class="muted small">{t("backup.never")}</p>

      <Show when={preview()}>
        {(p) => (
          <div class="backup-preview" role="group" aria-labelledby="backup-preview-title">
            <h3 id="backup-preview-title" tabindex="-1" ref={previewHeading}>
              {t("backup.previewTitle")}
            </h3>
            <p class="muted small">
              {t("backup.from", { platform: platform(p().platform), when: when(p().created_at) })}
            </p>
            <ul class="backup-sections">
              <For each={p().sections}>
                {(s) => (
                  <li>
                    <label class="backup-section">
                      <input
                        type="checkbox"
                        checked={picked().has(s.id)}
                        disabled={!s.changed}
                        onChange={(e) => toggle(s.id, e.currentTarget.checked)}
                      />
                      <span>
                        <strong>{t(SECTION_KEYS[s.id] ?? "backup.section.other")}</strong>
                        <span class="muted small">
                          {" · "}
                          {s.changed ? t("backup.willChange") : t("backup.same")}
                          {s.items > 0 ? ` · ${tn("backup.items", s.items)}` : ""}
                        </span>
                        <Show when={s.changed && s.keys.length > 0 && s.id !== "shortcuts" && s.id !== "page_layout"}>
                          <span class="backup-keys muted small">{s.keys.join(", ")}</span>
                        </Show>
                        <Show when={s.unknown_devices.length > 0}>
                          <span class="backup-warn small">
                            {t("backup.unknownDevices", { names: s.unknown_devices.join(", ") })}
                          </span>
                        </Show>
                      </span>
                    </label>
                  </li>
                )}
              </For>
            </ul>
            <div class="row backup-actions">
              <button type="button" class="btn btn-primary" disabled={busy() || picked().size === 0} onClick={apply}>
                {t("backup.apply")}
              </button>
              <button type="button" class="btn btn-ghost" disabled={busy()} onClick={cancel}>
                {t("settings.cancel")}
              </button>
            </div>
          </div>
        )}
      </Show>

      <Show when={result()}>
        {(r) => (
          <div class="backup-result" role="status">
            <Show when={r().failed.length > 0}>
              <p class="backup-warn small">{t("backup.someFailed")}</p>
              <ul class="small">
                <For each={r().failed}>{(f) => <li>{f}</li>}</For>
              </ul>
            </Show>
            <Show when={r().restart}>
              <p class="small">{t("backup.restartHint")}</p>
              <div class="row backup-actions">
                <button type="button" class="btn btn-primary" onClick={() => commands.restartApp()}>
                  {t("backup.restartNow")}
                </button>
                <button type="button" class="btn btn-ghost" onClick={() => setResult(undefined)}>
                  {t("backup.later")}
                </button>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </article>
  );
}
