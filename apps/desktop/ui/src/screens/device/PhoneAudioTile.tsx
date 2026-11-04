// The phone's media audio (music, videos, podcasts; never calls) on this
// computer's speakers (backend `phone_audio.rs`, mdcore
// `features::audio_share`). "Play this phone's audio here" asks the phone:
// its owner says yes there, then Android's own prompt. Or the phone starts
// it ("Play on <this computer>"): then this tile shows it playing. While it
// plays: the output, the volume, how it's going (buffered, underruns) and
// Stop. One phone at a time.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type AudioOutputs, type PhoneAudioStatus } from "../../bindings";
import { peerName } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";

export default function PhoneAudioTile(props: { node: string }) {
  const [st, setSt] = createSignal<PhoneAudioStatus | null>(null);
  const [outs, setOuts] = createSignal<AudioOutputs>({ outputs: [], default_output: null });
  const [stopped, setStopped] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const mine = () => st()?.node === props.node;
  const other = () => !!st()?.node && st()?.node !== props.node;

  const refresh = () => commands.phoneAudioStatus().then(setSt, () => {});
  const refreshOutputs = () => commands.phoneAudioOutputs().then(setOuts, () => {});
  let poll: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void) | undefined;
  onMount(async () => {
    refresh();
    refreshOutputs();
    // Started, stopped (here, on the phone, or the link went), playing.
    unlisten = await listen<PhoneAudioStatus>("phone-audio", (e) => {
      const was = mine();
      setSt(e.payload);
      if (was && !e.payload.node && e.payload.error) setStopped(e.payload.error);
      if (e.payload.node === props.node) setStopped(null);
    });
    // How it's going: twice a second while it plays here.
    poll = setInterval(() => {
      if (mine()) refresh();
    }, 500);
  });
  onCleanup(() => {
    unlisten?.();
    if (poll) clearInterval(poll);
  });

  const start = async () => {
    setBusy(true);
    setStopped(null);
    const r = await commands.phoneAudioStart(props.node);
    setBusy(false);
    if (r.status === "error") toast(friendlyError(r.error), "error");
    refresh();
  };
  const stop = () => commands.phoneAudioStop().then(refresh);
  const setOutput = (v: string) => commands.phoneAudioSetOutput(v || null).then(refresh);
  const setVolume = (v: number) => {
    setSt((s) => (s ? { ...s, volume: v } : s));
    commands.phoneAudioSetVolume(v);
  };
  // A chosen output that isn't there now still shows, so the choice is kept.
  const outputs = () => {
    const list = outs().outputs;
    const o = st()?.output;
    return o && !list.includes(o) ? [o, ...list] : list;
  };
  const stateText = () => {
    const s = st();
    if (!s) return "";
    if (s.state === "waiting") return t("phoneAudio.waiting", { name: name() });
    if (s.state === "buffering") return t("phoneAudio.buffering");
    return t("phoneAudio.playing", { name: name() });
  };

  return (
    <article class="tile">
      <h2>{t("phoneAudio.title")}</h2>
      <p class="muted small">{t("phoneAudio.hint")}</p>
      <Show when={mine()}>
        <div class="phone-audio stack">
          <p class="small" aria-live="polite">
            {stateText()}
          </p>
          <Show when={st()?.state !== "waiting"}>
            <p class="muted small phone-audio-stats">
              {t("phoneAudio.stats", {
                ms: st()?.buffered_ms ?? 0,
                khz: Math.round((st()?.rate ?? 0) / 100) / 10,
                underruns: st()?.underruns ?? 0,
              })}
            </p>
          </Show>
          <label class="row small">
            <span class="muted">{t("phoneAudio.output")}</span>
            <select aria-label={t("phoneAudio.output")} onFocus={refreshOutputs} onChange={(e) => setOutput(e.currentTarget.value)}>
              <option value="" selected={!st()?.output}>
                {outs().default_output
                  ? t("phoneAudio.defaultNamed", { name: outs().default_output! })
                  : t("phoneAudio.default")}
              </option>
              <For each={outputs()}>
                {(d) => (
                  <option value={d} selected={st()?.output === d}>
                    {d}
                  </option>
                )}
              </For>
            </select>
          </label>
          <label class="row small phone-audio-volume">
            <span class="muted">{t("phoneAudio.volume")}</span>
            <input
              type="range"
              min="0"
              max="100"
              step="1"
              aria-label={t("phoneAudio.volume")}
              value={st()?.volume ?? 100}
              onInput={(e) => setVolume(Number(e.currentTarget.value))}
            />
          </label>
          <p class="muted small">{t("phoneAudio.phoneVolume")}</p>
        </div>
      </Show>
      <Show when={other()}>
        <p class="muted small">{t("phoneAudio.other", { name: peerName(st()!.node!) })}</p>
      </Show>
      <Show when={!mine() && stopped()}>
        <p class="muted small">{stopped()}</p>
      </Show>
      <div class="row">
        <Show
          when={mine()}
          fallback={
            <button class="btn btn-soft btn-block" disabled={busy()} onClick={start}>
              {t("phoneAudio.start")}
            </button>
          }
        >
          <button class="btn btn-ghost btn-sm" onClick={stop}>
            {t("phoneAudio.stop")}
          </button>
        </Show>
      </div>
    </article>
  );
}
