// First-run setup (after signing in or joining; again from Settings): five
// short steps — name this computer, add your other devices, choose what to
// use, allow what that needs, test the connection. Every step can be
// skipped; "Later" closes it and Home offers to carry on at the same step;
// "Skip setup" ends it. Where it stands is kept by the core, per device.

import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { commands } from "../bindings";
import { setClipSync, setState, state, unwrap } from "../lib/core";
import {
  GROUPS,
  loadSetup,
  report,
  runHealth,
  setup,
  setupAct,
  setupFeatures,
  stepNumber,
  STEPS,
  type Step,
} from "../lib/health";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "../components/ui";
import { AddDevice } from "../components/AddDevice";
import { HealthRow, type HealthNav } from "./Health";
import { t, type Key } from "../i18n";
import { focusSoon } from "../lib/a11y";

/** The settings for the groups picked: clipboard sync follows the pick;
 *  the others are only turned off when left out (nothing that shares more
 *  is turned on behind your back; location sharing stays your switch in
 *  Settings; controlling this computer and seeing its screen are allowed
 *  device by device, all off to begin with). */
async function applyFeatures(picked: Set<string>) {
  const off = (g: string) => !picked.has(g);
  await setClipSync(picked.has("clipboard")).catch(() => {});
  if (off("notifications")) await commands.setNotificationsEnabled(false).catch(() => {});
  if (off("find") && (await commands.locationAvailable().catch(() => false))) {
    await commands.setShareLocation(false).catch(() => {});
  }
}

