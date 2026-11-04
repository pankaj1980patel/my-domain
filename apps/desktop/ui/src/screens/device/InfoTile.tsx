// What another device last said about itself: battery, storage, memory,
// processor and how long it has been on. Each row is shown only when its
// field is in the status (phones send no processor use; older versions
// send no uptime), and the tile only when there's something to show.

import { For, Show } from "solid-js";
import { peerName, state, type DeviceStatus, type Space } from "../../lib/core";
import { humanSize, niceName } from "../../lib/words";
import { t, tn } from "../../i18n";

/** One line: a label, an optional bar (percent full) and the value. */
type Row = { label: string; value: string; pct?: number };

export default function InfoTile(props: { node: string }) {
  const rows = () => rowsOf(state.status[props.node]);
  return (
    <Show when={rows().length}>
      <article class="tile">
        <h2>{t("info.title")}</h2>
        <p class="muted small">{t("info.hint", { name: niceName(peerName(props.node)) })}</p>
        <dl class="info-rows">
          <For each={rows()}>
            {(r) => (
              <div class="info-row">
                <dt class="small muted">{r.label}</dt>
                <dd class="small">{r.value}</dd>
                <Show when={r.pct !== undefined}>
                  <div class="progress" role="presentation">
                    <span style={{ width: `${r.pct}%` }} />
                  </div>
                </Show>
              </div>
            )}
          </For>
        </dl>
      </article>
    </Show>
  );
}

function rowsOf(st: DeviceStatus | undefined): Row[] {
  if (!st) return [];
  const rows: Row[] = [];
  if (st.battery !== undefined) {
    const level = st.battery;
    rows.push({
      label: t("info.battery"),
      value: st.charging ? t("info.batteryCharging", { level }) : t("info.batteryLevel", { level }),
      pct: level,
    });
  }
  // Disk and memory bars show how full they are.
  const space = (label: string, s: Space | undefined) => {
    if (!s) return;
    rows.push({
      label,
      value: t("info.free", { free: humanSize(s.free), total: humanSize(s.total) }),
      pct: Math.round(((s.total - s.free) / s.total) * 100),
    });
  };
  space(t("info.storage"), st.storage);
  space(t("info.memory"), st.ram);
  if (st.cpu !== undefined) rows.push({ label: t("info.cpu"), value: t("info.cpuBusy", { pct: st.cpu }), pct: st.cpu });
  if (st.uptimeS !== undefined) {
    // It answered at `ts`: count the time since then too.
    const secs = st.uptimeS + Math.max(0, Math.round((Date.now() - st.ts) / 1000));
    rows.push({ label: t("info.uptime"), value: uptimeText(secs) });
  }
  return rows;
}

/** 300 -> "5 minutes", 12000 -> "3 h 20 min", 190000 -> "2 days, 4 h". */
function uptimeText(secs: number): string {
  const mins = Math.floor(secs / 60);
  const hours = Math.floor(mins / 60);
  const days = Math.floor(hours / 24);
  if (days > 0) return tn("info.upDays", days, { h: hours % 24 });
  if (hours > 0) return t("info.upHours", { h: hours, m: mins % 60 });
  return tn("info.upMinutes", mins);
}
