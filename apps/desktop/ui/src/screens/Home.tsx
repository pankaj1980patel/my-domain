// Home: your devices as cards. Connected ones first.

import { createMemo, For, Show } from "solid-js";
import { connect, peerName, refreshDevices, state } from "../lib/core";
import { friendlyError } from "../lib/words";
import { DeviceCard, Empty, toast } from "../components/ui";
import { t } from "../i18n";
import { badCount } from "../lib/health";
import { SetupCard } from "./Setup";

export default function Home(props: {
  onOpen: (node: string) => void;
  onMap: () => void;
  onHealth: () => void;
  onSetup: () => void;
}) {
  const devices = createMemo(() =>
    [...state.peers].sort((a, b) => {
      const la = state.links[a.node_id] ? 0 : 1;
      const lb = state.links[b.node_id] ? 0 : 1;
      return la - lb || peerName(a.node_id).localeCompare(peerName(b.node_id));
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
        <div class="row">
        <Show when={state.peers.length > 0}>
          <button class="btn btn-soft" onClick={props.onMap}>
            {t("map.openMap")}
          </button>
        </Show>
        <button
          class="btn btn-ghost"
          onClick={() => refreshDevices().then(() => toast(t("home.refreshed"), "ok"), (e) => toast(friendlyError(e), "error"))}
        >
          {t("home.refresh")}
        </button>
        </div>
      </header>

      {/* Setup not finished: carry on where it stopped. */}
      <SetupCard onOpen={props.onSetup} />
      {/* The health check found something red. */}
      <Show when={badCount() > 0}>
        <article class="tile health-card">
          <h2>{t("health.homeTitle")}</h2>
          <p class="muted">{t("health.homeBody", { n: badCount() })}</p>
          <div class="row">
            <button class="btn btn-primary" onClick={props.onHealth}>
              {t("health.open")}
            </button>
          </div>
        </article>
      </Show>

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
