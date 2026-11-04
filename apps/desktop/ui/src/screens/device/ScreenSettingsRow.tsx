// The phone's screen settings: brightness, screen timeout, auto-rotate.

import { createEffect, createSignal, For, on, Show } from "solid-js";
import { peerName, runAction, type DeviceStatus } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { Gate, toast } from "../../components/ui";
import { t } from "../../i18n";

/** Screen timeouts offered, in seconds. */
const TIMEOUTS = [15, 30, 60, 120, 300, 600];

function timeoutLabel(s: number): string {
  return s < 60 ? t("screenSettings.seconds", { n: s }) : t("screenSettings.minutes", { n: Math.round(s / 6) / 10 });
}

/** Brightness (a level or automatic), how long the screen stays on, and
 *  auto-rotate on the phone. Changing them needs the phone's "Modify system
 *  settings" permission; until it's given the controls are greyed out, and
 *  "Allow on the phone" makes the phone open that setting. */
export default function ScreenSettingsRow(props: { node: string; st: DeviceStatus }) {
  const name = () => peerName(props.node);
  const off = () => (props.st.writeSettings === false ? t("screenSettings.off", { name: name() }) : undefined);
  // The slider's level while it's dragged, and until the phone answers.
  const [drag, setDrag] = createSignal<number | null>(null);
  createEffect(on(() => props.st.ts, () => setDrag(null), { defer: true }));
  const level = () => drag() ?? props.st.brightness ?? 50;

  const send = (args: Record<string, unknown>) =>
    runAction(props.node, "screen_settings", args).catch((e) => toast(friendlyError(e), "error"));
  const allow = () =>
    runAction(props.node, "screen_settings", {})
      .then(() => toast(t("screenSettings.asked", { name: name() })))
      .catch((e) => toast(friendlyError(e), "error"));

  const timeouts = () => {
    const now = props.st.screenTimeoutS;
    return now !== undefined && !TIMEOUTS.includes(now) ? [...TIMEOUTS, now].sort((a, b) => a - b) : TIMEOUTS;
  };

  return (
    <div class="stack screen-set">
      <div class="row screen-bright">
        <span class="small muted">
          {props.st.brightness !== undefined || drag() !== null
            ? t("screenSettings.brightnessLevel", { level: level() })
            : t("screenSettings.brightness")}
        </span>
        <Gate reason={off()}>
          <input
            type="range"
            min="0"
            max="100"
            step="1"
            aria-label={t("screenSettings.brightness")}
            disabled={!!off()}
            value={level()}
            onInput={(e) => setDrag(e.currentTarget.valueAsNumber)}
            onChange={(e) => send({ brightness: e.currentTarget.valueAsNumber })}
          />
        </Gate>
        <Show when={props.st.autoBrightness !== undefined}>
          <Gate reason={off()}>
            <label class="row small" title={t("screenSettings.autoHint")}>
              <span>{t("screenSettings.auto")}</span>
              <input
                type="checkbox"
                role="switch"
                disabled={!!off()}
                checked={props.st.autoBrightness}
                onChange={(e) => send({ auto_brightness: e.currentTarget.checked })}
              />
            </label>
          </Gate>
        </Show>
      </div>
      <div class="row">
        <Show when={props.st.screenTimeoutS !== undefined}>
          <span class="small muted">{t("screenSettings.timeout")}</span>
          <Gate reason={off()}>
            <select
              aria-label={t("screenSettings.timeout")}
              disabled={!!off()}
              onChange={(e) => send({ timeout_s: Number(e.currentTarget.value) })}
            >
              <For each={timeouts()}>{(s) => (
                  <option value={String(s)} selected={s === props.st.screenTimeoutS}>
                    {timeoutLabel(s)}
                  </option>
                )}</For>
            </select>
          </Gate>
        </Show>
        <span class="grow" />
        <Show when={props.st.autoRotate !== undefined}>
          <Gate reason={off()}>
            <label class="row small">
              <span>{t("screenSettings.rotate")}</span>
              <input
                type="checkbox"
                role="switch"
                disabled={!!off()}
                checked={props.st.autoRotate}
                onChange={(e) => send({ rotate: e.currentTarget.checked })}
              />
            </label>
          </Gate>
        </Show>
      </div>
      <Show when={off()}>
        {(reason) => (
          <div class="row">
            <span class="muted small grow">{reason()}</span>
            <button class="btn btn-ghost btn-sm" onClick={allow}>
              {t("screenSettings.allow")}
            </button>
          </div>
        )}
      </Show>
    </div>
  );
}
