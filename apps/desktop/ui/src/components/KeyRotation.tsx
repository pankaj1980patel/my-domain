// Changing the passphrase on all your devices (`mdcore::features::rekey`).
// Settings → "Change passphrase on all devices": type the new one twice
// (with a strength hint); the new key goes to each of your trusted devices,
// sealed to that device. The list shows each device (waiting / has it /
// updated / missed it); Finish switches everyone, Cancel (before Finish)
// drops it. A device that missed a change shows a banner asking for the new
// passphrase (`KeyRotationBanner`, over every page).

import { createSignal, For, Match, onMount, Show, Switch } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands, type KeyRotation, type RotationDevice } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";

const [rotation, setRotation] = createSignal<KeyRotation | null>(null);

async function load(): Promise<void> {
  setRotation(await commands.keyRotation());
}

let watching = false;
function watch(): void {
  if (watching) return;
  watching = true;
  load().catch(() => {});
  listen("key-rotation-changed", () => load().catch(() => {})).catch(() => {});
}

const STRENGTH = ["keyRotation.strength0", "keyRotation.strength1", "keyRotation.strength2", "keyRotation.strength3", "keyRotation.strength4"] as const;

function statusText(d: RotationDevice): string {
  switch (d.status) {
    case "waiting":
      return t("keyRotation.status.waiting");
    case "ready":
      return t("keyRotation.status.ready");
    case "updated":
      return t("keyRotation.status.updated");
    case "expired":
      return t("keyRotation.status.expired");
    default:
      return t("keyRotation.status.noKey");
  }
}

function expiresIn(s: number): string {
  if (s >= 2 * 86400) return t("keyRotation.expiresDays", { n: Math.floor(s / 86400) });
  if (s >= 3600) return t("keyRotation.expiresHours", { n: Math.floor(s / 3600) });
  return t("keyRotation.expiresMinutes", { n: Math.max(1, Math.floor(s / 60)) });
}

/** Over every page: this device missed a passphrase change. */
export function KeyRotationBanner() {
  onMount(watch);
  const [pass, setPass] = createSignal("");
  const save = async (e: Event) => {
    e.preventDefault();
    try {
      await unwrap(commands.setEncryptionKey(pass()));
      setPass("");
      toast(t("keyRotation.typedSaved"), "ok");
      await load();
    } catch (err) {
      toast(friendlyError(err), "error");
    }
  };
  return (
    <Show when={rotation()?.needs_passphrase ? rotation() : undefined}>
      {(r) => (
        <div class="update-banner" role="alert">
          <div class="update-banner-line">
            <strong>{t("keyRotation.needsTitle")}</strong>
          </div>
          <p class="small">{t("keyRotation.needsBody", { name: r().mismatch_name })}</p>
          <form class="row" onSubmit={save}>
            <input
              type="password"
              placeholder={t("keyRotation.newPassphrase")}
              aria-label={t("keyRotation.newPassphrase")}
              value={pass()}
              onInput={(e) => setPass(e.currentTarget.value)}
            />
            <button class="btn btn-soft btn-sm" type="submit" disabled={!pass()}>
              {t("settings.save")}
            </button>
            <button
              class="btn btn-ghost btn-sm"
              type="button"
              onClick={() => commands.dismissKeyRotation().then(load).catch(() => {})}
            >
              {t("keyRotation.notNow")}
            </button>
          </form>
        </div>
      )}
    </Show>
  );
}

