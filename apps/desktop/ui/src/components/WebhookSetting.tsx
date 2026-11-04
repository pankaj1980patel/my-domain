// "Webhooks": tools on this computer (Home Assistant, Shortcuts, a script)
// trigger actions over a small HTTP server on 127.0.0.1 (webhooks.rs). Off by
// default; the switch, the port, the bearer token (Copy, Make a new token)
// and a curl example to copy.

import { createSignal, onMount, Show } from "solid-js";
import { commands, type WebhookSettings_Serialize as WebhookSettings } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

export function WebhookSetting() {
  const [s, setS] = createSignal<WebhookSettings>();
  const [port, setPort] = createSignal("");
  const [showToken, setShowToken] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const take = (next: WebhookSettings) => {
    setS(next);
    setPort(String(next.port));
  };
  onMount(() => commands.webhookSettings().then(take, () => {}));

  const toggle = async (on: boolean) => {
    setBusy(true);
    try {
      take(await commands.setWebhooksEnabled(on));
    } finally {
      setBusy(false);
    }
  };
  const savePort = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    try {
      const next = await unwrap(commands.setWebhookPort(Number(port())));
      take(next);
      if (next.running) toast(t("webhooks.portSaved", { port: next.port }), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };
  const newToken = async () => {
    setConfirming(false);
    try {
      take(await unwrap(commands.newWebhookToken()));
      toast(t("webhooks.newTokenDone"), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };
  const copy = (text: string) =>
    navigator.clipboard.writeText(text).then(
      () => toast(t("webhooks.copied"), "ok"),
      (e) => toast(friendlyError(e), "error"),
    );

  const base = () => `http://127.0.0.1:${s()?.port ?? 0}`;
  const example = () =>
    [
      `curl -X POST ${base()}/v1/action \\`,
      `  -H "Authorization: Bearer ${s()?.token ?? ""}" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{"device": "My phone", "action": "ring"}'`,
    ].join("\n");

  return (
    <article class="tile" id="settings-webhooks">
      <h2>{t("webhooks.title")}</h2>
      <p class="muted small">{t("webhooks.hint")}</p>
      <label class="switch-row">
        <span>{t("webhooks.switch")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={s()?.enabled ?? false}
          disabled={!s() || busy()}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
      </label>
      <Show when={s()?.enabled ? s() : undefined}>
        {(w) => (
          <div class="stack">
            <Show
              when={w().running}
              fallback={<p class="webhook-error small">{w().error ?? t("webhooks.notListening")}</p>}
            >
              <p class="muted small">{t("webhooks.listening", { port: w().port })}</p>
            </Show>

            <form class="row webhook-port" onSubmit={savePort}>
              <label class="muted small" for="webhook-port">
                {t("webhooks.port")}
              </label>
              <input
                id="webhook-port"
                type="number"
                min="1024"
                max="65535"
                value={port()}
                onInput={(e) => setPort(e.currentTarget.value)}
              />
              <button class="btn btn-ghost btn-sm" type="submit" disabled={busy() || port() === String(w().port)}>
                {t("webhooks.savePort")}
              </button>
            </form>
            <p class="muted small">{t("webhooks.portHint", { port: w().port })}</p>

            <strong class="small">{t("webhooks.token")}</strong>
            <code class="webhook-token small">
              <Show
                when={showToken()}
                fallback={
                  <>
                    <span aria-hidden="true">{"•".repeat(24)}</span>
                    <span class="sr-only">{t("a11y.tokenHidden")}</span>
                  </>
                }
              >
                {w().token}
              </Show>
            </code>
            <div class="row">
              <button class="btn btn-ghost btn-sm" aria-pressed={showToken()} onClick={() => setShowToken(!showToken())}>

                {showToken() ? t("webhooks.hide") : t("webhooks.show")}
              </button>
              <button class="btn btn-ghost btn-sm" disabled={!w().token} onClick={() => copy(w().token)}>
                {t("webhooks.copy")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => setConfirming(true)}>
                {t("webhooks.newToken")}
              </button>
            </div>
            <Show when={confirming()}>
              <div class="webhook-confirm stack">
                <p class="small">{t("webhooks.newTokenSure")}</p>
                <div class="row">
                  <button class="btn btn-primary btn-sm" onClick={newToken}>
                    {t("webhooks.newTokenYes")}
                  </button>
                  <button class="btn btn-ghost btn-sm" onClick={() => setConfirming(false)}>
                    {t("webhooks.cancel")}
                  </button>
                </div>
              </div>
            </Show>
            <p class="muted small">{t("webhooks.tokenHint")}</p>

            <strong class="small">{t("webhooks.example")}</strong>
            <pre class="webhook-code">{example()}</pre>
            <div class="row">
              <button class="btn btn-ghost btn-sm" onClick={() => copy(example())}>
                {t("webhooks.copyExample")}
              </button>
            </div>
            <p class="muted small">{t("webhooks.exampleHint")}</p>

            <strong class="small">{t("webhooks.api")}</strong>
            <ul class="webhook-api small">
              <li>
                <code>GET /v1/devices</code> {t("webhooks.apiDevices")}
              </li>
              <li>
                <code>{'POST /v1/action {"device", "action", "args"}'}</code> {t("webhooks.apiAction")}
              </li>
              <li>
                <code>{'POST /v1/text {"device", "text", "as": "clipboard" | "notice"}'}</code> {t("webhooks.apiText")}
              </li>
              <li>
                <code>{'POST /v1/automation {"automation"}'}</code> {t("webhooks.apiAutomation")}
              </li>
            </ul>
            <p class="muted small">{t("webhooks.apiReply")}</p>
          </div>
        )}
      </Show>
    </article>
  );
}
