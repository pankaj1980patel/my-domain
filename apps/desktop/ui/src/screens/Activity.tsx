// What happened, in plain sentences.

import { For, Show } from "solid-js";
import { state } from "../lib/core";
import { timeAgo } from "../lib/words";
import { Empty } from "../components/ui";
import { t } from "../i18n";

export default function Activity() {
  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>{t("activity.title")}</h1>
          <p class="muted">{t("activity.subtitle")}</p>
        </div>
      </header>
      <Show when={state.activity.length > 0} fallback={<Empty icon="" title={t("activity.emptyTitle")}>{t("activity.emptyBody")}</Empty>}>
        <ul class="activity">
          <For each={state.activity}>
            {(a) => (
              <li class={`activity-item tone-${a.tone}`}>
                <span class="activity-icon" aria-hidden="true" />
                <span class="activity-text">{a.text}</span>
                <time class="muted small">{timeAgo(a.ts)}</time>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </section>
  );
}
