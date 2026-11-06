// This computer's sound on a phone (backend `computer_audio.rs`, mdcore
// `features::audio_share` the other way round): listen to the computer on
// the phone and the earbuds on it. "Play on <phone>" asks the phone: it
// plays only after a tap there. Or the phone starts it ("Play <computer>'s
// sound here"): then this tile shows it playing. Stop here, in the
// sidebar, the tray menu or on the phone. The state lives in the store
// (`state.computerAudio`, from the `computer-audio` event).

import { createSignal, Show } from "solid-js";
import { commands } from "../../bindings";
import { peerName, state } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";
import { NearbyBadge } from "../../components/NearbyBadge";

const isMac = () => /mac/i.test(navigator.platform || navigator.userAgent);

export default function ComputerAudioTile(props: { node: string }) {
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const st = () => state.computerAudio;
  const playing = () => !!st()?.playing_to.includes(props.node);
  const asked = () => !playing() && !!st()?.asked.includes(props.node);

  const start = async () => {
    setBusy(true);
    const r = await commands.computerAudioOffer(props.node);
    setBusy(false);
    if (r.status === "error") toast(friendlyError(r.error), "error");
  };
  const stop = () => commands.computerAudioStop(props.node);

  return (
    <article class="tile">
      <h2>{t("computerAudio.title")}</h2>
      <p class="muted small">{t("computerAudio.hint")}</p>
      <Show when={st()?.supported === false}>
        <p class="muted small">{t("computerAudio.unsupported")}</p>
      </Show>
      <Show when={st()?.supported !== false}>
        <Show when={isMac() && !playing()}>
          <p class="muted small">{t("computerAudio.hintMac")}</p>
        </Show>
        <div class="computer-audio stack">
          <Show when={playing()}>
            <p class="small computer-audio-now" aria-live="polite">
              <span class="computer-audio-dot" aria-hidden="true" />
              {t("computerAudio.playing", { name: name() })}
            </p>
            <NearbyBadge node={props.node} />
          </Show>
          <Show when={asked()}>
            <p class="small" aria-live="polite">
              {t("computerAudio.asked", { name: name() })}
            </p>
          </Show>
          <Show when={!playing() && !asked() && st()?.error}>
            <p class="muted small">{st()?.error}</p>
          </Show>
        </div>
        <div class="row">
          <Show
            when={playing() || asked()}
            fallback={
              <button class="btn btn-soft btn-block" disabled={busy()} onClick={start}>
                {t("computerAudio.start", { name: name() })}
              </button>
            }
          >
            <button class="btn btn-ghost btn-sm" onClick={stop}>
              {playing() ? t("computerAudio.stop") : t("computerAudio.cancel")}
            </button>
          </Show>
        </div>
      </Show>
    </article>
  );
}
