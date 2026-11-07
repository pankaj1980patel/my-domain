// Settings → Health: every check, green / grey / amber / red, worst first,
// each with the one button that fixes it. "Run again" asks the server too.
// Opened from Settings, the sidebar's badge, Home's card and a device's
// page when something is wrong there.

import { Icon } from "../components/Icon";
import { createSignal, For, onMount, Show } from "solid-js";
import { commands } from "../bindings";
import { connect, unwrap } from "../lib/core";
import {
  checkDetail,
  checkTitle,
  fixLabel,
  levelLabel,
  problemCount,
  ranAt,
  report,
  runHealth,
  running,
  warnCount,
  type HealthCheck,
} from "../lib/health";
import { friendlyError, timeAgo } from "../lib/words";
import { toast } from "../components/ui";
import { HelpLink } from "../help/HelpLink";
import { t, tn } from "../i18n";
import { IconChip } from "../components/Icon";
import { isVerified } from "../lib/verify";
import { state } from "../lib/core";

export type HealthNav = {
  onDevice: (node: string) => void;
  onMap: () => void;
  onSettings: (section: string) => void;
  onHelp: (topic: string) => void;
  /** The security log: Activity. */
  onActivity?: () => void;
  /** All devices (to verify them). */
  onDevices?: () => void;
};

/** What a check's button does. */
export async function fixCheck(c: HealthCheck, nav: HealthNav) {
  try {
    switch (c.fix) {
      case "retry":
        await runHealth(true);
        return;
      case "push_test":
        await unwrap(commands.fcmSelftest());
        toast(t("health.pushSent"), "ok");
        setTimeout(() => runHealth(false), 8000);
        return;
      case "open_permission":
        // Linux tools and portals have no settings page: how to install
        // them is in Help.
        if (/^(linux_|tool_)/.test(c.detail ?? "")) {
          nav.onHelp("troubleshooting");
          return;
        }
        await openPermissionPage(c);
        return;
      case "allow_permission": {
        // macOS's own prompt; the command returns once it's answered.
        const id = c.detail ?? "";
        const state = await unwrap(commands.requestPermission(id));
        await runHealth(false);
        if (id === "local_network") {
          // Its prompt can't be waited for: look again once it's had time.
          setTimeout(() => runHealth(false), 5000);
          setTimeout(() => runHealth(false), 20000);
        } else if (state === "denied") {
          toast(t("health.permDeniedHint"));
        }
        return;
      }
      case "free_space":
        await unwrap(commands.openDownloads());
        return;
      case "trust_key":
      case "permissions":
        if (c.node_id) nav.onDevice(c.node_id);
        return;
      case "encryption_key":
        nav.onSettings("settings-key");
        return;
      case "connect":
        if (c.node_id) {
          await connect(c.node_id);
          setTimeout(() => runHealth(false), 3000);
        }
        return;
      case "links_map":
        nav.onMap();
        return;
      case "update_other":
      case "update_app":
        nav.onHelp("troubleshooting");
        return;
    }
  } catch (e) {
    toast(friendlyError(e), "error");
  }
}

async function openPermissionPage(c: HealthCheck) {
  try {
    if (!(await unwrap(commands.openPermissionPage(c.detail ?? "")))) toast(t("health.noPage"));
  } catch (e) {
    toast(friendlyError(e), "error");
  }
}

const LEVEL_ICON: Record<string, "ok" | "alert" | "no" | "info"> = { ok: "ok", warn: "alert", bad: "no", info: "info" };

/** One line: an icon for the level, what it is, why, and its fix. A
 *  permission the OS can still ask for gets "Allow" (its prompt) and its
 *  settings page beside it. */
export function HealthRow(props: { check: HealthCheck; nav: HealthNav }) {
  const c = () => props.check;
  const [busy, setBusy] = createSignal(false);
  const fix = async () => {
    setBusy(true);
    try {
      await fixCheck(c(), props.nav);
    } finally {
      setBusy(false);
    }
  };
  return (
    <li class={`health-row health-${c().level}`}>
      {/* The level as an icon, not colour alone. */}
      <Icon name={LEVEL_ICON[c().level] ?? "info"} size={16} class="health-icon" />
      <div class="health-text">
        <strong>{checkTitle(c())}</strong>
        <span class="sr-only">, {levelLabel(c())}</span>
        <Show when={checkDetail(c())}>
          <span class="muted small">{checkDetail(c())}</span>
        </Show>
      </div>
      <Show when={c().fix === "allow_permission" && c().level !== "ok"}>
        <button class="btn btn-ghost btn-sm" onClick={() => openPermissionPage(c())}>
          {t("health.fix.open_permission")}
        </button>
      </Show>
      <Show when={c().fix && c().level !== "ok"}>
        <button class="btn btn-soft btn-sm" disabled={busy()} aria-busy={busy()} onClick={fix}>
          {busy() && c().fix === "allow_permission" ? t("health.asking") : fixLabel(c())}
        </button>
      </Show>
    </li>
  );
}

