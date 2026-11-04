// Scheduled actions for one device: "every weekday at 08:00, ringer to
// normal", "once at 18:30, ring it". The schedules live and run in Rust
// (`schedule.rs`); this tile lists, adds, changes and removes them. Only
// presets the device lists in its status (`actions`) are offered; ringing
// works on any device.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { commands, type Schedule_Serialize as Schedule } from "../../bindings";
import { peerName, state, unwrap } from "../../lib/core";
import { friendlyError, niceName } from "../../lib/words";
import { Gate, toast } from "../../components/ui";
import { t, type Key } from "../../i18n";

type Preset = { id: string; action: string; args: Record<string, unknown>; label: Key };

const PRESETS: Preset[] = [
  { id: "ringer_normal", action: "ringer", args: { mode: "normal" }, label: "schedule.preset.ringerNormal" },
  { id: "ringer_vibrate", action: "ringer", args: { mode: "vibrate" }, label: "schedule.preset.ringerVibrate" },
  { id: "ringer_silent", action: "ringer", args: { mode: "silent" }, label: "schedule.preset.ringerSilent" },
  { id: "lock", action: "lock", args: {}, label: "schedule.preset.lock" },
  { id: "ring", action: "ring", args: {}, label: "schedule.preset.ring" },
  { id: "torch_off", action: "flashlight", args: { on: false }, label: "schedule.preset.torchOff" },
  { id: "awake_off", action: "keep_awake", args: { minutes: 0 }, label: "schedule.preset.awakeOff" },
  { id: "media", action: "media", args: { key: "play_pause" }, label: "schedule.preset.media" },
];

/** 1 = Monday … 7 = Sunday, as the schedule stores them. */
const DAYS: [number, Key][] = [
  [1, "schedule.day.mon"],
  [2, "schedule.day.tue"],
  [3, "schedule.day.wed"],
  [4, "schedule.day.thu"],
  [5, "schedule.day.fri"],
  [6, "schedule.day.sat"],
  [7, "schedule.day.sun"],
];

/** How often the list is read again (a one-off turns itself off after running). */
const REFRESH_MS = 60_000;

