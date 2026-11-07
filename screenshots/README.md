# Screenshots

Every screen of the my-domain desktop app (and, once a phone is connected,
the Android app), made by scripts from **sample data only**: made-up
devices (“Pankaj’s MacBook Pro”, “Pixel 8”, “Office PC”, “Lab laptop”),
notes, transfers and so on. Nothing here comes from a real account.

See **[OVERVIEW.md](OVERVIEW.md)** for the design language, what’s done and
what’s next, with these pictures inline.

## Regenerate

**Desktop** (macOS with Google Chrome; node and the UI’s npm packages):

```sh
scripts/screenshots-desktop.sh           # all of them, into screenshots/desktop/
scripts/screenshots-desktop.sh settings  # only the shots whose name has “settings”
FORMAT=png scripts/screenshots-desktop.sh  # keep Chrome’s PNGs instead of JPEG
```

It builds the UI in screenshot mode (`vite build --mode mock` into
`apps/desktop/ui/dist-mock/`), serves it with `vite preview` on a free port
and opens each screen in headless Chrome at 1280×820. In that mode the page
starts at `src/dev/mockEntry.ts`, which, when the URL has `?mock=1`, installs
a stand-in backend (`src/dev/mockTauri.ts`: sample answers for the app’s
commands and events, plus empty-but-valid answers for every other command
from `src/dev/mockDefaults.json`). The normal build, the one Tauri ships,
never contains any of it (`vite.config.ts` only swaps the entry in the
`mock` mode; `App.tsx`’s URL handling is dropped by `import.meta.env.MODE`).
After the bindings change, run `node scripts/gen-mock-defaults.mjs` in
`apps/desktop/ui`.

Open one screen yourself: `cd apps/desktop/ui && npx vite --mode mock`, then
e.g. `http://localhost:1420/?mock=1&view=device&device=pixel8&theme=light`.
Parameters: `view` (home, device, notes, timers, readLater, linksMap, health,
setup, activity, transfers, settings, help, search, developer, signIn),
`device` (macbook, pixel8, officepc, lablaptop), `section` (a Settings
section), `scroll` (px), `step` (setup step), `dialog` (keyChanged,
permission, download, print, hotspot, call, ring), `window` (quick-panel),
`q`, `topic`, `note`, `theme` (dark, light), `lang` (en, hi, mr, gu, ta, es).

Pictures are JPEG (quality 80): the star-field background makes each PNG
about 0.5 MB.

**Android** (a phone with a **debug** build installed, on adb, unlocked):

```sh
scripts/screenshots-android.sh           # into screenshots/android/
```

Debug builds take launch extras for this (`MainActivity.debugLaunch`):
`--ez md_sample true` shows made-up devices (`core/SampleData.kt`; in memory
only, the engine and the real histories hidden meanwhile, nothing saved),
`--es md_screen <name>` opens a screen (`core/DebugScreens.kt`), with
`md_device`, `md_topic`, `md_query`, and `--es md_theme light|dark`. The
script never installs, uninstalls, grants permissions or touches the lock
screen; it puts the system’s dark mode back as it was and ends sample mode
by opening the app normally. Not run yet: no phone was connected, so
`screenshots/android/` is empty for now.

## Desktop (103 pictures, 14.4 MB)

