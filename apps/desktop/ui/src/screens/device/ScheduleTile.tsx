// Scheduled actions for one device: "every weekday at 08:00, ringer to
// normal", "once at 18:30, ring it". The list and form are `ScheduleList`
// (shared with this computer's schedules in Settings). Only presets the
// device lists in its status (`actions`) are offered; ringing works on any
// device.

import { peerName, state } from "../../lib/core";
import { niceName } from "../../lib/words";
import { ScheduleList } from "../../components/ScheduleList";
import { DEVICE_PRESETS } from "../../lib/presets";
import { t } from "../../i18n";

export default function ScheduleTile(props: { node: string }) {
  const name = () => niceName(peerName(props.node));
  return (
    <article class="tile tile-wide">
      <h2>{t("schedule.title")}</h2>
      <p class="muted small">{t("schedule.hint")}</p>
      <ScheduleList
        node={props.node}
        name={name()}
        presets={DEVICE_PRESETS}
        actions={state.status[props.node]?.actions}
        unknownHint={t("schedule.moreWhenConnected", { name: name() })}
      />
    </article>
  );
}
