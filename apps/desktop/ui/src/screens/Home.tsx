// Home: your devices as cards (connected ones first), each with one-click
// actions; under them, the latest files and messages. A health problem shows
// once, as the sidebar's status pill (not here too).

import { createMemo, For, Show } from "solid-js";
import { connect, peerName, refreshDevices, state } from "../lib/core";
import { friendlyError, timeAgo } from "../lib/words";
import { DeviceCard, Empty, toast } from "../components/ui";
import { t } from "../i18n";
import { SetupCard } from "./Setup";
import { Icon } from "../components/Icon";

/** One line of "Recent": a file either way, or a message that came in. */
type Recent = { key: string; ts: number; node: string; tab: "files" | "messages"; text: string; meta: string };

export default function Home(props: {
  onOpen: (node: string, tab?: string) => void;
  onMap: () => void;
  onSetup: () => void;
}) {
  const devices = createMemo(() =>
    [...state.peers].sort((a, b) => {
      const la = state.links[a.node_id] ? 0 : 1;
      const lb = state.links[b.node_id] ? 0 : 1;
      return la - lb || peerName(a.node_id).localeCompare(peerName(b.node_id));
    }),
  );
  // Already in memory (no call): the newest few files (not each file of a
  // folder) and incoming messages.
  const recent = createMemo<Recent[]>(() => {
    const files: Recent[] = Object.values(state.transfers)
      .filter((x) => !x.batch)
      .map((x) => ({
        key: `f-${x.id}`,
        ts: x.ts,
        node: x.node,
        tab: "files",
        text: x.name,
        meta: t(x.incoming ? "home.recentFrom" : "home.recentTo", { name: peerName(x.node) }),
      }));
    const messages: Recent[] = state.messages
      .filter((m) => m.dir === "in")
      .map((m) => ({
        key: `m-${m.id}`,
        ts: m.ts,
        node: m.node,
        tab: "messages",
        text: m.text,
        meta: t("home.recentFrom", { name: peerName(m.node) }),
      }));
    return [...files, ...messages].sort((a, b) => b.ts - a.ts).slice(0, 5);
  });
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
            <Icon name="map" />
            {t("map.openMap")}
          </button>
        </Show>
        <button
          class="btn btn-ghost"
          onClick={() => refreshDevices().then(() => toast(t("home.refreshed"), "ok"), (e) => toast(friendlyError(e), "error"))}
        >
          <Icon name="refresh" />
          {t("home.refresh")}
        </button>
        </div>
      </header>

      {/* Setup not finished: carry on where it stopped. */}
      <SetupCard onOpen={props.onSetup} />
      <Show
        when={devices().length > 0}
        fallback={
          <Empty icon="devices" title={t("home.emptyTitle")}>
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

      <Show when={recent().length > 0}>
        <section class="recent" aria-labelledby="home-recent">
          <h2 id="home-recent" class="eyebrow">
            {t("home.recent")}
          </h2>
          <ul class="recent-list">
            <For each={recent()}>
              {(r) => (
                <li>
                  <button type="button" class="recent-item" onClick={() => props.onOpen(r.node, r.tab)}>
                    <Icon name={r.tab === "files" ? "files" : "chat"} />
                    <span class="recent-text">{r.text}</span>
                    <span class="recent-meta muted small">
                      {r.meta} · {timeAgo(r.ts)}
                    </span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
    </section>
  );
}
