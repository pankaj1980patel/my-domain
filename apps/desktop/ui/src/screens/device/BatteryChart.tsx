// The last 24 hours of another device's battery, under its battery bar in
// the "This device" tile: a line over a soft area, charging stretches shaded,
// a tooltip with the time and level under the pointer, and a guess at how
// long it lasts (or takes to fill) from the last hour, when there's enough to
// go on. The samples are kept by the app (`battery_history`), at most one
// every 5 minutes, and fetched again whenever the device sends its status.

import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import { commands, type BatterySample } from "../../bindings";
import { state } from "../../lib/core";
import { t } from "../../i18n";

const DAY_S = 24 * 60 * 60;
/** Further apart than this, the line breaks (the device wasn't heard from). */
const GAP_S = 60 * 60;
/** The chart's drawing size; it stretches to the tile's width. */
const W = 300;
const H = 60;
const PAD = 3;

const nowS = () => Math.floor(Date.now() / 1000);

export default function BatteryChart(props: { node: string }) {
  const [samples, setSamples] = createSignal<BatterySample[]>([]);
  const [now, setNow] = createSignal(nowS());
  /** The pointer's place across the chart, 0-1 (none when it's elsewhere). */
  const [hover, setHover] = createSignal<number | null>(null);

  const load = async () => {
    try {
      const list = await commands.batteryHistory(props.node);
      setNow(nowS());
      setSamples(list);
    } catch {
      // Nothing to draw, then.
    }
  };
  // Again whenever a status arrives from it (the sample is in by then).
  createEffect(on(() => [props.node, state.status[props.node]?.ts], load));
  // "now" moves on even when it says nothing.
  const timer = setInterval(() => setNow(nowS()), 60_000);
  onCleanup(() => clearInterval(timer));

  const start = () => now() - DAY_S;
  // The last 24 h, plus the one before so the line comes in from the edge.
  const shown = createMemo(() => {
    const list = samples();
    const first = list.findIndex((s) => s.at >= start());
    if (first < 0) return [];
    return list.slice(Math.max(0, first - 1));
  });
  const x = (at: number) => ((at - start()) / DAY_S) * W;
  const y = (level: number) => PAD + (1 - level / 100) * (H - 2 * PAD);

  /** Runs of samples with no long silence between them. */
  const runs = createMemo(() => {
    const out: BatterySample[][] = [];
    for (const s of shown()) {
      const run = out[out.length - 1];
      if (run && s.at - run[run.length - 1].at <= GAP_S) run.push(s);
      else out.push([s]);
    }
    return out;
  });
  const line = (run: BatterySample[]) =>
    run.map((s, i) => `${i ? "L" : "M"}${x(s.at).toFixed(1)},${y(s.level).toFixed(1)}`).join(" ");
  const area = (run: BatterySample[]) =>
    `${line(run)} L${x(run[run.length - 1].at).toFixed(1)},${H} L${x(run[0].at).toFixed(1)},${H} Z`;
  /** Where it was charging: from a charging sample to the next one. */
  const charging = createMemo(() => {
    const out: { from: number; to: number }[] = [];
    for (const run of runs()) {
      for (let i = 0; i + 1 < run.length; i++) {
        if (!run[i].charging) continue;
        const last = out[out.length - 1];
        if (last && last.to === run[i].at) last.to = run[i + 1].at;
        else out.push({ from: run[i].at, to: run[i + 1].at });
      }
    }
    return out;
  });

  /** The sample nearest the pointer. */
  const picked = createMemo(() => {
    const h = hover();
    const list = shown().filter((s) => s.at >= start());
    if (h === null || !list.length) return null;
    const at = start() + h * DAY_S;
    return list.reduce((best, s) => (Math.abs(s.at - at) < Math.abs(best.at - at) ? s : best));
  });
  const move = (e: MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setHover(r.width ? Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) : null);
  };

  const guess = createMemo(() => estimateText(estimate(samples(), now())));

  /** The graph in words, for screen readers: where it started and is now,
   *  its lowest and highest, whether it's charging, and the guess. */
  const summary = createMemo(() => {
    const list = shown().filter((s) => s.at >= start());
    if (list.length < 2) return t("a11y.batteryNoTrend");
    const levels = list.map((s) => s.level);
    const last = list[list.length - 1];
    return [
      t("a11y.batterySummary", {
        from: list[0].level,
        to: last.level,
        low: Math.min(...levels),
        high: Math.max(...levels),
      }),
      last.charging ? t("a11y.batteryCharging") : "",
      guess() ?? "",
    ]
      .filter(Boolean)
      .join(" ");
  });

  return (
    <Show when={shown().length >= 2}>
      <div class="battery-chart">
        <div class="battery-chart-plot" onMouseMove={move} onMouseLeave={() => setHover(null)}>
          <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={summary()}>
            <For each={charging()}>
              {(c) => <rect class="battery-chart-charging" x={x(c.from)} y={0} width={x(c.to) - x(c.from)} height={H} />}
            </For>
            <For each={runs()}>
              {(run) => (
                <>
                  <path class="battery-chart-area" d={area(run)} />
                  <path class="battery-chart-line" d={line(run)} vector-effect="non-scaling-stroke" />
                </>
              )}
            </For>
            <Show when={picked()}>
              {(p) => (
                <line class="battery-chart-cursor" x1={x(p().at)} x2={x(p().at)} y1={0} y2={H} vector-effect="non-scaling-stroke" />
              )}
            </Show>
          </svg>
          <Show when={picked()}>
            {(p) => (
              <span
                class="battery-chart-tip small"
                aria-hidden="true"
                style={{ left: `${Math.min(85, Math.max(15, (x(p().at) / W) * 100))}%` }}
              >
                {t(p().charging ? "info.batteryTipCharging" : "info.batteryTip", {
                  time: new Date(p().at * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }),
                  level: p().level,
                })}
              </span>
            )}
          </Show>
        </div>
        <div class="battery-chart-axis small muted" aria-hidden="true">

          <span>{t("info.batteryDayAgo")}</span>
          <span>{t("info.batteryNow")}</span>
        </div>
        <Show when={guess()}>{(g) => <p class="small muted battery-chart-guess">{g()}</p>}</Show>
      </div>
    </Show>
  );
}

