// Account, this device, and preferences. Tap the version 7 times for the
// Developer screen.

import { createSignal, Show } from "solid-js";
import { commands } from "../bindings";
import { setClipSync, signOut, state, unwrap } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "../components/ui";
import { t, tRich } from "../i18n";

export default function Settings(props: { onDeveloper: () => void }) {
  const [taps, setTaps] = createSignal(0);
  const [newKey, setNewKey] = createSignal("");
  const [pass, setPass] = createSignal("");
  const [showKey, setShowKey] = createSignal(false);

  const tapVersion = () => {
    const n = taps() + 1;
    setTaps(n);
    if (n >= 7) {
      setTaps(0);
      toast(t("settings.developerUnlocked"), "ok");
      props.onDeveloper();
    }
  };

  return (
    <section class="page">
      <header class="page-head">
        <h1>{t("settings.title")}</h1>
      </header>

      <article class="tile">
        <h2>{t("settings.account")}</h2>
        <p>{tRich("settings.signedInAs", { name: <strong>{state.account.username || "…"}</strong> })}</p>
        <button class="btn btn-ghost" onClick={signOut}>
          {t("settings.signOut")}
        </button>
      </article>

      <article class="tile">
        <h2>{t("settings.thisDevice")}</h2>
        <p>{tRich("settings.seenAs", { name: <strong>{niceName(state.me?.name ?? "")}</strong> })}</p>
        <label class="switch-row">
          <span>{t("settings.clipSync")}</span>
          <input type="checkbox" role="switch" checked={state.clipSync} onChange={(e) => setClipSync(e.currentTarget.checked)} />
        </label>
      </article>

      <article class="tile">
        <h2>{t("settings.deviceKey")}</h2>
        <p class="muted small">{t("settings.deviceKeyHint")}</p>
        <Show
          when={showKey()}
          fallback={
            <button class="btn btn-ghost" onClick={() => setShowKey(true)}>
              {t("settings.changeKey")}
            </button>
          }
        >
          <form
            class="stack"
            onSubmit={async (e) => {
              e.preventDefault();
              try {
                await unwrap(commands.updateEncryptionKey(newKey(), pass()));
                toast(t("settings.keyChanged"), "ok");
                setShowKey(false);
                setNewKey("");
                setPass("");
              } catch (err) {
                toast(friendlyError(err), "error");
              }
            }}
          >
            <input type="password" placeholder={t("settings.newKey")} value={newKey()} onInput={(e) => setNewKey(e.currentTarget.value)} />
            <input type="password" placeholder={t("settings.yourPassword")} value={pass()} onInput={(e) => setPass(e.currentTarget.value)} />
            <div class="row">
              <button class="btn btn-primary" type="submit" disabled={!newKey() || !pass()}>
                {t("settings.save")}
              </button>
              <button class="btn btn-ghost" type="button" onClick={() => setShowKey(false)}>
                {t("settings.cancel")}
              </button>
            </div>
          </form>
        </Show>
      </article>

      <p class="version muted small" onClick={tapVersion}>
        {t("settings.version", { version: "0.1.0" })}
      </p>
    </section>
  );
}
