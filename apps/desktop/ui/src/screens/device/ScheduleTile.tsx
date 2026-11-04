// Scheduled actions for one device: "every weekday at 08:00, ringer to
// normal", "once at 18:30, ring it". The list and form are `ScheduleList`
// (shared with this computer's schedules in Settings). Only presets the
// device lists in its status (`actions`) are offered; ringing works on any
// device.

import { peerName, state } from "../../lib/core";
import { niceName } from "../../lib/words";
import { ScheduleList, type Preset } from "../../components/ScheduleList";
import { t } from "../../i18n";

const PRESETS: Preset[] = [
  { id: "ringer_normal", action: "ringer", args: { mode: "normal" }, label: "schedule.preset.ringerNormal" },
  { id: "ringer_vibrate", action: "ringer", args: { mode: "vibrate" }, label: "schedule.preset.ringerVibrate" },
  { id: "ringer_silent", action: "ringer", args: { mode: "silent" }, label: "schedule.preset.ringerSilent" },
  { id: "lock", action: "lock", args: {}, label: "schedule.preset.lock" },
  { id: "ring", action: "ring", args: {}, label: "schedule.preset.ring", always: true },
  { id: "torch_off", action: "flashlight", args: { on: false }, label: "schedule.preset.torchOff" },
  { id: "awake_off", action: "keep_awake", args: { minutes: 0 }, label: "schedule.preset.awakeOff" },
  { id: "media", action: "media", args: { key: "play_pause" }, label: "schedule.preset.media" },
];

export default function ScheduleTile(props: { node: string }) {
  const name = () => niceName(peerName(props.node));
  return (
    <article class="tile tile-wide">
      <h2>{t("schedule.title")}</h2>
      <p class="muted small">{t("schedule.hint")}</p>
      <ScheduleList
        node={props.node}
        name={name()}
        presets={PRESETS}
        actions={state.status[props.node]?.actions}
        unknownHint={t("schedule.moreWhenConnected", { name: name() })}
      />
    </article>
  );
}
