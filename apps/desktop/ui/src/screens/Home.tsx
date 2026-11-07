// Home, "All devices": the header with Add device; four counts (devices,
// online, issues from the health check, direct links); a filter bar (all /
// online / offline, type, sort, list or grid); the devices; then, in the
// grid, recent activity and Quick actions for the chosen device, or in the
// list a rail with the network's state, Quick actions and the last
// security check. `onlyDevices` (the sidebar's "All devices") shows the
// header, the filter bar and the devices alone.

import DeviceArt, { frameOf } from "../components/DeviceArt";
import { createEffect, createMemo, createSignal, For, on, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { commands } from "../bindings";
import { connect, peerIcon, peerName, refreshDevices, state, type Peer } from "../lib/core";
import { friendlyError, humanSize, presenceLabel, presenceTone, routeOf, timeAgo } from "../lib/words";
import { Empty, linkStateOf, RouteChip, toast } from "../components/ui";
import { t, tn } from "../i18n";
import { problemCount, ranAt, report } from "../lib/health";
import { isVerified } from "../lib/verify";
import { loadMap, mapView, myEdge } from "../lib/linksMap";
import { osName } from "../lib/deviceInfo";
import { pageKind } from "../lib/pageLayout";
import { quickRing, quickSendClipboard, quickSendFile } from "../lib/quickActions";
import { SetupCard } from "./Setup";
import { Icon, IconChip, type IconName } from "../components/Icon";
import { MenuButton, type MenuItem } from "../components/Menu";
import { RecentActivity } from "../components/RecentActivity";
import { QuickGrid } from "../components/QuickGrid";
import { AddDevice } from "../components/AddDevice";
import { rememberFocus, trapTab } from "../lib/a11y";

type Layout = "grid" | "list";
type Filter = "all" | "online" | "offline";
type Kind = "all" | "phone" | "computer";
type Sort = "recent" | "name";

/** List or grid: remembered on this computer. */
const LAYOUT_STORE = "md.homeLayout";
function savedLayout(): Layout {
  try {
    return localStorage.getItem(LAYOUT_STORE) === "list" ? "list" : "grid";
  } catch {
    return "grid";
  }
}

type Open = (node: string, tab?: string, tile?: string) => void;

export default function Home(props: {
  onlyDevices?: boolean;
  onOpen: Open;
  onMap: () => void;
  onSetup: () => void;
  onHealth: () => void;
  onActivity: () => void;
}) {
  const live = (node: string) => !!state.links[node];
  const online = () => state.peers.filter((p) => live(p.node_id)).length;
  const offline = () => state.peers.length - online();
  const relayed = () => state.peers.filter((p) => state.links[p.node_id] === "relay").length;
  const direct = () => online() - relayed();

  // Last seen and round trips, from the connections map (read once here).
  onMount(() => loadMap().catch(() => {}));
  const mapDevice = (node: string) => mapView()?.map.devices.find((d) => d.node_id === node);
  const lastSeen = (node: string) => mapDevice(node)?.last_seen_ago_s ?? null;
  const latency = () => {
    const rtts = (mapView()?.map.devices ?? []).map((d) => d.link?.rtt_ms).filter((x): x is number => x != null);
    return rtts.length ? Math.round(rtts.reduce((a, b) => a + b, 0) / rtts.length) : null;
  };

  // --- filter, type, sort, layout ---------------------------------------------
  const [filter, setFilter] = createSignal<Filter>("all");
  const [kind, setKind] = createSignal<Kind>("all");
  const [sort, setSort] = createSignal<Sort>("recent");
  const [layout, setLayoutSig] = createSignal<Layout>(
    import.meta.env.MODE === "mock" && new URLSearchParams(location.search).get("layout") === "list" ? "list" : savedLayout(),
  );
  const setLayout = (l: Layout) => {
    setLayoutSig(l);
    try {
      localStorage.setItem(LAYOUT_STORE, l);
    } catch {
      /* no storage: grid next time */
    }
  };
  const devices = createMemo(() => {
    const seen = (n: string) => (live(n) ? -1 : (lastSeen(n) ?? Number.MAX_SAFE_INTEGER));
    return state.peers
      .filter((p) => filter() === "all" || (filter() === "online") === live(p.node_id))
      .filter((p) => kind() === "all" || pageKind(p.node_id) === kind())
      .sort((a, b) =>
        sort() === "name"
          ? peerName(a.node_id).localeCompare(peerName(b.node_id))
          : seen(a.node_id) - seen(b.node_id) || peerName(a.node_id).localeCompare(peerName(b.node_id)),
      );
  });

  // The device Quick actions are for: the first one shown until you pick.
  const [chosen, setChosen] = createSignal<string | null>(null);
  const target = () => {
    const c = chosen();
    if (c && state.peers.some((p) => p.node_id === c)) return c;
    return devices()[0]?.node_id ?? state.peers[0]?.node_id ?? null;
  };
  const targetMenu = (): MenuItem[] =>
    state.peers.map((p) => ({ icon: "devices" as const, label: peerName(p.node_id), onSelect: () => setChosen(p.node_id) }));

  const [adding, setAdding] = createSignal(false);
  const kindMenu = (): MenuItem[] => [
    { label: t("home.typeAll"), onSelect: () => setKind("all") },
    { icon: "phone", label: t("home.typePhones"), onSelect: () => setKind("phone") },
    { icon: "monitor", label: t("home.typeComputers"), onSelect: () => setKind("computer") },
  ];
  const kindLabel = () =>
    kind() === "phone" ? t("home.typePhones") : kind() === "computer" ? t("home.typeComputers") : t("home.byType");
  const sortMenu = (): MenuItem[] => [
    { label: t("home.sortRecent"), onSelect: () => setSort("recent") },
    { label: t("home.sortName"), onSelect: () => setSort("name") },
  ];

  const quickHead = (node: string) => (
    <div class="dash-head">
      <Icon name="quick" tone="amber" />
      <h2 id="home-quick">{t("dash.quick")}</h2>
      <MenuButton
        label={t("home.target", { name: peerName(node) })}
        labelled
        triggerClass="link small dash-more home-target"
        items={targetMenu()}
      >
        {t("home.for", { name: peerName(node) })}
        <Icon name="chevron-down" size={14} />
      </MenuButton>
    </div>
  );
  const rail = () => !props.onlyDevices && layout() === "list";

  return (
    <section class="page page-fit home-page" classList={{ "home-wide": rail() }}>
      <header class="page-head home-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="devices" size={28} tone="cyan" />
        </span>
        <div class="grow">
          <h1>{t("home.allTitle")}</h1>
          <p class="muted">{t("home.allSub")}</p>
        </div>
        <AddDeviceButton onAdd={() => setAdding(true)} onMap={props.onMap} />
      </header>

      {/* Setup not finished: carry on where it stopped. */}
      <SetupCard onOpen={props.onSetup} />

      <div class="home-body" classList={{ "with-rail": rail() }}>
        <div class="home-left">
          <Show when={!props.onlyDevices && state.peers.length > 0}>
            <DeviceStats onHealth={props.onHealth} />
          </Show>

          {/* Which devices, in what order, as a list or a grid. */}
          <Show when={state.peers.length > 0}>
            <div class="home-filters" role="toolbar" aria-label={t("home.filters")}>
              <For each={[["all", state.peers.length], ["online", online()], ["offline", offline()]] as [Filter, number][]}>
                {([f, n]) => (
                  <button class="filter-chip" aria-pressed={filter() === f} onClick={() => setFilter(f)}>
                    <Show when={f !== "all"}>
                      <span class={`stat-dot small-dot ${f === "online" ? "" : "dot-off"}`} aria-hidden="true" />
                    </Show>
                    {t(f === "all" ? "home.filterAll" : f === "online" ? "home.filterOnline" : "home.filterOffline", { n })}
                  </button>
                )}
              </For>
              <MenuButton label={kindLabel()} labelled triggerClass="filter-chip" items={kindMenu()}>
                {kindLabel()}
                <Icon name="chevron-down" size={14} />
              </MenuButton>
              <span class="grow" />
              <MenuButton label={sort() === "name" ? t("home.sortName") : t("home.sortRecent")} labelled triggerClass="filter-chip" items={sortMenu()}>
                {sort() === "name" ? t("home.sortName") : t("home.sortRecent")}
                <Icon name="chevron-down" size={14} />
              </MenuButton>
              <div class="layout-toggle" role="group" aria-label={t("home.layout")}>
                <button aria-pressed={layout() === "list"} aria-label={t("home.viewList")} title={t("home.viewList")} onClick={() => setLayout("list")}>
                  <Icon name="list" />
                </button>
                <button aria-pressed={layout() === "grid"} aria-label={t("home.viewGrid")} title={t("home.viewGrid")} onClick={() => setLayout("grid")}>
                  <Icon name="apps" />
                </button>
              </div>
            </div>
          </Show>

          <div class="page-scroll home-cards">
            <Show
              when={state.peers.length > 0}
              fallback={
                <Empty icon="devices" title={t("home.emptyTitle")}>
                  {t("home.emptyBody")}
                </Empty>
              }
            >
              <Show when={devices().length > 0} fallback={<p class="muted small">{t("home.noneShown")}</p>}>
                <Show
                  when={layout() === "list"}
                  fallback={
                    <div class="home-grid">
                      <For each={devices()}>
                        {(p) => (
                          <HomeCard peer={p} chosen={!props.onlyDevices && target() === p.node_id} seen={lastSeen(p.node_id)} onOpen={props.onOpen} />
                        )}
                      </For>
                    </div>
                  }
                >
                  <ul class="home-list">
                    <For each={devices()}>
                      {(p) => (
                        <HomeRow peer={p} chosen={!props.onlyDevices && target() === p.node_id} seen={lastSeen(p.node_id)} onOpen={props.onOpen} />
                      )}
                    </For>
                  </ul>
                </Show>
              </Show>
            </Show>
          </div>

          <Show when={!props.onlyDevices && layout() === "grid" && target()}>
            {(node) => (
              <div class="home-bottom">
                <RecentActivity limit={3} onViewAll={props.onActivity} />
                <section class="tile dash-card" aria-labelledby="home-quick">
                  {quickHead(node())}
                  <QuickGrid node={node()} onGo={(tab, tile) => props.onOpen(node(), tab, tile)} />
                </section>
              </div>
            )}
          </Show>
        </div>

        <Show when={rail()}>
          <aside class="home-rail">
            <section class="tile dash-card" aria-labelledby="home-net">
              <div class="dash-head">
                <Icon name="wifi" tone="green" />
                <h2 id="home-net">{t("home.netStatus")}</h2>
              </div>
              <p class={`home-net-word ${relayed() + offline() === 0 ? "tone-green" : "tone-amber"}`}>
                <span class="hero-dot" aria-hidden="true" />
                {relayed() + offline() === 0 ? t("home.netHealthy") : t("home.netMixed")}
              </p>
              <p class="muted small">{t("home.netSummary", { direct: direct(), relayed: relayed(), offline: offline() })}</p>
              <Show when={latency() != null}>
                <p class="small">
                  <span class="muted">{t("home.latency")}</span> <strong>{latency()} ms</strong>
                </p>
              </Show>
            </section>
            <Show when={target()}>
              {(node) => (
                <section class="tile dash-card" aria-labelledby="home-quick">
                  {quickHead(node())}
                  <QuickGrid node={node()} vertical onGo={(tab, tile) => props.onOpen(node(), tab, tile)} />
                </section>
              )}
            </Show>
            <button class="tile dash-card home-protect" onClick={props.onHealth}>
              <IconChip name={problemCount() > 0 ? "alert" : "verify"} tone={problemCount() > 0 ? "amber" : "green"} size={22} />
              <span class="grow">
                <strong>{problemCount() > 0 ? tn("dash.problems", problemCount()) : t("home.protected")}</strong>
                <Show when={report() && ranAt()}>
                  <span class="muted small">{t("home.lastCheck", { ago: timeAgo(ranAt()) })}</span>
                </Show>
              </span>
              <Icon name="chevron-right" />
            </button>
          </aside>
        </Show>
      </div>

      <Show when={adding()}>
        <AddDialog onClose={() => setAdding(false)} />
      </Show>
    </section>
  );
}

/** What a card and a row share: the device's state in words and its
 *  actions. */
function useDevice(props: { peer: Peer; seen: number | null; onOpen: Open }) {
  const node = () => props.peer.node_id;
  const name = () => peerName(node());
  const s = () => linkStateOf(node());
  const live = () => s() === "direct" || s() === "relay";
  const st = () => state.status[node()];
  const has = (a: string) => !!st()?.actions.includes(a);
  const phone = () => pageKind(node()) === "phone";
  const [canWake, setCanWake] = createSignal(false);
  createEffect(on(() => [node(), st()] as const, ([n]) => commands.canWake(n).then(setCanWake, () => setCanWake(false))));
  const wake = () =>
    commands.wakeDevice(node()).then(
      (r) => (r.status === "ok" ? toast(t("wake.sent"), "ok") : toast(friendlyError(r.error), "error")),
      (e) => toast(friendlyError(e), "error"),
    );
  const doConnect = () => connect(node()).catch((e) => toast(friendlyError(e), "error"));
  /** "Online" / "Asleep" / "Offline": the same word on every screen. */
  const word = () => presenceLabel(s());
  const tone = () => presenceTone(s());
  /** Local network / Direct / Relay (only Relay is tinted). */
  const route = () => routeOf(s(), mapDeviceHow(node()));
  const when = () => (live() ? t("home.activeNow") : props.seen != null ? timeAgo(Date.now() - props.seen * 1000) : "");
  type Act = { icon: IconName; label: string; run: () => unknown; off?: string; lead?: boolean };
  const screen = (): Act => ({
    icon: "screen",
    label: t("quick.screen"),
    run: () => props.onOpen(node(), "remote", "screen"),
    off: has("screen") ? undefined : t("quick.noScreen", { name: name() }),
  });
  const chat = (): Act => ({ icon: "chat", label: t("quick.chat"), run: () => props.onOpen(node(), "messages", "chat") });
  const call = (): Act => ({
    icon: "call",
    label: t("control.dial"),
    run: () => props.onOpen(node(), "remote", "control"),
    off: has("dial") ? undefined : t("quick.noCall", { name: name() }),
  });
  /** The three that fit: a phone, a computer, or one that isn't connected. */
  const actions = (): Act[] => {
    const files: Act = { icon: "files", label: t("files.title"), run: () => props.onOpen(node(), "files", "files") };
    // Not reachable: Wake leads when it can be woken, then Connect (the
    // card itself opens the details).
    if (!live())
      return canWake()
        ? [
            { icon: "power", label: t("wake.button"), run: wake, lead: true },
            { icon: "connect", label: t("device.connect"), run: doConnect },
          ]
        : [{ icon: "connect", label: t("device.connect"), run: doConnect, lead: true }];
    if (phone()) return [screen(), chat(), files];
    return [screen(), { icon: "send-clip", label: t("quick.clipboard"), run: () => quickSendClipboard(node()) }, files];
  };
  const more = (): MenuItem[] => [
    { icon: "send-file", label: t("quick.sendFile"), onSelect: () => quickSendFile(node()) },
    { icon: "send-clip", label: t("quick.sendClipboard"), onSelect: () => quickSendClipboard(node()) },
    { icon: "ring", label: t("control.ring"), onSelect: () => quickRing(node()) },
    { icon: "devices", label: t("palette.open", { name: name() }), divider: true, onSelect: () => props.onOpen(node()) },
  ];
  return { node, name, s, live, st, word, tone, route, when, actions, more, screen, chat, call };
}
/** How this computer reaches it now ("lan", "relay"...), from the map's line. */
const mapDeviceHow = (node: string) => {
  const v = mapView();
  return v ? (myEdge(v, node)?.how ?? "") : "";
};

/** Open the card or row: a click anywhere that isn't one of its buttons. */
const opener = (onOpen: () => void) => (e: MouseEvent) => {
  if (!(e.target as HTMLElement).closest("button, a, [role=menu]")) onOpen();
};

/** Grid: art, name with a tick, a status pill, OS · last seen, a stats
 *  line, three actions and More. */
function HomeCard(props: { peer: Peer; chosen: boolean; seen: number | null; onOpen: Open }) {
  const d = useDevice(props);
  // A laptop or monitor is wide: drawn a little lower, so the card's
  // buttons keep their room.
  const wide = () => ["laptop", "monitor"].includes(frameOf(peerIcon(d.node())));
  return (
    <article class="tile home-card" classList={{ chosen: props.chosen }} data-drop-node={d.node()} onClick={opener(() => props.onOpen(d.node()))}>
      {/* The device as it looks: its frame with its wallpaper. */}
      <div class="device-art home-art">
        <DeviceArt node={d.node()} icon={peerIcon(d.node())} size={wide() ? 64 : 84} />
      </div>
      <div class="home-card-main">
        <div class="home-card-top">
          <h2 class="device-card-name">
            <button type="button" class="device-open" aria-describedby={`home-card-state-${d.node()}`} onClick={() => props.onOpen(d.node())}>
              {d.name()}
            </button>
          </h2>
          <Show when={isVerified(d.node())}>
            <span class="verified-tick" title={t("verify.badgeHint")}>
              <Icon name="verify" size={16} tone="cyan" />
              <span class="sr-only">{t("verify.badge")}</span>
            </span>
          </Show>
          <span class="grow" />
          <span class={`chip chip-sm status-chip tone-${d.tone()}`} id={`home-card-state-${d.node()}`}>
            <span class="hero-dot" aria-hidden="true" />
            {d.word()}
          </span>
        </div>
        <p class="muted small home-facts">
          {[osName(d.node()), d.live() ? d.when() : d.when() && t("home.lastSeen", { ago: d.when() })].filter(Boolean).join(" · ")}
        </p>
        {/* Live: battery, free storage and the route. Not live: nothing
            to show but how to get it. */}
        <Show
          when={d.live()}
          fallback={<p class="muted small home-statline">{d.actions()[0]?.icon === "power" ? t("home.wakeToSee") : t("home.connectToSee")}</p>}
        >
          <p class="home-statline small">
            <DeviceFactBits d={d} />
          </p>
        </Show>
        <div class="home-actions">
          <For each={d.actions()}>
            {(a) => (
              <button
                class="btn btn-sm"
                classList={{ "btn-soft": !a.lead, "btn-lead": !!a.lead }}
                disabled={!!a.off}
                title={a.off ?? a.label}
                onClick={() => a.run()}
              >
                <Icon name={a.icon} size={16} />
                <span class="btn-label">{a.label}</span>
                <Show when={a.off}>
                  <span class="sr-only">{t("a11y.unavailable", { reason: a.off ?? "" })}</span>
                </Show>
              </button>
            )}
          </For>
          <MenuButton label={t("device.more", { name: d.name() })} triggerClass="btn btn-soft btn-sm btn-icon" items={d.more()}>
            <Icon name="more" size={16} />
          </MenuButton>
        </div>
      </div>
    </article>
  );
}

/** List: small art, name with its OS, status · when, location; battery,
 *  storage and memory; round actions. */
function HomeRow(props: { peer: Peer; chosen: boolean; seen: number | null; onOpen: Open }) {
  const d = useDevice(props);
  const loc = () => d.st()?.location;
  const icon = (a: { icon: IconName; label: string; run: () => unknown; off?: string }) => {
    const off = () => a.off ?? (d.live() ? undefined : t("palette.offline", { name: d.name() }));
    return (
      <button class="round-btn" disabled={!!off()} aria-label={off() ? `${a.label}. ${off()}` : a.label} title={off() ?? a.label} onClick={() => a.run()}>
        <Icon name={a.icon} />
      </button>
    );
  };
  return (
    <li class="tile home-row" classList={{ chosen: props.chosen }} data-drop-node={d.node()} onClick={opener(() => props.onOpen(d.node()))}>
      <div class="device-art home-art-sm">
        <DeviceArt node={d.node()} icon={peerIcon(d.node())} size={48} />
      </div>
      <div class="home-row-main">
        <h2 class="device-card-name">
          <button type="button" class="device-open" onClick={() => props.onOpen(d.node())}>
            {d.name()}
          </button>
        </h2>
        {/* Presence, when, and the OS under the name. */}
        <p class="home-status small">
          <span class={`home-presence tone-${d.tone()}`}>
            <span class="hero-dot" aria-hidden="true" />
            {d.word()}
          </span>
          <span class="muted home-status-rest">
            {[d.when() && (d.live() ? d.when() : t("home.lastSeen", { ago: d.when() })), osName(d.node())].filter(Boolean).join(" · ")}
          </span>
        </p>
        <Show when={loc()}>
          {(l) => (
            <p class="muted small home-loc">
              <Icon name="find" size={13} />
              {`${l().lat.toFixed(2)}, ${l().lon.toFixed(2)}`}
            </p>
          )}
        </Show>
      </div>
      <div class="home-stats-chips">
        <Show when={d.live()}>
          <DeviceFactBits d={d} compact />
        </Show>
      </div>
      <div class="home-round">
        {icon(d.screen())}
        {icon(d.chat())}
        {icon(d.call())}
        <MenuButton label={t("device.more", { name: d.name() })} triggerClass="round-btn" items={d.more()}>
          <Icon name="more" />
        </MenuButton>
      </div>
    </li>
  );
}

/** The three facts a card and a row both show: battery, free storage, and
 *  the route (Local network / Direct / Relay). Only what's known. */
function DeviceFactBits(props: { d: ReturnType<typeof useDevice>; compact?: boolean }) {
  const st = () => props.d.st();
  return (
    <>
      <Show when={st()?.battery != null}>
        <span class="home-stat" title={t("info.battery")}>
          <Icon name="battery" size={14} />
          {st()!.battery}%<span class="sr-only"> {t("info.battery")}</span>
        </span>
      </Show>
      <Show when={st()?.storage}>
        {(x) => (
          <span class="home-stat" title={`${t("info.storage")}: ${t("dash.free", { free: humanSize(x().free) })}`}>
            <Icon name="storage" size={14} />
            {props.compact ? humanSize(x().free) : t("dash.free", { free: humanSize(x().free) })}
            <Show when={props.compact}>
              <span class="sr-only"> {t("dash.free", { free: "" }).trim()}</span>
            </Show>
          </span>
        )}
      </Show>
      <RouteChip route={props.d.route()} compact={props.compact} />
    </>
  );
}

/** The page's one primary: Add device (plus), with the map and Refresh in
 *  its menu. The same button on Overview and All devices. */
export function AddDeviceButton(props: { onAdd: () => void; onMap: () => void }) {
  const items = (): MenuItem[] => [
    { icon: "plus", label: t("join.addDevice"), onSelect: props.onAdd },
    { icon: "map", label: t("map.openMap"), divider: true, onSelect: props.onMap },
    {
      icon: "refresh",
      label: t("home.refresh"),
      onSelect: () =>
        refreshDevices().then(
          () => toast(t("home.refreshed"), "ok"),
          (e) => toast(friendlyError(e), "error"),
        ),
    },
  ];
  return (
    <MenuButton label={t("home.addDevice")} labelled triggerClass="btn btn-primary" items={items()}>
      <Icon name="plus" />
      {t("home.addDevice")}
      <Icon name="chevron-down" size={16} />
    </MenuButton>
  );
}

/** Add device: the existing invite flow (QR code and code) in a dialog. */
export function AddDialog(props: { onClose: () => void }) {
  let box: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview add-dialog"
          ref={box}
          role="dialog"
          aria-modal="true"
          aria-label={t("join.addDevice")}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, box);
          }}
        >
          <AddDevice startOpen onClose={close} />
        </div>
      </div>
    </Portal>
  );
}

