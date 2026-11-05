// Synced folders with this device: the ones there are (and invitations from
// it), and "Sync a folder…": pick a folder here and how it syncs; the device
// is asked and picks where it goes there. Only offered once its status says
// its app can sync (`sync`), so an older app is never asked.

import { createSignal, For, Show } from "solid-js";
import { peerName, pickFolder, state, syncCreate } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { modeLabel, SyncPairRow } from "../../components/SyncPairRow";
import { HelpLink } from "../../help/HelpLink";
import { t } from "../../i18n";

const MODES = ["both", "send", "receive"] as const;

export default function SyncTile(props: { node: string }) {
  const [path, setPath] = createSignal<string | null>(null);
  const [mode, setMode] = createSignal<string>("both");
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const pairs = () => state.syncPairs.filter((p) => p.peer === props.node);
  const supported = () => state.status[props.node]?.sync === true;

  const choose = async () => {
    const picked = await pickFolder().catch(() => null);
    if (picked) setPath(picked);
  };
  const start = async () => {
    const p = path();
    if (!p) return;
    setBusy(true);
    try {
      await syncCreate(props.node, p, mode());
      toast(t("sync.asked", { name: name() }), "ok");
      setPath(null);
      setMode("both");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <article class="tile">
      <h2>{t("sync.title")}</h2>
      <p class="muted small">
        {t("sync.hint", { name: name() })} <HelpLink topic="synced-folders" />
      </p>
      <Show when={pairs().length}>
        <ul class="sync-pairs">
          <For each={pairs()}>{(p) => <SyncPairRow pair={p} />}</For>
        </ul>
      </Show>
      <Show when={supported()} fallback={<p class="muted small">{t("sync.needsUpdate", { name: name() })}</p>}>
        <Show
          when={path()}
          fallback={
            <button class="btn btn-soft" onClick={choose}>
              {t("sync.add")}
            </button>
          }
        >
          {(chosen) => (
            <div class="stack sync-new">
              <p class="small sync-pair-path" title={chosen()}>
                {t("sync.chosen", { path: chosen() })}
              </p>
              <fieldset class="sync-modes">
                <legend class="small muted">{t("sync.modeLabel")}</legend>
                <For each={MODES}>
                  {(m) => (
                    <label class="row small">
                      <input
                        type="radio"
                        name={`sync-mode-${props.node}`}
                        value={m}
                        checked={mode() === m}
                        onChange={() => setMode(m)}
                      />
                      <span>{modeLabel(m)}</span>
                    </label>
                  )}
                </For>
              </fieldset>
              <div class="row">
                <button class="btn btn-primary btn-sm" disabled={busy()} onClick={start}>
                  {t("sync.start")}
                </button>
                <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={choose}>
                  {t("sync.otherFolder")}
                </button>
                <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => setPath(null)}>
                  {t("sync.cancel")}
                </button>
              </div>
            </div>
          )}
        </Show>
      </Show>
    </article>
  );
}