export default function Setup(props: { onClose: () => void; nav: HealthNav }) {
  const [name, setName] = createSignal("");
  const [picked, setPicked] = createSignal<Set<string>>(new Set(GROUPS));
  const [busy, setBusy] = createSignal(false);
  let heading: HTMLHeadingElement | undefined;

  onMount(async () => {
    const s = await loadSetup();
    setPicked(new Set(s.features ?? GROUPS));
    let n = niceName(state.me?.name ?? "");
    if (!n) n = (await commands.ownDeviceKind().catch(() => null))?.model ?? "";
    setName(n);
  });

  // The permission and connection steps show live checks.
  const timer = setInterval(() => {
    const s = setup()?.step;
    if (s === "permissions" || s === "connection") runHealth(false);
  }, 4000);
  onCleanup(() => clearInterval(timer));

  const step = () => setup()?.step ?? "name";
  const moved = () => focusSoon(() => heading);

  const act = async (action: string, to: Step | null = null) => {
    setBusy(true);
    try {
      const s = await setupAct(action, to);
      if (s.finished) props.onClose();
      else {
        if (s.step === "permissions" || s.step === "connection") runHealth(false);
        moved();
      }
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  /** "Next": save what this step chose first. */
  const next = async () => {
    try {
      if (step() === "name") {
        const kept = await unwrap(commands.setDeviceName(name()));
        if (state.me) setState("me", "name", kept);
      } else if (step() === "features") {
        await setupFeatures([...picked()]);
        await applyFeatures(picked());
      }
    } catch (e) {
      toast(friendlyError(e), "error");
      return;
    }
    await act(step() === "connection" ? "finish" : "next");
  };

  const toggle = (g: string, on: boolean) => {
    const s = new Set(picked());
    if (on) s.add(g);
    else s.delete(g);
    setPicked(s);
  };

  const perms = () => report()?.checks.filter((c) => c.id === "permission") ?? [];
  const links = () => report()?.checks.filter((c) => c.id === "link" || c.id === "key") ?? [];

  return (
    <section class="page setup-page">
      <header class="page-head">
        <div>
          <p class="muted small">{t("setup.stepOf", { n: stepNumber({ step: step() }), total: STEPS.length })}</p>
          <h1 ref={heading} tabIndex={-1}>
            {t(`setup.step.${step()}` as Key)}
          </h1>
        </div>
        <div class="row">
          <button class="btn btn-ghost" onClick={props.onClose}>
            {t("setup.later")}
          </button>
          <button class="btn btn-ghost" onClick={() => act("skip_all")}>
            {t("setup.skipAll")}
          </button>
        </div>
      </header>

      <ol class="setup-dots" aria-label={t("setup.title")}>
        <For each={STEPS}>
          {(s, i) => (
            <li>
              <button
                class={`setup-dot ${s === step() ? "on" : ""} ${setup()?.skipped.includes(s) ? "skipped" : ""}`}
                aria-current={s === step() ? "step" : undefined}
                aria-label={`${i() + 1}. ${t(`setup.step.${s}` as Key)}`}
                title={t(`setup.step.${s}` as Key)}
                onClick={() => act("goto", s)}
              >
                {i() + 1}
              </button>
            </li>
          )}
        </For>
      </ol>

      <article class="tile">
        <Switch>
          <Match when={step() === "name"}>
            <p class="muted">{t("setup.name.hint")}</p>
            <label class="stack">
              <span>{t("setup.name.label")}</span>
              <input
                value={name()}
                maxLength={40}
                onInput={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => e.key === "Enter" && name().trim() && next()}
              />
            </label>
          </Match>
          <Match when={step() === "devices"}>
            <p class="muted">{t("setup.devices.hint")}</p>
            <AddDevice />
            <p class="small">
              <Show when={state.peers.length > 0} fallback={<span class="muted">{t("setup.devices.none")}</span>}>
                {t("setup.devices.have", { names: state.peers.map((p) => niceName(p.name)).join(", ") })}
              </Show>
            </p>
          </Match>
          <Match when={step() === "features"}>
            <p class="muted">{t("setup.features.hint")}</p>
            <For each={GROUPS}>
              {(g) => (
                <>
                  <label class="switch-row">
                    <span>{t(`setup.group.${g}` as Key)}</span>
                    <input
                      type="checkbox"
                      role="switch"
                      checked={picked().has(g)}
                      onChange={(e) => toggle(g, e.currentTarget.checked)}
                    />
                  </label>
                  <p class="muted small">{t(`setup.why.${g}` as Key)}</p>
                </>
              )}
            </For>
          </Match>
          <Match when={step() === "permissions"}>
            <p class="muted">{t("setup.permissions.hint")}</p>
            <Show when={perms().length > 0} fallback={<p>{t("setup.permissions.none")}</p>}>
              <ul class="health-list">
                <For each={perms()}>{(c) => <HealthRow check={c} nav={props.nav} />}</For>
              </ul>
            </Show>
            <button class="btn btn-ghost btn-sm" onClick={() => runHealth(false)}>
              {t("setup.permissions.recheck")}
            </button>
          </Match>
          <Match when={step() === "connection"}>
            <p class="muted">{t("setup.connection.hint")}</p>
            <Show when={links().length > 0} fallback={<p>{t("setup.connection.none")}</p>}>
              <ul class="health-list">
                <For each={links()}>{(c) => <HealthRow check={c} nav={props.nav} />}</For>
              </ul>
            </Show>
          </Match>
        </Switch>
      </article>

      <div class="row setup-foot">
        <Show when={stepNumber({ step: step() }) > 1}>
          <button class="btn btn-ghost" disabled={busy()} onClick={() => act("back")}>
            {t("setup.back")}
          </button>
        </Show>
        <button class="btn btn-ghost" disabled={busy()} onClick={() => act("skip")}>
          {t("setup.skip")}
        </button>
        <button
          class="btn btn-primary"
          disabled={busy() || (step() === "name" && !name().trim())}
          onClick={next}
        >
          {step() === "connection" ? t("setup.done") : t("setup.next")}
        </button>
      </div>
    </section>
  );
}

/** Home's card while setup isn't finished: carries on where it stopped. */
export function SetupCard(props: { onOpen: () => void }) {
  onMount(() => loadSetup().catch(() => {}));
  return (
    <Show when={setup() && !setup()!.finished}>
      <article class="tile setup-card">
        <h2>{t("setup.card.title")}</h2>
        <p class="muted">
          {t("setup.card.body", {
            n: stepNumber(setup()!),
            total: STEPS.length,
            step: t(`setup.step.${setup()!.step}` as Key),
          })}
        </p>
        <div class="row">
          <button class="btn btn-primary" onClick={props.onOpen}>
            {t("setup.card.open")}
          </button>
          <button class="btn btn-ghost" onClick={() => setupAct("skip_all").catch(() => {})}>
            {t("setup.skipAll")}
          </button>
        </div>
      </article>
    </Show>
  );
}
