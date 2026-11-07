// "Recent activity": the newest few rows of the timeline (one device's, or
// all), each with a round icon in its kind's colour, and "View all".

import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type ActivityItem_Serialize as ActivityItem } from "../bindings";
import { peerName } from "../lib/core";
import { activityIcon, activityText } from "../lib/activity";
import { timeAgo } from "../lib/words";
import { Icon, IconChip, type Tone } from "./Icon";
import { t } from "../i18n";

/** Activity kinds by colour: files blue, messages violet, calls green... */
export const KIND_TONE: Record<string, Tone> = {
  files: "blue",
  clipboard: "cyan",
  messages: "violet",
  notifications: "violet",
  calls: "green",
  connections: "cyan",
  other: "amber",
};

export function RecentActivity(props: { node?: string; limit?: number; onViewAll: () => void }) {
  const [items, setItems] = createSignal<ActivityItem[] | null>(null);
  const limit = () => props.limit ?? 4;
  const load = () =>
    commands
      .activityFeed({ node: props.node ?? null, kinds: null, text: null, before_ts: null, before_id: null, limit: limit() })
      .then((p) => setItems(p.items.slice(0, limit())), () => setItems([]));
  createEffect(on(() => props.node, load));
  onMount(() => {
    const off = listen("activity-changed", load);
    onCleanup(() => off.then((f) => f()).catch(() => {}));
  });
  return (
    <section class="tile dash-card" aria-labelledby="recent-activity">
      <div class="dash-head">
        <Icon name="activity" tone="violet" />
        <h2 id="recent-activity">{t("dash.recent")}</h2>
        <button class="link small dash-more" onClick={props.onViewAll}>
          {t("dash.viewAll")}
          <Icon name="arrow-right" size={14} />
        </button>
      </div>
      <Show
        when={items()?.length}
        fallback={
          <p class="muted small">
            {items() ? (props.node ? t("dash.noActivity", { name: peerName(props.node) }) : t("activity.emptyTitle")) : "…"}
          </p>
        }
      >
        <ul class="dash-feed">
          <For each={items()}>
            {(a) => (
              <li>
                <IconChip name={activityIcon(a)} tone={KIND_TONE[a.kind] ?? "cyan"} />
                <span class="dash-feed-text">{activityText(a, a.node ? peerName(a.node) : "")}</span>
                <span class="muted small dash-feed-time">{timeAgo(a.ts ?? 0)}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
