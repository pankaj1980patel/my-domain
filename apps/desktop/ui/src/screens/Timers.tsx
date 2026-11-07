// Timers: countdown timers shared between your devices (`lib/timers.ts`).
// Start one here (a quick length or your own, a label, and which devices
// ring when it ends); every device of yours that has timers shows it
// counting down, and Pause, Resume, +1 min, Cancel and Stop work from any
// of them.

import { Icon } from "../components/Icon";
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { peerName, state } from "../lib/core";
import {
  addTime,
  clock,
  durationOf,
  hasEnded,
  PRESETS,
  progress,
  remaining,
  setRing,
  sortedTimers,
  startTimer,
  timerAction,
  timers,
  type Timer,
  type TimerAction,
} from "../lib/timers";
import { friendlyError, niceName } from "../lib/words";
import { Empty, toast } from "../components/ui";
import { HelpLink } from "../help/HelpLink";
import { t, tn } from "../i18n";

const MINUTE = 60_000;

/** Now, four times a second: the clocks tick with it. */
function useNow() {
  const [now, setNow] = createSignal(Date.now());
  const timer = window.setInterval(() => setNow(Date.now()), 250);
  onCleanup(() => clearInterval(timer));
  return now;
}

/** This computer and your devices that have timers (node ids). */
function useDevices() {
  const [others, setOthers] = createSignal<string[]>([]);
  onMount(() => commands.timerDevices().then(setOthers, () => {}));
  const me = () => state.me?.node_id ?? "";
  return createMemo(() => [me(), ...others().filter((n) => n !== me())].filter(Boolean));
}

/** A device's name here: "This computer" for this one. */
export function deviceName(node: string): string {
  return node === state.me?.node_id ? t("timers.thisComputer") : peerName(node);
}

/** Tick boxes for the devices that ring. */
function RingPicker(props: { devices: string[]; value: string[]; onChange: (v: string[]) => void; label: string }) {
  const toggle = (node: string, on: boolean) =>
    props.onChange(on ? [...props.value.filter((n) => n !== node), node] : props.value.filter((n) => n !== node));
  return (
    <fieldset class="timer-ring">
      <legend class="small muted">{props.label}</legend>
      <For each={props.devices}>
        {(node) => (
          <label class="timer-ring-device">
            <input type="checkbox" checked={props.value.includes(node)} onChange={(e) => toggle(node, e.currentTarget.checked)} />
            {deviceName(node)}
          </label>
        )}
      </For>
    </fieldset>
  );
}

