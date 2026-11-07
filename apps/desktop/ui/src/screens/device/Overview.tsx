// A device's Overview: a dashboard. Ring gauges for what it last said about
// itself (battery, storage, memory) and the link; its recent activity; big
// one-click actions (each only when the device offers it); on the right,
// the health check's word on it, what it may do here, and what it is.

import { createEffect, createSignal, For, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type ActivityItem_Serialize as ActivityItem, type PeerPermissions } from "../../bindings";
import { peerModel, peerName, runAction, state, type Space } from "../../lib/core";
import { activityIcon, activityText } from "../../lib/activity";
import { deviceProblems, runHealth, running } from "../../lib/health";
import { humanSize, linkLabel, timeAgo, friendlyError } from "../../lib/words";
import { linkStateOf, toast } from "../../components/ui";
import { Icon, IconChip, type IconName, type Tone } from "../../components/Icon";
import { quickRing, quickSendClipboard, quickSendFile } from "../../lib/quickActions";
import { osName } from "../../lib/deviceInfo";
import { t, type Key } from "../../i18n";

/** Activity kinds by colour: files blue, messages violet, calls green... */
const KIND_TONE: Record<string, Tone> = {
  files: "blue",
  clipboard: "cyan",
  messages: "violet",
  notifications: "violet",
  calls: "green",
  connections: "cyan",
  other: "amber",
};

