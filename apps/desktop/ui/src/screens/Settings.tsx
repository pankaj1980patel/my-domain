// Account, this device, and preferences. Tap the version 7 times for the
// Developer screen.

import { createSignal, Show } from "solid-js";
import { commands } from "../bindings";
import { setClipSync, signOut, state, unwrap } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "../components/ui";

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
      toast("Developer screen unlocked", "ok");
      props.onDeveloper();
    }
  };

  return (
    <section class="page">
      <header class="page-head">
        <h1>Settings</h1>
      </header>

      <article class="tile">
        <h2>Account</h2>
        <p>
          Signed in as <strong>{state.account.username || "…"}</strong>
        </p>
        <button class="btn btn-ghost" onClick={signOut}>
          Sign out
        </button>
      </article>

      <article class="tile">
        <h2>This device</h2>
        <p>
          Other devices see it as <strong>{niceName(state.me?.name ?? "")}</strong>.
        </p>
        <label class="switch-row">
          <span>Keep clipboards in sync with my devices</span>
          <input type="checkbox" role="switch" checked={state.clipSync} onChange={(e) => setClipSync(e.currentTarget.checked)} />
        </label>
      </article>

      <article class="tile">
        <h2>Device key</h2>
        <p class="muted small">All your devices must use the same key. Changing it here means changing it on every device.</p>
        <Show
          when={showKey()}
          fallback={
            <button class="btn btn-ghost" onClick={() => setShowKey(true)}>
              Change device key
            </button>
          }
        >
          <form
            class="stack"
            onSubmit={async (e) => {
              e.preventDefault();
              try {
                await unwrap(commands.updateEncryptionKey(newKey(), pass()));
                toast("Device key changed", "ok");
                setShowKey(false);
                setNewKey("");
                setPass("");
              } catch (err) {
                toast(friendlyError(err), "error");
              }
            }}
          >
            <input type="password" placeholder="New device key" value={newKey()} onInput={(e) => setNewKey(e.currentTarget.value)} />
            <input type="password" placeholder="Your password" value={pass()} onInput={(e) => setPass(e.currentTarget.value)} />
            <div class="row">
              <button class="btn btn-primary" type="submit" disabled={!newKey() || !pass()}>
                Save
              </button>
              <button class="btn btn-ghost" type="button" onClick={() => setShowKey(false)}>
                Cancel
              </button>
            </div>
          </form>
        </Show>
      </article>

      <p class="version muted small" onClick={tapVersion}>
        my-domain 0.1.0
      </p>
    </section>
  );
}
