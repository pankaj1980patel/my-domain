# Android battery and resource audit (2026-11)

Scope: `apps/android` (Kotlin), plus a read of the Rust core's long-running
loops (`crates/mdcore/src/engine/background.rs`, `mdns.rs`). Paths below are
relative to `apps/android/app/src/main/java/com/mydomain/android/`; line
numbers are after the fixes.

Status: **Fixed**, **OK** (looked at, nothing to do), **Open** (left, with why).

New pieces:

- `core/ResourceLedger.kt`: `ResourceBook` (pure, thread-safe) and the app's
  `ResourceLedger`. Every lock, foreground service, camera / audio / BLE /
  hotspot / GPS use and the long-lived listeners say `hold` / `release`;
  locks with a timeout drop out by themselves. Shown in the hidden Developer
  screen (Settings, tap the version 7 times) as **Resource ledger**
  (`ui/DeveloperScreen.kt:99`), refreshed every 2 s while on screen. An entry
  that never goes away is a leak.
- `Core.onScreen` / `Core.awaitOnScreen()` (`core/Core.kt:364`): the screens'
  repeating work waits while the app is off screen.
- `PowerPolicy.healthStep` / `multicastWanted`, `NetWatch.checkWakes`,
  `AutomationBook.waiting` / `nextTickMs`, `CameraFrames.pickFpsRange`: pure,
  tested in `BatteryTimersTest` and `ResourceLedgerTest` (JVM).

## 1. Wake locks, Wi-Fi locks, multicast lock, CPU loops

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 1 | `NetService.kt:473` | The Wi-Fi **multicast lock was held for the whole life of NetService** (always, screen off included). It turns off the Wi-Fi chip's multicast filter, so every mDNS / SSDP packet on the network wakes the phone. | **Fixed**: held only while the app is on screen, and for 30 s after a network change (`PowerPolicy.multicastWanted`, `MULTICAST_AFTER_CHANGE_MS`); not reference counted; released in `onDestroy`. In the background links open on demand and pushes reach the phone (the engine already closes idle links there). |
| 2 | `ComputerAudio.kt:377` | Partial wake lock taken with `acquire()` (no timeout); held until `stop()`. | **Fixed**: `acquire(60 s)`, renewed every 20 s by the listen loop while playing; a dead player can hold it at most a minute. Wi-Fi lock (no timeout API) still released on every stop path. |
| 3 | `ComputerAudio.kt:522` | When the output failed (`write < 0`) or threw, the player thread ended but the service kept going: the listen renewals, the Wi-Fi lock and the wake lock went on with nothing playing. | **Fixed**: the player's `finally` stops the service (with "can't play") if it was still meant to play. |
| 4 | `ComputerAudio.kt:492` | Waiting for the first frames, the player slept 10 ms in a loop (100 wake-ups a second for up to 45 s). | **Fixed**: 40 ms. |
| 5 | `MdMessagingService.kt:76` | Each push made a **new** 30 s wake lock; a burst of pushes stacked many. | **Fixed**: one non-reference-counted lock, each push restarts its 30 s timeout. |
| 6 | `DeviceActions.kt:283` | Keep awake: full wake lock with a timeout (user asked, at most 24 h), released on 0. | OK (ledger entry added). |
| 7 | `DeviceActions.kt` ring | `MediaPlayer.setWakeMode` while ringing; 30 s timeout and Stop release it. | OK |
| 8 | `PhoneAudio.kt` | No locks taken (capture runs inside a media-projection foreground service). | OK |

## 2. Foreground services

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 9 | `AndroidManifest.xml` | Types: NetService `dataSync`, ScreenShare `mediaProjection`, CameraShare `camera`, PhoneAudio `mediaProjection`, ComputerAudio `mediaPlayback`; each `startForeground` passes the same type on Q+/R+. | OK |
| 10 | `PhoneAudio.kt:442` | The capture loop released the `AudioRecord` only on a normal exit; an exception (e.g. from the core) left it recording and the service up. | **Fixed**: `try/finally` releases it on every path, then the service stops. |
| 11 | ScreenShare / CameraShare / PhoneAudio / ComputerAudio | Started only on consent; re-starts while running reuse the running one; stop on: Stop action, viewer/listener idle 5 s after the grace, screen off / keyguard, link lost, projection stopped, `onDestroy`. | OK (ledger entries added) |
| 12 | NetService | Always-on by design (sticky, the link to your computers); `ServiceKeeper` restarts it. | OK |
| 13 | `PrintToComputer.kt` | A `PrintService` (system-bound, not a foreground service); jobs cancelled in `end()`. | OK |
| 14 | `PhoneHotspot.kt:58` | LocalOnlyHotspot reservation: closed on "leave", on a replacing request, on a bad config; the core always sends "leave" (session idle 60 s / setup timeout). | OK (ledger entry added) |

