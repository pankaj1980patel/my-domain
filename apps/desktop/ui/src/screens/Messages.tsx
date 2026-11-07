// Messages & notifications: your devices as conversations on the left (the
// newest message under each name), and on the right the chosen one's chat,
// its notifications and their history, or which of its apps send them here.
// Built from the device page's own tiles.

import { createMemo, createSignal, For, Show } from "solid-js";
import { peerIcon, peerName, state } from "../lib/core";
import { timeAgo } from "../lib/words";
import { t } from "../i18n";
import { linkStateOf } from "../components/ui";
import { Icon } from "../components/Icon";
import { Tabs, panelProps } from "../components/Tabs";
import DeviceArt from "../components/DeviceArt";
import { pageKind } from "../lib/pageLayout";
import NotificationHistory from "./device/NotificationHistory";
import { AppsTile, ChatTile, NotificationsTile } from "./Device";

type Tab = "messages" | "notifications" | "apps";

export default function Messages(props: { onOpen: (node: string) => void }) {
  const [tab, setTab] = createSignal<Tab>("messages");
  const last = (node: string) => {
    const list = state.messages.filter((m) => m.node === node);
    return list[list.length - 1];
  };
  const unread = (node: string) => state.messages.filter((m) => m.node === node && m.dir === "in").length;
  // Newest conversation first, then connected, then by name.
  const devices = createMemo(() =>
    [...state.peers].sort(
      (a, b) =>
        (last(b.node_id)?.ts ?? 0) - (last(a.node_id)?.ts ?? 0) ||
        (state.links[a.node_id] ? 0 : 1) - (state.links[b.node_id] ? 0 : 1) ||
        peerName(a.node_id).localeCompare(peerName(b.node_id)),
    ),
  );
  // Notifications and apps come from phones.
  const shown = () => (tab() === "messages" ? devices() : devices().filter((p) => pageKind(p.node_id) === "phone"));
  const [chosen, setChosen] = createSignal<string | null>(null);
  const node = () => {
    const c = chosen();
    return c && shown().some((p) => p.node_id === c) ? c : shown()[0]?.node_id ?? null;
  };

  return (
    <section class="page page-fit msg-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="chat" size={26} tone="violet" />
        </span>
        <div class="grow">
          <h1>{t("msg.title")}</h1>
          <p class="muted">{t("msg.sub")}</p>
        </div>
      </header>
      <Tabs
        label={t("msg.title")}
        idBase="msg"
        tabs={[
          { id: "messages", label: t("chat.title"), icon: "chat" },
          { id: "notifications", label: t("notifications.title"), icon: "notifications" },
          { id: "apps", label: t("apps.title"), icon: "apps" },
        ]}
        selected={tab()}
        onSelect={(id) => setTab(id as Tab)}
      />
      <div class="msg-body" {...panelProps("msg", tab())}>
        <ul class="msg-list" aria-label={t("msg.devices")}>
          <For each={shown()} fallback={<li class="muted small">{t("msg.noPhones")}</li>}>
            {(p) => {
              const s = () => linkStateOf(p.node_id);
              return (
                <li>
                  <button class="msg-item" aria-current={node() === p.node_id ? "true" : undefined} onClick={() => setChosen(p.node_id)}>
                    <DeviceArt node={p.node_id} icon={peerIcon(p.node_id)} size={40} />
                    <span class="msg-item-text">
                      <span class="msg-item-top">
                        <strong>{peerName(p.node_id)}</strong>
                        <Show when={last(p.node_id)}>
                          {(m) => <span class="muted small">{timeAgo(m().ts)}</span>}
                        </Show>
                      </span>
                      <span class="muted small msg-preview">
                        <Show
                          when={tab() === "messages" && last(p.node_id)}
                          fallback={
                            <>
                              <span class={`hero-dot tone-${s() === "direct" ? "green" : s() === "relay" ? "amber" : "muted"}`} aria-hidden="true" />{" "}
                              {s() === "direct" || s() === "relay" ? t("dash.online") : t("dash.offline")}
                            </>
                          }
                        >
                          {(m) => m().text}
                        </Show>
                      </span>
                    </span>
                    <Show when={tab() === "messages" && unread(p.node_id) > 0}>
                      <span class="badge">{unread(p.node_id)}</span>
                    </Show>
                  </button>
                </li>
              );
            }}
          </For>
        </ul>
        <div class="msg-pane">
          <Show when={node()} keyed>
            {(n) => (
              <>
                <div class="msg-pane-head">
                  <strong>{peerName(n)}</strong>
                  <button class="link small dash-more" onClick={() => props.onOpen(n)}>
                    {t("palette.open", { name: peerName(n) })}
                    <Icon name="arrow-right" size={14} />
                  </button>
                </div>
                <Show when={tab() === "messages"}>
                  <ChatTile node={n} />
                </Show>
                <Show when={tab() === "notifications"}>
                  <div class="msg-scroll">
                    <NotificationsTile node={n} />
                    <NotificationHistory node={n} />
                  </div>
                </Show>
                <Show when={tab() === "apps"}>
                  <div class="msg-scroll">
                    <AppsTile node={n} />
                  </div>
                </Show>
              </>
            )}
          </Show>
        </div>
      </div>
    </section>
  );
}
