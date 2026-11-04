// Notification history for one device: what it shared, kept on this computer
// (the last 500, notif_history.rs), searchable and grouped by day, with a
// chip per app. "Mute" on a row stops that app's notifications showing or
// being kept here; muted apps are listed at the bottom to unmute. Replying
// and buttons stay in the Notifications tile, for the ones still on the phone.

import { createEffect, createMemo, createSignal, For, on, onMount, Show } from "solid-js";
import {
  clearNotificationHistory,
  historyTick,
  mutedApps,
  notificationHistory,
  peerName,
  setMutedApps,
  type HistoryEntry,
  type HistoryPage,
} from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";

/** Rows shown at first, and added by "Show more". */
const PAGE = 100;

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** "Today", "Yesterday", or the date. */
function dayLabel(ts: number, now = new Date()): string {
  const day = startOfDay(new Date(ts));
  const today = startOfDay(now);
  if (day === today) return t("history.today");
  if (day === startOfDay(new Date(today - 12 * 3600 * 1000))) return t("history.yesterday");
  const d = new Date(ts);
  return d.toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    year: d.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
}

const timeOf = (ts: number) => new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

export default function NotificationHistory(props: { node: string }) {
  const name = () => peerName(props.node);
  const [query, setQuery] = createSignal("");
  const [app, setApp] = createSignal<string | null>(null);
  const [limit, setLimit] = createSignal(PAGE);
  const [page, setPage] = createSignal<HistoryPage>({ entries: [], total: 0, kept: 0, apps: [] });
  const [muted, setMuted] = createSignal<string[]>([]);
  let asked = 0;

  onMount(() => mutedApps().then(setMuted).catch(() => {}));
  // A new search or app starts from the top again.
  createEffect(on([query, app, () => props.node], () => setLimit(PAGE), { defer: true }));
  createEffect(
    on([() => props.node, query, app, limit, historyTick], ([node, q, a, l]) => {
      const mine = ++asked;
      notificationHistory(node, q, a, l)
        .then((p) => {
          if (mine !== asked) return;
          setPage(p);
          // The chosen app is gone (cleared, muted): show them all.
          if (a && !p.apps.includes(a)) setApp(null);
        })
        .catch(() => {});
    }),
  );

  /** The shown rows, by day, newest first. */
  const days = createMemo(() => {
    const out: { label: string; rows: HistoryEntry[] }[] = [];
    for (const e of page().entries) {
      const label = dayLabel(e.ts ?? 0);
      const last = out[out.length - 1];
      if (last && last.label === label) last.rows.push(e);
      else out.push({ label, rows: [e] });
    }
    return out;
  });

  const changeMuted = async (apps: string[], done: string) => {
    try {
      setMuted(await setMutedApps(apps));
      toast(done, "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const mute = (a: string) => changeMuted([...muted(), a], t("history.muted", { app: a }));
  const unmute = (a: string) =>
    changeMuted(
      muted().filter((m) => m !== a),
      t("history.unmuted", { app: a }),
    );
  const clear = () =>
    clearNotificationHistory(props.node)
      .then(() => toast(t("history.cleared"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <article class="tile tile-wide">
      <div class="tile-head">
        <h2>{t("history.title")}</h2>
        <Show when={page().kept > 0}>
          <button class="btn btn-ghost btn-sm" title={t("history.clearHint", { name: name() })} onClick={clear}>
            {t("history.clear")}
          </button>
        </Show>
      </div>
      <Show
        when={page().kept > 0}
        fallback={<p class="muted small">{t("history.empty", { name: name() })}</p>}
      >
        <input
          type="search"
          class="history-search"
          placeholder={t("history.search")}
          aria-label={t("history.searchLabel")}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <Show when={page().apps.length > 1}>
          <div class="history-chips" role="group" aria-label={t("history.appsLabel")}>
            <button
              class="btn btn-sm"
              classList={{ "btn-soft": app() === null, "btn-ghost": app() !== null }}
              aria-pressed={app() === null}
              onClick={() => setApp(null)}
            >
              {t("history.allApps")}
            </button>
            <For each={page().apps}>
              {(a) => (
                <button
                  class="btn btn-sm"
                  classList={{ "btn-soft": app() === a, "btn-ghost": app() !== a }}
                  aria-pressed={app() === a}
                  onClick={() => setApp(app() === a ? null : a)}
                >
                  {a}
                </button>
              )}
            </For>
          </div>
        </Show>
        <Show when={page().total > 0} fallback={<p class="muted small">{t("history.noMatch")}</p>}>
          <div class="history-list">
            <For each={days()}>
              {(d) => (
                <section class="history-day">
                  <h3 class="history-day-label small muted">{d.label}</h3>
                  <ul class="notif-list history-rows">
                    <For each={d.rows}>
                      {(e) => (
                        <li class="notif" classList={{ "history-dismissed": e.dismissed }}>
                          <div class="row">
                            <span class="small muted grow notif-app">{e.app || name()}</span>
                            <Show when={e.dismissed}>
                              <span class="small muted">{t("history.dismissed")}</span>
                            </Show>
                            <span class="small muted">{timeOf(e.ts ?? 0)}</span>
                            <Show when={e.app}>
                              <button
                                class="btn btn-ghost btn-sm"
                                title={t("history.muteHint", { app: e.app })}
                                aria-label={t("history.muteLabel", { app: e.app })}
                                onClick={() => mute(e.app)}
                              >
                                {t("history.mute")}
                              </button>
                            </Show>
                          </div>
                          <Show when={e.title}>
                            <strong class="notif-title">{e.title}</strong>
                          </Show>
                          <Show when={e.body}>
                            <p class="notif-body small">{e.body}</p>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </section>
              )}
            </For>
          </div>
          <Show when={page().total > page().entries.length}>
            <div class="row">
              <span class="small muted grow">
                {t("history.showing", { shown: page().entries.length, total: page().total })}
              </span>
              <button class="btn btn-soft btn-sm" onClick={() => setLimit((l) => l + PAGE)}>
                {t("history.more")}
              </button>
            </div>
          </Show>
        </Show>
      </Show>
      <Show when={muted().length > 0}>
        <div class="history-muted">
          <span class="small muted">{t("history.mutedTitle")}</span>
          <div class="history-chips">
            <For each={muted()}>
              {(a) => (
                <span class="history-muted-app">
                  <span class="small">{a}</span>
                  <button
                    class="btn btn-ghost btn-sm"
                    aria-label={t("history.unmuteLabel", { app: a })}
                    onClick={() => unmute(a)}
                  >
                    {t("history.unmute")}
                  </button>
                </span>
              )}
            </For>
          </div>
        </div>
      </Show>
    </article>
  );
}
