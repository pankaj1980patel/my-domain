// "Join <phone>’s hotspot for this transfer?" (hotspot.rs, `hotspot-ask`):
// a big file with a phone that isn't on this computer's network can go over
// the phone's hotspot, but this computer leaves its Wi-Fi until it's done.
// Join says yes; Not now (or Escape) says no and the file keeps its usual
// route. Shown only with "Use the phone’s hotspot for big files" on "Ask".

import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../bindings";
import { peerName } from "../lib/core";
import { t } from "../i18n";

type Ask = { sid: string; node_id: string; wifi: string | null };

export function HotspotAsk() {
  const [ask, setAsk] = createSignal<Ask | null>(null);
  let off: (() => void) | undefined;
  onMount(() => {
    listen<Ask>("hotspot-ask", (e) => setAsk(e.payload))
      .then((u) => (off = u))
      .catch(() => {});
  });
  onCleanup(() => off?.());

  const answer = (yes: boolean) => {
    const a = ask();
    setAsk(null);
    if (a) commands.hotspotConsent(a.sid, yes).catch(() => {});
  };

  return (
    <Show when={ask()}>
      {(a) => (
        <Portal>
          <div class="send-to-backdrop">
            <div
              class="send-to"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="hotspot-ask-title"
              aria-describedby="hotspot-ask-body"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  answer(false);
                }
              }}
            >
              <h2 id="hotspot-ask-title">{t("hotspot.askTitle", { device: peerName(a().node_id) })}</h2>
              <p id="hotspot-ask-body" class="muted small">
                {a().wifi ? t("hotspot.askBody", { wifi: a().wifi ?? "" }) : t("hotspot.askBodyNoWifi")}
              </p>
              <div class="row">
                <button class="btn btn-primary" type="button" ref={(el) => setTimeout(() => el.focus())} onClick={() => answer(true)}>
                  {t("hotspot.join")}
                </button>
                <button class="btn btn-ghost" type="button" onClick={() => answer(false)}>
                  {t("hotspot.notNow")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
}
