// A timer ringing on this computer (`lib/timers.ts`, the backend's
// `timer-ring`): a card with Stop (stops it on every device), +1 min and
// Open, and a chime for up to a minute. It goes when the timer is stopped,
// given more time or cancelled here or on any device; × only silences it
// here.

import { createEffect, createMemo, For, on, onCleanup, Show } from "solid-js";
import { addTime, hasEnded, quietTimer, ringing, timerAction, timers, type Timer } from "../lib/timers";
import { startRing, stopRing } from "../lib/ring";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

/** How long the chime goes on by itself. */
const CHIME_MS = 60_000;

export default function TimerAlarm(props: { onOpen: () => void }) {
  const due = createMemo(() =>
    ringing.ids.map((id) => timers.byId[id]).filter((x): x is Timer => !!x && hasEnded(x, Date.now())),
  );
  // Stopped, given more time or gone (here or elsewhere): off the window.
  createEffect(() => {
    const keep = new Set(due().map((x) => x.id));
    for (const id of ringing.ids) if (!keep.has(id)) quietTimer(id);
  });
  // Chime while any rings.
  createEffect(
    on(
      () => due().length > 0,
      (any) => (any ? startRing(() => {}, CHIME_MS) : stopRing()),
    ),
  );
  onCleanup(stopRing);

  const fail = (e: unknown) => toast(friendlyError(e), "error");

  return (
    <Show when={due().length > 0}>
      <section class="reminder-stack timer-alarm" aria-label={t("timers.ringingRegion")}>
        <For each={due()}>
          {(x) => (
            <div class="reminder-card timer-alarm-card" role="alert">
              <div class="reminder-head">
                <span aria-hidden="true">⏰</span>
                <strong class="reminder-title">{x.label || t("timers.untitled")}</strong>
                <button
                  class="btn btn-ghost btn-sm reminder-close"
                  aria-label={t("timers.silence")}
                  title={t("timers.silence")}
                  onClick={() => quietTimer(x.id)}
                >
                  ×
                </button>
              </div>
              <p class="small">{t("timers.timesUp")}</p>
              <div class="reminder-actions">
                <button class="btn btn-primary btn-sm" onClick={() => timerAction(x.id, "dismiss").catch(fail)}>
                  {t("timers.stop")}
                </button>
                <button class="btn btn-soft btn-sm" onClick={() => addTime(x.id, 60_000).catch(fail)}>
                  {t("timers.plusMinute")}
                </button>
                <button
                  class="btn btn-ghost btn-sm"
                  onClick={() => {
                    quietTimer(x.id);
                    props.onOpen();
                  }}
                >
                  {t("timers.open")}
                </button>
              </div>
            </div>
          )}
        </For>
      </section>
    </Show>
  );
}
