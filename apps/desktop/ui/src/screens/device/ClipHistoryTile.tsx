// Clipboard history for one device: the text sent to it and received from it
// as clipboard, kept on this computer (the last 200, clip_history.rs),
// searchable. "Copy" puts a clip on this computer's clipboard; "Send again"
// puts it on the device's. Turned off in Settings, nothing is kept.

import { createEffect, createSignal, For, on, Show } from "solid-js";
import { clearClipHistoryOf, clipHistory, clipTick, copyText, peerName, sendClipAgain, type ClipPage } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";

/** Rows shown at first, and added by "Show more". */
const PAGE = 50;

/** The time, with the date when it wasn't today. */
function whenOf(ts: number, now = new Date()): string {
  const d = new Date(ts);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return time;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`;
}

export default function ClipHistoryTile(props: { node: string }) {
  const name = () => peerName(props.node);
  const [query, setQuery] = createSignal("");
  const [limit, setLimit] = createSignal(PAGE);
  const [page, setPage] = createSignal<ClipPage>({ entries: [], total: 0, kept: 0 });
  let asked = 0;

  // A new search starts from the top again.
  createEffect(on([query, () => props.node], () => setLimit(PAGE), { defer: true }));
  createEffect(
    on([() => props.node, query, limit, clipTick], ([node, q, l]) => {
      const mine = ++asked;
      clipHistory(node, q, l)
        .then((p) => {
          if (mine === asked) setPage(p);
        })
        .catch(() => {});
    }),
  );

  const copy = (text: string) =>
    copyText(text)
      .then(() => toast(t("clipHistory.copied"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));
  const sendAgain = (text: string) =>
    sendClipAgain(props.node, text)
      .then((how) =>
        toast(how === "queued" ? t("clipHistory.queued", { name: name() }) : t("clipHistory.sent", { name: name() }), "ok"),
      )
      .catch((e) => toast(friendlyError(e), "error"));
  const clear = () =>
    clearClipHistoryOf(props.node)
      .then(() => toast(t("clipHistory.cleared"), "ok"))
      .catch((e) => toast(friendlyError(e), "error"));

  return (
    <article class="tile tile-wide">
      <div class="tile-head">
        <h2>{t("clipHistory.title")}</h2>
        <Show when={page().kept > 0}>
          <button class="btn btn-ghost btn-sm" title={t("clipHistory.clearHint", { name: name() })} onClick={clear}>
            {t("clipHistory.clear")}
          </button>
        </Show>
      </div>
      <Show when={page().kept > 0} fallback={<p class="muted small">{t("clipHistory.empty", { name: name() })}</p>}>
        <input
          type="search"
          class="clip-log-search"
          placeholder={t("clipHistory.search")}
          aria-label={t("clipHistory.searchLabel")}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <Show when={page().total > 0} fallback={<p class="muted small">{t("clipHistory.noMatch")}</p>}>
          <ul class="clip-log">
            <For each={page().entries}>
              {(e) => (
                <li class="clip-log-row">
                  <span
                    class="clip-log-dir"
                    classList={{ "clip-log-in": e.direction === "in" }}
                    title={e.direction === "in" ? t("clipHistory.from", { name: name() }) : t("clipHistory.to", { name: name() })}
                    aria-label={e.direction === "in" ? t("clipHistory.from", { name: name() }) : t("clipHistory.to", { name: name() })}
                  >
                    {e.direction === "in" ? "↓" : "↑"}
                  </span>
                  <div class="clip-log-body">
                    <span class="clip-log-text">{e.text}</span>
                    <span class="small muted">
                      {whenOf(e.at ?? 0)}
                      <Show when={e.truncated}> · {t("clipHistory.truncated")}</Show>
                    </span>
                  </div>
                  <div class="clip-log-actions">
                    <button class="btn btn-ghost btn-sm" aria-label={t("clipHistory.copyLabel")} onClick={() => copy(e.text)}>
                      {t("clipHistory.copy")}
                    </button>
                    <button
                      class="btn btn-ghost btn-sm"
                      aria-label={t("clipHistory.sendAgainLabel", { name: name() })}
                      onClick={() => sendAgain(e.text)}
                    >
                      {t("clipHistory.sendAgain")}
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
          <Show when={page().total > page().entries.length}>
            <div class="row">
              <span class="small muted grow">
                {t("clipHistory.showing", { shown: page().entries.length, total: page().total })}
              </span>
              <button class="btn btn-soft btn-sm" onClick={() => setLimit((l) => l + PAGE)}>
                {t("clipHistory.more")}
              </button>
            </div>
          </Show>
        </Show>
      </Show>
    </article>
  );
}
