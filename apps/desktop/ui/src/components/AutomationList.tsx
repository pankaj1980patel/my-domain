// Automations: "when this happens, do that". The list (what starts it, what
// it does, on/off, delete) and the form that adds one. They live and run in
// Rust (`automations.rs`). What they can do is a schedule's presets
// (`lib/presets.ts`), on this computer or another device, or just a
// notification.

import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { commands, type Automation_Serialize as Automation } from "../bindings";
import { state, unwrap } from "../lib/core";
import { DEVICE_PRESETS, LOCAL_PRESETS, NOTIFY_PRESET, offeredPresets, type Preset } from "../lib/presets";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";

/** What can start one, as `Trigger.kind`, with how the form says it. */
const KINDS: [string, Key][] = [
  ["connects", "automation.kind.connects"],
  ["disconnects", "automation.kind.disconnects"],
  ["battery_below", "automation.kind.batteryBelow"],
  ["charging_starts", "automation.kind.chargingStarts"],
  ["charging_stops", "automation.kind.chargingStops"],
];

/** How the list says what starts one. */
const WHEN: Record<string, Key> = {
  connects: "automation.when.connects",
  disconnects: "automation.when.disconnects",
  battery_below: "automation.when.batteryBelow",
  charging_starts: "automation.when.chargingStarts",
  charging_stops: "automation.when.chargingStops",
  wakes: "automation.when.wakes",
};

/** How often the list is read again (when each one last fired). */
const REFRESH_MS = 60_000;

