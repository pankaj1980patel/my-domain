// Home: your devices as cards. Connected ones first.

import { createMemo, For, Show } from "solid-js";
import { connect, refreshDevices, state } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { DeviceCard, Empty, toast } from "../components/ui";
import { t } from "../i18n";

export default function Home(props: { onOpen: (node: string) => void }) {
  const devices = createMemo(() =>
    [...state.peers].sort((a, b) => {
      const la = state.links[a.node_id] ? 0 : 1;
      const lb = state.links[b.node_id] ? 0 : 1;
      return la - lb || niceName(a.name).localeCompare(niceName(b.name));
    }),
  );
  const online = () => state.peers.filter((p) => state.links[p.node_id]).length;

  const doConnect = async (node: string) => {
    try {
      await connect(node);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>{t("home.title")}</h1>
          <p class="muted">
            <Show when={state.peers.length > 0} fallback={t("home.emptyHint")}>
              {t("home.connectedCount", { online: online(), total: state.peers.length })}
            </Show>
          </p>
        </div>
        <button
          class="btn btn-ghost"
          onClick={() => refreshDevices().then(() => toast(t("home.refreshed"), "ok"), (e) => toast(friendlyError(e), "error"))}
        >
          {t("home.refresh")}
        </button>
      </header>

      <Show
        when={devices().length > 0}
        fallback={
          <Empty icon="" title={t("home.emptyTitle")}>
            {t("home.emptyBody")}
          </Empty>
        }
      >
        <div class="device-grid">
          <For each={devices()}>
            {(p) => <DeviceCard peer={p} onOpen={() => props.onOpen(p.node_id)} onConnect={() => doConnect(p.node_id)} />}
          </For>
        </div>
      </Show>
    </section>
  );
}