| File | What it shows | Theme |
|---|---|---|
| [001-sign-in-dark.jpg](desktop/001-sign-in-dark.jpg) | Sign in (username, password, device key; join with a code) | dark |
| [002-sign-in-light.jpg](desktop/002-sign-in-light.jpg) | Sign in (username, password, device key; join with a code) | light |
| [003-setup-name-dark.jpg](desktop/003-setup-name-dark.jpg) | First-run setup, step 1 Name this device | dark |
| [004-setup-devices-dark.jpg](desktop/004-setup-devices-dark.jpg) | First-run setup, step 2 Add your other devices | dark |
| [005-setup-features-dark.jpg](desktop/005-setup-features-dark.jpg) | First-run setup, step 3 Choose what to use | dark |
| [006-setup-permissions-dark.jpg](desktop/006-setup-permissions-dark.jpg) | First-run setup, step 4 Permissions | dark |
| [007-setup-connection-dark.jpg](desktop/007-setup-connection-dark.jpg) | First-run setup, step 5 Connection check | dark |
| [008-home-dark.jpg](desktop/008-home-dark.jpg) | Your devices: status, link quality, health banner | dark |
| [009-home-light.jpg](desktop/009-home-light.jpg) | Your devices: status, link quality, health banner | light |
| [010-device-phone-dark.jpg](desktop/010-device-phone-dark.jpg) | Pixel 8 (Android phone, through the relay): top of its page | dark |
| [011-device-phone-light.jpg](desktop/011-device-phone-light.jpg) | Pixel 8 (Android phone, through the relay): top of its page | light |
| [012-device-phone-00720px-dark.jpg](desktop/012-device-phone-00720px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (720 px) | dark |
| [013-device-phone-01440px-dark.jpg](desktop/013-device-phone-01440px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (1440 px) | dark |
| [014-device-phone-02160px-dark.jpg](desktop/014-device-phone-02160px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (2160 px) | dark |
| [015-device-phone-02880px-dark.jpg](desktop/015-device-phone-02880px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (2880 px) | dark |
| [016-device-phone-03600px-dark.jpg](desktop/016-device-phone-03600px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (3600 px) | dark |
| [017-device-phone-04320px-dark.jpg](desktop/017-device-phone-04320px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (4320 px) | dark |
| [018-device-phone-05040px-dark.jpg](desktop/018-device-phone-05040px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (5040 px) | dark |
| [019-device-phone-05760px-dark.jpg](desktop/019-device-phone-05760px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (5760 px) | dark |
| [020-device-phone-06480px-dark.jpg](desktop/020-device-phone-06480px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (6480 px) | dark |
| [021-device-phone-07200px-dark.jpg](desktop/021-device-phone-07200px-dark.jpg) | Pixel 8 (Android phone, through the relay): further down the page (7200 px) | dark |
| [022-device-mac-dark.jpg](desktop/022-device-mac-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): top of its page | dark |
| [023-device-mac-light.jpg](desktop/023-device-mac-light.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): top of its page | light |
| [024-device-mac-00720px-dark.jpg](desktop/024-device-mac-00720px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (720 px) | dark |
| [025-device-mac-01440px-dark.jpg](desktop/025-device-mac-01440px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (1440 px) | dark |
| [026-device-mac-02160px-dark.jpg](desktop/026-device-mac-02160px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (2160 px) | dark |
| [027-device-mac-02880px-dark.jpg](desktop/027-device-mac-02880px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (2880 px) | dark |
| [028-device-mac-03600px-dark.jpg](desktop/028-device-mac-03600px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (3600 px) | dark |
| [029-device-mac-04320px-dark.jpg](desktop/029-device-mac-04320px-dark.jpg) | Pankaj’s MacBook Pro (Mac laptop, connected directly): further down the page (4320 px) | dark |
| [030-device-windows-dark.jpg](desktop/030-device-windows-dark.jpg) | Office PC (Windows desktop, asleep): top of its page | dark |
| [031-device-windows-light.jpg](desktop/031-device-windows-light.jpg) | Office PC (Windows desktop, asleep): top of its page | light |
| [032-device-windows-00720px-dark.jpg](desktop/032-device-windows-00720px-dark.jpg) | Office PC (Windows desktop, asleep): further down the page (720 px) | dark |
| [033-device-windows-01440px-dark.jpg](desktop/033-device-windows-01440px-dark.jpg) | Office PC (Windows desktop, asleep): further down the page (1440 px) | dark |
| [034-device-windows-02160px-dark.jpg](desktop/034-device-windows-02160px-dark.jpg) | Office PC (Windows desktop, asleep): further down the page (2160 px) | dark |
| [035-device-windows-02880px-dark.jpg](desktop/035-device-windows-02880px-dark.jpg) | Office PC (Windows desktop, asleep): further down the page (2880 px) | dark |
| [036-device-windows-03600px-dark.jpg](desktop/036-device-windows-03600px-dark.jpg) | Office PC (Windows desktop, asleep): further down the page (3600 px) | dark |
| [037-device-linux-dark.jpg](desktop/037-device-linux-dark.jpg) | Lab laptop (Linux laptop, connected directly): top of its page | dark |
| [038-device-linux-light.jpg](desktop/038-device-linux-light.jpg) | Lab laptop (Linux laptop, connected directly): top of its page | light |
| [039-device-linux-00720px-dark.jpg](desktop/039-device-linux-00720px-dark.jpg) | Lab laptop (Linux laptop, connected directly): further down the page (720 px) | dark |
| [040-device-linux-01440px-dark.jpg](desktop/040-device-linux-01440px-dark.jpg) | Lab laptop (Linux laptop, connected directly): further down the page (1440 px) | dark |
| [041-device-linux-02160px-dark.jpg](desktop/041-device-linux-02160px-dark.jpg) | Lab laptop (Linux laptop, connected directly): further down the page (2160 px) | dark |
| [042-device-linux-02880px-dark.jpg](desktop/042-device-linux-02880px-dark.jpg) | Lab laptop (Linux laptop, connected directly): further down the page (2880 px) | dark |
| [043-device-linux-03600px-dark.jpg](desktop/043-device-linux-03600px-dark.jpg) | Lab laptop (Linux laptop, connected directly): further down the page (3600 px) | dark |
| [044-links-map-dark.jpg](desktop/044-links-map-dark.jpg) | How your devices are connected (map and list) | dark |
| [045-links-map-light.jpg](desktop/045-links-map-light.jpg) | How your devices are connected (map and list) | light |
| [046-health-dark.jpg](desktop/046-health-dark.jpg) | Health check: green, grey, amber and red lines with fixes | dark |
| [047-health-light.jpg](desktop/047-health-light.jpg) | Health check: green, grey, amber and red lines with fixes | light |
| [048-activity-dark.jpg](desktop/048-activity-dark.jpg) | Activity timeline (files, clipboard, messages, calls, connections) | dark |
| [049-activity-light.jpg](desktop/049-activity-light.jpg) | Activity timeline (files, clipboard, messages, calls, connections) | light |
| [050-transfers-dark.jpg](desktop/050-transfers-dark.jpg) | Transfers: under way and recent, with Pause, Retry, Show in Finder | dark |
| [051-transfers-light.jpg](desktop/051-transfers-light.jpg) | Transfers: under way and recent, with Pause, Retry, Show in Finder | light |
| [052-notes-dark.jpg](desktop/052-notes-dark.jpg) | Notes list | dark |
| [053-notes-light.jpg](desktop/053-notes-light.jpg) | Notes list | light |
| [054-notes-open-dark.jpg](desktop/054-notes-open-dark.jpg) | A checklist note open, with a reminder | dark |
| [055-timers-dark.jpg](desktop/055-timers-dark.jpg) | Shared timers (running, paused) and New timer | dark |
| [056-timers-light.jpg](desktop/056-timers-light.jpg) | Shared timers (running, paused) and New timer | light |
| [057-read-later-dark.jpg](desktop/057-read-later-dark.jpg) | Read later links | dark |
| [058-read-later-light.jpg](desktop/058-read-later-light.jpg) | Read later links | light |
| [059-search-dark.jpg](desktop/059-search-dark.jpg) | Search across messages, notes, transfers, clipboard and more | dark |
| [060-search-light.jpg](desktop/060-search-light.jpg) | Search across messages, notes, transfers, clipboard and more | light |
| [061-help-dark.jpg](desktop/061-help-dark.jpg) | Help topics | dark |
| [062-help-light.jpg](desktop/062-help-light.jpg) | Help topics | light |
| [063-help-topic-dark.jpg](desktop/063-help-topic-dark.jpg) | A Help topic (Getting started) | dark |
| [064-settings-dark.jpg](desktop/064-settings-dark.jpg) | Settings, top of the page | dark |
| [065-settings-light.jpg](desktop/065-settings-light.jpg) | Settings, top of the page | light |
| [066-settings-account-dark.jpg](desktop/066-settings-account-dark.jpg) | Settings → Account: signed-in user, Add a device, Sign out | dark |
| [067-settings-permissions-dark.jpg](desktop/067-settings-permissions-dark.jpg) | Settings → What each device may do here (control, screen, mouse & keyboard) | dark |
| [068-settings-removed-devices-dark.jpg](desktop/068-settings-removed-devices-dark.jpg) | Settings → Removed devices, with Allow again | dark |
| [069-settings-key-rotation-dark.jpg](desktop/069-settings-key-rotation-dark.jpg) | Settings → Change passphrase on all devices | dark |
| [070-settings-health-dark.jpg](desktop/070-settings-health-dark.jpg) | Settings → Health check and setup tile | dark |
| [071-settings-device-dark.jpg](desktop/071-settings-device-dark.jpg) | Settings → This device: name, notifications, clipboard privacy, codes, tray battery, nearby, Wi-Fi Direct | dark |
| [072-settings-shortcuts-dark.jpg](desktop/072-settings-shortcuts-dark.jpg) | Settings → Keyboard shortcuts | dark |
| [073-settings-calls-dark.jpg](desktop/073-settings-calls-dark.jpg) | Settings → Calls on this computer (Bluetooth hands-free), dial pad | dark |
| [074-settings-computer-notifs-dark.jpg](desktop/074-settings-computer-notifs-dark.jpg) | Settings → This computer’s notifications on your phone | dark |
| [075-settings-wake-dark.jpg](desktop/075-settings-wake-dark.jpg) | Settings → Wake this computer from your other devices | dark |
| [076-settings-file-manager-dark.jpg](desktop/076-settings-file-manager-dark.jpg) | Settings → File manager “Send to” menu | dark |
| [077-settings-sync-dark.jpg](desktop/077-settings-sync-dark.jpg) | Settings → Synced folders | dark |
| [078-settings-downloads-dark.jpg](desktop/078-settings-downloads-dark.jpg) | Settings → Download on another device | dark |
| [079-settings-commands-dark.jpg](desktop/079-settings-commands-dark.jpg) | Settings → Commands your devices may run here | dark |
| [080-settings-kvm-dark.jpg](desktop/080-settings-kvm-dark.jpg) | Settings → Share mouse and keyboard (layout of computers) | dark |
| [081-settings-insert-dark.jpg](desktop/081-settings-insert-dark.jpg) | Settings → Insert from phone (photo or scan) | dark |
| [082-settings-print-dark.jpg](desktop/082-settings-print-dark.jpg) | Settings → Print from your phone | dark |
| [083-settings-schedules-dark.jpg](desktop/083-settings-schedules-dark.jpg) | Settings → Schedules on this computer | dark |
| [084-settings-automations-dark.jpg](desktop/084-settings-automations-dark.jpg) | Settings → Automations (when this happens, do that) | dark |
| [085-settings-webhooks-dark.jpg](desktop/085-settings-webhooks-dark.jpg) | Settings → Webhooks (local HTTP API) | dark |
| [086-settings-browser-extension-dark.jpg](desktop/086-settings-browser-extension-dark.jpg) | Settings → Browser extension | dark |
| [087-settings-key-dark.jpg](desktop/087-settings-key-dark.jpg) | Settings → Device key | dark |
| [088-settings-backup-dark.jpg](desktop/088-settings-backup-dark.jpg) | Settings → Back up and restore settings | dark |
| [089-settings-updates-dark.jpg](desktop/089-settings-updates-dark.jpg) | Settings → Updates | dark |
| [090-settings-appearance-dark.jpg](desktop/090-settings-appearance-dark.jpg) | Settings → Appearance: five themes and the language | dark |
| [091-developer-dark.jpg](desktop/091-developer-dark.jpg) | Developer screen (hidden; tap the version 7 times) | dark |
| [092-quick-panel-dark.jpg](desktop/092-quick-panel-dark.jpg) | The menu-bar / tray quick panel (its own 360×540 window) | dark |
| [093-quick-panel-light.jpg](desktop/093-quick-panel-light.jpg) | The menu-bar / tray quick panel (its own 360×540 window) | light |
| [094-dialog-key-changed-dark.jpg](desktop/094-dialog-key-changed-dark.jpg) | A device’s security key changed (banner on its page) | dark |
| [095-dialog-permission-dark.jpg](desktop/095-dialog-permission-dark.jpg) | A device asks for a permission it hasn’t got | dark |
| [096-dialog-permission-light.jpg](desktop/096-dialog-permission-light.jpg) | A device asks for a permission it hasn’t got | light |
| [097-dialog-download-dark.jpg](desktop/097-dialog-download-dark.jpg) | A device asks this computer to download a link | dark |
| [098-dialog-print-dark.jpg](desktop/098-dialog-print-dark.jpg) | A phone asks to print here | dark |
| [099-dialog-incoming-call-dark.jpg](desktop/099-dialog-incoming-call-dark.jpg) | Incoming call bar (calls through the phone) | dark |
| [100-dialog-ring-dark.jpg](desktop/100-dialog-ring-dark.jpg) | Another device rings this one (find my device) | dark |
| [101-dialog-hotspot-dark.jpg](desktop/101-dialog-hotspot-dark.jpg) | Join the phone’s hotspot for a big transfer | dark |
| [102-home-hindi-dark.jpg](desktop/102-home-hindi-dark.jpg) | Your devices, in Hindi | dark |
| [103-device-phone-hindi-dark.jpg](desktop/103-device-phone-hindi-dark.jpg) | The phone’s page, in Hindi | dark |
