// "Calls on this computer" (calls.rs): off by default; pick a phone already
// paired with this computer, connect, choose where answered audio goes, and
// a dial pad. On macOS, Linux and Windows (call control only there: the
// audio stays on the phone); elsewhere it says it isn't available. A
// standing problem (Linux: another program has the hands-free role;
// Windows: Phone Link's Calls has the phone) shows with what to do about it.

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type CallDevice } from "../bindings";
import { unwrap } from "../lib/core";
import { callDo, callStatus, refreshCalls, setCallStatus, watchCalls, type AudioTarget } from "../lib/calls";
import { friendlyError } from "../lib/words";
import { CallAudioPicker } from "./CallAudioPicker";
import { Keypad } from "./CallOverlay";
import { toast } from "./ui";
import { t } from "../i18n";
import { HelpLink } from "../help/HelpLink";
import { Icon } from "./Icon";

export function CallsSetting() {
  const [phones, setPhones] = createSignal<CallDevice[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [number, setNumber] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const s = () => callStatus();

  const loadPhones = async () => {
    try {
      setPhones(await unwrap(commands.callsDevices()));
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setLoaded(true);
    }
  };
  onMount(async () => {
    watchCalls();
    await refreshCalls();
    if (s()?.available && s()?.enabled) loadPhones();
  });

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
      refreshCalls();
    }
  };
  const toggle = (on: boolean) =>
    run(async () => {
      setCallStatus(await unwrap(commands.callsSetEnabled(on)));
      if (on) await loadPhones();
    });
  const pick = (address: string) => run(async () => setCallStatus(await unwrap(commands.callsSetDevice(address || null))));
  const setAnswer = async (target: AudioTarget) => setCallStatus(await commands.callsSetAnswerAudio(target));
  const dial = (e: Event) => {
    e.preventDefault();
    if (number().trim()) callDo(commands.callDial(number()));
  };
  const mac = () => s()?.os === "macos";
  const deviceLabel = () => s()?.device_name ?? phones().find((p) => p.address === s()?.device)?.name ?? s()?.device ?? "";

  return (
    <article class="tile" id="settings-calls">
      <h2>
        {t("calls.title")}
        <HelpLink topic="calls" />
      </h2>
      <Show when={s()?.available} fallback={<p class="muted small">{t("calls.unavailable")}</p>}>
        <p class="muted small">{t(mac() ? "calls.hintMac" : s()?.os === "windows" ? "calls.hintWindows" : "calls.hint")}</p>
        <label class="switch-row">
          <span>{t("calls.switch")}</span>
          <input
            type="checkbox"
            role="switch"
            checked={s()?.enabled ?? false}
            disabled={busy()}
            onChange={(e) => toggle(e.currentTarget.checked)}
          />
        </label>
        <Show when={s()?.enabled}>
          <Show when={s()?.problem}>
            {(p) => (
              <div class="call-problem" role="alert">
                <strong class="small">{t("calls.problem")}</strong>
                <p class="small">{p()}</p>
              </div>
            )}
          </Show>
          <Show when={!loaded() || phones().length > 0} fallback={<p class="muted small">{t(mac() ? "calls.noPhonesMac" : "calls.noPhones")}</p>}>
            <label class="row small">
              <span class="muted">{t("calls.phone")}</span>
              <select aria-label={t("calls.phone")} disabled={busy()} onChange={(e) => pick(e.currentTarget.value)}>
                <option value="" selected={!s()?.device}>
                  {t("calls.pickPhone")}
                </option>
                <For each={phones()}>
                  {(p) => (
                    <option value={p.address} selected={s()?.device === p.address}>
                      {p.name}
                    </option>
                  )}
                </For>
              </select>
            </label>
          </Show>
          {/* Refresh and Connect; Disconnect at the far end, in the danger tone. */}
          <div class="row">
            <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={loadPhones}>
              <Icon name="refresh" size={16} />
              {t("calls.refresh")}
            </button>
            <Show when={!s()?.connected}>
              <button class="btn btn-primary btn-sm" disabled={busy() || !s()?.device} onClick={() => run(() => unwrap(commands.callsConnect()))}>
                {t("calls.connect")}
              </button>
            </Show>
            <span class="muted small grow" role="status">
              {s()?.connected ? t("calls.connected", { name: deviceLabel() }) : t("calls.notConnected")}
            </span>
            <Show when={s()?.connected}>
              <button class="btn btn-ghost btn-sm btn-danger-ghost" disabled={busy()} onClick={() => run(() => unwrap(commands.callsDisconnect()))}>
                <Icon name="disconnect" size={16} />
                {t("calls.disconnect")}
              </button>
            </Show>
          </div>
          <Show when={s()?.connected && (s()?.signal != null || s()?.battery != null)}>
            <p class="muted small">
              <Show when={s()?.signal != null}>{t("calls.signal", { n: s()?.signal ?? 0 })}</Show>
              <Show when={s()?.signal != null && s()?.battery != null}> · </Show>
              <Show when={s()?.battery != null}>{t("calls.battery", { n: s()?.battery ?? 0 })}</Show>
            </p>
          </Show>

          <h3 class="call-subhead">{t("calls.answerAudio")}</h3>
          <Show when={s()}>{(st) => <CallAudioPicker id="settings-calls-answer" value={st().answer} onChange={setAnswer} />}</Show>

          <h3 class="call-subhead">{t("calls.dialTitle")}</h3>
          <Show when={s()?.connected} fallback={<p class="muted small">{t("calls.dialNeedsLink")}</p>}>
            <form class="stack call-dial" onSubmit={dial}>
              <input
                type="tel"
                inputmode="tel"
                aria-label={t("calls.number")}
                placeholder={t("calls.number")}
                value={number()}
                onInput={(e) => setNumber(e.currentTarget.value)}
              />
              <Keypad id="settings-calls-keypad" onKey={(k) => setNumber(number() + k)} />
              <div class="row">
                <button class="btn btn-primary" type="submit" disabled={!number().trim() || s()?.phase !== "idle"}>
                  {t("calls.call")}
                </button>
                <button class="btn btn-ghost" type="button" disabled={s()?.phase !== "idle"} onClick={() => callDo(commands.callRedial())}>
                  {t("calls.redial")}
                </button>
              </div>
            </form>
          </Show>
        </Show>
      </Show>
    </article>
  );
}