/** Settings: start a change, follow it, finish or cancel it. */
export function KeyRotationSetting() {
  onMount(watch);
  const [open, setOpen] = createSignal(false);
  const [first, setFirst] = createSignal("");
  const [second, setSecond] = createSignal("");
  const [strength, setStrength] = createSignal(0);
  const [busy, setBusy] = createSignal(false);
  const mismatch = () => second().length > 0 && first() !== second();
  const typing = (v: string) => {
    setFirst(v);
    commands.passphraseStrength(v).then(setStrength).catch(() => {});
  };
  const act = async (f: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    try {
      await f();
      if (ok) toast(ok, "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
      await load().catch(() => {});
    }
  };
  const start = (e: Event) => {
    e.preventDefault();
    act(async () => {
      await unwrap(commands.startKeyRotation(first()));
      setOpen(false);
      setFirst("");
      setSecond("");
      setStrength(0);
    }, t("keyRotation.started"));
  };
  const state = () => rotation()?.state ?? "none";
  const active = () => state() === "preparing" || state() === "switching";
  return (
    <article class="tile" id="settings-key-rotation">
      <h2>{t("keyRotation.title")}</h2>
      <p class="muted small">{t("keyRotation.hint")}</p>
      <Show when={rotation()}>
        {(r) => (
          <Switch>
            <Match when={active() && !r().by_me}>
              <p class="small">
                {state() === "preparing"
                  ? t("keyRotation.otherPreparing", { name: r().by_name })
                  : t("keyRotation.otherSwitching", { name: r().by_name })}
              </p>
            </Match>
            <Match when={active() && r().by_me}>
              <p class="small">
                {state() === "preparing" ? t("keyRotation.preparing") : t("keyRotation.switching")}
              </p>
              <DeviceList devices={r().devices} />
              <p class="muted small">{expiresIn(r().expires_in_s)}</p>
              <div class="row">
                <button
                  class="btn btn-primary btn-sm"
                  disabled={busy()}
                  onClick={() => act(() => unwrap(commands.finishKeyRotation()))}
                >
                  {state() === "preparing" ? t("keyRotation.finish") : t("keyRotation.stopWaiting")}
                </button>
                <Show when={state() === "preparing"}>
                  <button
                    class="btn btn-ghost btn-sm"
                    disabled={busy()}
                    onClick={() => act(() => unwrap(commands.cancelKeyRotation()), t("keyRotation.cancelled"))}
                  >
                    {t("keyRotation.cancel")}
                  </button>
                </Show>
              </div>
              <p class="muted small">
                {state() === "preparing" ? t("keyRotation.finishHint") : t("keyRotation.stopWaitingHint")}
              </p>
            </Match>
            <Match when={!active() && state() !== "none"}>
              <p class="small">
                {state() === "done"
                  ? t("keyRotation.done")
                  : state() === "expired"
                    ? t("keyRotation.expired")
                    : t("keyRotation.wasCancelled")}
              </p>
              <Show when={r().by_me}>
                <DeviceList devices={r().devices} />
              </Show>
              <button class="btn btn-ghost btn-sm" onClick={() => commands.dismissKeyRotation().then(load).catch(() => {})}>
                {t("keyRotation.clear")}
              </button>
            </Match>
          </Switch>
        )}
      </Show>
      <Show when={!active()}>
        <Show
          when={open()}
          fallback={
            <button class="btn btn-ghost" onClick={() => setOpen(true)}>
              {t("keyRotation.change")}
            </button>
          }
        >
          <form class="stack" onSubmit={start}>
            <input
              type="password"
              autocomplete="new-password"
              placeholder={t("keyRotation.newPassphrase")}
              aria-label={t("keyRotation.newPassphrase")}
              value={first()}
              onInput={(e) => typing(e.currentTarget.value)}
            />
            <input
              type="password"
              autocomplete="new-password"
              placeholder={t("keyRotation.repeat")}
              aria-label={t("keyRotation.repeat")}
              value={second()}
              onInput={(e) => setSecond(e.currentTarget.value)}
            />
            <Show when={first()}>
              <p class="small muted" aria-live="polite">
                {t("keyRotation.strength", { level: t(STRENGTH[strength()] ?? STRENGTH[0]) })}
              </p>
            </Show>
            <Show when={mismatch()}>
              <p class="small" role="alert">{t("keyRotation.mismatch")}</p>
            </Show>
            <p class="muted small">{t("keyRotation.formHint")}</p>
            <div class="row">
              <button
                class="btn btn-primary"
                type="submit"
                disabled={busy() || !first() || first() !== second() || strength() < 1}
              >
                {t("keyRotation.send")}
              </button>
              <button class="btn btn-ghost" type="button" onClick={() => setOpen(false)}>
                {t("settings.cancel")}
              </button>
            </div>
          </form>
        </Show>
      </Show>
    </article>
  );
}

function DeviceList(props: { devices: RotationDevice[] }) {
  return (
    <Show when={props.devices.length > 0} fallback={<p class="muted small">{t("keyRotation.noDevices")}</p>}>
      <div class="removed-list">
        <For each={props.devices}>
          {(d) => (
            <div class="row removed-row">
              <div class="grow">{d.name}</div>
              <span class="small muted">{statusText(d)}</span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
