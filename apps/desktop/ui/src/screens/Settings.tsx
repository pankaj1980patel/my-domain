// Account, this device, and preferences. Tap the version 7 times for the
// Developer screen.

import { createSignal, For, onMount, Show } from "solid-js";
import {
  commands,
  type AwayLock,
  type BatteryAlertSettings_Serialize as BatteryAlertSettings,
  type LocalActions,
} from "../bindings";
import { chatKeep, setChatKeep, setClipSync, signOut, state, unwrap } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "../components/ui";
import { ThemePicker } from "../components/ThemePicker";
import { ShortcutSetting } from "../components/ShortcutSetting";
import { ScheduleList } from "../components/ScheduleList";
import { AutomationList } from "../components/AutomationList";
import { LOCAL_PRESETS } from "../lib/presets";
import { ShellCommandsSetting } from "../components/ShellCommandsSetting";
import { WebhookSetting } from "../components/WebhookSetting";
import { BrowserExtensionSetting } from "../components/BrowserExtensionSetting";
import { CallsSetting } from "../components/CallsSetting";
import { ComputerNotifsSetting } from "../components/ComputerNotifsSetting";
import { FileManagerSetting } from "../components/FileManagerSetting";
import { SyncSetting } from "../components/SyncSetting";
import { WakeSetting } from "../components/WakeSetting";
import { TrayBatterySetting } from "../components/TrayBatterySetting";
import { NearbySetting } from "../components/NearbySetting";
import { WifiDirectSetting } from "../components/WifiDirectSetting";
import { KvmSetting } from "../components/KvmSetting";
import { InsertSetting } from "../components/InsertSetting";
import { PrintSetting } from "../components/PrintSetting";
import { RemoteDownloadSetting } from "../components/RemoteDownloadSetting";
import { SettingsBackup } from "../components/SettingsBackup";
import { UpdatesSetting } from "../components/UpdatesSetting";
import { updateStatus } from "../lib/updates";
import { ClipPrivacySetting } from "../components/ClipPrivacySetting";
import { OtpSetting } from "../components/OtpSetting";
import { AddDevice } from "../components/AddDevice";
import { OwnFingerprint } from "../components/VerifyDevice";
import { RemovedDevicesSetting } from "../components/RemovedDevices";
import { PermissionsSummary } from "../components/PeerPermissions";
import { KeyRotationSetting } from "../components/KeyRotation";
import { t, tRich, type Key, LANGUAGES, languagePref, setLanguagePref, systemLanguage, type LangPref } from "../i18n";
import { HelpLink } from "../help/HelpLink";
import { focusSoon } from "../lib/a11y";
import { setupAct } from "../lib/health";


/** Which OS's location permission the "find my computer" hint explains. */
const locationHintKey: Key = (() => {
  const p = navigator.platform || navigator.userAgent;
  if (/Mac/i.test(p)) return "settings.shareLocationHint";
  if (/Win/i.test(p)) return "settings.shareLocationHintWindows";
  return "settings.shareLocationHintLinux";
})();