/** How long until it's empty (or full), from the slope of the last hour. */
type Estimate = { full: boolean; mins: number };

/** From the samples of the last hour since it last started or stopped
 *  charging: none when there are too few, they're over 30 minutes old, span
 *  under 20 minutes, or the level barely moves (under 1% an hour). */
function estimate(samples: BatterySample[], now: number): Estimate | null {
  const last = samples[samples.length - 1];
  if (!last || now - last.at > 30 * 60) return null;
  const recent: BatterySample[] = [];
  for (let i = samples.length - 1; i >= 0; i--) {
    const s = samples[i];
    if (s.at < last.at - 60 * 60 || s.charging !== last.charging) break;
    recent.unshift(s);
  }
  if (recent.length < 3 || last.at - recent[0].at < 20 * 60) return null;
  // Least squares: percent per hour.
  const n = recent.length;
  const mx = recent.reduce((a, s) => a + s.at, 0) / n;
  const my = recent.reduce((a, s) => a + s.level, 0) / n;
  let num = 0;
  let den = 0;
  for (const s of recent) {
    num += (s.at - mx) * (s.level - my);
    den += (s.at - mx) ** 2;
  }
  if (!den) return null;
  const perHour = (num / den) * 3600;
  let mins: number;
  if (last.charging) {
    if (perHour < 1 || last.level >= 100) return null;
    mins = ((100 - last.level) / perHour) * 60;
  } else {
    if (perHour > -1) return null;
    mins = (last.level / -perHour) * 60;
  }
  if (mins < 1 || mins > 48 * 60) return null;
  return { full: last.charging, mins };
}

/** "About 6 h left at this rate" / "Full in about 40 min". */
function estimateText(e: Estimate | null): string | null {
  if (!e) return null;
  let time: string;
  if (e.mins < 60) {
    time = t("info.batteryMin", { m: Math.min(55, Math.max(5, Math.round(e.mins / 5) * 5)) });
  } else if (e.mins < 3 * 60) {
    // To the nearest 10 minutes; 1 h 60 min is 2 h.
    const tens = Math.round(e.mins / 10);
    const h = Math.floor(tens / 6);
    const m = (tens % 6) * 10;
    time = m ? t("info.batteryHoursMin", { h, m }) : t("info.batteryHours", { h });
  } else {
    time = t("info.batteryHours", { h: Math.round(e.mins / 60) });
  }
  return t(e.full ? "info.batteryFullIn" : "info.batteryLeft", { time });
}
