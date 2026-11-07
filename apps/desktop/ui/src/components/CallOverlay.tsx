// A phone call taken on this computer (calls.rs): the incoming-call dialog
// (Answer / Decline) and, during a call, the in-call bar: who, how long,
// where the audio is (and a picker to move it: phone, this computer, another
// speaker or headset), Mute, Keypad, Hang up. On Windows the audio stays on
// the phone, so there's no Mute and nowhere to move it.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { callDo, callStatus, duration, KEYPAD, watchCalls, type AudioTarget } from "../lib/calls";
import { CallAudioPicker } from "./CallAudioPicker";
import { t } from "../i18n";
import { Icon } from "./Icon";
import { rememberFocus } from "../lib/a11y";

/** The incoming-call box: Answer takes focus when it appears (it isn't
 *  modal, so Tab can still leave it), and focus goes back where it was when
 *  the call is answered, declined or missed. */
function IncomingCall(props: { who: string; number?: string | null; showNumber: boolean }) {
  let answer: HTMLButtonElement | undefined;
  let root: HTMLDivElement | undefined;
  const restore = rememberFocus();
  onMount(() => requestAnimationFrame(() => answer?.focus()));
  onCleanup(() => {
    const at = document.activeElement;
    if (!at || at === document.body || root?.contains(at)) restore();
  });
  return (
    <div
      ref={root}
      class="call-incoming"
      role="alertdialog"
      aria-modal="false"
      aria-labelledby="call-incoming-title"
      aria-describedby="call-incoming-who"
    >
      <span class="ring-pulse" aria-hidden="true" />
      <div class="call-incoming-text">
        <span id="call-incoming-title" class="muted small">
          {t("calls.incoming")}
        </span>
        <strong id="call-incoming-who">{props.who}</strong>
        <Show when={props.showNumber}>
          <span class="muted small">{props.number}</span>
        </Show>
      </div>
      <div class="row call-actions">
        <button class="btn call-decline" onClick={() => callDo(commands.callDecline())}>
          <Icon name="phone-off" size={18} />
          {t("calls.decline")}
        </button>
        <button class="btn btn-primary call-answer" ref={answer} onClick={() => callDo(commands.callAnswer())}>
          <Icon name="phone" size={18} />
          {t("calls.answer")}
        </button>
      </div>
    </div>
  );
}

export function Keypad(props: { onKey: (k: string) => void; id: string }) {
  return (
    <div class="call-keypad" id={props.id} role="group" aria-label={t("calls.keypad")}>
      <For each={KEYPAD}>
        {(k) => (
          <button class="btn btn-soft call-key" type="button" aria-label={t("calls.key", { key: k })} onClick={() => props.onKey(k)}>
            {k}
          </button>
        )}
      </For>
    </div>
  );
}

export default function CallOverlay() {
  onMount(watchCalls);
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(tick));
  const [keypad, setKeypad] = createSignal(false);
  const [picker, setPicker] = createSignal(false);

  const s = () => callStatus();
  const phase = () => s()?.phase ?? "idle";
  const who = () => s()?.name || s()?.number || t("calls.unknown");
  const audioLabel = () => {
    const st = s();
    if (!st || st.audio === "none") return "";
    if (st.audio === "phone") return t("calls.audioOnPhone");
    return st.output ? t("calls.audioOnDevice", { name: st.output }) : t("calls.audioOnMac");
  };
  const moveAudio = (target: AudioTarget) => callDo(commands.callSetAudio(target));

  return (
    <>
      <Show when={phase() === "incoming"}>
        <IncomingCall who={who()} number={s()?.number} showNumber={!!(s()?.name && s()?.number)} />
      </Show>

      <Show when={phase() === "active" || phase() === "outgoing"}>
        <section class="call-bar" aria-label={t("calls.bar")}>
          <div class="call-bar-main">
            <div class="call-bar-who">
              <strong>{t("calls.inCall", { who: who() })}</strong>
              <span class="muted small" aria-live="polite">
                <Show when={phase() === "active" && s()?.since_ms} fallback={t("calls.calling")}>
                  {(since) => duration(since(), now())}
                </Show>
                <Show when={s()?.held}> · {t("calls.onHold")}</Show>
                <Show when={audioLabel()}> · {audioLabel()}</Show>
              </span>
            </div>
            <div class="row call-actions">
              {/* Windows: the call's audio and microphone are the phone's. */}
              <Show when={s()?.os !== "windows"}>
                <button
                  class="btn btn-soft"
                  aria-pressed={s()?.muted ?? false}
                  onClick={() => callDo(commands.callMute(!(s()?.muted ?? false)))}
                >
                  {s()?.muted ? t("calls.unmute") : t("calls.mute")}
                </button>
              </Show>
              <button
                class="btn btn-soft"
                aria-expanded={keypad()}
                aria-controls="call-bar-keypad"
                onClick={() => setKeypad(!keypad())}
                disabled={phase() !== "active"}
              >
                {keypad() ? t("calls.hideKeypad") : t("calls.keypad")}
              </button>
              <button
                class="btn btn-soft"
                aria-expanded={picker()}
                aria-controls="call-bar-audio"
                aria-label={t("calls.audioMenu")}
                onClick={() => setPicker(!picker())}
              >
                {t("calls.audio")}
              </button>
              <button class="btn call-decline" onClick={() => callDo(commands.callHangUp())}>
                {t("calls.hangUp")}
              </button>
            </div>
          </div>
          <Show when={picker()}>
            <div class="call-bar-panel" id="call-bar-audio">
              {/* Nowhere to move it when this computer can't carry call audio. */}
              <Show when={s()?.mac_audio !== false}>
                <div class="row">
                  <Show
                    when={s()?.audio === "mac"}
                    fallback={
                      <button class="btn btn-ghost btn-sm" onClick={() => callDo(commands.callAudioToMac())}>
                        {t("calls.useMac")}
                      </button>
                    }
                  >
                    <button class="btn btn-ghost btn-sm" onClick={() => callDo(commands.callAudioToPhone())}>
                      {t("calls.usePhone")}
                    </button>
                  </Show>
                </div>
              </Show>
              <Show when={s()}>{(st) => <CallAudioPicker id="call-bar-picker" value={st().target} onChange={moveAudio} />}</Show>
            </div>
          </Show>
          <Show when={keypad() && phase() === "active"}>
            <div class="call-bar-panel">
              <Keypad id="call-bar-keypad" onKey={(k) => callDo(commands.callDtmf(k))} />
            </div>
          </Show>
        </section>
      </Show>
    </>
  );
}