export default function ScheduleTile(props: { node: string }) {
  const [list, setList] = createSignal<Schedule[]>([]);
  const [preset, setPreset] = createSignal("");
  const [time, setTime] = createSignal("08:00");
  const [days, setDays] = createSignal<number[]>([1, 2, 3, 4, 5]);
  const [busy, setBusy] = createSignal(false);
  const name = () => niceName(peerName(props.node));
  const actions = () => state.status[props.node]?.actions;
  /** Ringing works everywhere; the rest only where the device lists it. */
  const offered = (action: string) => action === "ring" || !!actions()?.includes(action);
  const presets = () => PRESETS.filter((p) => offered(p.action));
  const chosen = () => presets().find((p) => p.id === preset()) ?? presets()[0];
  const mine = () => list().filter((s) => s.node === props.node);

  const load = () => commands.listSchedules().then(setList, () => {});
  onMount(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  const replace = (s: Schedule) => setList((l) => l.map((x) => (x.id === s.id ? s : x)));
  const change = (s: Schedule, patch: Partial<Schedule>) =>
    unwrap(commands.updateSchedule({ ...s, ...patch })).then(replace, (e) => toast(friendlyError(e), "error"));

  const add = async (e: Event) => {
    e.preventDefault();
    const p = chosen();
    if (!p || !time()) return;
    setBusy(true);
    try {
      const s = await unwrap(
        commands.addSchedule({
          node: props.node,
          action: p.action,
          args: JSON.stringify(p.args),
          label: t(p.label),
          time: time(),
          days: days(),
          enabled: true,
        }),
      );
      setList((l) => [...l, s]);
      toast(t("schedule.added", { name: name(), time: s.time }), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  const remove = (s: Schedule) =>
    commands.deleteSchedule(s.id).then(() => setList((l) => l.filter((x) => x.id !== s.id)));

  const runNow = (s: Schedule) =>
    unwrap(commands.runScheduleNow(s.id)).then(
      (via) => toast(via === "queued" ? t("schedule.queued", { name: name() }) : t("schedule.ran", { what: s.label }), "ok"),
      (e) => toast(friendlyError(e), "error"),
    );

  const flip = (list: number[], d: number) => (list.includes(d) ? list.filter((x) => x !== d) : [...list, d].sort());

  return (
    <article class="tile tile-wide">
      <h2>{t("schedule.title")}</h2>
      <p class="muted small">{t("schedule.hint")}</p>
      <Show when={mine().length} fallback={<p class="muted small">{t("schedule.empty")}</p>}>
        <ul class="schedule-list">
          <For each={mine()}>
            {(s) => {
              // The device stopped listing it (or its status isn't known yet).
              const off = () => (offered(s.action) ? undefined : t("schedule.notOffered", { name: name() }));
              return (
                <li class="schedule-row" classList={{ "schedule-off": !s.enabled }}>
                  <div class="row">
                    <strong class="grow">{s.label || s.action}</strong>
                    <input
                      type="time"
                      class="schedule-time"
                      value={s.time}
                      aria-label={t("schedule.timeLabel")}
                      onChange={(e) => e.currentTarget.value && change(s, { time: e.currentTarget.value })}
                    />
                    <input
                      type="checkbox"
                      role="switch"
                      checked={s.enabled}
                      aria-label={t("schedule.enabledLabel", { what: s.label })}
                      onChange={(e) => change(s, { enabled: e.currentTarget.checked })}
                    />
                  </div>
                  <DayChips days={s.days} onToggle={(d) => change(s, { days: flip(s.days, d) })} />
                  <div class="row">
                    <span class="muted small grow">
                      {[
                        s.days.length ? "" : t("schedule.once"),
                        s.last_run ? t("schedule.lastRun", { when: s.last_run.replace("T", " ") }) : "",
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </span>
                    <Gate reason={off()}>
                      <button class="btn btn-ghost btn-sm" disabled={!!off()} onClick={() => runNow(s)}>
                        {t("schedule.runNow")}
                      </button>
                    </Gate>
                    <button class="btn btn-ghost btn-sm" onClick={() => remove(s)}>
                      {t("schedule.delete")}
                    </button>
                  </div>
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
      <form class="stack schedule-new" onSubmit={add}>
        <span class="small">{t("schedule.new")}</span>
        <div class="row">
          <select
            class="schedule-select grow"
            aria-label={t("schedule.actionLabel")}
            onChange={(e) => setPreset(e.currentTarget.value)}
          >
            <For each={presets()}>
              {(p) => (
                <option value={p.id} selected={p.id === chosen()?.id}>
                  {t(p.label)}
                </option>
              )}
            </For>
          </select>
          <input
            type="time"
            class="schedule-time"
            value={time()}
            aria-label={t("schedule.timeLabel")}
            onInput={(e) => setTime(e.currentTarget.value)}
          />
        </div>
        <DayChips days={days()} onToggle={(d) => setDays((l) => flip(l, d))} />
        <p class="muted small">{days().length ? t("schedule.repeatHint") : t("schedule.onceHint")}</p>
        <Show when={!actions()}>
          <p class="muted small">{t("schedule.moreWhenConnected", { name: name() })}</p>
        </Show>
        <Gate reason={time() ? undefined : t("schedule.pickTime")}>
          <button class="btn btn-soft btn-block" type="submit" disabled={busy() || !time() || !chosen()}>
            {t("schedule.add")}
          </button>
        </Gate>
      </form>
    </article>
  );
}

function DayChips(props: { days: number[]; onToggle: (day: number) => void }) {
  return (
    <div class="row schedule-days" role="group" aria-label={t("schedule.daysLabel")}>
      <For each={DAYS}>
        {([d, label]) => (
          <button
            type="button"
            class="schedule-day"
            classList={{ "schedule-day-on": props.days.includes(d) }}
            aria-pressed={props.days.includes(d)}
            onClick={() => props.onToggle(d)}
          >
            {t(label)}
          </button>
        )}
      </For>
    </div>
  );
}
