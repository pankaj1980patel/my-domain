// Activity: one timeline of what happened between your devices, grouped by
// day (activity.rs merges the histories this computer keeps). Filters for a
// device, a kind and words; "Load more" goes further back. Kinds can be
// hidden; "Clear activity" empties only the timeline's own record (the
// histories stay, each with its own Clear). A row opens where it lives.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type ActivityItem_Serialize as ActivityItem } from "../bindings";
import { peerIcon, peerName, state } from "../lib/core";
import { DeviceIcon } from "../components/DeviceIcon";
import {
  ACTIVITY_KINDS,
  ACTIVITY_KIND_LABEL,
  activityFailed,
  activityIcon,
  activityMeta,
  activityTarget,
  activityText,
  byDay,
  clock,
  type ActivityKind,
  type ActivityTarget,
} from "../lib/activity";
import { Empty, toast } from "../components/ui";
import { askConfirm } from "../components/Confirm";
import { MenuButton } from "../components/Menu";
import { Icon, IconChip } from "../components/Icon";
import { KIND_TONE } from "../components/RecentActivity";
import { t } from "../i18n";
import { dateAndTime } from "../lib/words";

const PAGE = 50;
const SEARCH_DELAY = 200;
/** Fresh rows on top while the screen is open. */
const REFRESH_MS = 15_000;

// Kept while the app runs, so coming back from a row finds them as left.
const [text, setText] = createSignal("");
const [kind, setKind] = createSignal<ActivityKind | "">("");
const [node, setNode] = createSignal("");

