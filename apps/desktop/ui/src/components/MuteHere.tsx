// "Mute this computer while it plays on <phone>" (backend `mute_here.rs`):
// a switch per phone, remembered. The speakers here go quiet while this
// computer's sound plays on that phone and come back when it stops (also
// after a crash, at the next start). Not offered on Windows, or where
// muting would silence the phone too.

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type MuteHere as MuteHereState } from "../bindings";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

/** Why it can't, in words. */
export function whyText(why: string | null | undefined): string {
  if (why === "windows") return t("muteHere.windows");
  if (why === "silences") return t("muteHere.silences");
  return why ?? "";
}

export function MuteHere(props: { node: string; name: string }) {
  const [st, setSt] = createSignal<MuteHereState | null>(null);
  const refresh = () => commands.muteHere(props.node).then(setSt, () => {});
  let unlisten: (() => void) | undefined;
  onMount(async () => {
    refresh();
    unlisten = await listen("mute-here", refresh);
  });
  onCleanup(() => unlisten?.());
  const toggle = async (on: boolean) => {
    const r = await commands.setMuteHere(props.node, on);
    if (r.status === "error") toast(friendlyError(r.error), "error");
    refresh();
  };
  return (
    <div class="stack">
      <label class="row small">
        <input
          type="checkbox"
          checked={!!st()?.on}
          disabled={st()?.supported === false}
          onChange={(e) => toggle(e.currentTarget.checked)}
        />
        <span>{t("muteHere.label", { name: props.name })}</span>
      </label>
      <Show when={st()?.active}>
        <p class="muted small" aria-live="polite">
          {t("muteHere.active")}
        </p>
      </Show>
      <Show when={st()?.why}>
        <p class="muted small">{whyText(st()?.why)}</p>
      </Show>
    </div>
  );
}
