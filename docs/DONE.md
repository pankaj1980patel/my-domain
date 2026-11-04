# What's done

Status as of **2026-11-02**. Everything below is committed on the `monorepo`
branch, and was built and tested unless a line says otherwise. What's still
left is in [PENDING.md](PENDING.md).

## At a glance

| | |
|---|---|
| Tests | **122** core tests, plus **10** server tests and **1** desktop test |
| Where the core tests pass | macOS (dev Mac), Linux (Manjaro laptop), **Android phone** (I2405, Android 16, ARM64, run on the device) |
| Where the server and desktop tests pass | macOS and Linux |
| Default transport | QUIC (encrypted, reliable, multiplexed) |
| Running right now | Mac desktop, Linux desktop, Android app, all logged in as the test account `mdtest` |

## Phase 0: Ground work

- **Monorepo.** The six separate repos (core, server, desktop, android, ios,
  mac) are now one Cargo workspace, with every repo's git history kept
  (`git subtree`). Why: [ADR-0001](../learnings/decisions/ADR-0001-monorepo.md).
  - One `Cargo.lock` and one `target/` folder for all crates.
  - Shared dependency versions and lint rules
    ([Cargo.toml](../Cargo.toml), [clippy.toml](../clippy.toml)).
- **CI** in [.github/workflows](../.github/workflows):
  - `ci.yml`:
    - formatting
    - clippy with warnings treated as errors
    - tests on Linux and macOS
    - the Android APK build
    - licence and security-advisory checks (`cargo-deny`)
  - `server-release.yml`: builds the server binary for release.
- **Secrets cleaned up.**
  - The real `JWT_SECRET` was removed from `apps/server/deployment.dev.md`.
  - `.env`, `fcm-sa.json`, `google-services*.json` and the lab's
    `test-creds.env` are gitignored.
  - The full git history was checked: no real secret, Mongo password or
    private key is in it.
- **Device lab** in [scripts/lab](../scripts/lab):
  - The machines: the Mac, the Manjaro laptop `192.168.0.176` (Linux node and
    Android build host), and the phone on the laptop's USB.
  - Laptop setup: system upgraded, fast pacman mirrors chosen, parallel
    downloads turned on, JDK 17, Android SDK/NDK.
- **[learnings/](../learnings/README.md)**: the project explained for a
  Node.js developer. It has 8 guides, 6 decision records (ADRs) and a journal
  entry per phase.

## Phase 1: Tests first, then a safe refactor

**Refactors**
- **Typed errors.** `CoreError` replaces error strings across the engine
  ([error.rs](../crates/mdcore/src/error.rs)). Both apps now get structured
  errors.
- **Locks that can't get stuck after a crash.** `parking_lot` mutexes
  replace the standard ones. With the standard ones, one panic would poison a
  lock and stop the engine.
- **Events describe themselves.** `CoreEvent` now converts itself to JSON, so
  the hand-written event JSON in the desktop and Android apps is gone
  ([events.rs](../crates/mdcore/src/events.rs)). Contract tests pin the JSON
  format.
- **Settings and seams for tests.**
  - `CoreConfig` holds STUN servers, which connection methods are enabled,
    auto-QUIC, and the relay URL.
  - The `Signaler` trait lets tests use in-memory signaling.
- **Engine split up.** The 1,099-line `engine.rs` became
  [engine/](../crates/mdcore/src/engine): `session`, `peers`, `messaging`,
  `sharing`, `signaling`, `ladder`, `background`. Background threads now stop
  when the engine is dropped.
- **No more transport strings.** `SendPath` and `Directed` enums replace
  passing `"WS"`-style strings around.
- **One key-value store.** `FileKv` lives in the core and writes atomically.
  The desktop uses it, and Android has a persistent store for the first time.

**The testkit** ([md-testkit](../crates/md-testkit/src)):
- `MockPlatform`: a fake device that runs on loopback.
- `RecordingSink`: records every event, and a test can wait for a specific one.
- `TestNode`: one test engine.
- `SignalHub`: in-memory signaling between test engines.
- `FakeRegistry`: a fake registry server.
- `FakeRelay`: a fake relay server.

**Security fixes**
- The TCP receiver has a size cap, a timeout and a connection limit. Before,
  it had no cap, no timeout, and started a thread per connection.
- Messages that fail to decrypt are dropped, not shown in the UI. Floods of
  them are rate-limited.
- STUN replies must carry our transaction id, and stray packets are ignored.
- LAN beacons can't overwrite a peer the registry told us about (anti-spoofing).
  At most 256 LAN peers are kept.
- A WebRTC connection never replaces a link that is already live.
- Query strings are URL-encoded.

## Phase 2: Networking (QUIC + relay)

Why tokio and QUIC: [ADR-0002](../learnings/decisions/ADR-0002-tokio-quic.md).
How the networking works: [03-networking.md](../learnings/03-networking.md).

- **QUIC is the default transport.** `send()` dials QUIC the first time it's
  needed and reuses the connection after that
  ([transport/quic.rs](../crates/mdcore/src/transport/quic.rs)).
  - It replaces a new TCP connection per message and lossy UDP.
  - A failed dial is remembered for 30 s, so an unreachable peer doesn't cost
    a timeout on every message.
- **Hole punching now leads into QUIC.** After a successful punch, a QUIC
  connection runs on the same socket
  ([transport/punch.rs](../crates/mdcore/src/transport/punch.rs)). The old
  datagram chunking code (with its 1 MiB limit) was deleted.
- **The server stores `quic_port`.** Devices learn each other's QUIC port from
  the registry.
- **Relay fallback.**
  - Server: a WebSocket at `/relay`
    ([apps/server/src/relay.rs](../apps/server/src/relay.rs)). It only
    forwards between devices of the same account, and it can't read the
    messages because they are end-to-end encrypted.
  - Client: [transport/relay.rs](../crates/mdcore/src/transport/relay.rs).

### Connect ladder today

The engine tries each method in order, cheapest first, until one works
([engine/ladder.rs](../crates/mdcore/src/engine/ladder.rs)):

| Rung | Method | Needs the server? | Tested in |
|---|---|---|---|
| 0 | Direct QUIC (IPv6, router mapping, LAN IPv4) | no | [connect_ladder.rs](../crates/mdcore/tests/connect_ladder.rs) |
| 1 | Signaled QUIC: swap addresses through a signal, both sides dial | signal (own links, else FCM) | connect_ladder.rs |
| 2 | Relay through the server, right away; the rungs below try to upgrade it | yes | [relay.rs](../crates/mdcore/tests/relay.rs) |
| 3 | STUN + hole punch, then QUIC on the punched socket | signal + STUN | connect_ladder.rs |
| 4 | WebRTC | signal | connect_ladder.rs |

Every message rides a live link. The old one-off paths (a TCP connection or
a UDP datagram per message, direct and signaled WebSocket links, the LAN UDP
ping) were removed on 2026-11-02, about 800 lines; QUIC covers each case.

## Backups per computer, phone as touchpad, DND follows the phone (2026-11-03)

Checks: the new core tests (backup, remote_input, dnd, unit tests) and the
desktop crate's tests passed on the Linux laptop; on the Mac, clippy with
`-D warnings` for mdcore, mdctl and the desktop crate (so the macOS
Accessibility and enigo code compiles) and the new tests again. Still to
run when the laptop is back: workspace clippy and the full workspace tests.

Tried live (phone with the new APK, mdctl on the Mac as the computer with
`input-allow on`, on different networks: relay, then hole punched):
- The phone showed the "Touchpad & keyboard" tile for mdctl. A drag
  arrived as about one `move` per 20 ms (summed), then a tap as a left
  click, Right click, and "hello phone" from the Type field.
- Do Not Disturb on the phone (on, then off): mdctl got `dnd-changed`
  on=true and on=false, and `dnd-active` listed the phone, then nobody.
- Added after: three- and four-finger gestures and an Enlarge button.
  A three-finger tap is a middle click. Swipes go as `gesture {fingers,
  dir}` and the computer picks its own shortcut: three fingers sideways
  switch apps (Cmd+Tab on a Mac, Alt+Tab elsewhere), four fingers sideways
  switch desktops (Ctrl+arrow on a Mac, Ctrl+Win+arrow on Windows,
  Ctrl+Alt+arrow on Linux), up shows all windows, down the desktop (App
  Exposé on a Mac). Enlarge opens the pad full screen with Left, Right and
  Done. mdctl: `gesture <node> 3|4 left|right|up|down`, `click <node>
  middle`. The full-screen pad is its own screen (`TouchpadActivity`): as
  a Compose dialog its window was fitted between the system bars but laid
  out at the screen's full height, so the bottom row sat under the
  navigation bar. Tried on the phone: everything above the bars, pad,
  buttons and Done work. The gestures need real fingers (adb fakes only
  one).
- On the vivo phone a three-finger swipe still takes a screenshot: the
  phone handles it before any app, and apps can't turn it off. A setting
  ("Block screenshots on the full-screen touchpad", on by default) marks
  the full-screen pad's window secure, which helps on some phones but not
  this one. So the full-screen pad also has buttons for the swipes (◀ App,
  App ▶, All windows, ◀ Desktop, Desktop ▶, Show desktop); all six reached
  mdctl as the right `gesture` events.
- The ←, ↑, ↓, →, Enter, ⌫ and Esc buttons: all seven arrived as `key`
  events.
- Two-finger scroll works (tried by hand). The Mac app (`tauri dev`, with
  Accessibility allowed) lists `input` in its status with no
  `input_needs`, so it plays input back.
- On the Mac, the full-screen pad's switch buttons all work (apps,
  desktops, all windows, show desktop), tried by hand.
- Not tried yet: real finger swipes (three and four fingers) and backup
  per computer on the phone.

