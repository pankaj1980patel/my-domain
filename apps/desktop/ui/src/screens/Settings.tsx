// Settings in two columns: the categories (and a search box that looks
// through all of them) on the left, the chosen category on the right; one
// column under 720 px, the list first. Every section keeps its
// `settings-<id>` element id (deep links open its category). Tap the
// version 7 times (Advanced) for the Developer screen.

import { createEffect, createSignal, For, on, onMount, Show, type JSX } from "solid-js";
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
import { WallpaperShareSetting } from "../components/DeviceArt";
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
import { fold, queryWords } from "../lib/search";
import { SettingRow, SwitchRow } from "../components/SettingRow";
import { Icon, IconChip, type IconName, type Tone } from "../components/Icon";
import { ChangePassword, Sessions, SignOutAll } from "../components/Account";

/** The categories, in the list's order. */
const CATEGORIES = [
  { id: "general", label: "settings.cat.general", icon: "settings", tone: "cyan" },
  { id: "account", label: "settings.cat.account", icon: "shield", tone: "green" },
  { id: "devices", label: "settings.cat.devices", icon: "devices", tone: "cyan" },
  { id: "clipboard", label: "settings.cat.clipboard", icon: "clipboard", tone: "blue" },
  { id: "notifications", label: "settings.cat.notifications", icon: "notifications", tone: "violet" },
  { id: "automation", label: "settings.cat.automation", icon: "workflow", tone: "amber" },
  { id: "appearance", label: "settings.cat.appearance", icon: "palette", tone: "violet" },
  { id: "advanced", label: "settings.cat.advanced", icon: "code", tone: "blue" },
] as const satisfies readonly { id: string; label: Key; icon: IconName; tone: Tone }[];
type CategoryId = (typeof CATEGORIES)[number]["id"];



/** Which OS's location permission the "find my computer" hint explains. */
const locationHintKey: Key = (() => {
  const p = navigator.platform || navigator.userAgent;
  if (/Mac/i.test(p)) return "settings.shareLocationHint";
  if (/Win/i.test(p)) return "settings.shareLocationHintWindows";
  return "settings.shareLocationHintLinux";
})();

