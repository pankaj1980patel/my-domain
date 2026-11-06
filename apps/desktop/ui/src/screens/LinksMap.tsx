// "How your devices are connected": this computer in the middle, your other
// devices around it, a line per link saying how it's carried (or dashed and
// grey: not connected, with when it was last seen), and the links your
// other devices reported between themselves. Click a device or a line for
// the details; "Connection details" opens that device's Connection tile.
// The same is listed as text below the picture (the picture itself is one
// image to a screen reader; the list's buttons open the same details).

import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { peerIcon, peerName, state } from "../lib/core";
import { DeviceIcon } from "../components/DeviceIcon";
import { mapView, myEdge, watchMap, type MapDevice, type MapEdge, type MapView } from "../lib/linksMap";
import { niceName, timeAgo } from "../lib/words";
import { HelpLink } from "../help/HelpLink";
import KeyChangeBanner from "../components/KeyChangeBanner";
import { VerifiedBadge } from "../components/VerifyDevice";
import { t, type Key } from "../i18n";

type Pick = { kind: "device"; node: string } | { kind: "edge"; id: string } | null;

const ago = (s: number) => timeAgo(Date.now() - s * 1000);

const REASON: Record<string, Key> = {
  push_failed: "conn.reason.pushFailed",
  no_answer: "conn.reason.noAnswer",
  relay_no_answer: "conn.reason.relayNoAnswer",
  relay_unreachable: "conn.reason.relayUnreachable",
  no_path: "conn.reason.noPath",
  dropped: "conn.reason.dropped",
  key_changed: "conn.reason.keyChanged",
};

const TRANSPORT: Record<string, Key> = {
  quic: "conn.transport.quic",
  punch: "conn.transport.punch",
  webrtc: "conn.transport.webrtc",
  relay: "conn.transport.relay",
  ble: "conn.transport.ble",
};

const ADDR: Record<string, Key> = {
  local: "map.addr.local",
  ipv6: "map.addr.ipv6",
  mapped: "map.addr.mapped",
  public: "map.addr.public",
};

const PATH: Record<string, Key> = {
  lan: "map.path.lan",
  direct: "map.path.direct",
  punched: "map.path.punched",
  webrtc: "map.path.webrtc",
  relay: "map.path.relay",
  ble: "map.path.ble",
};

/** The label's words: how, and the round trip where known. */
function label(e: MapEdge): string {
  const how = e.nearby ? `${t(e.label_key as Key)} · ${t("nearby.badge")}` : t(e.label_key as Key);
  return e.latency ? `${how} · ${e.latency}` : how;
}

/** "Not connected" with when it was last seen (or why it isn't). */
function offline(d: MapDevice): string {
  const seen = d.last_seen_ago_s != null ? t("map.lastSeen", { ago: ago(d.last_seen_ago_s) }) : t("map.neverSeen");
  return `${t("map.how.none")} · ${seen}`;
}

type Why = MapEdge["why"][number];

/** One reason in a line: "Local network: no answer". */
function whyLine(w: Why, name: string): string {
  return `${t(w.path_key as Key)}: ${t(w.what_key as Key, { name })}`;
}

function encryption(how: string): string {
  if (how === "relay") return t("map.enc.relay");
  if (how === "ble") return t("map.enc.ble");
  return t("map.enc.direct");
}

