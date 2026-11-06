// Clipboard privacy (clip_privacy.rs): "Never sync passwords and other
// concealed clips" (on by default) and the apps whose copies stay on this
// computer, with a button to add the app you last copied from. Linux can't
// tell either, so it says so.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type ClipPrivacy_Serialize as ClipPrivacy } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export function ClipPrivacySetting() {
  const [s, setS] = createSignal<ClipPrivacy>();
  const [name, setName] = createSignal("");

  const load = async () => {
    try {
      setS(await commands.clipPrivacy());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  onMount(load);

  const toggle = async (on: boolean) => {
    try {
      await commands.setClipHideConcealed(on);
      await load();
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const setApps = async (apps: string[]) => {
    try {
      const excluded = await commands.setClipExcludedApps(apps);
      const cur = s();
      if (cur) setS({ ...cur, excluded });
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const add = (app: string) => {
    if (!app.trim()) return;
    setApps([...(s()?.excluded ?? []), app.trim()]);
    setName("");
  };
  const remove = (app: string) => setApps((s()?.excluded ?? []).filter((a) => a !== app));
  const lastApp = () => {
    const a = s()?.last_app;
    return a && !(s()?.excluded ?? []).some((e) => e.toLowerCase() === a.toLowerCase()) ? a : null;
  };

  return (
    <div class="clip-privacy">
      <label class="switch-row">
        <span>
          {t("clipPrivacy.hide")}
          <HelpLink topic="messages-clipboard" />
        </span>
        <input
          type="checkbox"
          role="switch"
          checked={s()?.hide_concealed ?? true}
          disabled={!s()}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
      </label>
      <p class="muted small">{t(s()?.detects === false ? "clipPrivacy.hideHintLinux" : "clipPrivacy.hideHint")}</p>
      <Show when={s()?.knows_apps}>
        <h3 class="clip-privacy-title">{t("clipPrivacy.apps")}</h3>
        <p class="muted small">{t("clipPrivacy.appsHint")}</p>
        <Show when={(s()?.excluded.length ?? 0) > 0} fallback={<p class="muted small">{t("clipPrivacy.none")}</p>}>
          <ul class="clip-privacy-apps">
            <For each={s()?.excluded}>
              {(app) => (
                <li>
                  <span class="clip-privacy-app">{app}</span>
                  <button
                    class="btn btn-ghost btn-sm"
                    aria-label={t("clipPrivacy.removeLabel", { app })}
                    onClick={() => remove(app)}
                  >
                    {t("clipPrivacy.remove")}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <form
          class="clip-privacy-add"
          onSubmit={(e) => {
            e.preventDefault();
            add(name());
          }}
        >
          <input
            type="text"
            value={name()}
            placeholder={t("clipPrivacy.placeholder")}
            aria-label={t("clipPrivacy.placeholder")}
            onInput={(e) => setName(e.currentTarget.value)}
          />
          <button class="btn btn-ghost btn-sm" type="submit" disabled={!name().trim()}>
            {t("clipPrivacy.add")}
          </button>
        </form>
        <Show
          when={lastApp()}
          fallback={<p class="muted small">{t("clipPrivacy.noLast")}</p>}
        >
          {(app) => (
            <button class="btn btn-ghost btn-sm" onClick={() => add(app())}>
              {t("clipPrivacy.addLast", { app: app() })}
            </button>
          )}
        </Show>
      </Show>
    </div>
  );
}
