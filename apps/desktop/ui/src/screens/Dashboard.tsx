// Overview: a greeting by the time of day, the four counts, Quick actions
// for a chosen device, your devices at a glance, recent activity, and
// "What's next": suggestions from what isn't set up yet (each gone once it
// is).

import { createMemo, createSignal, For, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { peerIcon, peerName, state } from "../lib/core";
import { t, tn } from "../i18n";
import { problemCount } from "../lib/health";
import { isVerified } from "../lib/verify";
import { linkStateOf } from "../components/ui";
import { presenceLabel, presenceTone } from "../lib/words";
import { Icon, IconChip, type IconName, type Tone } from "../components/Icon";
import { MenuButton, type MenuItem } from "../components/Menu";
import { RecentActivity } from "../components/RecentActivity";
import { QuickGrid } from "../components/QuickGrid";
import DeviceArt from "../components/DeviceArt";
import { AddDeviceButton, AddDialog, DeviceStats } from "./Home";
import { SetupCard } from "./Setup";

const STRIP = 4;

/** "pankaj" -> "Pankaj". */
const cap = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

export default function Dashboard(props: {
  onOpen: (node: string, tab?: string, tile?: string) => void;
  onAllDevices: () => void;
  onMap: () => void;
  onHealth: () => void;
  onActivity: () => void;
  onSettings: (section: string) => void;
  onAutomations: () => void;
  onNote: () => void;
  onSetup: () => void;
}) {
  const hour = new Date().getHours();
  const greeting = () =>
    t(hour < 12 ? "dash.morning" : hour < 18 ? "dash.afternoon" : "dash.evening", { name: cap(state.account.username || "") });

  const devices = createMemo(() =>
    [...state.peers].sort(
      (a, b) =>
        (state.links[a.node_id] ? 0 : 1) - (state.links[b.node_id] ? 0 : 1) || peerName(a.node_id).localeCompare(peerName(b.node_id)),
    ),
  );
  const [chosen, setChosen] = createSignal<string | null>(null);
  const target = () => (chosen() && state.peers.some((p) => p.node_id === chosen()) ? chosen() : devices()[0]?.node_id ?? null);
  const pick = (): MenuItem[] =>
    devices().map((p) => ({ icon: "devices" as const, label: peerName(p.node_id), onSelect: () => setChosen(p.node_id) }));

  const [adding, setAdding] = createSignal(false);

  // What's next: each only while it isn't done.
  const [automations, setAutomations] = createSignal<number | null>(null);
  onMount(() => commands.listAutomations().then((l) => setAutomations(l.length), () => setAutomations(null)));
  type Tip = { icon: IconName; tone: Tone; title: string; body: string; run: () => void };
  const tips = (): Tip[] => {
    const out: Tip[] = [];
    if (problemCount() > 0)
      out.push({ icon: "alert", tone: "amber", title: tn("next.fix", problemCount()), body: t("next.fixBody"), run: props.onHealth });
    if (state.peers.length < 2)
      out.push({ icon: "plus", tone: "cyan", title: t("next.add"), body: t("next.addBody"), run: () => setAdding(true) });
    const unverified = state.peers.filter((p) => !isVerified(p.node_id)).length;
    if (unverified > 0)
      out.push({ icon: "verify", tone: "green", title: tn("next.verify", unverified), body: t("next.verifyBody"), run: props.onAllDevices });
    if (state.syncPairs.length === 0)
      out.push({ icon: "folder-sync", tone: "blue", title: t("next.sync"), body: t("next.syncBody"), run: () => props.onSettings("settings-sync") });
    if (automations() === 0)
      out.push({ icon: "workflow", tone: "violet", title: t("next.automation"), body: t("next.automationBody"), run: props.onAutomations });
    return out;
  };

  return (
    <section class="page page-fit dash-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="home" size={26} tone="cyan" />
        </span>
        <div class="grow">
          <h1>{greeting()}</h1>
          <p class="muted">{t("dash.sub")}</p>
        </div>
        <AddDeviceButton onAdd={() => setAdding(true)} onMap={props.onMap} />
      </header>

      <SetupCard onOpen={props.onSetup} />

      <Show when={state.peers.length > 0}>
        <DeviceStats onHealth={props.onHealth} />
      </Show>

      <Show when={target()}>
        {(node) => (
          <section class="tile dash-card" aria-labelledby="ov-quick">
            <div class="dash-head">
              <Icon name="quick" tone="amber" />
              <h2 id="ov-quick">{t("dash.quick")}</h2>
              <MenuButton label={t("home.target", { name: peerName(node()) })} labelled triggerClass="link small dash-more home-target" items={pick()}>
                {t("home.for", { name: peerName(node()) })}
                <Icon name="chevron-down" size={14} />
              </MenuButton>
            </div>
            <div class="ov-quick">
              <QuickGrid node={node()} onNote={props.onNote} onGo={(tab, tile) => props.onOpen(node(), tab, tile)} />
            </div>
          </section>
        )}
      </Show>

      <div class="ov-cols">
        <div class="ov-left">
          <section class="tile dash-card" aria-labelledby="ov-devices">
            <div class="dash-head">
              <Icon name="devices" tone="cyan" />
              <h2 id="ov-devices">{t("dash.yourDevices")}</h2>
              <button class="link small dash-more" onClick={props.onAllDevices}>
                {t("dash.viewAll")}
                <Icon name="arrow-right" size={14} />
              </button>
            </div>
            <div class="ov-strip">
              <For each={devices().slice(0, STRIP)}>
                {(p) => {
                  const s = () => linkStateOf(p.node_id);
                  return (
                    <button class="ov-device" onClick={() => props.onOpen(p.node_id)}>
                      <DeviceArt node={p.node_id} icon={peerIcon(p.node_id)} size={52} />
                      <span class="ov-device-text">
                        <strong>{peerName(p.node_id)}</strong>
                        <span class={`small ov-device-state tone-${presenceTone(s())}`}>
                          <span class="hero-dot" aria-hidden="true" />
                          {presenceLabel(s())}
                          <Show when={state.status[p.node_id]?.battery != null}>
                            <span class="muted"> · {state.status[p.node_id]!.battery}%</span>
                          </Show>
                        </span>
                      </span>
                    </button>
                  );
                }}
              </For>
              <Show when={devices().length > STRIP}>
                <button class="ov-device ov-more" onClick={props.onAllDevices}>
                  {t("dash.more", { n: devices().length - STRIP })}
                </button>
              </Show>
            </div>
          </section>
          <RecentActivity limit={3} onViewAll={props.onActivity} />
        </div>
        <section class="tile dash-card ov-next" aria-labelledby="ov-next">
          <div class="dash-head">
            <Icon name="quick" tone="violet" />
            <h2 id="ov-next">{t("next.title")}</h2>
          </div>
          <Show when={tips().length} fallback={<p class="muted small">{t("next.none")}</p>}>
            <ul class="ov-tips">
              <For each={tips()}>
                {(x) => (
                  <li>
                    <button class="ov-tip" onClick={x.run}>
                      <IconChip name={x.icon} tone={x.tone} />
                      <span class="grow">
                        <strong>{x.title}</strong>
                        <span class="muted small">{x.body}</span>
                      </span>
                      <Icon name="chevron-right" />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      </div>

      <Show when={adding()}>
        <AddDialog onClose={() => setAdding(false)} />
      </Show>
    </section>
  );
}