export default function LinksMap(props: { onDevice: (node: string) => void }) {
  const [pick, setPick] = createSignal<Pick>(null);
  onMount(() => {
    const stop = watchMap();
    onCleanup(stop);
  });
  const meName = () => niceName(state.me?.name ?? "") || t("map.you");
  const nameOf = (v: MapView, node: string) => (node === v.me.node_id ? meName() : peerName(node));
  const device = (v: MapView, node: string) => v.map.devices.find((d) => d.node_id === node);
  const edgeById = (v: MapView, id: string) => v.edges.find((e) => e.id === id);
  const others = createMemo(() => mapView()?.edges.filter((e) => !e.mine) ?? []);
  const notSharing = createMemo(() => mapView()?.map.devices.filter((d) => d.link && !d.shares_links) ?? []);
  const close = () => setPick(null);
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();

  return (
    <section class="page links-map-page" onKeyDown={onKey}>
      <header class="page-head">
        <div>
          <h1>
            {t("map.title")} <HelpLink topic="troubleshooting" />
          </h1>
          <p class="muted">{t("map.subtitle")}</p>
        </div>
      </header>

      <Show when={mapView()} fallback={<p class="muted">{t("map.loading")}</p>}>
        {(v) => (
          <>
            <Show when={v().map.devices.length === 0}>
              <p class="muted">{t("map.empty")}</p>
            </Show>
            <div class="map-wrap">
              <div class="tile map-canvas">
                <svg
                  viewBox={`0 0 ${v().width} ${v().height}`}
                  role="img"
                  aria-label={t("a11y.mapImage")}
                  class="links-map"
                >
                  <For each={v().edges}>
                    {(e) => (
                      <g
                        class={`map-edge tone-${e.tone}`}
                        classList={{ picked: pick()?.kind === "edge" && (pick() as { id: string }).id === e.id }}
                        onClick={() => setPick({ kind: "edge", id: e.id })}
                      >
                        <path d={e.path} class="map-hit" />
                        <path d={e.path} class="map-line" stroke-dasharray={e.dash ?? undefined} />
                      </g>
                    )}
                  </For>
                  <For each={v().edges}>
                    {(e) => (
                      <g
                        class={`map-label tone-${e.tone}`}
                        transform={`translate(${e.label_x} ${e.label_y})`}
                        onClick={() => setPick({ kind: "edge", id: e.id })}
                      >
                        <rect x={-label(e).length * 3.3 - 8} y={-11} width={label(e).length * 6.6 + 16} height={22} rx={11} />
                        <text text-anchor="middle" dominant-baseline="central">
                          {label(e)}
                        </text>
                      </g>
                    )}
                  </For>
                  <For each={v().spots}>
                    {(s) => (
                      <g
                        class="map-node"
                        classList={{ offline: !s.connected, picked: pick()?.kind === "device" && (pick() as { node: string }).node === s.node_id }}
                        transform={`translate(${s.x} ${s.y})`}
                        onClick={() => setPick({ kind: "device", node: s.node_id })}
                      >
                        <circle r={24} />
                        <g class="map-icon" transform="translate(-13 -13)">
                          <DeviceIcon icon={peerIcon(s.node_id)} size={26} />
                        </g>
                        <text class="map-name" y={40} text-anchor="middle">
                          {peerName(s.node_id)}
                        </text>
                        <Show when={device(v(), s.node_id)?.verified}>
                          <g class="map-verified" transform="translate(18 -18)">
                            <title>{t("verify.badge")}</title>
                            <circle r={8} />
                            <path d="M-3.5 0.5 L-1 3 L3.5 -2.5" />
                          </g>
                        </Show>
                      </g>
                    )}
                  </For>
                  <g class="map-node map-me" transform={`translate(${v().me.x} ${v().me.y})`}>
                    <circle r={32} />
                    <g class="map-icon" transform="translate(-16 -16)">
                      <DeviceIcon icon={state.ownIcon} size={32} />
                    </g>
                    <text class="map-name" y={50} text-anchor="middle">
                      {t("map.you")}
                    </text>
                  </g>
                </svg>
                <ul class="map-legend small" aria-label={t("map.legend")}>
                  <For each={["lan", "direct", "relay", "ble", "none"]}>
                    {(tone) => (
                      <li class={`tone-${tone}`}>
                        <span class="map-swatch" aria-hidden="true" classList={{ dashed: tone === "none", dotted: tone === "ble" }} />
                        {t(`map.how.${tone}` as Key)}
                      </li>
                    )}
                  </For>
                </ul>
              </div>

              <Show when={pick()}>
                {(p) => (
                  <aside class="tile map-details" aria-labelledby="map-details-title">
                    <Show
                      when={p().kind === "device" ? myEdge(v(), (p() as { node: string }).node) : edgeById(v(), (p() as { id: string }).id)}
                    >
                      {(e) => (
                        <Details
                          view={v()}
                          edge={e()}
                          nameOf={(n) => nameOf(v(), n)}
                          device={device(v(), e().mine ? e().b : "")}
                          onOpen={props.onDevice}
                          onClose={close}
                        />
                      )}
                    </Show>
                  </aside>
                )}
              </Show>
            </div>

            <section class="tile map-list" aria-labelledby="map-list-title">
              <h2 id="map-list-title">{t("map.listTitle")}</h2>
              <ul class="stack">
                <For each={v().map.devices}>
                  {(d) => {
                    const e = () => myEdge(v(), d.node_id);
                    return (
                      <li class="map-row">
                        <span class={`map-dot tone-${e()?.tone ?? "none"}`} aria-hidden="true" />
                        <span class="grow">
                          <strong>{peerName(d.node_id)}</strong>{" "}
                          <VerifiedBadge verified={d.verified} />
                          {": "}
                          {d.link ? label(e()!) : offline(d)}
                          <Show when={(e()?.why.length ?? 0) > 0}>
                            <span class="map-why-line muted small">
                              {(e()?.why ?? []).map((w) => whyLine(w, peerName(d.node_id))).join(" · ")}
                            </span>
                          </Show>
                        </span>
                        <button class="btn btn-ghost btn-sm" onClick={() => setPick({ kind: "device", node: d.node_id })}>
                          {t("map.details")}
                          <span class="sr-only">: {peerName(d.node_id)}</span>
                        </button>
                      </li>
                    );
                  }}
                </For>
              </ul>
              <h3 class="conn-sub">{t("map.others")}</h3>
              <Show when={others().length > 0} fallback={<p class="muted small">{t("map.othersNone")}</p>}>
                <ul class="stack">
                  <For each={others()}>
                    {(e) => (
                      <li class="map-row">
                        <span class={`map-dot tone-${e.tone}`} aria-hidden="true" />
                        <span class="grow">
                          {t("map.between", { a: nameOf(v(), e.a), b: nameOf(v(), e.b), how: label(e) })}
                        </span>
                        <button class="btn btn-ghost btn-sm" onClick={() => setPick({ kind: "edge", id: e.id })}>
                          {t("map.details")}
                          <span class="sr-only">: {t("map.pair", { a: nameOf(v(), e.a), b: nameOf(v(), e.b) })}</span>
                        </button>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
              <For each={notSharing()}>{(d) => <p class="muted small">{t("map.oldApp", { name: peerName(d.node_id) })}</p>}</For>
            </section>
          </>
        )}
      </Show>
    </section>
  );
}

/** What's known about one line. */
function Details(props: {
  view: MapView;
  edge: MapEdge;
  device?: MapDevice;
  nameOf: (node: string) => string;
  onOpen: (node: string) => void;
  onClose: () => void;
}) {
  let closeBtn: HTMLButtonElement | undefined;
  onMount(() => closeBtn?.focus());
  const e = () => props.edge;
  const link = () => props.device?.link ?? null;
  const other = () => props.view.map.others.find((o) => `${o.a}|${o.b}` === e().id);
  const title = () =>
    e().mine ? props.nameOf(e().b) : t("map.pair", { a: props.nameOf(e().a), b: props.nameOf(e().b) });
  const why = () => {
    const r = props.device?.reason;
    return r && REASON[r] ? t(REASON[r], { name: props.nameOf(e().b) }) : null;
  };
  return (
    <div class="stack">
      <div class="tile-head">
        <h2 id="map-details-title">{title()}</h2>
        <button class="btn btn-ghost btn-sm" ref={closeBtn} onClick={props.onClose}>
          {t("map.close")}
        </button>
      </div>
      <p class={`map-how tone-${e().tone}`}>{t(e().label_key as Key)}</p>
      <Show when={e().nearby}>
        <p class="small">
          <span class="nearby-badge" title={t("nearby.badgeHint")}>
            {t("nearby.badge")}
          </span>
        </p>
      </Show>
      <Show when={e().mine && props.device?.verified}>
        <p class="small">
          <VerifiedBadge verified />
        </p>
      </Show>
      <Show when={e().mine && props.device?.problem === "key_changed"}>
        <KeyChangeBanner node={e().b} showDismissed />
      </Show>
      <dl class="kv">
        <Show when={e().how !== "none" && PATH[e().how]}>
          <dt>{t("map.d.path")}</dt>
          <dd>{t(PATH[e().how], { a: props.nameOf(e().a), b: props.nameOf(e().b) })}</dd>
        </Show>
        <Show when={link()}>
          {(l) => (
            <>
              <dt>{t("map.d.transport")}</dt>
              <dd>{TRANSPORT[l().transport] ? t(TRANSPORT[l().transport]) : l().transport}</dd>
              <Show when={l().remote}>
                <dt>{t("map.d.address")}</dt>
                <dd>
                  {l().remote}
                  {l().addr_kind && ADDR[l().addr_kind!] ? ` · ${t(ADDR[l().addr_kind!])}` : ""}
                </dd>
              </Show>
              <Show when={l().since_s != null}>
                <dt>{t("map.d.since")}</dt>
                <dd>{ago(l().since_s!)}</dd>
              </Show>
              <dt>{t("map.d.heard")}</dt>
              <dd>{ago(l().heard_ago_s)}</dd>
              <Show when={l().loss_pct != null}>
                <dt>{t("conn.loss")}</dt>
                <dd>{t("conn.lossValue", { pct: (l().loss_pct ?? 0).toFixed(1) })}</dd>
              </Show>
            </>
          )}
        </Show>
        <Show when={e().latency}>
          <dt>{t("conn.rtt")}</dt>
          <dd>{e().latency}</dd>
        </Show>
        <Show when={e().how !== "none"}>
          <dt>{t("map.d.encryption")}</dt>
          <dd>{encryption(e().how)}</dd>
        </Show>
        <Show when={e().mine && !link() && props.device}>
          <dt>{t("map.d.lastSeen")}</dt>
          <dd>{props.device?.last_seen_ago_s != null ? ago(props.device.last_seen_ago_s) : t("map.neverSeen")}</dd>
          <Show when={why()}>
            <dt>{t("map.d.why")}</dt>
            <dd>{why()}</dd>
          </Show>
        </Show>
        <Show when={other()}>
          {(o) => (
            <>
              <dt>{t("map.d.reportedBy")}</dt>
              <dd>{t("map.reportedValue", { name: props.nameOf(o().reported_by), ago: ago(o().reported_ago_s) })}</dd>
            </>
          )}
        </Show>
      </dl>
      <Show when={e().why.length > 0}>
        <h3 class="conn-sub">{e().how === "none" ? t("map.whyTitle") : t("map.whyNotDirect")}</h3>
        <ul class="map-why">
          <For each={e().why}>
            {(w) => (
              <li>
                <span>
                  <strong>{t(w.path_key as Key)}</strong>
                  {": "}
                  {t(w.what_key as Key, { name: props.nameOf(e().b) })}
                  <Show when={w.ago_s != null}>
                    <span class="muted"> · {ago(w.ago_s!)}</span>
                  </Show>
                </span>
                <span class="muted small">{t(w.fix_key as Key, { name: props.nameOf(e().b) })}</span>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <div class="row">
        <Show
          when={e().mine}
          fallback={
            <For each={[e().a, e().b]}>
              {(n) => (
                <button class="btn btn-soft btn-sm" onClick={() => props.onOpen(n)}>
                  {t("map.openFor", { name: props.nameOf(n) })}
                </button>
              )}
            </For>
          }
        >
          <button class="btn btn-soft" onClick={() => props.onOpen(e().b)}>
            {t("map.open")}
          </button>
        </Show>
      </div>
    </div>
  );
}
