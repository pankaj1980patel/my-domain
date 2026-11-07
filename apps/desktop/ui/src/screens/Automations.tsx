// Automations: your rules (when this happens, do that) with their switches
// and the new-rule form; this computer's schedules; webhooks and shell
// commands others may run. The same components Settings shows.

import { createSignal, onMount, Show } from "solid-js";
import { commands, type LocalActions } from "../bindings";
import { unwrap } from "../lib/core";
import { t } from "../i18n";
import { Icon } from "../components/Icon";
import { Tabs, panelProps } from "../components/Tabs";
import { AutomationList } from "../components/AutomationList";
import { ScheduleList } from "../components/ScheduleList";
import { WebhookSetting } from "../components/WebhookSetting";
import { ShellCommandsSetting } from "../components/ShellCommandsSetting";
import { LOCAL_PRESETS } from "../lib/presets";

type Tab = "rules" | "schedules" | "hooks";

export default function Automations() {
  const [tab, setTab] = createSignal<Tab>("rules");
  const [local, setLocal] = createSignal<LocalActions>();
  onMount(() => unwrap(commands.localActions()).then(setLocal, () => {}));
  let body: HTMLDivElement | undefined;
  // "New rule": the form under the list, in view and focused.
  const newRule = () => {
    setTab("rules");
    requestAnimationFrame(() => {
      const form = body?.querySelector<HTMLElement>(".schedule-new");
      form?.scrollIntoView({ block: "nearest" });
      form?.querySelector<HTMLElement>("select, input, button")?.focus();
    });
  };
  return (
    <section class="page page-fit auto-page">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="workflow" size={26} tone="violet" />
        </span>
        <div class="grow">
          <h1>{t("automation.title")}</h1>
          <p class="muted">{t("auto.sub")}</p>
        </div>
        <button class="btn btn-primary" onClick={newRule}>
          <Icon name="quick" />
          {t("auto.newRule")}
        </button>
      </header>
      <Tabs
        label={t("automation.title")}
        idBase="auto"
        tabs={[
          { id: "rules", label: t("auto.rules"), icon: "workflow" },
          { id: "schedules", label: t("schedule.local.title"), icon: "calendar" },
          { id: "hooks", label: t("auto.hooks"), icon: "webhook" },
        ]}
        selected={tab()}
        onSelect={(id) => setTab(id as Tab)}
      />
      <div class="page-scroll auto-body" ref={body} {...panelProps("auto", tab())}>
        <Show when={local()} fallback={<p class="muted small">{t("schedule.local.loading")}</p>}>
          {(me) => (
            <>
              <Show when={tab() === "rules"}>
                <article class="tile">
                  <p class="muted small">{t("automation.hint")}</p>
                  <AutomationList me={me().node} localActions={me().actions} />
                </article>
              </Show>
              <Show when={tab() === "schedules"}>
                <article class="tile">
                  <p class="muted small">{t("schedule.local.hint")}</p>
                  <ScheduleList node={me().node} name={t("schedule.local.name")} presets={LOCAL_PRESETS} actions={me().actions} />
                </article>
              </Show>
              <Show when={tab() === "hooks"}>
                <WebhookSetting />
                <ShellCommandsSetting />
              </Show>
            </>
          )}
        </Show>
      </div>
    </section>
  );
}
