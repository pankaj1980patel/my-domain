// App shell: sign in first, then a sidebar with Devices / Activity /
// Settings. A device opens its detail page; the Developer screen is hidden.

import { createSignal, Match, onMount, Show, Switch } from "solid-js";
import { resume, state } from "./lib/core";
import { niceName } from "./lib/words";
import { Toasts } from "./components/ui";
import SignIn from "./screens/SignIn";
import Home from "./screens/Home";
import Device from "./screens/Device";
import Activity from "./screens/Activity";
import Settings from "./screens/Settings";
import Developer from "./screens/Developer";

type View = { name: "home" } | { name: "device"; node: string } | { name: "activity" } | { name: "settings" } | { name: "developer" };

export default function App() {
  // null while checking for a saved session, so sign-in doesn't flash.
  const [signedIn, setSignedIn] = createSignal<boolean | null>(null);
  onMount(() => resume().then(setSignedIn, () => setSignedIn(false)));
  const [view, setView] = createSignal<View>({ name: "home" });
  const go = (v: View) => setView(v);
  const is = (n: View["name"]) => view().name === n || (n === "home" && view().name === "device");
  const connected = () => state.peers.filter((p) => state.links[p.node_id]).length;

  return (
    <>
      <Show when={signedIn() !== null}>
        <Show when={signedIn()} fallback={<SignIn onDone={() => setSignedIn(true)} />}>
        <div class="shell">
          <nav class="sidebar" aria-label="Main">
            <div class="brand">
              <img src="/logo.png" alt="" />
              <span>my-domain</span>
            </div>
            <button class={`nav-item ${is("home") ? "active" : ""}`} onClick={() => go({ name: "home" })}> Devices
              <Show when={connected() > 0}>
                <span class="nav-count">{connected()}</span>
              </Show>
            </button>
            <button class={`nav-item ${is("activity") ? "active" : ""}`} onClick={() => go({ name: "activity" })}> Activity
            </button>
            <button class={`nav-item ${is("settings") || is("developer") ? "active" : ""}`} onClick={() => go({ name: "settings" })}> Settings
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
      <Toasts />
    </>
  );
}