export default function Overview(props: {
  node: string;
  /** Open a tab of the page (at one of its tiles). */
  onGo: (tab: string, tile?: string) => void;
  onActivity: () => void;
}) {
  const st = () => state.status[props.node];
  const name = () => peerName(props.node);
  const s = () => linkStateOf(props.node);
  const live = () => s() === "direct" || s() === "relay";

  // --- recent activity (the timeline, this device only) ---------------------
  const [recent, setRecent] = createSignal<ActivityItem[] | null>(null);
  const loadRecent = () =>
    commands
      .activityFeed({ node: props.node, kinds: null, text: null, before_ts: null, before_id: null, limit: 4 })
      .then((p) => setRecent(p.items.slice(0, 4)), () => setRecent([]));
  createEffect(on(() => props.node, loadRecent));
  onMount(() => {
    const off = listen("activity-changed", loadRecent);
    onCleanup(() => off.then((f) => f()).catch(() => {}));
  });

  // --- permissions (what it may do on this computer) ------------------------
  const [perms, setPerms] = createSignal<PeerPermissions>();
  createEffect(on(() => props.node, (n) => commands.peerPermissions(n).then(setPerms, () => {})));

  // --- gauges ------------------------------------------------------------------
  const used = (x: Space | undefined) => (x && x.total > 0 ? Math.round(((x.total - x.free) / x.total) * 100) : undefined);
  const batteryTone = (b: number): Tone => (b <= 15 ? "red" : b <= 35 ? "amber" : "green");
  const linkPct = () => {
    if (!live()) return 0;
    const q = state.quality[props.node];
    return q === "excellent" ? 100 : q === "good" ? 70 : q === "weak" ? 35 : 100;
  };

  // --- quick actions -------------------------------------------------------------
  const has = (a: string) => !!st()?.actions.includes(a);
  const locate = () =>
    runAction(props.node, "locate", {})
      .then(() => toast(t("locate.asking")))
      .catch((e) => toast(friendlyError(e), "error"));
  const actions = (): { icon: IconName; tone: Tone; label: string; run: () => unknown; off?: string }[] => {
    const out: { icon: IconName; tone: Tone; label: string; run: () => unknown; off?: string }[] = [
      { icon: "send-file", tone: "blue", label: t("quick.sendFile"), run: () => quickSendFile(props.node) },
      { icon: "send-clip", tone: "cyan", label: t("quick.sendClipboard"), run: () => quickSendClipboard(props.node) },
    ];
    if (has("screen"))
      out.push({
        icon: "screen",
        tone: "blue",
        label: st()?.platform === "android" ? t("phoneScreen.title") : t("screen.title"),
        run: () => props.onGo("remote", "screen"),
      });
    if (has("dial")) out.push({ icon: "call", tone: "green", label: t("control.dial"), run: () => props.onGo("remote", "control") });
    out.push({ icon: "ring", tone: "amber", label: t("control.ring"), run: () => quickRing(props.node) });
    if (has("locate"))
      out.push({
        icon: "find",
        tone: "red",
        label: t("locate.locate"),
        run: locate,
        off: st()?.locationSharing ? undefined : t("locate.offOn", { name: name() }),
      });
    return out;
  };

  // --- health ------------------------------------------------------------------------
  const problems = () => deviceProblems(props.node);
  const bad = () => problems().some((c) => c.level === "bad");

  return (
    <div class="dash">
      <div class="dash-main">
        {/* What it last said about itself, and the link. */}
        <div class="stat-row">
          <Show when={st()?.battery !== undefined}>
            <Stat
              pct={st()!.battery!}
              tone={batteryTone(st()!.battery!)}
              icon="battery"
              value={`${st()!.battery}%`}
              label={t("info.battery")}
              sub={st()?.charging ? t("dash.charging") : undefined}
            />
          </Show>
          <Show when={used(st()?.storage) !== undefined}>
            <Stat
              pct={used(st()?.storage)!}
              tone="violet"
              icon="storage"
              value={`${used(st()?.storage)}%`}
              label={t("info.storage")}
              sub={t("dash.free", { free: humanSize(st()!.storage!.free) })}
            />
          </Show>
          <Show when={used(st()?.ram) !== undefined}>
            <Stat
              pct={used(st()?.ram)!}
              tone="blue"
              icon="memory"
              value={`${used(st()?.ram)}%`}
              label={t("info.memory")}
              sub={`${humanSize(st()!.ram!.total - st()!.ram!.free)} / ${humanSize(st()!.ram!.total)}`}
            />
          </Show>
          <Stat
            pct={linkPct()}
            tone={s() === "relay" ? "amber" : live() ? "green" : "red"}
            icon={live() ? "wifi" : "link-offline"}
            value={live() ? t("dash.online") : t("dash.offline")}
            label={t("dash.connection")}
            sub={linkLabel(s())}
          />
        </div>

        <div class="dash-pair">
          <section class="tile dash-card" aria-labelledby="dash-recent">
            <div class="dash-head">
              <Icon name="activity" tone="violet" />
              <h2 id="dash-recent">{t("dash.recent")}</h2>
              <button class="link small dash-more" onClick={props.onActivity}>
                {t("dash.viewAll")}
                <Icon name="arrow-right" size={14} />
              </button>
            </div>
            <Show when={recent()?.length} fallback={<p class="muted small">{recent() ? t("dash.noActivity", { name: name() }) : "…"}</p>}>
              <ul class="dash-feed">
                <For each={recent()}>
                  {(a) => (
                    <li>
                      <IconChip name={activityIcon(a)} tone={KIND_TONE[a.kind] ?? "cyan"} />
                      <span class="dash-feed-text">{activityText(a, name())}</span>
                      <span class="muted small dash-feed-time">{timeAgo(a.ts ?? 0)}</span>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </section>

          <section class="tile dash-card" aria-labelledby="dash-quick">
            <div class="dash-head">
              <Icon name="quick" tone="amber" />
              <h2 id="dash-quick">{t("dash.quick")}</h2>
            </div>
            <div class="quick-grid">
              <For each={actions()}>
                {(a) => (
                  <button class="quick-tile" disabled={!!a.off} title={a.off ?? a.label} onClick={() => a.run()}>
                    <Icon name={a.icon} tone={a.tone} size={24} />
                    <span>{a.label}</span>
                    <Show when={a.off}>
                      <span class="sr-only">{t("a11y.unavailable", { reason: a.off ?? "" })}</span>
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </section>
        </div>
      </div>

      <aside class="dash-side">
        <section class="tile dash-card" aria-labelledby="dash-health">
          <div class="dash-health">
            <IconChip name={problems().length ? "alert" : "verify"} tone={bad() ? "red" : problems().length ? "amber" : "green"} size={22} />
            <div class="grow">
              <h2 id="dash-health" class="small muted dash-label">
                {t("dash.health")}
              </h2>
              <p
                class="dash-health-word"
                classList={{ "tone-red": bad(), "tone-amber": !bad() && problems().length > 0, "tone-green": !problems().length }}
              >
                {problems().length ? t("dash.healthBad") : t("dash.healthGood")}
              </p>
            </div>
          </div>
          <p class="muted small">
            {problems().length ? t("dash.healthBadHint", { n: problems().length }) : t("dash.healthGoodHint", { name: name() })}
          </p>
          <button class="btn btn-soft btn-sm" disabled={running()} onClick={() => runHealth(true)}>
            <Icon name="health" />
            {running() ? t("health.running") : t("dash.runCheck")}
          </button>
        </section>

        <section class="tile dash-card" aria-labelledby="dash-perms">
          <div class="dash-head">
            <Icon name="lock" tone="amber" />
            <h2 id="dash-perms">{t("dash.perms")}</h2>
            <button class="link small dash-more" onClick={() => props.onGo("settings", "permissions")}>
              {t("dash.manage")}
              <Icon name="arrow-right" size={14} />
            </button>
          </div>
          <ul class="dash-checks">
            <For each={["control", "screen", "kvm"] as const}>
              {(k) => (
                <li>
                  <Icon name={perms()?.[k] ? "ok" : "no"} tone={perms()?.[k] ? "green" : undefined} />
                  <span classList={{ muted: !perms()?.[k] }}>{cap(t(`perm.name.${k}` as Key))}</span>
                  <span class="sr-only">{perms()?.[k] ? t("dash.allowed") : t("dash.notAllowed")}</span>
                </li>
              )}
            </For>
          </ul>
        </section>

      </aside>
    </div>
  );
}

const cap = (s: string) => s.charAt(0).toLocaleUpperCase() + s.slice(1);

/** One gauge: a ring filled to `pct`, the value, its name and a line under. */
function Stat(props: { pct: number; tone: Tone; icon: IconName; value: string; label: string; sub?: string }): JSX.Element {
  const r = 21;
  const c = 2 * Math.PI * r;
  return (
    <div class="tile stat-card">
      <div class={`ring tone-${props.tone}`} role="img" aria-label={[props.label, props.value, props.sub].filter(Boolean).join(", ")}>
        <svg width="52" height="52" viewBox="0 0 52 52" aria-hidden="true">
          <circle class="ring-track" cx="26" cy="26" r={r} />
          <circle
            class="ring-fill"
            cx="26"
            cy="26"
            r={r}
            stroke-dasharray={`${(c * Math.max(0, Math.min(100, props.pct))) / 100} ${c}`}
            transform="rotate(-90 26 26)"
          />
        </svg>
        <Icon name={props.icon} size={18} class="ring-icon" />
      </div>
      <div class="stat-text" aria-hidden="true">
        <strong class="stat-value">{props.value}</strong>
        <span class="small muted">{props.label}</span>
        <Show when={props.sub}>
          <span class="small stat-sub">{props.sub}</span>
        </Show>
      </div>
    </div>
  );
}

/** What the device is (model, OS, address, where it said it was): the
 *  hero's facts. Only what's known. */
export function DeviceFacts(props: { node: string }) {
  const st = () => state.status[props.node];
  const peer = () => state.peers.find((p) => p.node_id === props.node);
  const info = (): [Key, string][] => {
    const rows: [Key, string][] = [];
    const model = peerModel(props.node);
    if (model) rows.push(["dash.model", model]);
    const os = osName(props.node);
    if (os) rows.push(["dash.os", os]);
    const ip = peer()?.ip;
    if (ip && ip !== "0.0.0.0") rows.push(["dash.ip", ip]);
    const loc = st()?.location;
    if (loc) rows.push(["dash.location", `${loc.lat.toFixed(3)}, ${loc.lon.toFixed(3)}`]);
    return rows;
  };
  return (
    <Show when={info().length}>
      <dl class="dash-info hero-facts-list" aria-label={t("dash.info")}>
        <For each={info()}>
          {([k, v]) => (
            <>
              <dt class="muted small">{t(k)}</dt>
              <dd class="small">{v}</dd>
            </>
          )}
        </For>
      </dl>
    </Show>
  );
}