export function AutomationList(props: {
  /** This computer's node id. */
  me: string;
  /** What this computer can do on itself. */
  localActions: string[];
}) {
  const [list, setList] = createSignal<Automation[]>([]);
  const peers = () => state.peers.filter((p) => p.node_id !== props.me);
  const [watch, setWatch] = createSignal("");
  const [kind, setKind] = createSignal("connects");
  const [level, setLevel] = createSignal(20);
  const [target, setTarget] = createSignal("");
  const [preset, setPreset] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  // The device watched; this computer only wakes.
  const watched = () => watch() || peers()[0]?.node_id || props.me;
  const kinds = (): [string, Key][] => (watched() === props.me ? [["wakes", "automation.kind.wakes"]] : KINDS);
  const chosenKind = () => kinds().find(([k]) => k === kind())?.[0] ?? kinds()[0][0];
  const where = () => target() || props.me;
  const presets = (): Preset[] =>
    where() === props.me
      ? [...offeredPresets(LOCAL_PRESETS, props.localActions), NOTIFY_PRESET]
      : offeredPresets(DEVICE_PRESETS, state.status[where()]?.actions);
  const chosen = () => presets().find((p) => p.id === preset()) ?? presets()[0];

  const name = (node: string) =>
    !node || node === props.me
      ? t("automation.thisComputer")
      : niceName(state.peers.find((p) => p.node_id === node)?.name ?? t("activity.someDevice"));

  const load = () => commands.listAutomations().then(setList, () => {});
  onMount(() => {
    load();
    const timer = setInterval(load, REFRESH_MS);
    onCleanup(() => clearInterval(timer));
  });

  const change = (a: Automation, patch: Partial<Automation>) =>
    unwrap(commands.updateAutomation({ ...a, ...patch })).then(
      (n) => setList((l) => l.map((x) => (x.id === n.id ? n : x))),
      (e) => toast(friendlyError(e), "error"),
    );

  const remove = (a: Automation) =>
    commands.deleteAutomation(a.id).then(() => setList((l) => l.filter((x) => x.id !== a.id)));

  const add = async (e: Event) => {
    e.preventDefault();
    const p = chosen();
    if (!p) return;
    const k = chosenKind();
    setBusy(true);
    try {
      const a = await unwrap(
        commands.addAutomation({
          trigger: { kind: k, node: k === "wakes" ? "" : watched(), level: k === "battery_below" ? level() : null },
          node: where(),
          action: p.action,
          args: JSON.stringify(p.args),
          label: t(p.label),
          enabled: true,
        }),
      );
      setList((l) => [...l, a]);
      toast(t("automation.added"), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  const when = (a: Automation) =>
    t(WHEN[a.trigger.kind] ?? "automation.when.connects", { name: name(a.trigger.node), level: a.trigger.level ?? 0 });
  const then = (a: Automation) =>
    a.action === NOTIFY_PRESET.action
      ? t("automation.then.notify")
      : // This computer's presets already say so ("Lock this computer").
        a.node === props.me
        ? a.label || a.action
        : t("automation.then", { what: a.label || a.action, name: name(a.node) });

  return (
    <>
      <Show when={list().length} fallback={<p class="muted small">{t("automation.empty")}</p>}>
        <ul class="schedule-list">
          <For each={list()}>
            {(a) => (
              <li class="schedule-row" classList={{ "schedule-off": !a.enabled }}>
                <div class="row">
                  <span class="grow">
                    <strong>{when(a)}</strong>
                    <span class="automation-arrow" aria-hidden="true">
                      {" → "}
                    </span>
                    <span>{then(a)}</span>
                  </span>
                  <input
                    type="checkbox"
                    role="switch"
                    checked={a.enabled}
                    aria-label={t("automation.enabledLabel", { what: when(a) })}
                    onChange={(e) => change(a, { enabled: e.currentTarget.checked })}
                  />
                </div>
                <div class="row">
                  <span class="muted small grow">
                    {a.last_fired ? t("automation.lastFired", { when: a.last_fired }) : t("automation.neverFired")}
                  </span>
                  <button class="btn btn-ghost btn-sm" onClick={() => remove(a)}>
                    {t("schedule.delete")}
                  </button>
                </div>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <form class="stack schedule-new" onSubmit={add}>
        <span class="small">{t("automation.new")}</span>
        <div class="row automation-line">
          <span class="muted small">{t("automation.whenLabel")}</span>
          <select
            class="schedule-select"
            aria-label={t("automation.watchLabel")}
            onChange={(e) => setWatch(e.currentTarget.value)}
          >
            <For each={peers()}>
              {(p) => (
                <option value={p.node_id} selected={p.node_id === watched()}>
                  {niceName(p.name)}
                </option>
              )}
            </For>
            <option value={props.me} selected={watched() === props.me}>
              {t("automation.thisComputerOption")}
            </option>
          </select>
          <select
            class="schedule-select grow"
            aria-label={t("automation.kindLabel")}
            onChange={(e) => setKind(e.currentTarget.value)}
          >
            <For each={kinds()}>
              {([k, label]) => (
                <option value={k} selected={k === chosenKind()}>
                  {t(label)}
                </option>
              )}
            </For>
          </select>
          <Show when={chosenKind() === "battery_below"}>
            <input
              type="number"
              class="automation-level"
              min="1"
              max="99"
              value={level()}
              aria-label={t("automation.levelLabel")}
              onInput={(e) => setLevel(Math.round(Number(e.currentTarget.value)))}
            />
            <span class="small">%</span>
          </Show>
        </div>
        <div class="row automation-line">
          <span class="muted small">{t("automation.doLabel")}</span>
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
          <span class="muted small">{t("automation.onLabel")}</span>
          <select
            class="schedule-select"
            aria-label={t("automation.targetLabel")}
            onChange={(e) => setTarget(e.currentTarget.value)}
          >
            <option value={props.me} selected={where() === props.me}>
              {t("automation.thisComputerOption")}
            </option>
            <For each={peers()}>
              {(p) => (
                <option value={p.node_id} selected={p.node_id === where()}>
                  {niceName(p.name)}
                </option>
              )}
            </For>
          </select>
        </div>
        <Show when={where() !== props.me && !state.status[where()]?.actions}>
          <p class="muted small">{t("automation.moreWhenConnected", { name: name(where()) })}</p>
        </Show>
        <button
          class="btn btn-soft btn-block"
          type="submit"
          disabled={busy() || !chosen() || (chosenKind() === "battery_below" && !(level() >= 1 && level() <= 99))}
        >
          {t("automation.add")}
        </button>
      </form>
    </>
  );
}