- **Photo backup remembers each computer** (`features/backup.rs`). What was
  sent is kept per target device (`backup_sent_<tag>_<n>` chunks, the tag
  a short hash of the node id). Choosing another computer drops waiting
  items it already has, and the phone's next scan (it runs on every
  `backup_configure`) hands over everything since `since`; the core keeps
  only what this computer lacks. Choosing the old one again sends only
  what it's missing. The old shared list is moved to the first computer
  chosen after the update (then emptied), so nothing goes twice.
  `BackupStatus.sent` now counts the chosen computer's items.
  - Tests: `switching_computers_sends_what_the_new_one_lacks`,
    `the_old_shared_list_counts_for_the_chosen_computer`.
- **Phone as touchpad and keyboard** (`features/input.rs`,
  `engine/control.rs`). `input { from, events: [...] }`, each event
  `move {dx, dy}`, `click {button: left|right}`, `scroll {dx, dy}`,
  `text {text}` or `key {key: enter|backspace|left|right|up|down|esc}`
  (`model::InputEvent`).
  - API: `input_move`, `input_scroll` (batched: they add up and go out
    every 20 ms as one message), `input_click`, `input_text`, `input_key`,
    `send_input` (these flush pending moves first, so the order holds);
    `remote_input_allowed` / `set_remote_input_allowed` on the computer (on
    by default, kept in `kv`).
  - Only over a live link, never queued: with no link the call fails with
    `NoLink` and the connect ladder starts (at most every 5 s), so a click
    can't land minutes later.
  - The computer plays back only from devices in its list (and the message
    decrypted with your key), only while allowed; events are cleaned (64
    per message, moves within ±4000 px, text without control characters, up
    to 500 characters). Arrives as `CoreEvent::Input` (`input-event`).
  - Desktop (`remote_input.rs`): `enigo` 0.6 on one thread, in order;
    scroll pixels become wheel steps (20 px each). Status lists the `input`
    action while allowed, plus `input_needs` ("accessibility" on a Mac
    without the permission, "display" on Linux without an X server). On a
    Mac without Accessibility one notice per run says so instead of
    failing silently. Settings switch "Let my phone control this computer";
    commands `remote_input_allowed`, `set_remote_input_allowed`.
  - Android (`ui/TouchpadUi.kt`): a "Touchpad & keyboard" tile on the
    device screen when the computer lists `input`: a pad (drag to move,
    tap = left click, two-finger drag = scroll, two-finger tap = right
    click), Left/Right click buttons, a text field with Type, and arrow,
    Enter, Backspace and Esc keys. Each touch move goes to the core, which
    batches.
  - mdctl: `move <node> <dx> <dy> [times]`, `click <node> left|right`,
    `scroll <node> <dx> <dy>`, `type <node> <text...>`,
    `key <node> <key>`, `input-allow on|off`.
  - Tests: `touchpad_moves_are_batched_and_keep_their_order`,
    `a_computer_with_remote_input_off_ignores_it`,
    `input_is_never_queued_for_later`,
    `unknown_buttons_keys_and_devices_are_refused`, unit tests in
    `features/input.rs`, `events::input_and_dnd_events_keep_their_shape`,
    desktop `scroll_pixels_become_wheel_steps`.
- **Do Not Disturb follows the phone** (`features/dnd.rs`).
  `dnd { from, on, at }`, sent the reliable way, only on a change
  (`Engine::set_dnd`; the last state sent is kept, so a restart doesn't
  repeat it). Receivers keep the newest `at` per sender, keep who is in DND
  (`Engine::dnd_active`, forgotten after 12 h without news) and get
  `CoreEvent::Dnd` (`dnd-changed`).
  - Android (`DndWatch.kt`): reads `NotificationManager
    .getCurrentInterruptionFilter()` and listens for
    `ACTION_INTERRUPTION_FILTER_CHANGED` (plus the notification listener's
    `onInterruptionFilterChanged`). Reading needs no permission; nothing
    asks for notification-policy access, and the OS state is never changed.
  - Desktop: Settings switch "Follow my phone's Do Not Disturb" (off by
    default). While on and a device is in DND, my-domain's pop-ups are held;
    when DND ends one pop-up says how many came ("3 notifications: Asha,
    Files, Mom"). A ring and "can't use your phone as a touchpad" still
    show. Commands `follow_phone_dnd`, `set_follow_phone_dnd`,
    `dnd_devices`.
  - mdctl: `dnd on|off`, `dnd-active`.
  - Tests: `the_computer_hears_when_the_phone_goes_into_and_out_of_dnd`,
    `a_restarted_phone_does_not_repeat_its_state`, desktop
    `held_pop_ups_are_summed_up`.

## Copied pictures between devices (2026-11-03)

"Send the copied picture" in a device's Clipboard tile (desktop and phone):
the picture on this clipboard goes as a PNG file whose offer says
`"clipboard": true`; the receiver saves it like any file and also puts it
on its clipboard (`FileReceived.clipboard`).

- Core: `Engine::send_clipboard_image(node, path)`; incoming transfers
  remember the flag. Test `files::a_copied_picture_arrives_marked_for_the_clipboard`.
- Desktop (`clip_image.rs`): `arboard` image ↔ PNG with the `png` crate
  (already in the lock file); one clipboard handle kept for the app's life
  (an X11 picture lives as long as its owner). Command
  `send_clipboard_image` (binding by hand).
- Android: reads the clipboard's image (a button press, so the app is on
  screen as Android 10+ needs), re-encodes it as PNG and sends it; a
  received one goes on the clipboard through its saved file's Uri.
- mdctl: `send-picture <node> <png>`.

Tried: mdctl → the phone, the picture was saved (`file-sent`); whether it
landed on the phone's clipboard couldn't be seen over adb (no clipboard
command; the screen-share prompt was on top). Desktop side: clippy only.

## Find this phone: its location, and Location on/off (2026-11-04)

- `PhoneLocation.kt`, action `locate`: the phone puts `location {lat,
  lon, acc, at}` in its status: the last known position at once, then a
  fresh fix. The phone stays in charge:
  - off until "Let my computers see where this phone is" is turned on in
    its Settings ("Find this phone" tile); asked while off, it opens that
    screen (or notifies) and shares nothing;
  - a notification on the phone each time it's shared, naming who asked;
  - Android's location permission and Location switch still apply (the
    phone opens the prompt / settings when one is missing);
  - only on request, never in the background.
- Location on/off is a radio like the others (`location_on` in the
  status; the Accessibility helper taps "Location", Shizuku runs
  `cmd location set-location-enabled`).
- Desktop: Location among the radio buttons; a "Where it is" row in the
  device's controls (Locate, the position with accuracy and age, Open
  map). Command `open_map(lat, lon)` opens OpenStreetMap; it takes only
  coordinates, so the page can't open arbitrary links.
- Tried: with sharing off, Locate from mdctl opened my-domain on the phone
  and the status carried no position.
- Unavailable on the other device = disabled, with the reason on hover
  (desktop `Gate`: the wrapper carries the tooltip, as WebKit shows none
  on a disabled button): Locate while the phone's sharing is off; the
  phone keys while its Accessibility helper is off (the status now says
  `helper`, and `key` is always listed).

## Disconnect / Reconnect, Clear chat, the phone's Wi-Fi networks (2026-11-04)

- `Engine::disconnect` (close the link now; nothing is blocked, the next
  send connects again) and `Engine::reconnect` (close and connect afresh;
  just connect when there's no link). Desktop commands `disconnect`,
  `reconnect`; FFI; mdctl `disconnect <node>`, `reconnect <node>`. Tests
  `tests/link_buttons.rs`. Tried live against the phone from mdctl: both
  dropped the link; Reconnect brought it straight back over QUIC.
- Device page, both apps: Reconnect and Disconnect next to the status
  while connected (Connect when not, as before); Clear on the chat (this
  device's copy only; chat lives only in the apps' memory).
- The phone's Wi-Fi networks (`WifiNetworks.kt`, action `wifi`): `scan`
  (results in the status as `wifi_networks`: up to 20, one per name,
  strongest first, with signal, secured, saved, current), `connect`
  (a saved network, or a new one with its password), `disconnect`,
  `reconnect`. Possible because the app targets Android 9. Android shows
  scan results only with the location permission and Location on: the
  status says which is missing (`wifi_scan_needs`) and the phone posts a
  notification that fixes it with a tap; the location is never read.
  Desktop: a Wi-Fi section in the device's controls (Show networks, Join,
  a password box for a new secured network, Leave network, Rejoin).
  The prompt (or Location settings) opens on the phone straight away when
  the app may open a screen (on screen, or the Accessibility helper on);
  denied for good, the app's own settings open instead; a notification
  only as the last resort. Tried live: 17 networks on the computer,
  strongest first, current and saved marked; joining "WorkSpaceCo. Rise"
  from the desktop worked.
- Android builds for the phone now use `-Prust.targets=arm64` (CI still
  builds all four), to save disk.

## Targeting Android 9 on purpose; radios tried live (2026-11-03)

The app now targets SDK 28 (compileSdk stays 35, so newer APIs are still
called behind version checks). Apps targeting 28 may switch Wi-Fi
themselves (the Termux:API trick); the cost is no Play Store.
- `Sdk.rules(level)`: a newer Android's rules apply only when the phone
  runs it and the app targets it. Used for the Bluetooth (12), notification
  and photo (13) permissions. Legacy `BLUETOOTH`/`BLUETOOTH_ADMIN`,
  `READ_EXTERNAL_STORAGE` lost their `maxSdkVersion`; `CHANGE_WIFI_STATE`
  added. Lint's old-target checks are off for this.
