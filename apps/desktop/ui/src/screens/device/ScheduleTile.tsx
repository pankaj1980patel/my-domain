// Scheduled actions for one device: "every weekday at 08:00, ringer to
// normal", "once at 18:30, ring it". The list and form are `ScheduleList`
// (shared with this computer's schedules in Settings). Only presets the
// device lists in its status (`actions`) are offered; ringing works on any
// device, and waking on a computer that said it may be woken.

import { peerName, state } from "../../lib/core";
import { ScheduleList } from "../../components/ScheduleList";
import { DEVICE_PRESETS, withWake } from "../../lib/presets";
import { createCanWake } from "../../lib/canWake";
import { t } from "../../i18n";

export default function ScheduleTile(props: { node: string }) {
  const name = () => peerName(props.node);
  const canWake = createCanWake(() => props.node);
  return (
    <article class="tile tile-wide">
      <h2>{t("schedule.title")}</h2>
      <p class="muted small">{t("schedule.hint")}</p>
      <ScheduleList
        node={props.node}
        name={name()}
        presets={withWake(DEVICE_PRESETS, canWake())}
        actions={state.status[props.node]?.actions}
        unknownHint={t("schedule.moreWhenConnected", { name: name() })}
      />
    </article>
  );
}
