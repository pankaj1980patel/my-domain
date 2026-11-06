// "Browser extension": lets Chrome, Edge, Brave, Chromium and Firefox start
// my-domain for its extension (ext_bridge.rs writes each browser's native
// messaging host manifest; Remove takes them away). Published Chrome Web
// Store / Edge Add-ons ids can be added to the unpacked one.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type BrowserExtension_Serialize as BrowserExtension } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function BrowserExtensionSetting() {
  const [s, setS] = createSignal<BrowserExtension>();
  const [ids, setIds] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const take = (next: BrowserExtension) => {
    setS(next);
    setIds(next.chrome_ids.join(", "));
  };
  onMount(() => commands.browserExtensionStatus().then(take, () => {}));

  const installed = () => s()?.browsers.some((b) => b.installed) ?? false;

  const install = async () => {
    setBusy(true);
    try {
      const list = ids()
        .split(/[\s,]+/)
        .map((x) => x.trim())
        .filter(Boolean);
      take(await unwrap(commands.browserExtensionInstall(list)));
      toast(t("browserExt.installed"), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };
  const remove = async () => {
    setBusy(true);
    try {
      take(await unwrap(commands.browserExtensionRemove()));
      toast(t("browserExt.removed"), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="tile" id="settings-browser-extension">
      <h2>{t("browserExt.title")}</h2>
      <p class="muted small">{t("browserExt.hint")}</p>
      <Show when={s()}>
        {(x) => (
          <div class="stack">
            <ul class="browser-hosts small">
              <For each={x().browsers}>
                {(b) => (
                  <li>
                    <span>{b.name}</span>
                    <span class={b.installed ? "ok" : "muted"}>
                      {b.installed ? t("browserExt.ready") : b.found ? t("browserExt.notInstalled") : t("browserExt.notFound")}
                    </span>
                  </li>
                )}
              </For>
            </ul>
            <Show when={!x().listening}>
              <p class="webhook-error small">{t("browserExt.notListening", { error: x().error ?? "" })}</p>
            </Show>
            <label class="muted small" for="browser-ext-ids">
              {t("browserExt.ids")}
            </label>
            <input
              id="browser-ext-ids"
              type="text"
              spellcheck={false}
              placeholder={t("browserExt.idsPlaceholder")}
              value={ids()}
              onInput={(e) => setIds(e.currentTarget.value)}
            />
            <p class="muted small">{t("browserExt.idsHint", { dev: x().dev_chrome_id, firefox: x().firefox_id })}</p>
            <div class="row">
              <button class="btn btn-primary btn-sm" disabled={busy()} onClick={install}>
                {installed() ? t("browserExt.update") : t("browserExt.install")}
              </button>
              <button class="btn btn-ghost btn-sm" disabled={busy() || !installed()} onClick={remove}>
                {t("browserExt.remove")}
              </button>
            </div>
            <p class="muted small">{t("browserExt.loadHint")}</p>
          </div>
        )}
      </Show>
    </article>
  );
}
