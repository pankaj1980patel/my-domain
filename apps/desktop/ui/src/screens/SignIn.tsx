// Welcome / sign in. The server is pre-filled and tucked under "Advanced":
// nobody should have to type a URL.

import { createSignal, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { signIn } from "../lib/core";
import { friendlyError } from "../lib/words";
import { t } from "../i18n";
import { logoSrc, Tagline, Wordmark } from "../components/Brand";

const DEFAULT_SERVER = "https://mydomain.servehttp.com";

export default function SignIn(props: { onDone: () => void }) {
  const [create, setCreate] = createSignal(false);
  const [server, setServer] = createSignal(DEFAULT_SERVER);
  const [user, setUser] = createSignal("");
  const [pass, setPass] = createSignal("");
  const [key, setKey] = createSignal("");
  const [advanced, setAdvanced] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const submit = async (e?: Event) => {
    e?.preventDefault();
    if (!user().trim() || !pass() || !key().trim()) {
      setError(t("signIn.missingFields"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      await signIn(server().trim() || DEFAULT_SERVER, user().trim(), pass(), key().trim(), create());
      props.onDone();
    } catch (err) {
      setError(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  onMount(async () => {
    const saved = await commands.getSavedSession();
    if (saved.server_url) setServer(saved.server_url);
    if (saved.username) setUser(saved.username);
    // Device lab: debug builds started with MD_DEV_* sign in by themselves.
    const dev = await commands.devCredentials().catch(() => null);
    if (dev) {
      setServer(dev.server);
      setUser(dev.user);
      setPass(dev.pass);
      setKey(dev.key);
      await submit();
    }
  });

  return (
    <main class="signin">
      <form class="signin-card" onSubmit={submit}>
        <img class="signin-logo" src={logoSrc()} alt="" />
        <div class="signin-brand">
          <Wordmark class="wordmark-lg" />
          <Tagline />
        </div>
        <h1>{create() ? t("signIn.titleCreate") : t("signIn.titleWelcome")}</h1>
        <p class="muted">{t("signIn.tagline")}</p>

        <label class="field">
          <span>{t("signIn.username")}</span>
          <input value={user()} onInput={(e) => setUser(e.currentTarget.value)} autocomplete="username" autofocus />
        </label>
        <label class="field">
          <span>{t("signIn.password")}</span>
          <input
            type="password"
            value={pass()}
            onInput={(e) => setPass(e.currentTarget.value)}
            autocomplete={create() ? "new-password" : "current-password"}
          />
        </label>
        <label class="field">
          <span>{t("signIn.deviceKey")}</span>
          <div class="field-row">
            <input type="password" value={key()} onInput={(e) => setKey(e.currentTarget.value)} />
            <Show when={create()}>
              <button type="button" class="btn btn-ghost btn-sm" onClick={async () => setKey(await commands.generateKey())}>
                {t("signIn.generate")}
              </button>
            </Show>
          </div>
          <small class="muted">{t("signIn.keyHint")}</small>
        </label>

        <button type="button" class="link" onClick={() => setAdvanced(!advanced())} aria-expanded={advanced()}>
          {advanced() ? t("signIn.hideAdvanced") : t("signIn.advanced")}
        </button>
        <Show when={advanced()}>
          <label class="field">
            <span>{t("signIn.server")}</span>
            <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} />
          </label>
        </Show>

        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>

        <button class="btn btn-primary btn-block" type="submit" disabled={busy()}>
          {busy() ? t("signIn.signingIn") : create() ? t("signIn.createAccount") : t("signIn.signIn")}
        </button>
        <p class="muted center">
          {create() ? t("signIn.haveAccount") : t("signIn.newHere")}{" "}
          <button type="button" class="link" onClick={() => setCreate(!create())}>
            {create() ? t("signIn.signIn") : t("signIn.createAnAccount")}
          </button>
        </p>
      </form>
    </main>
  );
}
