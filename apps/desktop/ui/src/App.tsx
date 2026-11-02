// App shell: sign in first, then a sidebar with Devices / Activity /
// Settings. A device opens its detail page; the Developer screen is hidden.

import { createSignal, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { dismissRing, onOpenDevice, peerName, resume, state } from "./lib/core";
import { unlockAudio } from "./lib/ring";
import { niceName } from "./lib/words";
import { Toasts } from "./components/ui";
import SignIn from "./screens/SignIn";
import Home from "./screens/Home";
import Device from "./screens/Device";
import Activity from "./screens/Activity";
import Settings from "./screens/Settings";
import Developer from "./screens/Developer";
import { t } from "./i18n";

type View = { name: "home" } | { name: "device"; node: string } | { name: "activity" } | { name: "settings" } | { name: "developer" };

export default function App() {
  // null while checking for a saved session, so sign-in doesn't flash.
  const [signedIn, setSignedIn] = createSignal<boolean | null>(null);
  onMount(() => {
    unlockAudio();
    resume().then(setSignedIn, () => setSignedIn(false));
  });
  const [view, setView] = createSignal<View>({ name: "home" });
  const go = (v: View) => setView(v);
  // The tray's "Open" on a device jumps to that device's page.
  onMount(() => {
    const off = onOpenDevice((node) => go({ name: "device", node }));
    onCleanup(() => off.then((f) => f()));
  });
  const is = (n: View["name"]) => view().name === n || (n === "home" && view().name === "device");
  const connected = () => state.peers.filter((p) => state.links[p.node_id]).length;

  return (
    <>
      <Show when={signedIn() !== null}>
        <Show when={signedIn()} fallback={<SignIn onDone={() => setSignedIn(true)} />}>
        <div class="shell">
          <nav class="sidebar" aria-label={t("nav.label")}>
            <div class="brand">
              <img src="/logo.png" alt="" />
              <span>{t("app.name")}</span>
            </div>
            <button class={`nav-item ${is("home") ? "active" : ""}`} onClick={() => go({ name: "home" })}>
              {t("nav.devices")}
              <Show when={connected() > 0}>
                <span class="nav-count">{connected()}</span>
              </Show>
            </button>
            <button class={`nav-item ${is("activity") ? "active" : ""}`} onClick={() => go({ name: "activity" })}>
              {t("nav.activity")}
            </button>
            <button class={`nav-item ${is("settings") || is("developer") ? "active" : ""}`} onClick={() => go({ name: "settings" })}>
              {t("nav.settings")}
            </button>
            <div class="sidebar-foot muted small">{niceName(state.me?.name ?? "")}</div>
          </nav>
          <main class="content">
            <Switch>
              <Match when={view().name === "home"}>
                <Home onOpen={(node) => go({ name: "device", node })} />
              </Match>
              <Match when={view().name === "device" && view()}>
                {(v) => <Device node={(v() as { node: string }).node} onBack={() => go({ name: "home" })} />}
              </Match>
              <Match when={view().name === "activity"}>
                <Activity />
              </Match>
              <Match when={view().name === "settings"}>
                <Settings onDeveloper={() => go({ name: "developer" })} />
              </Match>
              <Match when={view().name === "developer"}>
                <Developer onBack={() => go({ name: "settings" })} />
              </Match>
            </Switch>
          </main>
        </div>
        </Show>
      </Show>
      <Show when={state.ringing}>
        {(r) => (
          // Another device is looking for this one: any click silences it.
          <div class="ring-banner" role="alertdialog" aria-live="assertive" onClick={dismissRing}>
            <span class="ring-pulse" aria-hidden="true" />
            <strong>{t("ring.title", { name: niceName(peerName(r().from)) })}</strong>
            <button class="btn btn-primary" onClick={dismissRing} autofocus>
              {t("ring.stop")}
            </button>
          </div>
        )}
      </Show>
      <Toasts />
    </>
  );
}
