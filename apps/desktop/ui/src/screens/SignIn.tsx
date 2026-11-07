// Welcome / sign in. The server is pre-filled and tucked under "Advanced":
// nobody should have to type a URL. "Join with a code" signs in with the
// 6-digit code a signed-in device shows under Add a device (no password,
// no key to type).

import { createSignal, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { joinAccount, signIn } from "../lib/core";
import { friendlyError } from "../lib/words";
import { t } from "../i18n";
import { logoSrc, Tagline, Wordmark } from "../components/Brand";
import { Icon } from "../components/Icon";

const DEFAULT_SERVER = "https://mydomain.servehttp.com";

export default function SignIn(props: { onDone: () => void }) {
  const [create, setCreate] = createSignal(false);
  const [join, setJoin] = createSignal(false);
  const [code, setCode] = createSignal("");
  const [server, setServer] = createSignal(DEFAULT_SERVER);
  const [user, setUser] = createSignal("");
  const [pass, setPass] = createSignal("");
  const [key, setKey] = createSignal("");
  const [advanced, setAdvanced] = createSignal(false);
  const [showPass, setShowPass] = createSignal(false);
  const [showKey, setShowKey] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const submitJoin = async () => {
    const c = code().trim();
    if (!/^mydomain:\/\/join\?/i.test(c) && c.replace(/[\s-]/g, "").length !== 6) {
      setError(t("signIn.joinMissing"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      await joinAccount(server().trim() || DEFAULT_SERVER, c);
      props.onDone();
    } catch (err) {
      setError(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  const submit = async (e?: Event) => {
    e?.preventDefault();
    if (join()) return submitJoin();
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
        <h1>{join() ? t("signIn.joinTitle") : create() ? t("signIn.titleCreate") : t("signIn.titleWelcome")}</h1>
        <p class="muted">{join() ? t("signIn.joinHint") : t("signIn.tagline")}</p>

        <Show when={join()}>
          <label class="field">
            <span>{t("signIn.joinCode")}</span>
            <input
              class="signin-code"
              value={code()}
              onInput={(e) => setCode(e.currentTarget.value)}
              inputmode="numeric"
              autocomplete="one-time-code"
              ref={(el) => requestAnimationFrame(() => el.focus())}
              aria-invalid={!!error()}
            />
          </label>
        </Show>
        <Show when={!join()}>
          <label class="field">
            <span>{t("signIn.username")}</span>
            <input
              value={user()}
              onInput={(e) => setUser(e.currentTarget.value)}
              autocomplete="username"
              ref={(el) => requestAnimationFrame(() => el.focus())}
              aria-invalid={!!error() && !user().trim()}
            />
          </label>
          <label class="field">
            <span>{t("signIn.password")}</span>
            <span class="field-with-btn">
              <input
                type={showPass() ? "text" : "password"}
                value={pass()}
                onInput={(e) => setPass(e.currentTarget.value)}
                autocomplete={create() ? "new-password" : "current-password"}
                aria-invalid={!!error() && !pass()}
              />
              <RevealButton shown={showPass()} onToggle={() => setShowPass(!showPass())} what={t("signIn.password")} />
            </span>
          </label>
          <label class="field">
            <span>{t("signIn.deviceKey")}</span>
            <div class="field-row">
              <span class="field-with-btn">
                <input
                  type={showKey() ? "text" : "password"}
                  value={key()}
                  onInput={(e) => setKey(e.currentTarget.value)}
                  aria-describedby="signin-key-hint"
                  aria-invalid={!!error() && !key().trim()}
                />
                <RevealButton shown={showKey()} onToggle={() => setShowKey(!showKey())} what={t("signIn.deviceKey")} />
              </span>
              <Show when={create()}>
                <button type="button" class="btn btn-ghost btn-sm" onClick={async () => setKey(await commands.generateKey())}>
                  {t("signIn.generate")}
                </button>
              </Show>
            </div>
            <small class="muted" id="signin-key-hint">
              {t("signIn.keyHint")}
            </small>
          </label>
        </Show>

        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>

        <button class="btn btn-primary btn-block" type="submit" disabled={busy()}>
          {join()
            ? busy()
              ? t("signIn.joining")
              : t("signIn.join")
            : busy()
              ? t("signIn.signingIn")
              : create()
                ? t("signIn.createAccount")
                : t("signIn.signIn")}
        </button>
        {/* Below the primary: join with a code, and Advanced (the server). */}
        <p class="center signin-links">
          <button
            type="button"
            class="link"
            onClick={() => {
              setJoin(!join());
              setError("");
            }}
          >
            {join() ? t("signIn.usePassword") : t("signIn.joinWithCode")}
          </button>
          <span aria-hidden="true">·</span>
          <button type="button" class="link" onClick={() => setAdvanced(!advanced())} aria-expanded={advanced()}>
            {advanced() ? t("signIn.hideAdvanced") : t("signIn.advanced")}
          </button>
        </p>
        <Show when={advanced()}>
          <label class="field">
            <span>{t("signIn.server")}</span>
            <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} />
          </label>
        </Show>
        <p class="muted center" hidden={join()}>
          {create() ? t("signIn.haveAccount") : t("signIn.newHere")}{" "}
          <button type="button" class="link" onClick={() => setCreate(!create())}>
            {create() ? t("signIn.signIn") : t("signIn.createAnAccount")}
          </button>
        </p>
      </form>
    </main>
  );
}

/** The eye inside a secret field: show or hide what's typed. */
function RevealButton(props: { shown: boolean; onToggle: () => void; what: string }) {
  const label = () => (props.shown ? t("signIn.hide", { what: props.what }) : t("signIn.show", { what: props.what }));
  return (
    <button type="button" class="btn btn-ghost btn-sm btn-icon" aria-pressed={props.shown} aria-label={label()} title={label()} onClick={props.onToggle}>
      <Icon name={props.shown ? "eye-off" : "eye"} size={16} />
    </button>
  );
}
