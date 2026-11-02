// Welcome / sign in. The server is pre-filled and tucked under "Advanced":
// nobody should have to type a URL.

import { createSignal, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { signIn } from "../lib/core";
import { friendlyError } from "../lib/words";

const DEFAULT_SERVER = "https://152-67-7-50.sslip.io";

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
      setError("Fill in your username, password and device key.");
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
        <img class="signin-logo" src="/logo.png" alt="" />
        <h1>{create() ? "Create your account" : "Welcome back"}</h1>
        <p class="muted">All your devices, working together. Privately.</p>

        <label class="field">
          <span>Username</span>
          <input value={user()} onInput={(e) => setUser(e.currentTarget.value)} autocomplete="username" autofocus />
        </label>
        <label class="field">
          <span>Password</span>
          <input
            type="password"
            value={pass()}
            onInput={(e) => setPass(e.currentTarget.value)}
            autocomplete={create() ? "new-password" : "current-password"}
          />
        </label>
        <label class="field">
          <span>Device key</span>
          <div class="field-row">
            <input type="password" value={key()} onInput={(e) => setKey(e.currentTarget.value)} />
            <Show when={create()}>
              <button type="button" class="btn btn-ghost btn-sm" onClick={async () => setKey(await commands.generateKey())}>
                Generate
              </button>
            </Show>
          </div>
          <small class="muted">Use the same key on all your devices. It encrypts everything they send each other.</small>
        </label>

        <button type="button" class="link" onClick={() => setAdvanced(!advanced())} aria-expanded={advanced()}>
          {advanced() ? "Hide advanced" : "Advanced"}
        </button>
        <Show when={advanced()}>
          <label class="field">
            <span>Server</span>
            <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} />
          </label>
        </Show>

        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>

        <button class="btn btn-primary btn-block" type="submit" disabled={busy()}>
          {busy() ? "Signing in…" : create() ? "Create account" : "Sign in"}
        </button>
        <p class="muted center">
          {create() ? "Already have an account?" : "New here?"}{" "}
          <button type="button" class="link" onClick={() => setCreate(!create())}>
            {create() ? "Sign in" : "Create an account"}
          </button>
        </p>
      </form>
    </main>
  );
}