## 3. Timers, handlers, coroutines

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 15 | `ui/AppShell.kt:206` | Health check every minute (server every 10) from a `LaunchedEffect`, which **kept running while the app was in the background** (a stopped activity keeps its composition). | **Fixed**: waits while off screen; `PowerPolicy.healthStep`: every 1 min / server 10 min, while saving battery every 5 min / server 30 min; a return to the screen runs it at once if due. |
| 16 | `ui/AppShell.kt:239` | Link-quality poll every 5 s, also in the background. | **Fixed**: only on screen. |
| 17 | `ui/AppShell.kt:78` | Not signed in: `Core.reload()` every 500 ms forever, background included. | **Fixed**: only on screen. |
| 18 | `ui/MediaTile.kt:63` | A computer's media **watch renewed in the background** (the computer kept sending media updates), plus a 1 s clock. | **Fixed**: both wait while off screen (the watch lapses). |
| 19 | `ui/TouchpadUi.kt:147` | The computer's screen watch renewed while the app was off screen: **the computer kept streaming its screen to a phone in a pocket**. | **Fixed**: waits while off screen. |
| 20 | `ui/SettingsScreen.kt:305,336,363`, `ui/SetupScreen.kt:174`, `ui/AudioRouteUi.kt:89`, `ui/LinksMapScreen.kt:185`, `ui/TransfersScreen.kt:92`, `ui/TimersScreen.kt:83,307` (250 ms), `ui/NotesScreen.kt:193` | Polls (2-5 s) and clocks running while the app was off screen. | **Fixed**: all wait while off screen. |
| 21 | `ServiceKeeper.kt:84` | A **waking** alarm every ~15 min forever (plus a prefs write each time), even while the service was fine. | **Fixed**: the usual check is a non-waking alarm (runs at the phone's next wake; a push starts the service anyway); only the quicker checks after the service was killed wake the phone (`NetWatch.checkWakes`); prefs written only on change. |
| 22 | `Automations.kt:234` | A 15 s handler tick for as long as NetService runs, with or without automations. | **Fixed**: 15 s only while a link is down (waiting to count as "disconnects") or a Wi-Fi name may still come (8 looks); otherwise every 5 min as a safety net (`AutomationBook.nextTickMs`). Links and Wi-Fi changes still arrive by events. |
| 23 | `PhoneLocation.kt:105` | Before Android 11, `requestSingleUpdate` had **no timeout: indoors the GPS could search indefinitely**. On 11+, a new thread per fix (never shut down). | **Fixed**: removeUpdates after 60 s; one shared executor and a cancel signal on 11+. |
| 24 | `NetService.kt` | Network-change debounce, status push only on change, no reconnect loop. | OK |
| 25 | `NoteReminders.kt`, `TimerAlarms.kt` | Exact alarms only for user-set reminders/timers (inexact fallback). | OK |
| 26 | `ScreenSettings.kt` grant watch | 2 s check bounded by a deadline. | OK |
| 27 | `WifiDirect.kt` | Group / owner polls bounded by `POLLS`. | OK |
| 28 | Core `engine/background.rs` | 1-2 s loops (outbox retry, peer cache, clipboard), heartbeat for WebRTC only, idle links closed after 3 min in the background (`idle_close_after`), mDNS thread `recv_timeout(500 ms)`. Thread sleeps don't wake a suspended phone; the radio-waking part (keep-alives) is already handled by the idle closer. | OK, no Rust change |

## 4. Listeners and callbacks

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 29 | `NetService.kt:564` | The **clipboard listener was never removed**: a recreated NetService added another (duplicate syncs) and leaked the old service. | **Fixed**: kept and removed in `onDestroy`. |
| 30 | `Radios.kt:64`, `WifiNetworks.kt:68`, `ScreenSettings.kt:70` | Receivers / content observers registered on the app context, never unregistered. | **Fixed**: `stop()` from `NetService.onDestroy` (init registers again). |
| 31 | `WifiDirect.kt:262` | The Wi-Fi P2P channel stayed open for the life of the process after one transfer. | **Fixed**: closed 10 s after the last request left (API 27+); the next request opens a new one. |
| 32 | `BleLink.kt:117` | `stop()` before the GATT service was added left `pendingAdvertiser` set: advertising could start after the stop. | **Fixed**: cleared on stop; `onServiceAdded` checks the server is still open. |
| 33 | NetService network callback, battery receiver; PowerSaver; PhotoBackup; DndWatch; LostMode; Automations Wi-Fi callback | Registered once, unregistered in `stop()` / `onDestroy`. | OK (ledger entries for NetService and Automations) |
| 34 | CameraShare / InsertCapture | CameraDevice, session, ImageReaders, orientation listener and screen-off receiver closed on every stop path (on the camera thread, then `quitSafely`). | OK |
| 35 | ScreenShare | VirtualDisplay, ImageReader, projection, screen-off receiver released in `stop()`. | OK |
| 36 | `NowPlaying.kt` | MediaSession listener for the process (needs notification access); controller callbacks re-registered on each session change. | OK (ledger entry) |
| 37 | NsdManager / BLE scanning | Not used on Android (mDNS is in the Rust core; the phone only advertises BLE, never scans). | OK |

## 5. BLE duty cycle and mDNS

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 38 | `BleLink.kt` | Connectable advertising, `ADVERTISE_MODE_LOW_POWER` (~1 Hz), no scanning. | OK |
| 39 | Core `mdns.rs` | Advertise + browse for the engine's lifetime. With the multicast lock off in the background (#1) the phone no longer receives the LAN's multicast while in a pocket. | Fixed via #1 |
| 40 | BLE advertising while Battery Saver | Still advertises (it's the fallback with no shared network). | **Open**: could pause while `PowerSaver.current.saving`; left on purpose (low power mode, and it's how a computer reaches the phone with no Wi-Fi). |

## 6. Memory

| # | Where | Finding | Status |
|---|-------|---------|--------|
| 41 | `ScreenShare.kt:448` | Two new ~4 MB bitmaps every 500 ms while sharing. | **Fixed**: one reused bitmap (recycled at stop, on the share's thread); the cropped copy is recycled after encoding. |
| 42 | `CameraShare.kt:480,459` | Every preview frame (~30 a second) made two calls (lock screen, who watches) before the 12/s throttle; the camera ran at its full rate. | **Fixed**: throttle first; the AE target range is the slowest one of at least 12 fps (`CameraFrames.pickFpsRange`). |
| 43 | `NowPlaying.kt:182` | The artwork cache kept the app's (possibly large) artwork bitmap alive; scaled copies and drawn icons not recycled; icon cache unbounded. | **Fixed**: weak reference; temporaries recycled; icons at most 32 (LRU). |
| 44 | `NotifListener.kt:265,304` | Per notification: scaled copy, the JPEG canvas and drawn icons left to GC. | **Fixed**: recycled after encoding. |
| 45 | `Gallery.kt`, `core/ClipLog.kt` | Thumbnails recycled after encoding. | OK |
| 46 | `InsertCapture.kt` | Scan pages kept as bitmaps while the scan screen is open (shot at most 2400 px). | **Open**: not recycled when a filter replaces one (Compose may still draw the old one); freed by GC when the screen closes. |
| 47 | Feeds and caches | ActivityFeed 1000, BatteryLog 1000/device, ChatLog / ClipLog / TransferLog capped, NotifListener shared keys 500, code dedupe pruned at 2 min. | OK |

## Totals

47 findings: 24 fixed (#1-5, 10, 15-23, 29-32, 39, 41-44), 21 OK, 2 open (#40, #46).

## Not done here

- No device measurements (no adb in this round): `adb shell dumpsys batterystats`
  / `dumpsys power` with the screen off for an hour, before and after, would
  confirm #1, #15-22. The Developer screen's Resource ledger is the quick
  check: with the app in the background and nothing shared, it should list
  only NetService, its listeners and (if Bluetooth is on) BLE advertising.
- No Rust changes: the core's loops were reviewed (#28) and left alone.