export default function Settings(props: { onDeveloper: () => void; onHealth: () => void; onSetup: () => void }) {
  const [taps, setTaps] = createSignal(0);
  const [newKey, setNewKey] = createSignal("");
  const [pass, setPass] = createSignal("");
  const [showKey, setShowKey] = createSignal(false);
  // The key form replaces its button: focus moves into it, and back to the
  // button when it closes (it isn't lost to the page).
  let keyButton: HTMLButtonElement | undefined;
  let keyInput: HTMLInputElement | undefined;
  const openKey = () => {
    setShowKey(true);
    focusSoon(() => keyInput);
  };
  const closeKey = () => {
    setShowKey(false);
    focusSoon(() => keyButton);
  };
  const [notifs, setNotifs] = createSignal(true);
  const [followDnd, setFollowDnd] = createSignal(false);
  const [quickPanel, setQuickPanel] = createSignal(true);
  const [shareLabels, setShareLabels] = createSignal(true);
  const [shareLocation, setShareLocation] = createSignal(false);
  const [clipHistory, setClipHistory] = createSignal(true);
  const [canLocate, setCanLocate] = createSignal(false);
  const [awayLock, setAwayLock] = createSignal<AwayLock>({ on: false, minutes: 5, can_lock: false });
  const [battery, setBattery] = createSignal<BatteryAlertSettings>({ enabled: true, low: 15, charged: 0 });
  const [local, setLocal] = createSignal<LocalActions>();
  onMount(async () => {
    unwrap(commands.localActions()).then(setLocal, () => {});
    setNotifs(await commands.notificationsEnabled());
    setFollowDnd(await commands.followPhoneDnd());
    setQuickPanel(await commands.quickPanelEnabled());
    setShareLabels(await commands.labelsShare());
    setShareLocation(await commands.shareLocation());
    setClipHistory(await commands.clipHistoryEnabled());
    setCanLocate(await commands.locationAvailable());
    setAwayLock(await commands.awayLock());
    setBattery(await commands.batteryAlerts());
  });
  const toggleNotifs = async (on: boolean) => {
    await commands.setNotificationsEnabled(on);
    setNotifs(on);
  };
  const toggleFollowDnd = async (on: boolean) => {
    await commands.setFollowPhoneDnd(on);
    setFollowDnd(on);
  };
  const toggleQuickPanel = async (on: boolean) => {
    await commands.setQuickPanelEnabled(on);
    setQuickPanel(on);
  };
  const toggleShareLabels = async (on: boolean) => {
    await commands.setLabelsShare(on);
    setShareLabels(on);
  };
  const toggleShareLocation = async (on: boolean) => {
    await commands.setShareLocation(on);
    setShareLocation(on);
  };
  const toggleClipHistory = async (on: boolean) => {
    await commands.setClipHistoryEnabled(on);
    setClipHistory(on);
  };
  const changeAwayLock = async (on: boolean, minutes: number) => {
    await commands.setAwayLock(on, minutes);
    setAwayLock({ ...awayLock(), on, minutes });
  };
  const changeBattery = async (change: Partial<BatteryAlertSettings>) => {
    const next = { ...battery(), ...change };
    try {
      await unwrap(commands.setBatteryAlerts(next));
      setBattery(next);
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const tapVersion = () => {
    const n = taps() + 1;
    setTaps(n);
    if (n >= 7) {
      setTaps(0);
      toast(t("settings.developerUnlocked"), "ok");
      props.onDeveloper();
    }
  };

  return (
    <section class="page">
      <header class="page-head">
        <h1>{t("settings.title")}</h1>
      </header>

      <article class="tile" id="settings-account">
        <h2>{t("settings.account")}</h2>
        <p>{tRich("settings.signedInAs", { name: <strong>{state.account.username || "…"}</strong> })}</p>
        <div class="join-actions">
          <AddDevice />
          <button class="btn btn-ghost" onClick={signOut}>
            {t("settings.signOut")}
          </button>
        </div>
      </article>

      <RemovedDevicesSetting />

      <KeyRotationSetting />

      <article class="tile" id="settings-health">
        <h2>{t("health.settingsTile")}</h2>
        <p class="muted small">{t("health.settingsHint")}</p>
        <div class="row">
          <button class="btn btn-soft" onClick={props.onHealth}>
            {t("health.open")}
          </button>
          <button class="btn btn-ghost" onClick={() => setupAct("restart").then(props.onSetup, () => {})}>
            {t("setup.again")}
          </button>
        </div>
      </article>

      <article class="tile" id="settings-device">
        <h2>{t("settings.thisDevice")}</h2>
        <p>{tRich("settings.seenAs", { name: <strong>{niceName(state.me?.name ?? "")}</strong> })}</p>
        <OwnFingerprint />
        <label class="switch-row">
          <span>{t("settings.clipSync")}</span>
          <input type="checkbox" role="switch" checked={state.clipSync} onChange={(e) => setClipSync(e.currentTarget.checked)} />
        </label>
        <label class="switch-row">
          <span>{t("settings.clipHistory")}</span>
          <input type="checkbox" role="switch" checked={clipHistory()} onChange={(e) => toggleClipHistory(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.clipHistoryHint")}</p>
        <ClipPrivacySetting />
        <OtpSetting />
        <label class="switch-row">
          <span>{t("settings.chatHistory")}</span>
          <input type="checkbox" role="switch" checked={chatKeep()} onChange={(e) => setChatKeep(e.currentTarget.checked).catch(() => {})} />
        </label>
        <p class="muted small">{t("settings.chatHistoryHint")}</p>
        <label class="switch-row">
          <span>{t("settings.notifications")}</span>
          <input type="checkbox" role="switch" checked={notifs()} onChange={(e) => toggleNotifs(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.notificationsHint")}</p>
        <label class="switch-row">
          <span>{t("settings.followDnd")}</span>
          <input type="checkbox" role="switch" checked={followDnd()} onChange={(e) => toggleFollowDnd(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.followDndHint")}</p>
        <label class="switch-row">
          <span>{t("settings.quickPanel")}</span>
          <input type="checkbox" role="switch" checked={quickPanel()} onChange={(e) => toggleQuickPanel(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.quickPanelHint")}</p>
        <TrayBatterySetting />
        <NearbySetting />
        <WifiDirectSetting />
        <PermissionsSummary />
        <label class="switch-row">
          <span>{t("settings.shareLabels")}</span>
          <input type="checkbox" role="switch" checked={shareLabels()} onChange={(e) => toggleShareLabels(e.currentTarget.checked)} />
        </label>
        <p class="muted small">{t("settings.shareLabelsHint")}</p>
        <Show when={canLocate()}>
          <label class="switch-row">
            <span>{t("settings.shareLocation")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={shareLocation()}
              onChange={(e) => toggleShareLocation(e.currentTarget.checked)}
            />
          </label>
          <p class="muted small">
            {t(locationHintKey)} <HelpLink topic="find" />
          </p>
        </Show>
        <Show when={awayLock().can_lock}>
          <label class="switch-row">
            <span>{t("settings.awayLock")}</span>
            <input
              type="checkbox"
              role="switch"
              checked={awayLock().on}
              onChange={(e) => changeAwayLock(e.currentTarget.checked, awayLock().minutes)}
            />
          </label>
          <Show when={awayLock().on}>
            <label class="row small">
              <span class="muted">{t("settings.awayLockAfter")}</span>
              <select
                aria-label={t("settings.awayLockAfter")}
                onChange={(e) => changeAwayLock(true, Number(e.currentTarget.value))}
              >
                <For each={[2, 5, 10]}>
                  {(m) => (
                    <option value={String(m)} selected={m === awayLock().minutes}>
                      {t("settings.awayLockMinutes", { n: m })}
                    </option>
                  )}
                </For>
              </select>
            </label>
          </Show>
          <p class="muted small">{t("settings.awayLockHint")}</p>
        </Show>
        <label class="switch-row">
          <span>{t("settings.batteryAlerts")}</span>
          <input
            type="checkbox"
            role="switch"
            checked={battery().enabled}
            onChange={(e) => changeBattery({ enabled: e.currentTarget.checked })}
          />
        </label>
        <Show when={battery().enabled}>
          <label class="row small">
            <span class="muted">{t("settings.batteryCharged")}</span>
            <select
              aria-label={t("settings.batteryCharged")}
              onChange={(e) => changeBattery({ charged: Number(e.currentTarget.value) })}
            >
              <For each={[0, 80, 90, 100]}>
                {(pct) => (
                  <option value={String(pct)} selected={pct === battery().charged}>
                    {pct === 0 ? t("settings.batteryChargedOff") : t("settings.batteryChargedPct", { n: pct })}
                  </option>
                )}
              </For>
            </select>
          </label>
        </Show>
        <p class="muted small">{t("settings.batteryAlertsHint", { low: battery().low })}</p>
      </article>

      <ShortcutSetting me={local()?.node} />

      <CallsSetting />

      <ComputerNotifsSetting />

      <WakeSetting />

      <FileManagerSetting />

      <SyncSetting />

      <RemoteDownloadSetting />

      <ShellCommandsSetting />

      <KvmSetting />

      <InsertSetting />
      <PrintSetting />

      <article class="tile" id="settings-schedules">
        <h2>{t("schedule.local.title")}</h2>
        <p class="muted small">{t("schedule.local.hint")}</p>
        <Show when={local()} fallback={<p class="muted small">{t("schedule.local.loading")}</p>}>
          {(me) => (
            <ScheduleList
              node={me().node}
              name={t("schedule.local.name")}
              presets={LOCAL_PRESETS}
              actions={me().actions}
            />
          )}
        </Show>
      </article>

      <article class="tile" id="settings-automations">
        <h2>{t("automation.title")}</h2>
        <p class="muted small">{t("automation.hint")}</p>
        <Show when={local()} fallback={<p class="muted small">{t("schedule.local.loading")}</p>}>
          {(me) => <AutomationList me={me().node} localActions={me().actions} />}
        </Show>
      </article>

      <WebhookSetting />

      <BrowserExtensionSetting />

      <article class="tile" id="settings-key">
        <h2>{t("settings.deviceKey")}</h2>
        <p class="muted small">{t("settings.deviceKeyHint")}</p>
        <Show
          when={showKey()}
          fallback={
            <button class="btn btn-ghost" ref={keyButton} onClick={openKey}>
              {t("settings.changeKey")}
            </button>
          }
        >
          <form
            class="stack"
            onSubmit={async (e) => {
              e.preventDefault();
              try {
                await unwrap(commands.updateEncryptionKey(newKey(), pass()));
                toast(t("settings.keyChanged"), "ok");
                closeKey();
                setNewKey("");
                setPass("");
              } catch (err) {
                toast(friendlyError(err), "error");
              }
            }}
          >
            <input
              type="password"
              ref={keyInput}
              placeholder={t("settings.newKey")}
              aria-label={t("settings.newKey")}
              value={newKey()}
              onInput={(e) => setNewKey(e.currentTarget.value)}
            />
            <input
              type="password"
              placeholder={t("settings.yourPassword")}
              aria-label={t("settings.yourPassword")}
              autocomplete="current-password"
              value={pass()}
              onInput={(e) => setPass(e.currentTarget.value)}
            />
            <div class="row">
              <button class="btn btn-primary" type="submit" disabled={!newKey() || !pass()}>
                {t("settings.save")}
              </button>
              <button class="btn btn-ghost" type="button" onClick={closeKey}>
                {t("settings.cancel")}
              </button>
            </div>
          </form>
        </Show>
      </article>

      <UpdatesSetting />

      <article class="tile" id="settings-appearance">
        <h2>{t("settings.appearance")}</h2>
        <p class="muted small">{t("settings.appearanceHint")}</p>
        <ThemePicker />
        <label class="row small language-row">
          <span>{t("settings.language")}</span>
          <select
            aria-label={t("settings.language")}
            onChange={(e) => setLanguagePref(e.currentTarget.value as LangPref)}
          >
            <option value="system" selected={languagePref() === "system"}>
              {t("settings.languageSystem", { name: LANGUAGES.find((l) => l.id === systemLanguage())!.name })}
            </option>
            <For each={LANGUAGES}>
              {(l) => (
                <option value={l.id} lang={l.id} selected={languagePref() === l.id}>
                  {l.name}
                </option>
              )}
            </For>
          </select>
        </label>
        <p class="muted small">{t("settings.languageHint")}</p>
      </article>

      <SettingsBackup />

      {/* A button (no look of one), so the 7 taps work from the keyboard too. */}
      <button type="button" class="version muted small" onClick={tapVersion}>
        {t("settings.version", { version: updateStatus()?.current ?? "" })}
      </button>

    </section>
  );
}