- Tried on the vivo (Android 16), from mdctl on the Mac:
  - Wi-Fi on and off **directly**, no prompt, a few seconds each (even
    locked, by design).
  - Bluetooth: Android 16/vivo refuses `enable()` even for old targets,
    so it goes through the Accessibility helper: off and on in 6–8 s,
    tapping vivo's switch described "Bluetooth". The phone remembers the
    refusal and skips the direct call next time.
  - Phone key Home: the phone went from my-domain to the home screen.
  - Status lists `key` and `radio_direct` for all five radios.
  - Not tried remotely: airplane mode and mobile data (either can cut the
    phone off from the computer); the desktop now warns about that.
- Fixed in the helper: it searched whatever window was in front before
  Settings opened, and its "first switch" fallback tapped a checkbox
  outside Settings. It now acts only in Settings windows, and prefers a
  switch whose own label matches, then its row's.
- Photo access: the phone now asks for the older storage permission
  (it showed as not granted after the change); grant it again for photo
  backup and browsing.

## Radios switched on any phone: the Accessibility helper (2026-11-03)

Shizuku needs Developer options, which most phones never turn on, so the
no-tap path for everyone is an Accessibility service (`PhoneHelper.kt`),
turned on once in Settings → Accessibility. (Device Administrator was
considered: it can lock and wipe, but can't switch any radio.)

- Radios: the service opens the switch's settings panel, finds the switch
  (a checkable node whose row says Wi-Fi / Bluetooth / Airplane / Mobile
  data / Hotspot; for Wi-Fi and Bluetooth the panel's first switch), taps
  it, checks the radio really changed, and closes the panel. Not found in
  6 s (another maker's Settings): the panel stays open for you. Only while
  the phone is unlocked; locked, it falls back to the notification.
- Order in `Radios.set`: Shizuku, then the old direct APIs (Wi-Fi before
  Android 10, Bluetooth before 13), then the helper, then Android's
  Bluetooth prompt, then the panel. The status lists `radio_direct`.
- Phone keys: action `key {back|home|recents|notifications|quick_settings|lock}`
  while the helper is on.
- Desktop: the radio row says whether a tap switches directly or opens on
  the phone; a row of phone keys for phones that list `key`.
- Phone Settings: a "Control from your computers" tile (on/off, opens
  Accessibility settings).
- Checks: APK builds, installs, the service is listed; UI type check. Not
  tried yet: the service isn't turned on (your consent, on the phone).

## First CI run on main (2026-11-03)

`monorepo` fast-forwarded into `main` (307 commits) and CI run. Fixed what
it found:
- A flaky DND test that was a real bug: the connect ladder's "already
  running" guard was process-wide, keyed by device id, so a ladder left
  over from a replaced engine silently stopped the new engine's. Now keyed
  per engine (its QUIC `boot`).
- Restarts: a peer back with a new `boot` gets the outbox, our addresses
  and our Do Not Disturb again; a computer that restarted learns the
  phone's DND when the link comes up (test
  `a_restarted_computer_learns_the_state_when_the_link_comes_up`).
- The wake test accepts "couldn't send" (CI's macOS can't broadcast).
- The Android job's SDK setup asked for the removed `tools` package.
- cargo-deny: rustls 0.23.46, rustls-webpki 0.103.16, webpki-roots
  1.0.11, tokio 1.54.1, anyhow 1.0.104; the advisories with no reachable
  fix are listed in `deny.toml` with a reason each.
- Server redeployed from the CI release build.
- Two timing-sensitive tests made patient (network change, relay to
  direct). CI is green on `main` (run 41630842469): fmt, clippy,
  cargo-deny, the desktop UI, the Android build, tests on Linux and macOS.

## The phone's radios from a computer (2026-11-03)

Wave E's first step ([ADR-0006](../learnings/decisions/ADR-0006-shizuku.md),
the fallback half).

- Android (`Radios.kt`): the status now also says airplane mode, mobile
  data (the Settings switch) and hotspot, next to Wi-Fi and Bluetooth; a
  change by anyone pushes a new status at once.
- Action `radio {what: wifi|bluetooth|airplane|data|hotspot, on}`. Android
  lets an ordinary app switch few radios, so: Wi-Fi directly before
  Android 10, Bluetooth directly before 13 and on through Android's own
  "turn on Bluetooth?" prompt after; anything else opens the switch's
  settings panel on the phone (a notification to tap when the app isn't on
  screen). Switching with no tap waits for Shizuku.
- Desktop: a row of radio buttons in the device's controls, lit when on;
  a tap asks the phone and says to confirm there.
- Shizuku (`Privileged.kt`, `dev.rikka.shizuku:api` 13.1.5): with Shizuku
  running and allowed, Wi-Fi, Bluetooth, mobile data and airplane mode
  (Android 11+) switch with no tap (`svc` / `cmd connectivity`); the
  status lists them as `radio_direct`. The hotspot still opens Settings
  (it needs a name and password). The phone's Settings has a "Switch
  radios from your computers" tile: Get Shizuku / Open Shizuku / Allow,
  whichever is missing.
- Checks: APK builds and starts, UI type check. Tried live (2026-11-03,
  after the server redeploy): the phone, away from the Mac on mobile
  data, answered with the radio fields over a punched link, and a `radio`
  Wi-Fi-on from mdctl posted "mac lab: turn on Wi-Fi" on it. Shizuku not
  tried (not installed on the phone).

## Control another computer from this one (2026-11-03)

A computer's "Its screen" tile, for a device that also lists `input`:
clicking the picture clicks there (a `point` then a `click`), right-click
right-clicks, the wheel scrolls, and with the picture focused, keys
(Enter, Backspace, Esc, arrows) and typed characters go there. Desktop
command `send_input(node_id, events_json)` takes any list of input events
(binding by hand). Checks: desktop clippy, UI type check. Not tried live
yet (needs a second computer; the laptop is offline).

## See the phone's screen on the computer (2026-11-03)

The same `features::screen` the other way round: a computer watches the
phone.

- Android (`ScreenShare.kt`): the phone lists `screen` in its status.
  When a computer starts watching, Android's own screen-recording prompt
  shows (straight away on screen, else from a notification: Android won't
  start screens from the background); nothing is shared without a yes,
  every time. Then a foreground service (type `mediaProjection`) takes a
  720 px wide JPEG about twice a second while anyone watches, and stops
  30 s after the last watcher (or from its notification's Stop). FFI
  `screen_watchers`, `screen_frame`. Snapshots aren't parsed twice or
  logged.
- Desktop: command `screen_watch` (binding by hand), an "Its screen" tile
  on a device's page for devices that list `screen`: Show its screen /
  Stop, renewing every 10 s.

Checks: desktop clippy, UI type check, APK build. Tried: mdctl watching
the phone brought up Android's prompt; with one app shared (YouTube),
snapshots arrived about twice a second (about 100 KB each); after mdctl
stopped watching, they stopped and the sharing service ended by itself
within 30 s. The desktop tile itself: not tried yet.
Found on the way: after the phone's app restarted (a reinstall), mdctl
still counted its old link as live and sent into it ("queued" only showed
on the next status); see Known issues.

## Point & click on the computer's screen (2026-11-03)

With the computer's screen showing behind the full-screen pad, "Point &
click" makes a tap put the pointer where it landed on the picture, then
click there (the bars around a fitted picture don't count). New input
event `point {x, y}`: ten-thousandths of the main screen's width and
height from the top left (`features::input::POINT_SCALE`), clamped by
`clean`; the desktop scales it to the main display with `enigo`. Core
`Engine::input_point`, FFI `input_point`, mdctl `point <node> <x> <y>`.

Tried: mdctl → the Mac app, `point 5000 5000` put the pointer in the
middle of the 1280x800 screen and `point 2500 7500` at (319, 599 from the
top). On the phone, against the Mac app: the Mac's screen showed behind
the full-screen pad, and with Point & click on, a tap on the middle of the
picture's menu bar put the Mac's pointer at (639, 12) of 1280x800.

## Wake a sleeping computer from the phone (2026-11-03)

- Core (`features/wake.rs`): a computer's status may say its network card
  (`"wake": {"mac": "…"}`); the core remembers it per device (a sleeping
  computer sends no status). `Engine::can_wake`, `Engine::wake_device`
  broadcast the magic packet (six 0xFF, then the address sixteen times) to
  255.255.255.255 and this network's .255, UDP ports 9 and 7. Tests:
  `wake::addresses_parse_and_bad_ones_dont`,
  `the_magic_packet_is_six_ones_then_the_address_sixteen_times`,
  `device_actions::a_computer_that_says_how_to_wake_it_can_be_woken_later`.
- Desktop: the status carries the default route's card (macOS `route` +
  `ifconfig`, Linux `ip route` + `/sys/class/net`).
- Android: a "Wake it" button on a computer's page while it isn't
  connected and has said its address once.
- mdctl: `wake <node>`.

Tried: the Mac app (rebuilt by `tauri dev`) reports its address; mdctl
learned it and `wake` sent the packet. Actually waking a sleeping
computer: not tried yet.

## See a computer's screen on the phone (2026-11-03)

The full-screen touchpad shows the computer's screen behind the pad, so
you see what the pointer does.

- Core (`features/screen.rs`): `screen_watch {on}` phone → computer and
  `screen_frame {seq, data}` (base64 JPEG) back. A watch lasts 25 s; the
  phone renews it every 10 s while the screen shows, so a phone that went
  away stops the snapshots by itself. Only your own devices, only while
  allowed (`set_screen_allowed`, on by default, kept across restarts;
  turning it off ends every watch). Frames go only over a live link, at
  most 600 KB. `CoreEvent::ScreenWatch` (computer) and `ScreenFrame`
  (phone). Tests: `screen::a_watched_screen_sends_its_snapshots_to_the_phone`,
  `a_computer_that_doesnt_share_its_screen_ignores_watchers`,
  `snapshots_too_big_or_without_a_link_are_refused`.
- Desktop (`screen_share.rs`): about two snapshots a second while watched,
  1280 px wide, from the OS's tools: macOS `screencapture` + `sips` (about
  80 KB and 0.4 s a frame; needs Screen Recording, else the status says
  `screen_needs: screen_recording`), Linux `grim` (Wayland) or ImageMagick
  `import` (X11). A notice says when a device starts watching. Status lists
  `screen` while allowed. Commands `screen_allowed`, `set_screen_allowed`
  (bindings updated by hand, same shape as the generated ones).
- Android: the full-screen pad watches while open and draws the latest
  snapshot behind the pad (snapshots are decoded off the main thread and
  kept out of the developer log); a hint when the computer needs a
  permission or a tool.
- mdctl: `screen <node> [off]`, `screen-allow on|off`, `screen-send <jpeg>`.

Checks: mdcore, mdctl and desktop clippy on the Mac, the new tests, the
APK build. The Linux capture code hasn't been compiled (laptop offline).
Tried: mdctl watching the real Mac app got about two snapshots a second
(about 74 KB each) and they stopped when it stopped watching. On the
phone, the full-screen pad showed the Mac's screen behind it.
Phone → computer: a watch from mdctl brought up Android's "Record or share
your screen with my-domain" prompt on the phone (the yes is the owner's).

## Phone buttons for a computer's slides, keep awake, search and lock (2026-11-03)

The computer already offered these actions; the phone's device page had no
buttons for them. "Find & control" now shows, for each action the computer
lists: slides (◀ Slide, Black screen, Slide ▶, with a hint when the
computer needs Accessibility or xdotool/wtype), keep it awake (30 min,
1 hour, Let it sleep), a search box with YouTube, Spotify and Web, and
Lock it.

Tried on the phone against the Mac app: 30 min started `caffeinate -d -i
-t 1800` and Let it sleep stopped it; a web search opened in Chrome on the
Mac. Not pressed: Lock it and the slide buttons (they'd act on whatever
was in front on the Mac in use).

## Gallery, photo backup, battery alerts, keep awake (2026-11-03)

- Verified live 2026-11-03 from mdctl on the Mac (photo access granted
  with `pm grant`, as the Photos tile's button would):
  - Gallery: the phone's five newest photos and videos listed; with
    `thumbs`, three 160 px JPEG thumbnails arrived; a photo downloaded at
    exactly its listed size.
  - Battery alerts: the phone, at 100% and charging, sent "full" to the
    Mac.
  - Keep awake on the phone: the screen wake lock held for the minute
    asked and was released by "0".
  - Keep awake on the Linux laptop: refused by polkit, because the lab
    starts the app over ssh, outside the desktop session. It used to fail
    silently; now the computer shows a notice saying so.
  - Photo backup: turned on in the phone's Settings, Photos tile, with the
    Linux laptop as the target. With Wi-Fi off it said "Waiting for
    Wi-Fi"; once on Wi-Fi, a new image in DCIM/Camera reached
    `~/Downloads/Phone backup/I2405/2026-11/` within seconds, byte for byte
    (same MD5), and the tile said "Backed up: 1. Nothing waiting."
    Switching backup off and on again didn't send it a second time.

Core and both apps' backends are built and tested; the desktop has commands
but no screens for these yet.

- **Gallery** (`features/gallery.rs`): a computer asks a phone for its
  photos and videos, newest first, a page at a time
  (`Engine::list_gallery(node, offset, limit, thumbs)` → request id). On
  the phone the core asks the host (`GalleryRequest` event); the host
  answers with `gallery_reply` (items with their path and a small JPEG).
  The asker gets `GalleryListing` (id, name, type, size, date taken, size
  in pixels, video length; no paths) and then a `GalleryThumb` per item
  (base64 JPEG, sent one per message over the live link).
  `download_gallery_item(node, id, name)` fetches one with the normal
  resumable download; the phone only sends items it listed (it keeps id →
  path for the last 5000), so an id it never showed is refused ("list it
  again"). A device with no gallery (a computer) answers "no photo gallery";
  a phone without photo access answers so.
  - Android (`Gallery.kt`): MediaStore (images and videos, by date added),
    160 px thumbnails (`loadThumbnail` on Android 10+), at most 48 KB.
    `requestLegacyExternalStorage` added so Android 10 can read the files
    by path too.
  - Desktop commands `list_gallery`, `download_gallery_item`; events
    `gallery-listing`, `gallery-thumb`. `mdctl`: `gallery <node> [offset]
    [limit]`, `gallery-get <node> <id> [name]`.
- **Photo backup** (`features/backup.rs`): the phone sends new photos and
  videos to one chosen computer by itself. The core owns the queue and the
  decisions; the host reports new media (`backup_enqueue`) and conditions
  (`backup_conditions(unmetered, charging)`).
  - Only on unmetered Wi-Fi, optionally only while charging; when that
    ends mid-file the transfer is dropped quietly (`files::forget`) and the
    computer keeps its partial file; the same transfer id (from computer,
    item, try) resumes it later, also after a restart.
  - Everything sent is remembered in the key-value store (in chunks of 500
    keys), so nothing goes twice; and the computer answers "done" at once
    for a file it already has (same name, size and SHA-256), so even a
    reinstalled phone doesn't send it again.
  - Saved on the computer in `<downloads>/Phone backup/<phone name>/YYYY-MM`
    (the phone's local month). The file offer carries a `backup` field;
    the desktop shows no notice per backed-up file.
  - `backup_configure(enabled, target, charging_only)`, `backup_state()`
    (also as `BackupState` events: queued, sent, what it waits for, the
    current file and its progress, the last error).
  - Android (`PhotoBackup.kt`): a ContentObserver on photos and videos plus
    a network callback inside the running NetService (no WorkManager),
    scanning everything added since the backup was turned on. **UI hook**:
    a "Photos" tile in Settings with "Allow photo access", the backup switch,
    the computer to back up to, "Only while charging", a status line, and
    the battery-alerts switch (otherwise these couldn't be turned on).
- **Battery alerts** (`features/battery.rs`): worked out from the status
  each device already pushes (`set_device_status`), so phones and laptops
  both send them. Once per crossing: "low" at or below the level (default
  15%) while not charging, re-armed on charging or 5 points up; "full" when
  charging reaches 100%, re-armed at 97%. Which ones went out survives a
  restart. Sent to every device the reliable way; arrives as
  `BatteryAlert` (desktop and phone show a notification).
  `set_battery_alerts({enabled, low})`; desktop commands `battery_alerts`,
  `set_battery_alerts`; `mdctl battery-alerts on|off [low]`.
- **Keep awake**: `Engine::keep_awake(node, minutes)` (0 stops) sends the
  `keep_awake` action. The desktop runs `caffeinate -d -i -t` (macOS) or
  `systemd-inhibit --what=idle:sleep ... sleep` (Linux), one at a time, and
  lists `keep_awake` (and `awake_until`) in its status only where the tool
  exists. The phone keeps its screen on with a timed wake lock. Desktop
  command `keep_awake`; `mdctl awake <node> <minutes>`.
- Tests: `tests/gallery.rs` (3), `tests/backup.rs` (4: only on Wi-Fi and
  never twice across a restart, charging only and off, cut off Wi-Fi
  mid-video resumes past where it stopped, a file the computer has isn't
  sent), `tests/battery_alerts.rs` (3, with keep awake), unit tests for
  the crossings, months, ids and paths, desktop `keep_awake_runs_the_os_tool_for_the_time_asked`.
  Android compiled only.

## Bluetooth LE: messages with no network at all (2026-11-03)

- The core takes links the platform carries (`engine/ble.rs`:
  `Engine::ble_opened`, `ble_frame`, `ble_closed`): same `Hello`/`Msg`
  frames, so messages stay end-to-end encrypted and every feature works
  over it. Like the relay it is a fallback (`TransportKind::Ble`, never
  direct); any direct link replaces it. Tests: `crates/mdcore/tests/ble.rs`.
- Phone (`BleLink.kt`): advertises the my-domain GATT service and carries
  frames for computers that connect (writes in, notifications out, frames
  split into packets with a one-byte "last" flag; long writes handled).
- Computer (`crates/md-ble`, btleplug): scans for the service, connects to
  phones it has no direct link to, drops the connection once it has one.
  `mdctl --ble` turns it on.
- Verified 2026-11-03 with the phone's Wi-Fi and mobile data off: mdctl on
  the Mac found it over Bluetooth, sent a message, and got the delivery
  receipt back the same way.

## Now playing, richer status, slide remote, play on device (2026-11-03)

- Verified live 2026-11-03: the phone's status, asked from mdctl on the
  Mac, carried battery temperature and health, Wi-Fi (connected, signal
  4), Bluetooth, memory and storage. The lab scenarios pass on the new
  build.
- Also verified live the same day, from mdctl on the Mac:
  - Play on device: a YouTube link opened in the phone's YouTube app. A
    Spotify search (no Spotify installed) opened Spotify's website in
    Chrome; Spotify's site drops the search for visitors not logged in.
    With the app off screen, both came as notifications to tap
    ("lab box: search for ..."), as Android requires.
  - Now playing: the phone's status showed the YouTube video playing.
    Pausing and playing with the media key pushed "paused" and
    "playing" to the Mac at once.
  - Slide remote on the Linux laptop (xdotool): a terminal there received
    Page Down, Page Up and "b".
  - Not tried: the slide remote on macOS (needs Accessibility).
- Fixed while testing: the Android app thought it was off screen while it
  was in use. A second MainActivity (a share from another app, or `am
  start`) started, then the first one stopped and set "not visible". It
  now counts started activities. Off screen, links only come as
  notifications and the engine closes idle links, so this mattered.

- **Now playing** (`features/media.rs`): `Engine::set_now_playing(json)`
  from the host; status replies carry `now_playing`, and devices that
  asked for the status in the last 10 minutes get a `now_playing` push on
  each change (live links only: a song change never wakes a device).
  Event `NowPlaying` / `now-playing`; `now_playing_watched()` lets a host
  read the OS only while someone looks.
  - Android: media sessions through the notification listener's access
    (`NowPlaying.kt`, `Core.nowPlaying`).
  - Linux: `playerctl metadata`, polled every 3 s while watched.
  - macOS: AppleScript to Spotify or Music, only when already running.
    Nothing else works without private frameworks, so other players and
    browsers aren't seen. Info.plist gets NSAppleEventsUsageDescription
    (macOS asks once per app).
- **Richer status**: `wifi` {on, connected, ssid?, signal 0-4},
  `bluetooth`, `ram` {free, total}, and on Android `battery_temp` (C) and
  `battery_health`. Desktop reads them with nmcli / bluetoothctl /
  /proc/meminfo (Linux) and networksetup / ipconfig / system_profiler /
  vm_stat (macOS; no signal, network name usually hidden by macOS).
  Android shows the network name only if it reveals it without location
  access (usually not).
- **Presentation remote**: desktop action `slides {key: next | previous |
  black}` (Page Down / Page Up / B to the front window). Linux: xdotool,
  or wtype on Wayland. macOS: CoreGraphics key events (direct FFI), only
  with Accessibility allowed. Listed in `actions` only when it can work;
  otherwise `slides_needs` says what's missing. Android: `Core.slide`.
- **Play on device**: `Engine::open_url_in(node, url, app_package)` opens a
  link in that app on the phone (the browser if it's missing; a computer
  always uses the browser). `search_in_app(node, youtube | spotify | web,
  query)` is the `search` action: the phone opens the app's own search
  (browser fallback) and lists what it has in `search_apps`; a computer
  opens the search page in the browser. Desktop commands
  `open_url_in_app`, `search_on`; `mdctl`: `open <node> <url> [app]`,
  `search <node> <app> <text>`. Android: `Core.openUrlIn`, `Core.search`.
- **features.md** statuses refreshed against the code.
- Tests: `tests/media.rs` (3), media and package-name unit tests,
  `device_actions` (richer status, slide keys, link in an app, search),
  desktop parser tests (now playing, nmcli, bluetoothctl,
  system_profiler, meminfo, vm_stat, Wi-Fi device, search pages). The
  Kotlin compiles; nothing new tried on the phone or a Mac. The macOS
  code (AppleScript now playing, CGEvent keys, sysinfo) isn't compiled
  yet.

## Browse and download; send text; storage (2026-11-03)

- Verified live 2026-11-03 against the phone from mdctl on the Mac: the
  shared folders and Download/my-domain listed, a 1 MiB file downloaded
  identical to the phone's copy, and text sent to the clipboard showed as
  a notification on the phone.

- **Browse another device's folders and pull a file** (`features/browse.rs`).
  Each device chooses what it shares: `Engine::set_shared_folders` (name +
  path). `list_folder(node, root, path, offset)` returns a request id; the
  answer is `FolderListing` (pages of 200, folders first; `root` "" lists
  the shared folders). `download_file(node, root, path)` returns a transfer
  id at once; the device then pushes the file under that id with the normal
  resumable transfer (`files::start_with_id`), so the asker sees the usual
  `FileProgress` / `FileReceived` / `FileFailed`. Refused on the sharing
  device: anything outside a shared folder (`..`, links that lead out),
  hidden names (including `.mydomain-partial`).
  - Desktop shares its Downloads folder as "Downloads". Commands
    `list_folder`, `download_file`, `shared_folders`; event
    `folder-listing`. No UI yet (bindings only).
  - Android shares "my-domain" (public Downloads/my-domain on Android 11+,
    the app folder on 9 and older, nothing on 10) and, once photo access is
    allowed, "Pictures" and "Camera". The app doesn't ask for photo access
    yet (Settings > Apps > my-domain > Photos). `Core.listFolder`,
    `Core.downloadFile`, `Core.listings`; no screen yet.
  - `mdctl`: `share <name> <dir>`, `ls <node> [root] [path] [offset]`,
    `get <node> <root> <path>`.
- **Send text to a device** (`features/share.rs`): `Engine::send_text(node,
  text, "clipboard" | "notify")` → `TextShared` there. A desktop puts
  clipboard text on its clipboard and in the history (core side) and shows
  a notice; the phone writes its clipboard in the app and posts a
  notification (tap opens it if it's a link). Desktop command `send_text`;
  `mdctl`: `text <node> <mode> <text...>`.
- **Android share sheet takes text and links too**: a lone web link opens
  on the chosen device (`open_url`), other text goes onto its clipboard.
  The existing "Send to" dialog (`ShareChooser`) got the text case.
- **Storage in the status**: desktop (`df`) and Android (`StatFs`) report
  `"storage": {"free", "total"}` bytes.
- Tests: `tests/browse.rs` (11: listing, pages, escapes and links refused,
  nothing shared, download, a download interrupted by a restart resumes,
  failures with reasons, folder names, text to clipboard / as notice / to a
  device without a clipboard), browse unit tests, the mdctl scenario
  `a_script_browses_a_shared_folder_and_downloads_from_it`, desktop
  `reads_df`. Android compiled only; not tried on the phone.

## Device keys, first step (2026-11-03)

- Each install makes an Ed25519 key once (`device_key.rs`), kept in the OS
  secure store, or the kv store where there is none (mdctl). Signing out
  keeps it.
- QUIC certificates are made from it, and both ends present one (the
  acceptor asks the dialer for its certificate). The handshake proves
  which key each side holds.
- Trust on first use: the key is remembered under the device's node id
  the first time. A connection claiming that node id with another key,
  or with none, is refused. Devices from before still connect with a
  throwaway certificate, unremembered.
- A locked keyring at startup doesn't replace the key: that run uses a
  temporary one, and the next start finds the real key again.
- Found on the way: a device that restarted on the same QUIC port was
  refused by peers still holding its dead connection ("duplicate", same
  direction, same address) until that timed out, up to 60 s. Each start
  now has a boot id in the QUIC `Hello`; a new one replaces the old
  connection at once.
- Tests: `a_device_id_with_another_key_is_refused` (impostor refused, the
  real device back at once after a restart) and the unit tests in
  `device_key.rs`.
- Mistake, fixed the same morning (649609d): the first build kept the key
  under `device_key`, the secret the session already used for the
  passphrase key, and overwrote it. The phone, restarted on that build,
  lost its passphrase key and has to sign in again. The key now lives
  under `identity_key`; a key found in the wrong place is moved there and
  the place cleared. Tests: `the_session_key_is_left_alone`,
  `a_misplaced_key_is_moved_and_its_place_cleared`.

## Simulated routers: hole punching tested (2026-11-03)

- `scripts/lab/nat-sim.sh` (Linux, no root): two mdctl devices, each behind
  its own simulated home router (NAT plus a firewall that drops unasked-for
  WAN traffic), with the test servers (`md-fake-server`: registry, relay,
  two STUN addresses) on a simulated internet. Everything runs in an
  unprivileged user + network namespace and disappears afterwards.
  Each case sends a message and waits for the answer, then checks the path:

  | Router A | Router B | Path |
  |---|---|---|
  | easy | easy | hole punch |
  | easy | symmetric | relay |
  | symmetric | symmetric | relay (punching skipped up front) |

  Easy + symmetric can't punch through Linux routers: they only let in
  replies from the exact address and port they sent to, and a symmetric
  router answers from a new port.
- Found and fixed: a device that signed in after us could message us, but
  our answer failed with "peer not found" until our next refresh (`send`
  only goes to listed peers). A link from a device we don't list now
  fetches the list again (at most every 10 s per device). Test:
  `a_device_that_signed_in_later_can_be_answered`.
- mdctl gained `--stun`, `--relay`, `--signal-channel` and `--no-lan`; the
  testkit's fake servers can listen on any address.
- Lesson for the simulation: a router that accepts unasked-for packets
  breaks punching. The kernel keeps a connection entry for each one, and
  that entry takes the port the router's own outgoing mapping needs, so
  each side gets a new port. Real routers drop such packets, and a dropped
  packet leaves no entry.

## Connectivity improvements (started 2026-11-02)

- **HTTPS, live:** `https://152-67-7-50.sslip.io`, with Caddy and a Let's
  Encrypt certificate on `oracle-dev`.
  - The relay runs over `wss://` (rustls).
  - The server sees real client IPs; `X-Forwarded-For` is trusted only
    from the local proxy.
  - Both apps default to this URL.
  - An opt-in live test ([live_server.rs](../crates/mdcore/tests/live_server.rs))
    logs two engines in to the real server and chats through the relay.
  - `deploy.sh BIN_SOURCE=docker` cross-compiles in seconds and keeps
    `.prev` for rollback.
- **NAT type detection** ([nat/mod.rs](../crates/mdcore/src/nat/mod.rs)):
  - Two STUN servers (Google + Cloudflare) are asked for our public address
    from one socket:
    - same answer: **easy** NAT
    - different answers: **symmetric**
  - The type is measured at start and after network changes, and published
    through the registry.
  - When both devices are symmetric, the ladder skips the hole punch and
    WebRTC (`nat_skip`) and goes straight on. Before, that took ~40 s of
    guaranteed timeouts.
- **Network changes handled**
  ([engine/netchange.rs](../crates/mdcore/src/engine/netchange.rs)):
  - If our address moved, links on the old network are closed at once
    instead of after a ~20 s timeout.
  - We re-announce on the LAN, re-register, refresh the peer list and
    re-measure the NAT.
  - Every peer that was connected before is re-dialed through the ladder.
  - Desktop notices IP changes by itself; Android reports them.
  - A QUIC link from a peer that moved (same device, new address) replaces
    its stale link instead of being rejected as a duplicate.

- **IPv6:**
  - Each device publishes its global IPv6 address.
  - The QUIC endpoint is dual-stack.
  - A peer's addresses are **dialed in parallel**: IPv6, then the router
    mapping, then the LAN IPv4. The first handshake wins, so a dead address
    never costs a timeout.
- **UPnP port mapping**
  ([nat/portmap.rs](../crates/mdcore/src/nat/portmap.rs)):
  - The engine asks the home router to forward its QUIC port, renews the
    lease every 20 minutes, re-maps after a network change and unmaps on
    exit.
  - It refuses mappings behind carrier-grade NAT, which would be unreachable.
  - **Verified on the real home router:** the Oracle server got a QUIC
    reply from the laptop through `1.22.92.77:40815`.
- **QUIC connection migration:** links we dialed survive a network change;
  QUIC moves them to the new address. Only links we accepted are re-dialed.
- **mDNS / DNS-SD discovery** ([mdns.rs](../crates/mdcore/src/mdns.rs)):
  - Devices advertise `_mydomain._udp.local.` and find each other
    continuously, with no Scan button.
  - Found peers go through the same anti-spoofing rules as beacons.
- **Fixed a serious Android build bug:** every APK since 2026-11-01 had
  shipped a **stale, pre-QUIC Rust core**, because Gradle's JNI merge kept
  reporting UP-TO-DATE. The merge now always re-runs, and the phone runs the
  current core.

- **Phone-friendly keep-alive:** pings every 20 s with a 60 s idle timeout
  (was 5 s / 20 s), configurable. The app-level heartbeat now only covers
  UDP and WebRTC links; QUIC has its own.
- **Connection quality:** RTT and loss from QUIC, rated "Excellent",
  "Good" or "Weak connection" (`link_stats`).
- **Relay fair use:** a per-account token bucket (default 4 MiB/s, 4x
  burst) that slows heavy senders and never drops their frames; aggregate
  counters at `/relay/stats`.
- **Signaled QUIC (rung 3):** devices swap their QUIC addresses through a
  signal, and the responder dials back when it can't be reached.
- **Glare-safe links:** when both sides dial at once, they agree on one
  connection (the one dialed by the lower node id), with no "disconnected"
  flap.
- **Relay first, upgrade later:** a hard-to-reach peer connects through the
  relay within seconds, then hole punching or WebRTC replace it with a
  direct link. Retries continue every 60 s.

- **Less dependence on the server.** The principle: the server introduces
  devices; it isn't needed to keep them connected.
  - **Peer cache:** every known peer and all its addresses (LAN, IPv6,
    router mapping, QUIC port) are saved on the device and loaded at start.
    After a restart a device reconnects directly, even when the server is
    down.
  - **Addresses stay fresh peer-to-peer:** a device sends its current
    addresses (LAN, IPv6, router mapping) over its live links when a link
    comes up, after a network change, and when its mapping changes. Peers
    update their cache with no server involved.
  - **Signals over our own links:** a signal for a peer we're linked to
    goes straight to it, with no server. Otherwise it passes through linked
    devices, up to 3 of them, and the server is tried too. Each device
    forwards a given signal once, so loops can't echo. The receiver keeps
    the first copy and drops the rest.
    - Example: a phone and a desktop that were never linked set up a direct
      QUIC connection with only the laptop passing their signals, no server.
  - **Same QUIC port after a restart:** the port is saved on the device and
    reused, so peers' cached addresses and the router mapping stay valid.
  - **On the LAN:** mDNS plus direct QUIC needs no server at all.
  - **Signals:** the server uses a device's WebSocket for a signal only if
    one happens to be open (e.g. during a relay session), and FCM otherwise.
    The always-on WebSocket (`signal_channel`) exists but is **off by
    default**.

- **FCM on every device (2026-11-02).** FCM is the signaling path: it's
  free, so the server only forwards pushes and holds no connections.
  - **Android:** a `FirebaseMessagingService` feeds signals to the core, and
    starts the engine first if the app was asleep. Firebase is initialised
    in code from the gitignored `apps/android/google-services-android.json`.
  - **Desktop:** `fcm_receiver_rs` now registers as the project's Firebase
    **Android** app. Tokens from its web (Chrome) mode are rejected by FCM
    as `UNREGISTERED` the moment they're issued. Registration is retried on
    the same checked-in device, because Google refuses the first try
    (`PHONE_REGISTRATION_ERROR`).
  - Verified on the real devices: a server push reaches the phone, the Mac
    and the laptop; a `quic_offer` → `quic_answer` round trip runs over FCM;
    the phone on IPv6-only mobile data (Jio) sent a message the Mac received
    over **direct QUIC**, through the Mac's router mapping.

## New app screens (2026-11-02)

Both apps were rebuilt from scratch for people who aren't technical: plain
words, no protocol names, technical tools behind a hidden Developer screen
(tap the version number 7 times). Both use the same deep-space design tokens (a Milky Way background cropped from `docs/brand/space-source.png`, frosted-glass panels)
and the galaxy logo (`docs/brand/logo-1024.png`, cropped as-is from
`logo-source.png`).
- **Desktop** (`apps/desktop/ui`): SolidJS + TypeScript on typed commands
  generated by `tauri-specta`; CI fails if the bindings are stale.
- **Android** (`apps/android/app/src/main/java/com/mydomain/android/ui`):
  Compose screens calling the UniFFI `MdEngine` directly. `core/Core.kt`
  holds the observable state, fed by pushed core events; the `RustNet`
  string facade is gone. Peers now report their link transport
  (`PeerInfo.link`) and the bridge exposes the session (`session()`).
  - Checked on the phone: sign-in, lab auto-login, the device list and the
    device screen render in dark mode. A live message test from the Mac
    is pending: the office network's Fortinet firewall intercepts TLS, so
    the Mac can't reach the server from there.

## Stay signed in, and company networks (2026-11-02)

- **The session survives a restart.** The login token (7 days) and the
  derived device key go to the OS secure store through a new
  `Platform::secret_get/secret_set` hook: the macOS Keychain, Secret
  Service or Windows Credential Manager on desktop (`keyring`), and an
  AES-GCM store under an Android Keystore key on the phone
  (`KeystoreSecrets.kt`, via a UniFFI `SecretStore` callback). At start the
  engine restores them if the token has at least an hour left, then
  re-registers in the background, so a push that wakes a killed app finds
  it signed in. Signing out deletes them. Test:
  `registry::a_restarted_device_is_still_signed_in_until_it_signs_out`.
  - Checked on the phone: after its process was killed, it reopened on
    the device list signed in; a server push (self-ping) to the killed app
    was handled, not dropped; Sign out emptied the Keystore store.
- **HTTPS trusts the OS certificate store too** (`mdcore::tls`), for both
  the registry calls and the `wss://` relay. Company proxies that re-sign
  HTTPS with their own CA (installed by IT) no longer break sign-in with
  "UnknownIssuer". Messages stay end-to-end encrypted.

## Idle links close in the background (2026-11-02)

- `Engine::set_background(true)` (Android: the app's UI is off screen, or a
  push started it) closes links that carried no messages for
  `CoreConfig::idle_close_after` (3 min), so their keep-alive pings stop
  waking the radio every 20 s. A send re-opens a link on demand, and a push
  reaches a phone with none. Each link records when it last carried a
  message (`P2pConn::last_used`); keep-alives don't count. Test:
  `tests/idle_links.rs`.

## Desktop idle CPU: stray WebRTC sessions end (2026-11-03)

- The Linux desktop used 2.8% of a core while idle (debug build). One
  thread woke about 150 times a second: a WebRTC session answered from a
  pushed offer whose data channel never opened, because the peer had
  linked over QUIC instead. Nothing ended it, so its ICE checks ran for
  good.
- A session that hasn't opened its channel now stops after 30 s
  (`OPEN_WITHIN`), or as soon as the peer is linked directly another way.
- Measured on the laptop: idle CPU went from 3.3 s to 0.55 s per 2 minutes.
- The background loops polled for shutdown every 100 ms, so six threads
  each woke 10 times a second for nothing. They now sleep on a condition
  variable that `Engine::drop` signals (`engine::Shutdown`), so they still
  stop at once. The idle desktop went from over 3,600 to about 1,100
  wakeups a minute, and to 18 CPU ticks a minute.

## Review fixes: fewer pushes to unreachable devices (2026-11-03)

An independent review of the night's networking changes found four
problems; all are fixed.

- **The outbox's dead-link check fired on links that weren't live.** The
  relay rung registers its route before the peer answers, so a message
  to a phone that's off went unacked over it. After 10 s the outbox
  reconnected, which pushed the phone again, every 10–15 s for 10 minutes.
  The outbox now resends, and judges links, only when the link is live
  (`transport::is_live`).
- **Ladders overlapped.** Each one signals (pushes) the peer. Now only one
  runs per peer at a time. A request that arrives meanwhile makes the
  running one go round once more if the peer still isn't live, which
  covers a first ladder that started before the peer's address was known.
- **File re-offers skipped their backoff.** The dead-link special case
  reset it. It's gone: re-offers go through the reliable path, linked or
  not, so the outbox handles dead links. "Linked" now means a live link.
  This also removed the false alarm for a receiver that hashes a large
  partial file before answering: its ack goes out before the handler runs.
- **The relay socket.** A route that never went live no longer keeps it
  open, and closing it removes its routes. `ensure_relay` marks an existing
  socket as just used, so the idle closer can't close it under a relay
  offer. The connect guard is released even if the connect panics.

## Battery: an hour with the screen off (2026-11-03)

- Measured on the phone, simulated unplugged (`dumpsys battery unplug`),
  screen off, deep Doze, one hour, on the build from before the relay
  changes. The app held wake locks for 2 min 9 s and used 30 s of CPU.
  Nearly all of it came from 24 deferred pushes, each holding the 30 s
  post-push wake lock. All of them came from the Linux desktop, in bursts
  of three signals.
- Cause: the desktop's relay link to the phone stayed "live" (the phone
  never closed its relay socket), so the ladder's background upgrade
  retried every 60 s, up to 10 times per connection. Each try signals,
  and that wakes a sleeping phone by push.
- Fixed: an upgrade try is skipped while the relayed link carried no
  message in the last minute (`transport::used_within`). The phone now
  also closes its idle relay socket (below), which ends the "live" link.
- Measured again with the fixes (same setup, one hour, deep Doze):

  | | Before | After |
  |---|---|---|
  | Pushes received | 24 | 2 |
  | Wake lock time | 2 min 9 s | 11 s |
  | CPU time | ~30 s | ~3 s |

  The remaining pushes were three bursts from the Linux desktop (one
  connection attempt each, the first at its startup), held by Doze until
  its maintenance windows.
- An overnight run (01:21 to 07:01) gave no result: the phone left USB
  before the end, so there was no second snapshot.
- Linux desktop memory overnight, same process, 01:48 to 07:01: 211,116 kB
  → 211,312 kB resident, 55 threads both times. No leak.

## Relay socket closes when idle in the background (2026-11-02)

- Once a device used the relay, its socket stayed open for good, pinging
  every 20 s: an always-on WebSocket on the phone, and the stale socket
  the server then sent signals into. In the background the idle closer
  now also closes the relay socket once no peer is routed through it
  (`relay::close_if_unused`), unless `signal_channel` keeps it open on
  purpose. A send or a push opens it again. Test:
  `in_the_background_an_unused_relay_socket_is_closed`.
  - Only a socket unused for the whole idle time is closed, so one just
    opened for a push's relay offer isn't shut before its peer is routed.
    The closing socket leaves the engine at once, so a link that needs the
    relay meanwhile opens a fresh one.
  - Fixed while testing: two relay offers arriving together opened two
    sockets. The loser's close removed the winner's registration, so the
    device looked connected to a relay that no longer knew it. Relay
    connects are now one at a time per device. The fake relay also removed
    a newer registration on close; it now checks, like the real server.

## Server: no signals into a sleeping device's relay socket (2026-11-02)

- The server delivered signals over a device's relay WebSocket whenever one
  was registered, instead of pushing. A phone that joined the relay and
  then went to sleep (screen off) silently lost those signals, so the
  "missing FCM pushes" in the screen-off test were never sent at all.
- Each relay connection now records when the server last heard from it.
  Clients ping every 20 s, and signals use the socket only if it was heard
  from within 30 s (`SIGNAL_FRESH`); otherwise they go by FCM. Test:
  `a_silent_socket_gets_no_signals_until_it_speaks_again`. Live once the
  server is redeployed.

## Lab scenario runner; first-contact and stale-link fixes (2026-11-02)

- `scripts/lab/scenarios.sh`: runs the `mdctl` scenarios (receipts,
  controls, file transfer) from the Mac against each lab device and
  prints a pass/fail table. It exits non-zero on any failure.
- Fixed: a file offer delivered as the link came up was accepted with no
  link back, and the accept was lost, so the transfer stalled for 11 min.
  Offer replies that can't go out now are queued reliably.
- Fixed: a message arriving by relay from a peer we hold a QUIC link to
  now probes that link. It pings and watches QUIC's received-datagram
  count for 3 s; if nothing comes back, the link is closed and replies go
  by relay. (A phone that moved off Wi-Fi kept answering into its dead
  link.)
- Verified: all scenarios pass against the Linux desktop and the phone,
  including over mobile data. A 1 MB file sent while the phone dropped
  Wi-Fi mid-transfer arrived with a matching SHA-256.

## Desktop media keys and lock (2026-11-02)

- The desktop lists `media` and `lock` in its status when it can do them.
  The phone's existing media buttons work against a Linux computer.
  - `media {key}` (Linux): through `playerctl`, aimed at the player that
    is playing, else a paused one, rather than an idle browser tab.
    macOS isn't offered yet: posting media keys needs the Accessibility
    permission.
  - `lock`: macOS sleeps the display, which locks it when a password is
    required after sleep. Linux locks the user's graphical sessions
    through logind. Where nothing answers that (a bare i3 or sway), it
    runs `i3lock`, `swaylock` or `slock`.
- Verified live on the Linux laptop (i3) from Mac `mdctl`: play/pause
  toggled VLC, and lock brought up i3lock. Mac lock not tried, because
  that Mac is in use. Neither app has a Lock button yet; it's for the new
  UI.

## Linux desktop verified; dead links during a transfer (2026-11-02)

- The workspace tests pass on Linux (rustc 1.99.1; specta needs a recent
  stable). Run three times in a row with no failures.
- Verified live, Mac `mdctl` to the Linux desktop app over direct QUIC:
  status (battery, charging, volume, muted); volume 75 → 85 → 75 and mute
  on/off through `pactl`; a 1 MB file arrived with a matching SHA-256.
- Paused transfers: when a re-offer over a link that looks alive gets no
  answer by the time the next one is due, the link is closed and the
  next re-offer reconnects. (A killed app or a phone that left without
  saying so keeps its QUIC link looking alive until the 60 s idle
  timeout.) This fixed a flaky resume test.
- The relay upgrade test waits for both sides to upgrade and counts only
  messages. A late hello may still be greeted back by relay.

## Desktop tray; phone controls verified live (2026-11-02)

- Desktop tray / menu-bar icon: Open my-domain; a submenu per device
  (marked when connected) with Ring, Send a file…, Open; Quit. Rebuilt
  (debounced) when devices or links change. Closing the window hides it to
  the tray; the dock icon brings it back.
- Verified live from `mdctl` over a direct QUIC link, each step
  acknowledged: ring; flashlight on (status `torch: true`) and off;
  ringer to vibrate and back to normal (status follows); dial (opens the
  dialer with the number); launch (open Settings).

## Dial from the computer (2026-11-02)

- Android `dial {number}` action (phones only): opens the dialer with the
  number filled in, or a tap-to-open notification in the background. It
  never places the call itself and needs no permission. Desktop: a "Call"
  field in Find & control when the phone lists `dial`.

## Notification actions (2026-11-02)

- A notification forwarded from a subscribed phone app now carries its key
  and its buttons (`share_app_notification_with`); another device can press
  a button, reply (buttons that take text) or dismiss it
  (`notification_action`, index -1 = dismiss; reliable, so it waits for
  the phone). Android fills the reply into the button's RemoteInput and
  fires it; Activity says what was done, never the text.
- Desktop: a "Notifications" tile per device (last 30, memory only) with
  the buttons, a reply box and Dismiss; handled items dim.
- `mdctl`: `subscribe <node> <json pkgs>`, `notifact <node> <key> <index>
  [text]`.
- Verified live: `mdctl` subscribed to the phone's shell notifications, got
  a posted notification with its key, and dismissed it remotely (gone from
  the phone). Buttons and replies are covered by the core and bridge tests.
- The desktop's bindings test now fails if export fails (an `i64` command
  argument made specta skip the file silently).

## Verification codes to the computer (2026-11-02)

- Android (off by default: Settings "Send verification codes to my
  computers"): when a notification carries a one-time code (`findCode`,
  11 JVM unit tests: Indian bank/SMS formats, Google "G-", Hindi, and
  negatives like amounts, cards, phone numbers, promo codes), the phone
  shares it. Never logged.
- Core: `share_code` sends it the reliable way to all your devices (it
  connects if needed), stamped with the time; a device with a clipboard
  copies it unless it's older than 2 minutes (`CodeCopied`). The desktop
  shows "Code copied" and an Activity entry.
- Verified live: a test OTP notification on the phone was on the Mac's
  clipboard 4 s later (the phone had no link and connected on demand).
- Also verified live after the last install: receipts, status (with the
  phone's actions), media volume up/down, and a 1.2 MB file to the phone
  (hash matches; the link upgraded from relay to punched mid-transfer).

## Desktop notifications; open a phone app (2026-11-02)

- Desktop system notifications go through `tauri-plugin-notification`
  (held at 2.4 so tauri stays on 2.11; see Cargo.toml) instead of
  `osascript`/`notify-send`: ring, and, while the window isn't focused, a
  chat message, a saved file, and a notification shared from the phone.
  Settings has a "Show notifications" switch (on by default, kept in
  `prefs.json`).
- Android takes a `launch {pkg}` action (opens the app, or a tap-to-open
  notification when in the background); the desktop's Apps tile has an
  Open button per app when the phone lists `launch`.

## File transfer hardening; relay route back (2026-11-02)

From a code review of `features/files.rs`:
- A sender whose last `file_done` got lost no longer sits at 100 %
  forever: it pauses, re-offers, and the receiver (which remembers
  finished and cancelled transfers) answers again.
- Stalls are measured per transfer (when its acks last moved), so a busy
  second transfer can't hide a stalled one. A gap report from the receiver
  makes the sender resend at once instead of after the 5 s stall. Late acks
  from a replaced link can't move a transfer backwards.
- A transfer paused for an hour is given up ("the device wasn't
  reachable"); a source file that can't be read or shrank fails at once on
  both sides; a partial incoming file idle for 2 hours is deleted.
- The received file's name is claimed atomically (`create_new`), so two
  files finishing at once can't overwrite each other or an existing file.
  A write error (storage full) fails the transfer with that reason.
- Resuming hashes the partial file outside the lock; names can't end up
  as `.`/`..` or end in dots or spaces.
- Tests: two files at once; a file that shrinks while sending.

Also: control handlers take the sender's id from the link, not from the
message's own `"from"` field (`ControlMsg::sender`); finished and
cancelled transfers are remembered in the platform store, so a receiver
that restarted still answers a repeated offer instead of taking the file
twice.

Relay: a message from a device we had no relay route to (its hello, or
the push that set it up, was lost) now sets the route up, so our receipts
and replies reach it. Found live: the phone received `mdctl`'s actions but
its answers had nowhere to go.

## Clipboard history (2026-11-02)

- The core keeps the last 20 clipboard values (synced from a device,
  pulled, or copied here while sync is on), newest first, each with the
  device it came from; a repeat moves to the top. In memory only, so codes
  and passwords never reach the disk. `Engine::clipboard_history`,
  `clear_clipboard_history`, `copy_to_clipboard`; event
  `clipboard-history`.
- Desktop: "Recent" list in the Clipboard tile with Copy and Clear.
- Android (app layer, since its core has no clipboard): the same list,
  memory only, cleared on sign-out. Android 10+ only reports local copies
  while the app is on screen. Not yet tried on the phone.
- Test: `the_clipboard_history_keeps_recent_values_newest_first`.

## Device controls (2026-11-02)

- Core: `run_action(node, name, args_json)` → `CoreEvent::Action` on the
  target (`features/device.rs`; names are short lowercase words). Each app
  lists the actions it takes in its status (`"actions": [...]`) plus their
  current values, so the other side shows only buttons that work.
- Android takes `flashlight {on}`, `volume {step}`, `media {key}`
  (play_pause / next / previous) and `ringer {mode}` (silent falls back to
  vibrate without Do Not Disturb access); reports `torch`, `volume`,
  `ringer`. Desktop takes `volume {step}` (10 %) and `mute {on}` via
  `osascript` (macOS) or `pactl` (Linux); reports `volume`, `muted`.
- Both apps show the matching buttons in "Find & control", and ask for the
  status again after an action. `mdctl`: `action <node> <name> [json]`.
- Verified live: `mdctl` read the Mac's status and moved its volume
  86 → 96 → 86. The Android side builds and passes the bridge test; it
  hasn't been tried on the phone yet.
- `send` now answers "queued" when the message went to a route that isn't
  confirmed live yet (a relay route waiting for the peer), not "QUIC".

## Sending files (2026-11-02)

- `Engine::send_file(node, path)` / `cancel_file(id)` / `set_download_dir`
  (`features/files.rs`). 64 KiB chunks, at most 1 MiB unacknowledged; the
  receiver writes `<downloads>/.mydomain-partial/<id>.part` and acks its
  length (a gap makes the sender rewind). The offer goes the reliable way
  (queued, wakes a sleeping device). A dropped link pauses the transfer;
  it is offered again on the next link (with no link: retries back off
  from 15 s to 10 min) and resumes from the partial file, also after the
  receiver restarts. SHA-256 checked at the end; the file is saved under
  a free name ("notes (1).txt"); names can't escape the folder.
- Events `FileProgress`, `FileReceived`, `FileSent`, `FileFailed`.
  `mdctl`: `sendfile <node> <path>`, `downloads <dir>`.
- Desktop: Files tile (system picker, drag and drop onto the window,
  progress, Cancel, Open / Show in folder); saved to ~/Downloads.
  Android: Files tile (picker), the share sheet ("Send to" a device),
  progress and "saved" notifications; saved to Downloads/my-domain via
  MediaStore.
- Verified live (2026-11-02): `mdctl` sent 2.5 MB to the Mac app and to
  the phone on mobile data (over the relay); both copies' SHA-256 match.
  The same run checked a chat receipt and the phone's status (battery 90%,
  charging).
- Tests (`tests/files.rs`): small and 3 MB files, a device without a
  downloads folder declines, a missing file fails at once, and a 40 MB
  transfer interrupted by a receiver restart resumes where it stopped.

## Ring, open a link, battery (2026-11-02)

- Core (`features/device.rs`): `request_status`, `ring`, `open_url` go the
  reliable way (queued; a sleeping phone is woken by push), `set_device_status`
  is pushed by the host. Only http(s) links are sent or opened.
- Android: ring plays the alarm tone at alarm volume for up to 30 s with a
  Stop notification; a link opens directly when the app is on screen,
  otherwise as a notification; battery pushed on change.
- Desktop: ring shows and focuses the window with a chime and a system
  notification; links open in the default browser; battery from `pmset`
  (macOS) or `/sys/class/power_supply` (Linux).
- Both: a "Find & control" tile (Ring, Open a link, battery). Status is
  asked only over a live link, so opening a device's screen never wakes it.

## Strings in resource files; desktop FCM stop (2026-11-02)

- Android UI text is in `res/values/strings.xml` (117 entries, plurals for
  counts and "time ago"); desktop UI text is in `ui/src/i18n/en.ts` with a
  typed `t(key, params)` / `tn` (plural) / `tRich` helper. Developer screens
  stay inline.
- Desktop FCM: our own MTalk listen loop (1 s read timeout) replaces the
  crate's, so a receiver replaced by re-registration stops within a second
  instead of lingering until its next reconnect. It also sends heartbeats
  on an idle connection and acks the server's. Verified live: connected,
  self-test round trips and signals from the phone.

## Faster fallback to the relay (2026-11-02)

- The connect ladder races its two QUIC rungs: direct QUIC first, signaled
  QUIC after a 500 ms head start (so a reachable peer still costs no
  signal). If neither gives a direct link within 3 s the relay comes up,
  instead of after up to 14 s; signaled QUIC carries on and can still
  upgrade the link. Hole punching and WebRTC start once both are done.

## Quieter LAN announce; pull to refresh (2026-11-02)

- A LAN announce sends multicast and broadcasts only; the /24 unicast
  sweep (254 packets per interface) is added from the second round on,
  and only if no LAN device is known yet (mDNS or a beacon found none).
- Android Home: pull down to refresh the device list (the Refresh button
  is gone; the empty state says "Pull down to refresh").

## Delivery receipts (2026-11-02)

- Every chat message carries an id (`Plaintext.id`). The receiver acks it
  (`Plaintext.ack`) every time and shows it only the first time (ids
  remembered per sender). The sender keeps the message in the outbox until
  the ack: it is resent on every new or upgraded link and every 10 s on a
  live one, so a message sent as a link is swapped (glare, relay to
  direct) is no longer lost.
- New event `MessageDelivered { node_id, id }` (`message-delivered`);
  `Engine::send_with_id` lets the UI pick the id. Both apps show a bubble
  as pending until its receipt arrives ("Sends when connected" only while
  there is no link); desktop marks the newest delivered message.
- Tests: `the_sender_hears_when_a_message_arrived`,
  `a_resent_message_is_shown_once_and_still_acknowledged`, the bridge test.

## Messages wait for a link; relay route fix (2026-11-02)

- **Outbox:** a message sent while a device has no link is held
  (`transport::outbox`; up to 100 per device, 10 minutes) and sent the
  moment a link comes up; `send` returns `"queued"` instead of a "not
  connected yet, try again" error. Both apps show such a message dimmed,
  with "Sends when connected", until the link is up. Checked live: Mac
  (`mdctl`) to the sleeping, locked phone: queued, woken by push, relay,
  then a punched link.
- **Fix: a relayed peer could stay "live" with nowhere to send.** When a
  direct link replaced the relay route and then died, the next relay
  connection skipped creating the peer's outgoing queue. Found live on the
  phone; regression test
  `relay::the_relay_route_comes_back_after_a_direct_link_dies`.

## LAN discovery lists only your own devices (2026-11-02)

- LAN announcements (mDNS TXT `tag`, UDP beacon `tag`) carry
  `crypto::lan_tag`, a one-way hash of the device key; devices list only
  announcements with their own tag. Before, any my-domain device on the
  network was listed: test runs on the Mac left 46 entries in the desktop
  app, and on shared Wi-Fi other people's devices would show. Devices
  re-announce when their key is set (sign-in), and mDNS re-publishes.
- Devices found on the LAN aren't restored from the peer cache (they
  re-announce within seconds), so stale ones don't stay listed.
- Tests: `mdns::only_devices_with_the_same_key_are_listed`,
  `mdns::devices_found_on_the_lan_are_not_restored_from_the_cache`.

## mdctl scenarios (2026-11-02)

- `mdctl --script FILE` runs a scenario (`#` comments, `${VAR}` from the
  environment, exit code 1 at the first failure). New commands: `wait
  <event> [field=value ...] [timeout=N]`, `echo on|off` (answer every
  message, for round-trip checks), `sleep`, and `@name` for any node id.
  Scenarios in `scripts/lab/scenarios/` (`chat-roundtrip`,
  `wake-by-push`). Test: `apps/mdctl/tests/scenario.rs` runs two real
  `mdctl` processes.

## Device lab and dev tools

- **`scripts/lab/start-all.sh`** starts the app on all three devices, logged
  in with a test account:
  - the macOS desktop on the Mac
  - the Linux desktop on the laptop
  - the Android app on the phone
- **The test account `mdtest`** is registered on the live registry. Its
  credentials are in the gitignored `scripts/lab/test-creds.env`. Only debug
  builds fill the login screen from it; a release build never reads
  credentials from outside.
- **Other lab commands:**
  - `linux.sh test`: run the workspace tests on Linux.
  - `android.sh coretest`: cross-compile the core tests and run them on the
    phone.
  - `android.sh start|install|logcat`: install, launch and watch the app on
    the phone.
  - `watch.sh`: follow the laptop's live log from the Mac.
- **Desktop FCM recovers by itself.** When Google rejects a saved push token,
  the app registers a new one and runs the self-test again. Before, push
  stayed broken until someone deleted `fcm_creds.json` by hand.
