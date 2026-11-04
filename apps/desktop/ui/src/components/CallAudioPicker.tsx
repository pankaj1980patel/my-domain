// Where a call's audio goes: the phone, or this Mac on a speaker and a
// microphone of your choice (built-in, AirPods, a USB headset, a display…)
// or the system default. Used in the in-call bar and in Settings (for
// answered calls). The device list follows devices coming and going.

import { For, onMount, Show } from "solid-js";
import { audioDevices, callStatus, refreshAudioDevices, type AudioTarget } from "../lib/calls";
import { t } from "../i18n";

export function CallAudioPicker(props: { value: AudioTarget; onChange: (t: AudioTarget) => void; id: string }) {
  onMount(refreshAudioDevices);
  const named = (name: string | null) => (name ? t("calls.systemDefaultNamed", { name }) : t("calls.systemDefault"));
  const set = (change: Partial<AudioTarget>) => props.onChange({ ...props.value, ...change });
  // A chosen device that isn't there now still shows, so the choice is kept.
  const outputs = () => {
    const list = audioDevices().outputs;
    const o = props.value.output;
    return o && !list.includes(o) ? [o, ...list] : list;
  };
  const inputs = () => {
    const list = audioDevices().inputs;
    const i = props.value.input;
    return i && !list.includes(i) ? [i, ...list] : list;
  };
  // macOS refused call audio here before: the phone is the only choice.
  if (callStatus()?.mac_audio === false) {
    return <p class="call-audio-picker small muted">{t("calls.macAudioRefused")}</p>;
  }
  return (
    <div class="call-audio-picker stack">
      <label class="row small">
        <span class="muted">{t("calls.audio")}</span>
        <select
          id={`${props.id}-where`}
          aria-label={t("calls.audio")}
          onChange={(e) => set({ phone: e.currentTarget.value === "phone" })}
        >
          <option value="mac" selected={!props.value.phone}>
            {t("calls.audioMac")}
          </option>
          <option value="phone" selected={props.value.phone}>
            {t("calls.audioPhone")}
          </option>
        </select>
      </label>
      <Show when={!props.value.phone}>
        <label class="row small">
          <span class="muted">{t("calls.output")}</span>
          <select aria-label={t("calls.output")} onChange={(e) => set({ output: e.currentTarget.value || null })}>
            <option value="" selected={!props.value.output}>
              {named(audioDevices().default_output)}
            </option>
            <For each={outputs()}>
              {(d) => (
                <option value={d} selected={props.value.output === d}>
                  {d}
                </option>
              )}
            </For>
          </select>
        </label>
        <label class="row small">
          <span class="muted">{t("calls.input")}</span>
          <select aria-label={t("calls.input")} onChange={(e) => set({ input: e.currentTarget.value || null })}>
            <option value="" selected={!props.value.input}>
              {named(audioDevices().default_input)}
            </option>
            <For each={inputs()}>
              {(d) => (
                <option value={d} selected={props.value.input === d}>
                  {d}
                </option>
              )}
            </For>
          </select>
        </label>
      </Show>
    </div>
  );
}
