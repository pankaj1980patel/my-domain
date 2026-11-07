// Settings → Health: every check, green / grey / amber / red, worst first,
// each with the one button that fixes it. "Run again" asks the server too.
// Opened from Settings, the sidebar's badge, Home's card and a device's
// page when something is wrong there.

import { createSignal, For, Show } from "solid-js";
import { commands } from "../bindings";
import { connect, unwrap } from "../lib/core";
import {
  checkDetail,
  checkTitle,
  fixLabel,
  levelLabel,
  ranAt,
  report,
  runHealth,
  running,
  type HealthCheck,
} from "../lib/health";
import { friendlyError, timeAgo } from "../lib/words";
import { toast } from "../components/ui";
import { HelpLink } from "../help/HelpLink";
import { t } from "../i18n";

export type HealthNav = {
  onDevice: (node: string) => void;
  onMap: () => void;
  onSettings: (section: string) => void;
  onHelp: (topic: string) => void;
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

/** One line: a dot for the level, what it is, why, and its fix. A
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
      <span class="health-dot" aria-hidden="true" />
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
  const summary = () => {
    const r = report();
    if (!r) return "";
    if (r.bad === 0 && r.warn === 0) return t("health.allGood");
    const parts: string[] = [];
    if (r.bad) parts.push(t("health.summaryBad", { n: r.bad }));
    if (r.warn) parts.push(t("health.summaryWarn", { n: r.warn }));
    return parts.join(" · ");
  };
  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>
            {t("health.title")} <HelpLink topic="troubleshooting" />
          </h1>
          <p class="muted">{t("health.subtitle")}</p>
        </div>
        <button class="btn btn-primary" disabled={running()} onClick={() => runHealth(true)}>
          {running() ? t("health.running") : t("health.rerun")}
        </button>
      </header>
      <Show when={report()} fallback={<p class="muted">{t("health.running")}</p>}>
        {(r) => (
          <article class="tile">
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
    </section>
  );
}
