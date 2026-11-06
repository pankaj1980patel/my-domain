// The small bar at the top of the screen while another computer controls
// this one ("Share mouse and keyboard", kvm.rs), in its own always-on-top
// window that never takes the focus. "Take back" ends it from here.

import { createSignal, onCleanup, onMount } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type KvmState } from "../bindings";
import { niceName } from "../lib/words";
import { t } from "../i18n";

export default function KvmOverlay() {
  const [name, setName] = createSignal("");
  onMount(() => {
    commands.kvmSettings().then((s) => setName(s.state.name ?? ""), () => {});
    const un = listen<{ state: KvmState }>("kvm-changed", (e) => {
      if (e.payload.state.role === "controlled") setName(e.payload.state.name ?? "");
    });
    onCleanup(() => un.then((f) => f()));
  });
  return (
    <div class="kvm-overlay" role="status">
      <span class="kvm-dot" aria-hidden="true" />
      <span class="grow">{t("kvm.overlay", { name: niceName(name()) })}</span>
      <button class="btn btn-ghost btn-sm" onClick={() => commands.kvmStop()}>
        {t("kvm.takeBack")}
      </button>
    </div>
  );
}
