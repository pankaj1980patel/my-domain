// One synced folder: how it stands (in sync, syncing N files, conflicts,
// paused, a problem), when it was last in sync, and Pause / Sync now / Open
// folder / Remove. Shown on the device's page and in Settings; an invitation
// (`invited`) gets Accept (pick where it goes) and Decline instead.

import { createSignal, Show } from "solid-js";
import {
  peerName,
  pickFolder,
  syncAccept,
  syncNow,
  syncOpenFolder,
  syncPause,
  syncRemove,
  type SyncPair,
} from "../lib/core";
import { friendlyError, timeAgo } from "../lib/words";
import { toast } from "./ui";
import { t, tn, type Key } from "../i18n";

const MODE_KEYS: Record<string, Key> = {
  both: "sync.mode.both",
  send: "sync.mode.send",
  receive: "sync.mode.receive",
};

export function modeLabel(mode: string): string {
  return t(MODE_KEYS[mode] ?? "sync.mode.both");
}

/** What the pair is doing, in words. */
function stateLine(p: SyncPair): string {
  const name = peerName(p.peer);
  switch (p.state) {
    case "invited":
      return t("sync.state.invited");
    case "pending":
      return t("sync.state.pending", { name });
    case "paused":
      return t("sync.state.paused");
    case "error":
      return t("sync.state.error", { error: p.error ?? "" });
    case "syncing":
      return tn("sync.state.syncing", Math.max(1, p.syncing));
    case "waiting":
      return t("sync.state.waiting", { name });
    default:
      return t("sync.state.inSync");
  }
}

export function SyncPairRow(props: { pair: SyncPair; showDevice?: boolean }) {
  const [busy, setBusy] = createSignal(false);
  const [confirming, setConfirming] = createSignal(false);
  const p = () => props.pair;
  const name = () => peerName(p().peer);
  const run = (f: () => Promise<void>, done?: string) => async () => {
    setBusy(true);
    try {
      await f();
      if (done) toast(done, "ok");
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const accept = run(async () => {
    const path = await pickFolder();
    if (!path) return;
    await syncAccept(p().id, path);
    toast(t("sync.accepted", { folder: p().name, name: name() }), "ok");
  });
  const remove = run(async () => {
    const invited = p().state === "invited";
    const folder = p().name;
    await syncRemove(p().id);
    setConfirming(false);
    if (!invited) toast(t("sync.removed", { folder }), "ok");
  });

  return (
    <li class="sync-pair" classList={{ "sync-pair-problem": p().state === "error" }}>
      <div class="sync-pair-head">
        <strong class="sync-pair-name">{p().name}</strong>
        <span class={`sync-badge sync-badge-${p().state}`}>{stateLine(p())}</span>
      </div>
      <Show
        when={p().state === "invited"}
        fallback={
          <>
            <p class="muted small sync-pair-path" title={p().path}>
              {props.showDevice ? t("sync.pathWith", { path: p().path, name: name() }) : p().path}
            </p>
            <p class="muted small">
              {modeLabel(p().mode)}
              <Show when={p().files > 0}> · {tn("sync.files", p().files)}</Show>
              <Show when={p().conflicts > 0}> · {tn("sync.conflicts", p().conflicts)}</Show>
              <Show when={p().skipped > 0}> · {tn("sync.skipped", p().skipped)}</Show>
              <Show when={p().renamed > 0}>
                {" · "}
                <span title={t("sync.renamedHint")}>{tn("sync.renamed", p().renamed)}</span>
              </Show>
              <Show when={(p().last_synced ?? 0) > 0}> · {t("sync.lastSynced", { when: timeAgo((p().last_synced ?? 0) * 1000) })}</Show>
            </p>
          </>
        }
      >
        <p class="small">{t("sync.invitedLine", { name: name(), folder: p().name, mode: modeLabel(p().mode) })}</p>
      </Show>
      <Show
        when={!confirming()}
        fallback={
          <div class="sync-confirm stack">
            <p class="small">{t("sync.removeConfirm", { folder: p().name, name: name() })}</p>
            <div class="row">
              <button class="btn btn-ghost btn-sm sync-remove" disabled={busy()} onClick={remove}>
                {t("sync.remove")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => setConfirming(false)}>
                {t("sync.cancel")}
              </button>
            </div>
          </div>
        }
      >
        <div class="row sync-actions">
          <Show
            when={p().state === "invited"}
            fallback={
              <>
                <Show when={p().state !== "pending"}>
                  <button
                    class="btn btn-ghost btn-sm"
                    disabled={busy()}
                    onClick={run(() => syncPause(p().id, !p().paused))}
                  >
                    {p().paused ? t("sync.resume") : t("sync.pause")}
                  </button>
                  <button
                    class="btn btn-ghost btn-sm"
                    disabled={busy() || p().paused}
                    onClick={run(() => syncNow(p().id), t("sync.nowStarted"))}
                  >
                    {t("sync.now")}
                  </button>
                  <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={run(() => syncOpenFolder(p().id))}>
                    {t("sync.open")}
                  </button>
                </Show>
                <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => setConfirming(true)}>
                  {t("sync.remove")}
                </button>
              </>
            }
          >
            <button class="btn btn-primary btn-sm" disabled={busy()} onClick={accept}>
              {t("sync.accept")}
            </button>
            <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={remove}>
              {t("sync.decline")}
            </button>
          </Show>
        </div>
      </Show>
    </li>
  );
}