function NewTimer(props: { devices: string[] }) {
  const [label, setLabel] = createSignal("");
  const [h, setH] = createSignal("");
  const [m, setM] = createSignal("5");
  const [s, setS] = createSignal("");
  // Rings here unless you choose otherwise.
  const [ring, setRingOn] = createSignal<string[] | null>(null);
  const ringOn = () => ring() ?? (state.me?.node_id ? [state.me.node_id] : []);
  const [busy, setBusy] = createSignal(false);

  const start = async (ms: number | null) => {
    if (ms == null) return toast(t("timers.badLength"), "error");
    setBusy(true);
    try {
      await startTimer(label(), ms, ringOn());
      setLabel("");
      if (ringOn().length === 0) toast(t("timers.ringsNowhere"));
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const custom = () => durationOf(h(), m(), s());

  return (
    <section class="tile timer-new" aria-labelledby="timer-new-title">
      <h2 id="timer-new-title">{t("timers.new")}</h2>
      <label class="field">
        <span>{t("timers.label")}</span>
        <input
          type="text"
          maxLength={60}
          placeholder={t("timers.labelHint")}
          value={label()}
          onInput={(e) => setLabel(e.currentTarget.value)}
        />
      </label>
      <div class="timer-presets" role="group" aria-label={t("timers.quick")}>
        <For each={PRESETS}>
          {(mins) => (
            <button class="btn btn-soft btn-sm" disabled={busy()} onClick={() => void start(mins * MINUTE)}>
              {mins >= 60 ? t("timers.hoursShort", { n: mins / 60 }) : t("timers.minsShort", { n: mins })}
            </button>
          )}
        </For>
      </div>
      <form
        class="timer-custom"
        onSubmit={(e) => {
          e.preventDefault();
          void start(custom());
        }}
      >
        <label class="field">
          <span>{t("timers.hours")}</span>
          <input type="number" min="0" max="168" inputMode="numeric" value={h()} onInput={(e) => setH(e.currentTarget.value)} />
        </label>
        <label class="field">
          <span>{t("timers.minutes")}</span>
          <input type="number" min="0" max="59" inputMode="numeric" value={m()} onInput={(e) => setM(e.currentTarget.value)} />
        </label>
        <label class="field">
          <span>{t("timers.seconds")}</span>
          <input type="number" min="0" max="59" inputMode="numeric" value={s()} onInput={(e) => setS(e.currentTarget.value)} />
        </label>
        <button class="btn btn-primary" type="submit" disabled={busy() || custom() == null}>
          {t("timers.start")}
        </button>
      </form>
      <RingPicker devices={props.devices} value={ringOn()} onChange={setRingOn} label={t("timers.ringOn")} />
    </section>
  );
}

function TimerCard(props: { timer: Timer; now: number; devices: string[] }) {
  const x = () => props.timer;
  const ended = () => hasEnded(x(), props.now);
  const [editRing, setEditRing] = createSignal(false);
  const act = (a: TimerAction) => () => timerAction(x().id, a).catch((e) => toast(friendlyError(e), "error"));
  const more = () => addTime(x().id, MINUTE).catch((e) => toast(friendlyError(e), "error"));
  const status = () => {
    if (ended()) return t("timers.ended");
    if (x().state === "paused") return t("timers.paused");
    const at = new Date(x().ends_at ?? 0).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    return t("timers.endsAt", { time: at });
  };
  const ringNames = () =>
    x().ring_on.length ? x().ring_on.map(deviceName).join(", ") : t("timers.nowhere");
  // Devices it rings on that aren't in the list (not linked any more) stay.
  const choices = () => [...props.devices, ...x().ring_on.filter((n) => !props.devices.includes(n))];

  return (
    <li class={`tile timer-card ${ended() ? "timer-ended" : ""} ${x().state === "paused" ? "timer-paused" : ""}`}>
      <div class="tile-head">
        <strong class="timer-label">{x().label || t("timers.untitled")}</strong>
        <span class="small muted">{status()}</span>
      </div>
      <div class="timer-clock" role="timer" aria-live="off" aria-label={t("timers.left", { time: clock(remaining(x(), props.now)) })}>
        {ended() ? "0:00" : clock(remaining(x(), props.now))}
      </div>
      <div class="timer-bar" aria-hidden="true">
        <span style={{ width: `${Math.round(progress(x(), props.now) * 1000) / 10}%` }} />
      </div>
      <p class="small muted">
        {t("timers.ringsOn", { names: ringNames() })}{" "}
        <button class="btn btn-ghost btn-sm" aria-expanded={editRing()} onClick={() => setEditRing(!editRing())}>
          {t("timers.change")}
        </button>
      </p>
      <Show when={editRing()}>
        <RingPicker
          devices={choices()}
          value={x().ring_on}
          label={t("timers.ringOn")}
          onChange={(v) => setRing(x().id, v).catch((e) => toast(friendlyError(e), "error"))}
        />
      </Show>
      <Show when={x().by_name}>
        <p class="small muted">{t("timers.by", { name: niceName(x().by_name) })}</p>
      </Show>
      <div class="timer-actions">
        <Show
          when={ended()}
          fallback={
            <Show
              when={x().state === "paused"}
              fallback={
                <button class="btn btn-soft btn-sm" onClick={act("pause")}>
                  {t("timers.pause")}
                </button>
              }
            >
              <button class="btn btn-primary btn-sm" onClick={act("resume")}>
                {t("timers.resume")}
              </button>
            </Show>
          }
        >
          <button class="btn btn-primary btn-sm" onClick={act("dismiss")}>
            {t("timers.stop")}
          </button>
          <button class="btn btn-soft btn-sm" onClick={act("restart")}>
            {t("timers.restart")}
          </button>
        </Show>
        <button class="btn btn-soft btn-sm" onClick={more}>
          {t("timers.plusMinute")}
        </button>
        <Show when={!ended()}>
          <button class="btn btn-ghost btn-sm" onClick={act("cancel")}>
            {t("timers.cancel")}
          </button>
        </Show>
      </div>
    </li>
  );
}

export default function Timers() {
  const now = useNow();
  const devices = useDevices();
  const list = createMemo(() => sortedTimers());
  const running = () => list().filter((x) => x.state === "running" && !hasEnded(x, now())).length;

  return (
    <section class="page timers-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="timers" size={26} tone="amber" />
        </span>
        <div class="grow">
          <h1>
            {t("timers.title")} <HelpLink topic="timers" />
          </h1>
          <p class="muted">{t("timers.subtitle")}</p>
        </div>
        <Show when={running() > 0}>
          <span class="badge">{tn("timers.running", running())}</span>
        </Show>
      </header>
      <NewTimer devices={devices()} />
      <Show
        when={list().length > 0}
        fallback={
          <Show when={timers.loaded}>
            <Empty icon="timers" title={t("timers.emptyTitle")}>
              {t("timers.emptyBody")}
            </Empty>
          </Show>
        }
      >
        <ul class="timer-list" aria-label={t("timers.title")}>
          <For each={list()}>{(x) => <TimerCard timer={x} now={now()} devices={devices()} />}</For>
        </ul>
      </Show>
    </section>
  );
}
