// Settings: "Synced folders": every synced folder on this computer, with how
// it stands, and the size limit for synced files. New ones start on a
// device's page.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { state, unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { SyncPairRow } from "./SyncPairRow";
import { HelpLink } from "../help/HelpLink";
import { t } from "../i18n";

/** Size limits offered, in MB. */
const LIMITS = [100, 500, 1024, 4096, 16384];

export function SyncSetting() {
  const [limit, setLimit] = createSignal(1024);
  onMount(() => commands.syncMaxSizeMb().then(setLimit, () => {}));

  const change = async (mb: number) => {
    try {
      await unwrap(commands.syncSetMaxSizeMb(mb));
      setLimit(mb);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };
  const label = (mb: number) => (mb >= 1024 ? t("sync.sizeGb", { n: mb / 1024 }) : t("sync.sizeMb", { n: mb }));
  const options = () => (LIMITS.includes(limit()) ? LIMITS : [...LIMITS, limit()].sort((a, b) => a - b));

  return (
    <article class="tile" id="settings-sync">
      <h2>{t("sync.title")}</h2>
      <p class="muted small">
        {t("sync.settingsHint")} <HelpLink topic="synced-folders" />
      </p>
      <Show when={state.syncPairs.length} fallback={<p class="muted small">{t("sync.settingsEmpty")}</p>}>
        <ul class="sync-pairs">
          <For each={state.syncPairs}>{(p) => <SyncPairRow pair={p} showDevice />}</For>
        </ul>
      </Show>
      <label class="row small">
        <span class="muted">{t("sync.maxSize")}</span>
        <select aria-label={t("sync.maxSize")} onChange={(e) => change(Number(e.currentTarget.value))}>
          <For each={options()}>
            {(mb) => (
              <option value={String(mb)} selected={mb === limit()}>
                {label(mb)}
              </option>
            )}
          </For>
        </select>
      </label>
    </article>
  );
}