export default function Activity(props: { onOpen: (target: Exclude<ActivityTarget, null>) => void }) {
  const [items, setItems] = createSignal<ActivityItem[]>([]);
  const [more, setMore] = createSignal(false);
  const [hidden, setHidden] = createSignal<string[]>([]);
  const [clearedAt, setClearedAt] = createSignal<number | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [choosing, setChoosing] = createSignal(false);
  let generation = 0;

  const query = (extra: { before?: ActivityItem; limit?: number }) => ({
    node: node() || null,
    kinds: kind() ? [kind()] : null,
    text: text().trim() || null,
    before_ts: extra.before?.ts ?? null,
    before_id: extra.before?.id ?? null,
    limit: extra.limit ?? PAGE,
  });

  /** The first page again (as many rows as are shown, so a refresh keeps the place). */
  const reload = async (keep = false) => {
    const mine = ++generation;
    setLoading(true);
    const limit = keep ? Math.min(200, Math.max(PAGE, items().length)) : PAGE;
    try {
      const page = await commands.activityFeed(query({ limit }));
      if (mine !== generation) return;
      setItems(page.items);
      setMore(page.more);
      setHidden(page.hidden);
      setClearedAt(page.cleared_at);
    } finally {
      if (mine === generation) setLoading(false);
    }
  };

  const loadMore = async () => {
    const last = items()[items().length - 1];
    if (!last) return;
    const mine = generation;
    setLoading(true);
    try {
      const page = await commands.activityFeed(query({ before: last }));
      if (mine !== generation) return;
      setItems([...items(), ...page.items]);
      setMore(page.more);
    } finally {
      if (mine === generation) setLoading(false);
    }
  };

  // Filters changed: from the top (words after a pause in typing).
  let timer: number | undefined;
  createEffect(
    on([text, kind, node], () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void reload(), SEARCH_DELAY);
    }),
  );
  onMount(() => {
    const off = listen("activity-changed", () => void reload(true));
    const tick = window.setInterval(() => void reload(true), REFRESH_MS);
    onCleanup(() => {
      window.clearInterval(tick);
      window.clearTimeout(timer);
      off.then((f) => f());
    });
  });

  const toggleKind = async (k: ActivityKind, show: boolean) => {
    const next = show ? hidden().filter((h) => h !== k) : [...hidden(), k];
    setHidden(await commands.setActivityHidden(next));
    if (kind() && next.includes(kind())) setKind("");
    void reload();
  };

  const clear = async () => {
    
    await commands.clearActivity();
    toast(t("activity.clearedToast"), "ok");
    void reload();
  };
  const restore = async () => {
    await commands.restoreActivity();
    void reload();
  };

  const open = (a: ActivityItem) => {
    const target = activityTarget(a);
    if (target) props.onOpen(target);
  };
  const reveal = (a: ActivityItem) => {
    commands.transferReveal(a.target).then(
      (r) => r.status === "error" && toast(t("activity.revealFailed"), "error"),
      () => toast(t("activity.revealFailed"), "error"),
    );
  };
  const filtered = () => !!(text().trim() || kind() || node());
  const deviceOf = (a: ActivityItem) => (a.node ? peerName(a.node) : t("activity.someDevice"));

  return (
    <section class="page page-fit activity-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="activity" size={26} tone="violet" />
        </span>
        <div class="grow">
          <h1>{t("activity.title")}</h1>
          <p class="muted">{t("activity.subtitle")}</p>
        </div>
        {/* Choose what shows; Clear (destructive) apart, in the "⋯", asked first. */}
        <div class="activity-actions">
          <button class="btn btn-soft btn-sm" aria-expanded={choosing()} onClick={() => setChoosing(!choosing())}>
            <Icon name="control" size={16} />
            {t("activity.choose")}
          </button>
          <MenuButton
            label={t("activity.more")}
            items={[
              {
                icon: "remove",
                label: t("activity.clear"),
                danger: true,
                onSelect: async () => {
                  if (await askConfirm({ title: t("activity.clearTitle"), body: t("activity.clearBody"), action: t("activity.clear") })) clear();
                },
              },
            ]}
          >
            <Icon name="more" />
          </MenuButton>
        </div>
      </header>

      <Show when={choosing()}>
        <fieldset class="tile activity-kinds">
          <legend class="small">{t("activity.chooseHint")}</legend>
          <For each={[...ACTIVITY_KINDS]}>
            {(k) => (
              <label class="activity-kind-toggle">
                <input type="checkbox" checked={!hidden().includes(k)} onChange={(e) => void toggleKind(k, e.currentTarget.checked)} />
                {t(ACTIVITY_KIND_LABEL[k])}
              </label>
            )}
          </For>
        </fieldset>
      </Show>

      {/* Row 1: search and the device; row 2: what kind. */}
      <div class="activity-toolbar" role="group" aria-label={t("activity.filters")}>
        <input
          type="search"
          class="search-input"
          aria-label={t("activity.searchLabel")}
          placeholder={t("activity.searchPlaceholder")}
          autocomplete="off"
          spellcheck={false}
          value={text()}
          onInput={(e) => setText(e.currentTarget.value)}
        />
        <label class="select-with-icon">
          <Icon name="devices" size={16} />
          <select aria-label={t("activity.filterDevice")} value={node()} onChange={(e) => setNode(e.currentTarget.value)}>
            <option value="">{t("activity.anyDevice")}</option>
            <For each={state.peers}>{(p) => <option value={p.node_id}>{peerName(p.node_id)}</option>}</For>
          </select>
        </label>
        <Show when={filtered()}>
          <button
            class="btn btn-ghost btn-sm search-reset"
            onClick={() => {
              setText("");
              setKind("");
              setNode("");
            }}
          >
            {t("activity.clearFilters")}
          </button>
        </Show>
      </div>
      <div class="activity-chips">
        {/* What kind: one chip each (the kinds hidden from the timeline aren't offered). */}
        <div class="home-filters" role="group" aria-label={t("activity.filterKind")}>
          <button class="filter-chip" aria-pressed={kind() === ""} onClick={() => setKind("")}>
            {t("activity.anyKind")}
          </button>
          <For each={ACTIVITY_KINDS.filter((k) => !hidden().includes(k))}>
            {(k) => (
              <button class="filter-chip" aria-pressed={kind() === k} onClick={() => setKind(k)}>
                {t(ACTIVITY_KIND_LABEL[k])}
              </button>
            )}
          </For>
        </div>
      </div>

      <Show when={clearedAt()}>
        {(at) => (
          <p class="small muted activity-cleared">
            {t("activity.clearedSince", { time: dateAndTime(at()) })}{" "}
            <button class="btn btn-ghost btn-sm" onClick={restore}>
              {t("activity.showEarlier")}
            </button>
          </p>
        )}
      </Show>

      <p class="sr-only" role="status">
        {loading() ? "" : items().length === 0 && filtered() ? t("activity.noMatch") : ""}
      </p>
      {/* The list scrolls in its own panel; the page stays put. */}
      <div class="page-scroll">
      <Show
        when={items().length > 0}
        fallback={
          <Show when={!loading()}>
            <Show when={filtered()} fallback={<Empty icon="activity" title={t("activity.emptyTitle")}>{t("activity.emptyBody")}</Empty>}>
              <p class="muted search-hint">{t("activity.noMatch")}</p>
            </Show>
          </Show>
        }
      >
        <For each={byDay(items())}>
          {(group) => (
            <section class="activity-day" aria-label={group.day}>
              <h2 class="activity-day-head small muted">{group.day}</h2>
              <ul class="activity">
                <For each={group.items}>
                  {(a) => {
                    const meta = activityMeta(a);
                    const target = activityTarget(a);
                    return (
                      <li class="activity-item activity-row" classList={{ "tone-error": activityFailed(a) }}>
                        <IconChip name={activityIcon(a)} tone={KIND_TONE[a.kind] ?? "cyan"} />
                        <button class="activity-main" disabled={!target} onClick={() => open(a)}>
                          <span class="activity-text">{activityText(a, deviceOf(a))}</span>
                          <span class="activity-meta small muted">
                            <Show when={a.node}>{(n) => <DeviceIcon icon={peerIcon(n())} size={14} />}</Show>
                            {[a.node ? peerName(a.node) : "", ...meta].filter(Boolean).join(" · ")}
                          </span>
                        </button>
                        <Show when={a.kind === "files" && a.path && (a.action === "file_received" || a.action === "file_sent")}>
                          <button class="btn btn-ghost btn-sm" onClick={() => reveal(a)}>
                            {t("activity.reveal")}
                          </button>
                        </Show>
                        <time class="muted small" dateTime={new Date(a.ts ?? 0).toISOString()} title={new Date(a.ts ?? 0).toLocaleString()}>
                          {clock(a.ts ?? 0)}
                        </time>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </section>
          )}
        </For>
        <Show when={more()}>
          <div class="activity-more">
            <button class="btn btn-ghost" disabled={loading()} onClick={loadMore}>
              {t("activity.loadMore")}
            </button>
          </div>
        </Show>
      </Show>
      </div>
    </section>
  );
}