export default function Settings(props: {
  /** Open at this section's element id (`settings-<id>`). */
  section?: { id: string; at: number };
  onDeveloper: () => void;
  onHealth: () => void;
  onSetup: () => void;
}) {
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

  // --- categories and search ------------------------------------------------
  // null: the landing (every category as a card).
  const [cat, setCat] = createSignal<CategoryId | null>(null);
  const [query, setQuery] = createSignal("");
  const [found, setFound] = createSignal<Record<string, number>>({});
  const pick = (id: CategoryId | null) => {
    setCat(id);
    setQuery("");
    // On one column the list sits above: bring the chosen section up.
    requestAnimationFrame(() => {
      if (body && body.getBoundingClientRect().top > window.innerHeight * 0.6) body.scrollIntoView({ block: "start" });
    });
  };
  let body: HTMLDivElement | undefined;
  // Search: every category shows; a tile stays when its words match.
  // (Read from the page itself, so a setting added to any tile is found.)
  const filter = () => {
    if (!body) return;
    const words = queryWords(query());
    const counts: Record<string, number> = {};
    for (const sec of Array.from(body.querySelectorAll<HTMLElement>(".settings-cat"))) {
      const id = sec.dataset.cat ?? "";
      const units = Array.from(sec.querySelectorAll<HTMLElement>(":scope > .settings-units > .set-sec"));
      let n = 0;
      for (const u of units) {
        const hay = fold(`${u.textContent ?? ""} ${sec.dataset.label ?? ""}`);
        const hit = words.every((w) => hay.includes(w));
        u.classList.toggle("set-miss", words.length > 0 && !hit);
        if (hit) n++;
      }
      counts[id] = words.length ? n : units.length;
    }
    setFound(counts);
  };
  createEffect(on([query, cat, () => languagePref()], () => requestAnimationFrame(filter)));
  const searching = () => queryWords(query()).length > 0;
  const shows = (id: CategoryId) => (searching() ? (found()[id] ?? 0) > 0 : cat() === id);
  const nothing = () => searching() && CATEGORIES.every((c) => !found()[c.id]);

  // Opened at a section (the command palette, search, the health check):
  // its category, then the section in view.
  createEffect(
    on(
      () => props.section,
      (ask) => {
        if (!ask) return;
        requestAnimationFrame(() => {
          const el = document.getElementById(ask.id);
          const home = el?.closest<HTMLElement>(".settings-cat")?.dataset.cat as CategoryId | undefined;
          if (!el || !home) return;
          setQuery("");
          setCat(home);
          const secId = el.closest<HTMLElement>(".set-sec")?.dataset.sec;
          if (secId) pickSub(home, secId);
          requestAnimationFrame(() => el.scrollIntoView({ block: "start" }));
        });
      },
    ),
  );
  // Screenshot mode: ?sq= types into the search box.
  if (import.meta.env.MODE === "mock") {
    const sq = new URLSearchParams(location.search).get("sq");
    if (sq) setQuery(sq);
    const c = CATEGORIES.find((x) => x.id === new URLSearchParams(location.search).get("cat"));
    if (c) setCat(c.id);
  }

  // Each category's sections: one shows at a time (the list above them
  // picks it), so no category is a long page. Searching shows every match.
  const [sub, setSub] = createSignal<Partial<Record<CategoryId, string>>>({});
  const subOf = (c: CategoryId) => sub()[c] ?? SECTIONS[c][0].id;
  const pickSub = (c: CategoryId, id: string) => setSub({ ...sub(), [c]: id });

  type Sec = { id: string; icon: IconName; title: Key; body: () => JSX.Element };
  const SECTIONS: Record<CategoryId, Sec[]> = {
    general: [
      {
        id: "this-device",
        icon: "monitor",
        title: "settings.thisDevice",
        body: () => (
          <article class="tile" id="settings-device">
            <h2>{t("settings.thisDevice")}</h2>
            <p class="muted small">{tRich("settings.seenAs", { name: <strong>{niceName(state.me?.name ?? "")}</strong> })}</p>
            <SwitchRow
              icon="app-window"
              label={t("settings.quickPanel")}
              hint={t("settings.quickPanelHint")}
              checked={quickPanel()}
              onChange={toggleQuickPanel}
            />
            <SwitchRow
              icon="chat"
              label={t("settings.chatHistory")}
              hint={t("settings.chatHistoryHint")}
              checked={chatKeep()}
              onChange={(on) => setChatKeep(on).catch(() => {})}
            />
            <TrayBatterySetting />
          </article>
        ),
      },
      { id: "shortcuts", icon: "keyboard", title: "shortcuts.title", body: () => <ShortcutSetting me={local()?.node} /> },
      { id: "updates", icon: "download", title: "updates.title", body: () => <UpdatesSetting /> },
      {
        id: "health",
        icon: "health",
        title: "health.settingsTile",
        body: () => (
          <article class="tile" id="settings-health">
            <h2>{t("health.settingsTile")}</h2>
            <SettingRow icon="health" label={t("health.open")} hint={t("health.settingsHint")}>
              <div class="row">
                <button class="btn btn-ghost btn-sm" onClick={() => setupAct("restart").then(props.onSetup, () => {})}>
                  {t("setup.again")}
                </button>
                <button class="btn btn-soft btn-sm" onClick={props.onHealth}>
                  {t("health.open")}
                </button>
              </div>
            </SettingRow>
          </article>
        ),
      },
    ],
    account: [
      {
        id: "account",
        icon: "user",
        title: "settings.account",
        body: () => (
          <>
            <article class="tile" id="settings-account">
              <h2>{t("settings.account")}</h2>
              <SettingRow icon="user" label={state.account.username || "…"} hint={state.account.server || t("settings.account")}>
                <AddDevice />
              </SettingRow>
              <ChangePassword />
              <Sessions />
            </article>
          </>
        ),
      },
      {
        id: "security",
        icon: "shield",
        title: "settings.security",
        body: () => (
          <>
            <article class="tile">
              <h2>{t("settings.security")}</h2>
              <OwnFingerprint />
              <Show when={awayLock().can_lock}>
                <SwitchRow
                  icon="lock"
                  label={t("settings.awayLock")}
                  hint={t("settings.awayLockHint")}
                  checked={awayLock().on}
                  onChange={(on) => changeAwayLock(on, awayLock().minutes)}
                />
                <Show when={awayLock().on}>
                  <SettingRow icon="timers" label={t("settings.awayLockAfter")}>
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
                  </SettingRow>
                </Show>
              </Show>
            </article>
            <KeyRotationSetting />
          </>
        ),
      },
      {
        id: "key",
        icon: "key",
        title: "settings.deviceKey",
        body: () => (
          <article class="tile" id="settings-key">
            <h2>{t("settings.deviceKey")}</h2>
            <Show
              when={showKey()}
              fallback={
                <SettingRow icon="key" label={t("settings.changeKey")} hint={t("settings.deviceKeyHint")}>
                  <button class="btn btn-ghost btn-sm" ref={keyButton} onClick={openKey}>
                    {t("settings.changeKey")}
                  </button>
                </SettingRow>
              }
            >
              <p class="muted small">{t("settings.deviceKeyHint")}</p>
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
                <div class="row dialog-actions">
                  <button class="btn btn-ghost" type="button" onClick={closeKey}>
                    {t("settings.cancel")}
                  </button>
                  <button class="btn btn-primary" type="submit" disabled={!newKey() || !pass()}>
                    {t("settings.save")}
                  </button>
                </div>
              </form>
            </Show>
          </article>
        ),
      },
      {
        // Apart from the rest, at the end: what signs devices out.
        id: "danger",
        icon: "alert",
        title: "settings.danger",
        body: () => (
            <article class="tile danger-zone" id="settings-danger">
              <h2>
                <Icon name="alert" tone="red" />
                {t("settings.danger")}
              </h2>
              <SettingRow icon="disconnect" label={t("settings.signOut")} hint={t("settings.signOutHint")}>
                <button class="btn btn-danger btn-sm" onClick={signOut}>
                  {t("settings.signOut")}
                </button>
              </SettingRow>
              <SignOutAll />
            </article>
        ),
      },
    ],
    devices: [
      {
        id: "sharing",
        icon: "devices",
        title: "settings.cat.devicesTile",
        body: () => (
          <article class="tile">
            <h2>{t("settings.cat.devicesTile")}</h2>
            <SwitchRow
              icon="tags"
              label={t("settings.shareLabels")}
              hint={t("settings.shareLabelsHint")}
              checked={shareLabels()}
              onChange={toggleShareLabels}
            />
            <Show when={canLocate()}>
              <SwitchRow
                icon="locate"
                label={t("settings.shareLocation")}
                hint={
                  <>
                    {t(locationHintKey)} <HelpLink topic="find" />
                  </>
                }
                checked={shareLocation()}
                onChange={toggleShareLocation}
              />
            </Show>
            <NearbySetting />
            <WifiDirectSetting />
          </article>
        ),
      },
      {
        id: "permissions",
        icon: "verify",
        title: "perm.summaryTitle",
        body: () => (
          <article class="tile">
            <PermissionsSummary />
          </article>
        ),
      },
      { id: "wake", icon: "power", title: "wake.title", body: () => <WakeSetting /> },
      { id: "kvm", icon: "mouse", title: "kvm.title", body: () => <KvmSetting /> },
      { id: "removed", icon: "remove", title: "removedDevices.title", body: () => <RemovedDevicesSetting /> },
    ],
    clipboard: [
      {
        id: "clipboard",
        icon: "clipboard",
        title: "clipboard.title",
        body: () => (
          <article class="tile">
            <h2>{t("clipboard.title")}</h2>
            <SwitchRow icon="refresh" label={t("settings.clipSync")} checked={state.clipSync} onChange={setClipSync} />
            <SwitchRow
              icon="history"
              label={t("settings.clipHistory")}
              hint={t("settings.clipHistoryHint")}
              checked={clipHistory()}
              onChange={toggleClipHistory}
            />
            <ClipPrivacySetting />
            <OtpSetting />
          </article>
        ),
      },
      { id: "file-manager", icon: "files", title: "sendTo.settingTitle", body: () => <FileManagerSetting /> },
      { id: "sync", icon: "folder-sync", title: "sync.title", body: () => <SyncSetting /> },
      { id: "downloads", icon: "download", title: "download.settingsTitle", body: () => <RemoteDownloadSetting /> },
      { id: "insert", icon: "scan", title: "insert.settingsTitle", body: () => <InsertSetting /> },
      { id: "print", icon: "printer", title: "print.title", body: () => <PrintSetting /> },
    ],
    notifications: [
      {
        id: "notifications",
        icon: "notifications",
        title: "notifications.title",
        body: () => (
          <article class="tile">
            <h2>{t("notifications.title")}</h2>
            <SwitchRow
              icon="notifications"
              label={t("settings.notifications")}
              hint={t("settings.notificationsHint")}
              checked={notifs()}
              onChange={toggleNotifs}
            />
            <SwitchRow
              icon="moon"
              label={t("settings.followDnd")}
              hint={t("settings.followDndHint")}
              checked={followDnd()}
              onChange={toggleFollowDnd}
            />
            <SwitchRow
              icon="battery"
              label={t("settings.batteryAlerts")}
              hint={t("settings.batteryAlertsHint", { low: battery().low })}
              checked={battery().enabled}
              onChange={(on) => changeBattery({ enabled: on })}
            />
            <Show when={battery().enabled}>
              <SettingRow icon="battery" label={t("settings.batteryCharged")}>
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
              </SettingRow>
            </Show>
          </article>
        ),
      },
      { id: "computer-notifs", icon: "app-window", title: "computerNotifs.title", body: () => <ComputerNotifsSetting /> },
      { id: "calls", icon: "phone", title: "calls.title", body: () => <CallsSetting /> },
    ],
    automation: [
      {
        id: "schedules",
        icon: "calendar",
        title: "schedule.local.title",
        body: () => (
          <article class="tile" id="settings-schedules">
            <h2>{t("schedule.local.title")}</h2>
            <p class="muted small">{t("schedule.local.hint")}</p>
            <Show when={local()} fallback={<p class="muted small">{t("schedule.local.loading")}</p>}>
              {(me) => (
                <ScheduleList node={me().node} name={t("schedule.local.name")} presets={LOCAL_PRESETS} actions={me().actions} />
              )}
            </Show>
          </article>
        ),
      },
      {
        id: "automations",
        icon: "workflow",
        title: "automation.title",
        body: () => (
          <article class="tile" id="settings-automations">
            <h2>{t("automation.title")}</h2>
            <p class="muted small">{t("automation.hint")}</p>
            <Show when={local()} fallback={<p class="muted small">{t("schedule.local.loading")}</p>}>
              {(me) => <AutomationList me={me().node} localActions={me().actions} />}
            </Show>
          </article>
        ),
      },
      { id: "commands", icon: "terminal", title: "commands.title", body: () => <ShellCommandsSetting /> },
      { id: "webhooks", icon: "webhook", title: "webhooks.title", body: () => <WebhookSetting /> },
    ],
    appearance: [
      {
        id: "appearance",
        icon: "palette",
        title: "settings.appearance",
        body: () => (
          <article class="tile" id="settings-appearance">
            <h2>{t("settings.appearance")}</h2>
            <p class="muted small">{t("settings.appearanceHint")}</p>
            <ThemePicker />
            <WallpaperShareSetting />
            <SettingRow icon="languages" label={t("settings.language")} hint={t("settings.languageHint")}>
              <select aria-label={t("settings.language")} onChange={(e) => setLanguagePref(e.currentTarget.value as LangPref)}>
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
            </SettingRow>
          </article>
        ),
      },
    ],
    advanced: [
      { id: "browser-extension", icon: "puzzle", title: "browserExt.title", body: () => <BrowserExtensionSetting /> },
      { id: "backup", icon: "archive", title: "backup.title", body: () => <SettingsBackup /> },
      {
        id: "about",
        icon: "info",
        title: "settings.about",
        body: () => (
          <article class="tile settings-version">
            <h2>{t("settings.about")}</h2>
            {/* A button (no look of one), so the 7 taps work from the keyboard too. */}
            <button type="button" class="version muted small" onClick={tapVersion}>
              {t("settings.version", { version: updateStatus()?.current ?? "" })}
            </button>
          </article>
        ),
      },
    ],
  };

  /** One category: its title, the list of its sections, and each section
   *  (an icon before its tile's title). */
  const Category = (p: { id: CategoryId }) => {
    const c = CATEGORIES.find((x) => x.id === p.id)!;
    const label = () => t(c.label);
    const secs = SECTIONS[p.id];
    return (
      <section
        class="settings-cat"
        data-cat={p.id}
        data-label={label()}
        hidden={!shows(p.id)}
        aria-labelledby={`settings-cat-${p.id}`}
      >
        <h2 class="settings-cat-title" id={`settings-cat-${p.id}`}>
          <Icon name={c.icon} />
          {label()}
        </h2>
        <Show when={secs.length > 1 && !searching()}>
          <ul class="sub-nav" aria-label={t("settings.sectionsOf", { name: label() })}>
            <For each={secs}>
              {(x) => (
                <li>
                  <button
                    type="button"
                    class="sub-nav-item"
                    classList={{ active: subOf(p.id) === x.id }}
                    aria-current={subOf(p.id) === x.id ? "true" : undefined}
                    onClick={() => pickSub(p.id, x.id)}
                  >
                    <Icon name={x.icon} />
                    {t(x.title)}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <div class="settings-units">
          <For each={secs}>
            {(x) => (
              <div class="set-sec icon-slot" data-sec={x.id} hidden={!searching() && subOf(p.id) !== x.id}>
                <Icon name={x.icon} class="slot-icon" />
                {x.body()}
              </div>
            )}
          </For>
        </div>
      </section>
    );
  };

  return (
    <section class="page page-wide">
      <header class="page-head">
        <span class="head-tile" aria-hidden="true">
          <Icon name="settings" size={26} tone="blue" />
        </span>
        <div class="grow">
          <h1>{t("settings.title")}</h1>
          <p class="muted">{t("settings.subtitle")}</p>
        </div>
      </header>

      <div class="settings-layout">
        <nav class="settings-nav" aria-label={t("settings.categories")}>
          <input
            type="search"
            class="settings-search"
            placeholder={t("settings.searchPlaceholder")}
            aria-label={t("settings.searchPlaceholder")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Escape" && query() && (e.preventDefault(), setQuery(""))}
          />
          <ul class="settings-cats">
            <li>
              <button
                type="button"
                class="nav-item settings-cat-btn"
                classList={{ active: !searching() && cat() === null }}
                aria-current={!searching() && cat() === null ? "true" : undefined}
                onClick={() => pick(null)}
              >
                <Icon name="home" />
                <span class="nav-label">{t("device.tab.overview")}</span>
              </button>
            </li>
            <For each={CATEGORIES}>
              {(c) => (
                <li>
                  <button
                    type="button"
                    class="nav-item settings-cat-btn"
                    classList={{ active: !searching() && cat() === c.id, dim: searching() && !found()[c.id] }}
                    aria-current={!searching() && cat() === c.id ? "true" : undefined}
                    onClick={() => pick(c.id)}
                  >
                    <Icon name={c.icon} tone={c.tone} />
                    <span class="nav-label">{t(c.label)}</span>
                    <Show when={searching() && found()[c.id]}>
                      <span class="nav-count">{found()[c.id]}</span>
                    </Show>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </nav>

        <div class="settings-body" ref={body}>
          <Show when={nothing()}>
            <p class="muted settings-none" role="status">
              {t("settings.noMatch", { query: query().trim() })}
            </p>
          </Show>
          {/* The landing: every category as a card, what's in it below its name. */}
          <Show when={cat() === null && !searching()}>
            <div class="cat-grid">
              <For each={CATEGORIES}>
                {(c) => (
                  <button class="tile cat-card" onClick={() => pick(c.id)}>
                    <IconChip name={c.icon} tone={c.tone} size={22} />
                    <span class="cat-card-text">
                      <strong>{t(c.label)}</strong>
                      <span class="muted small">{SECTIONS[c.id].map((x) => t(x.title)).join(" · ")}</span>
                    </span>
                    <Icon name="chevron-right" />
                  </button>
                )}
              </For>
            </div>
          </Show>
          <For each={CATEGORIES}>{(c) => <Category id={c.id} />}</For>
        </div>
      </div>
    </section>
  );
}