/** The four counts, each an icon chip, a value and a one-line label:
 *  devices, online, problems from the health check (opens it), direct links
 *  (and how many go through the relay). */
export function DeviceStats(props: { onHealth: () => void }) {
  const online = () => state.peers.filter((p) => !!state.links[p.node_id]).length;
  const relayed = () => state.peers.filter((p) => state.links[p.node_id] === "relay").length;
  const direct = () => online() - relayed();
  const problems = () => problemCount();
  return (
    <div class="home-stats">
      <div class="tile stat-card">
        <IconChip name="devices" tone="cyan" size={20} />
        <div class="stat-text">
          <strong class="stat-value">{state.peers.length}</strong>
          <span class="small muted stat-label">{t("home.total")}</span>
        </div>
      </div>
      <div class="tile stat-card">
        <IconChip name="circle-dot" tone="green" size={20} />
        <div class="stat-text">
          <strong class="stat-value">{online()}</strong>
          <span class="small muted stat-label">{t("home.online")}</span>
        </div>
      </div>
      <button class="tile stat-card stat-link" onClick={props.onHealth}>
        <IconChip name={problems() > 0 ? "alert" : "check"} tone={problems() > 0 ? "amber" : "green"} size={20} />
        <span class="stat-text">
          <strong class="stat-value">{problems()}</strong>
          <span class="small muted stat-label">{problems() > 0 ? tn("home.issues", problems()) : t("home.noIssues")}</span>
        </span>
      </button>
      <div class="tile stat-card">
        <IconChip name="link-direct" tone="violet" size={20} />
        <div class="stat-text">
          <strong class="stat-value">{direct()}</strong>
          <span class="small muted stat-label">
            {relayed() > 0 ? `${t("home.direct")} · ${t("home.relayed", { n: relayed() })}` : t("home.direct")}
          </span>
        </div>
      </div>
    </div>
  );
}