export default function Health(props: HealthNav) {
  const verified = () => state.peers.filter((p) => isVerified(p.node_id)).length;
  const [sessions, setSessions] = createSignal<number | null>(null);
  onMount(() => unwrap(commands.accountSessions()).then((l) => setSessions(l.length), () => {}));
  const summary = () => {
    const r = report();
    if (!r) return "";
    if (r.bad === 0 && r.warn === 0) return t("health.allGood");
    const parts: string[] = [];
    if (r.bad) parts.push(tn("health.summaryBad", r.bad));
    if (r.warn) parts.push(tn("health.summaryWarn", r.warn));
    return parts.join(" · ");
  };
  return (
    <section class="page page-fit health-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="security" size={26} tone="green" />
        </span>
        <div class="grow">
          <h1>
            {t("nav.securityHealth")} <HelpLink topic="troubleshooting" />
          </h1>
          <p class="muted">{t("sec.sub")}</p>
        </div>
      </header>
      <div class="sec-body">
      <div class="sec-left">
        {/* The verdict, then what it rests on. */}
        <section class="tile sec-status" classList={{ "sec-bad": problemCount() > 0 }}>
          <IconChip name={problemCount() > 0 ? "alert" : "verify"} tone={problemCount() > 0 ? "amber" : "green"} size={30} />
          <div class="grow">
            <span class="small muted">{t("sec.overall")}</span>
            <strong class="sec-word">
              {problemCount() > 0
                ? tn("dash.problems", problemCount())
                : warnCount() > 0
                  ? tn("health.summaryWarn", warnCount())
                  : t("sec.allGood")}
            </strong>
            <span class="small muted">{summary()}</span>
          </div>
          <div class="sec-actions">
            <button class="btn btn-primary btn-sm" disabled={running()} onClick={() => runHealth(true)}>
              <Icon name="health" />
              {running() ? t("health.running") : t("sec.runFull")}
            </button>
            <Show when={props.onActivity}>
              <button class="btn btn-soft btn-sm" onClick={() => props.onActivity?.()}>
                <Icon name="activity" />
                {t("sec.log")}
              </button>
            </Show>
          </div>
        </section>
        <div class="sec-stats">
          <div class="tile stat-card">
            <IconChip name="verify" tone="green" />
            <div class="stat-text">
              <strong class="stat-value">
                {verified()} / {state.peers.length}
              </strong>
              <span class="small muted">{t("sec.verified")}</span>
            </div>
          </div>
          <div class="tile stat-card">
            <IconChip name="user" tone="blue" />
            <div class="stat-text">
              <strong class="stat-value">{sessions() ?? "—"}</strong>
              <span class="small muted">{t("sec.sessions")}</span>
            </div>
          </div>
          <div class="tile stat-card">
            <IconChip name="clock" tone="violet" />
            <div class="stat-text">
              <strong class="stat-value sec-when">{ranAt() ? timeAgo(ranAt()) : "—"}</strong>
              <span class="small muted">{t("sec.lastCheck")}</span>
            </div>
          </div>
        </div>
        <ul class="sec-rows">
          <li>
            <button class="sec-row" onClick={() => props.onSettings("settings-permissions")}>
              <IconChip name="lock" tone="amber" />
              <span class="grow">
                <strong>{t("perm.summaryTitle")}</strong>
                <span class="muted small">{t("sec.permsBody")}</span>
              </span>
              <Icon name="chevron-right" />
            </button>
          </li>
          <li>
            <button class="sec-row" onClick={() => (props.onDevices ? props.onDevices() : props.onMap())}>
              <IconChip name="verify" tone={verified() < state.peers.length ? "amber" : "green"} />
              <span class="grow">
                <strong>{t("sec.verification")}</strong>
                <span class="muted small">{t("sec.verificationBody", { n: verified(), total: state.peers.length })}</span>
              </span>
              <Icon name="chevron-right" />
            </button>
          </li>
          <li>
            <button class="sec-row" onClick={() => props.onSettings("settings-key")}>
              <IconChip name="key" tone="blue" />
              <span class="grow">
                <strong>{t("sec.encryption")}</strong>
                <span class="muted small">{t("sec.encryptionBody")}</span>
              </span>
              <Icon name="chevron-right" />
            </button>
          </li>
        </ul>
      </div>
      <Show when={report()} fallback={<p class="muted">{t("health.running")}</p>}>
        {(r) => (
          <article class="tile sec-checks">
            <h2>{t("sec.checks")}</h2>
            <p class="muted small">{t("health.subtitle")}</p>
            <p role="status">
              <strong>{summary()}</strong>
              <Show when={ranAt()}>
                <span class="muted small"> · {t("health.lastRun", { ago: timeAgo(ranAt()) })}</span>
              </Show>
            </p>
            <ul class="health-list">
              <For each={r().checks}>{(c) => <HealthRow check={c} nav={props} />}</For>
            </ul>
          </article>
        )}
      </Show>
      </div>
    </section>
  );
}
