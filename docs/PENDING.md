# What's pending

Status as of **2026-11-02**, in priority order. What's already built is in
[DONE.md](DONE.md). The full plan for every phase lives in the rebuild plan
and in [learnings/](../learnings/README.md).

## 1. Needs a decision or an action from you

| Item | Why |
|---|---|
| **Rotate the server `JWT_SECRET`** | The old value was in a committed file before it was redacted. Changing it logs every device out once. |
| **Change the laptop password** | It was shared in chat. |
| **Dirty NTFS partition** on the laptop (`/dev/nvme0n1p6`) | Was already dirty before this work; left untouched. Fix it from Windows (`chkdsk`) or with `ntfsfix`. |
| **Archive the six old GitHub repos** | They're merged into this monorepo with full history. |
| **Merge `monorepo` → `main`** (open a PR) | CI runs only on pushes to `main` and on PRs, so this branch hasn't been through CI yet. |

## 2. Connectivity improvements (do these first)

In priority order. Each item makes devices connect in more places, faster,
or with less battery.

1. ~~Serve the server on port 443 with HTTPS~~: **done**. Live at
   `https://mydomain.servehttp.com` and `https://mydomain.work.gd`
   (Caddy + Let's Encrypt; the old `152-67-7-50.sslip.io` name is still
   served); both apps default to servehttp.
   - Later: close `:6969` to the public, once no install uses it.
2. ~~Detect NAT type up front~~: **done**.
3. ~~Re-check the network when it changes~~: **done**.
4. ~~IPv6~~: **done**, on Android too (the phone publishes its global
   IPv6 on mobile data).
5. ~~Ask the home router to open a port~~: **done** for UPnP-IGD (verified
   on the home router: the internet reaches the laptop through the
   mapping).
   - NAT-PMP / PCP **done** too, tried after UPnP. (The home router doesn't
     answer PCP; UPnP covers it.)
6. ~~QUIC connection migration~~: **done**.
7. ~~Standard LAN discovery (mDNS / DNS-SD)~~: **done**.
   - ~~Send the /24 sweep only when mDNS finds nothing~~: **done**
     (2026-11-02): an announce sends multicast and broadcasts, and adds
     the 254-host sweep only if no LAN device is known after the first
     round.
8. ~~Keep-alive that suits phones~~: **done** (20 s / 60 s, configurable).
   - ~~Close idle links while the screen is off and wake by push~~:
     **done** (2026-11-02). Still to do: measure the battery saving on the
     phone over a day.
   - Verified on the phone (vivo I2405): in the background with the screen
     on, a link closes 3 min after its last message, as designed. With the
     **screen off**, a punched link dropped within seconds. Likely the
     phone's power saving cutting a background app's UDP; investigate with
     `files/events.log` (below) and the battery-optimisation exemption.
9. ~~Connection quality~~: **done** (`Engine::link_stats()`, desktop
   command `link_stats`).
   - ~~Show it in the new UI~~: **done** (2026-11-02): signal bars and
     "Excellent/Good/Weak connection" next to each device's status.
   - Still to do: use it to choose between two live paths.
10. **Better relay:** per-account fair-use cap and `/relay/stats` are
    **done**. Still to do:
    - Relays in more than one region.
    - A QUIC-based relay.
11. **Bluetooth LE.**
    - Find nearby devices and send small commands (ring, media keys, wake,
      "connect to my hotspot") when there is no shared Wi-Fi or no internet
      at all.
    - It can also be the out-of-band channel for pairing and for starting
      Wi-Fi Direct.
12. ~~Clean up old paths~~: **done** (2026-11-02). One-off TCP/UDP sends,
    the WebSocket rungs, the LAN UDP rung and the directed-transport setting
    are gone; every message rides a live link.

- ~~Even less server dependence~~: **done** (peer cache, `addr_update`,
  signals over our own links, forwarded up to 3 hops, QUIC port kept
  across restarts, FCM on every device). Next idea:
  - A LAN device acts as "home base" for phones away from home.

## 3. Phase 2 leftovers (networking)

- ~~Signaled QUIC as rung 3~~: **done**, and the WebSocket rungs are
  removed (item 12).
- ~~Relay right away, upgrade in the background~~: **done** (relay after
  the quick rungs; punch/WebRTC upgrade it; retries every 60 s).
  - ~~Race the rungs~~: **done** (2026-11-02) for the QUIC rungs: direct
    QUIC gets a 500 ms head start, then signaled QUIC joins; the relay
    comes up after 3 s without a direct link (was up to 14 s) while
    signaled QUIC keeps trying. Punch and WebRTC still run after them.
  - ~~A message sent as a link is replaced could be lost~~: **fixed**
    (2026-11-02) by delivery receipts: every chat message has an id, the
    receiver acks it and drops repeats, and the sender resends until acked.
- **Move the remaining loops to tokio.** Discovery, the heartbeat and the
  feature/background loops still run on their own threads (the TCP/UDP/WS
  listeners are gone). Moving them to
  tokio tasks with a `CancellationToken` makes shutdown clean and immediate.
  Probably best done with Phase 3, because the UniFFI bridge wants an async
  engine anyway.
- **Switch `ws_conns` to tokio channels.** This removes the thread that sits
  between the threaded code and the async code (the "bridge thread").
- ~~Relay over `wss://` (TLS)~~: **done**. The live test chats through the
  real HTTPS server's relay.
- ~~Test the engine against the real server's relay~~: **done**
  (`tests/live_server.rs`, ignored by default; it cleans up its devices).
- ~~Clean engine shutdown~~: **done** (2026-11-02). The background loops
  now wake within 100 ms of shutdown, so a dropped `Engine` releases its
  QUIC port in under half a second (`tests/shutdown.rs`). Short-lived
  threads (ladder, signals) still finish their current step.

## 4. Phase 3: Bridges and apps

- ~~Android FCM receiver~~: **done** (2026-11-02), plus the desktop moved
  to Android-mode FCM registration. See DONE.md.
- **UniFFI bridge** (`apps/android/rust`): **built** (2026-11-02). Typed
  `MdEngine`, events pushed to Kotlin, generated bindings; NetService no
  longer polls. Verified on the phone (2026-11-02, mobile data): FCM
  signals in, relay then hole-punched direct link, the message delivered to
  Kotlin through the listener. The new Compose screens call `MdEngine`
  directly and `RustNet.kt` is deleted (2026-11-02).
- **Desktop UI in SolidJS + TypeScript**, with typed commands (`tauri-specta`)
  and native OS notifications. Why:
  [ADR-0004](../learnings/decisions/ADR-0004-solidjs.md).
- **`mdctl`** (`apps/mdctl`): **built** (2026-11-02). A device with no UI:
  logs in (flags or `MD_DEV_*`), prints every event and result as JSON
  lines, takes commands on stdin (`peers`, `send`, `connect`, `add`,
  `links`, `forget`, ...). Scripted scenarios: **done** (`--script`,
  `wait`, `echo`; `scripts/lab/scenarios/`).
- **Simulated-NAT test.** Two engines, each behind its own simulated router
  (Linux network namespaces), must connect by hole punching. Each simulated
  device has to be its own process, which is why this needs `mdctl`.
- **Lab scenario tests** (`scripts/lab/scenarios/*.toml`), run across the
  Mac, the laptop and the phone.

## 5. UX for everyone (design, colour, polish)

Goal: someone who isn't technical can install it, pair their devices and use
every feature without help, and enjoy doing it. Today the apps are
developer-facing: raw server URLs, "encryption key" fields, transport names
like QUIC/RELAY, and a debug log.

Planned with the Phase 3 UI rewrite (SolidJS desktop, Compose Android) and
kept up in every later phase.

### Start fresh: rebuild the UI, don't restyle it

The current screens were built to **debug the networking**, so they expose
internals everywhere. Restyling them would keep the clutter, so both apps get
**new screens built from scratch** on the design system below:
- **Desktop:** **built** (2026-11-02, `apps/desktop/ui`): SolidJS + TypeScript on the generated bindings, a deep-space theme (star-field background, glass panels), the galaxy logo, plain-language statuses, a hidden Developer screen (tap the version 7 times). It replaced `apps/desktop/src/index.html`,
  `main.js` and `styles.css` (about 1,100 lines of vanilla JS).
- **Android:** **built** (2026-11-02, `apps/android/app/.../ui`): Compose
  screens on `MdEngine`, the same tokens, words and screen map as the
  desktop (sign in, Devices, Device, Activity, Settings, hidden Developer).
  State lives in `core/Core.kt`, fed by pushed core events. It replaced
  `MainScreen.kt`, `LoginScreen.kt`, `SettingsDialog.kt` and `RustNet.kt`.
  Still to do on both apps: QR pairing, and pull-to-refresh on Android.

The old UI is deleted, not kept alongside. The core API stays the same; only
the screens change.

| Today's element | New home |
|---|---|
| Per-message transport picker, UDP/TCP "directed transport" toggle | **Removed**: the engine picks the best path itself |
| Manual peer entry (IP + TCP/UDP/WS ports) | **Removed**: QR pairing replaces it (kept on the Developer screen) |
| "Scan LAN" and "Refresh" buttons | **Removed**: discovery and refresh run automatically (pull-to-refresh on Android) |
| Firewall check button | **Removed**: an automatic "This Mac can't be reached. Fix it" hint with steps |
| FCM self-test toast | **Developer screen** |
| Floating debug panel (log, filters, copy, clear) | **Developer screen** |
| Protocol tags in the activity log (`[UDP]`, `[QUIC]`) | **Removed**: plain sentences ("You sent 'hi' to Pixel") |
| IP addresses and node ids on device cards and in settings | **Developer screen** |
| Raw server URL on the login screen | Pre-filled; editable only under "Advanced" |
| "Encryption key" field and "Generate" button | Replaced by device keys and QR pairing (Phase 4) |
| "Skip login (LAN only)" | **Developer screen** |
| Device list, message, clipboard, notifications, app list, account settings | **Kept, redesigned** as device cards and feature tiles |

**Developer screen:** hidden, shown in debug builds or unlocked by tapping the
version number 7 times. It collects everything a developer needs:
- the debug log, with filters and export
- the connection path for each device
- the FCM self-test
- the firewall check
- manual peers
- the transport override

**Screen map for the new app:**
1. Welcome / sign in
2. Pair a device (QR)
3. Home: device cards
4. Device detail: one tile per feature
5. Activity, in plain words
6. Settings: account, devices, permissions
7. Developer (hidden)

**Done when:**
- A non-technical tester can sign in, pair a device and send a file without
  seeing a single technical term.
- The old desktop and Android UI files are deleted.
- `scripts/lab/start-all.sh` still logs every device in with the test
  account.

- **Design system first**, shared by desktop and Android so they feel like one product:
  - Colour palette: a calm brand colour, plus success, warning and error colours.
  - Light and dark themes. Every colour pair meets WCAG AA contrast.
  - Type scale, spacing, rounded corners and elevation, kept as design tokens.
  - One icon set, and friendly illustrations for empty states.
  - Subtle motion, such as devices "popping in" when they're found. It
    respects "reduce motion".
- **No jargon on screen:**
  - "Connected directly", "Connected via internet" and "Connected via relay"
    replace QUIC/WS/RELAY. The technical details go behind an "Advanced" or
    debug panel.
  - Clear error messages that tell people what to do, e.g. "Your phone is
    offline. Open my-domain on it.", not `connect failed: os error 61`.
- **Onboarding in minutes:**
  - A welcome flow with sign-in and the server pre-filled. Nobody types a
    server URL.
  - Pair a new device by **scanning a QR code**, not by typing a shared
    encryption key.
  - Ask for each permission when its feature is first used, with a
    one-line "why".
- **Home screen built around devices:**
  - Cards with device type icons, battery, online status and last seen.
  - One-tap actions on each card: send file, ring, share clipboard, control
    media.
- **Simple sharing and permissions:**
  - Presets ("Family", "Guest", "Just me") instead of raw flags.
  - A clear `ask` prompt: "Laptop wants to see your notifications. Allow
    once / always / no."
- **Accessibility:**
  - Screen-reader labels on everything.
  - Large touch targets.
  - Keyboard navigation on desktop.
  - Works with system font scaling.
- ~~Localisation-ready~~: **done** (2026-11-02): Android strings in
  `res/values/strings.xml` (with plurals), desktop strings in
  `ui/src/i18n/en.ts` (typed keys, `t`/`tn`). Still to do: a second
  language (Hindi first?).
- **How it's checked:**
  - A clickable prototype (Figma) reviewed before building.
  - Usability tests with 3–5 non-technical people per release: can they
    pair two devices and send a file unaided?
  - Screenshot tests for both themes, so visual regressions are caught in CI.

## 6. Phase 4: Identity, domains, permissions

Why: [ADR-0005](../learnings/decisions/ADR-0005-device-keys-domains.md).

- **A key per device** (Ed25519/X25519), stored in the OS keystore:
  Keychain on macOS, Android Keystore, Secret Service on Linux.
  - Pair your own devices by QR code or a short code.
  - QUIC certificates come from these keys and are pinned. This closes the
    current gap where peers aren't authenticated (`AcceptAnyCert`).
  - The shared passphrase key is retired.
- **Domains:**
  - Membership certificates signed by the domain owner.
  - Server routes `/domains/*` and `/invites/*`.
  - Server integration tests against a real MongoDB in a container.
- **RPC:** request/response calls between devices. Every call goes through
  grant check → `ask` prompt if needed → handler → audit log.
  - Grants are enforced on the target device: `off`/`ask`/`on`, plus expiry,
    time windows, LAN-only and rate limits.
- **Capability advertisement:** each device announces what it can do, so the
  UI only shows actions that will actually work.
- **UI:**
  - domain list
  - invite by QR code
  - grant editor
  - the `ask` prompt
  - audit log
  - "Revoke all"
- **Tests:**
  - A permission test matrix for every method.
  - Forged-certificate and replay tests.

## 7. Phase 5: Feature waves (see [features.md](features.md))

| Wave | Features |
|---|---|
| A: quick wins | ~~Battery and device info, ring device, open link~~ (done 2026-11-02), ~~flashlight, ringer, volume~~ (done 2026-11-02; Android ones not yet tried on the phone), ~~launch app~~ (done 2026-11-02, untried on the phone), ~~clipboard history~~ (done 2026-11-02), notification actions |
| B: media and phone | ~~Media control for any app~~ (play/pause, next, previous: done 2026-11-02), call log, live call alert, dial and speaker, answer/end, contacts, SMS, ~~OTP → desktop clipboard~~ (done 2026-11-02) |
| C: files | ~~File push (resumable)~~ (done 2026-11-02), browse, download, gallery, photo backup, **Wi-Fi Direct fast path** for big transfers (below) |
| D: screen and control | Screen share, remote input, phone as touchpad/keyboard, presentation remote |
| E: connectivity (Shizuku, see [ADR-0006](../learnings/decisions/ADR-0006-shizuku.md)) | Wi-Fi scan/toggle/connect, Bluetooth, airplane mode, mobile data, hotspot |
| F: find, security, automation | Locate, lost mode, wipe, triggers, scenes, schedules, webhooks |

### Fast path for large transfers: Wi-Fi Direct

When a transfer is large (a big file, a gallery backup, a folder), upgrade to
a **direct Wi-Fi link between the two devices** if both support it, then
switch back afterwards. This is the same idea as Android's Quick Share
"bandwidth upgrade".

- **Why it's faster:**
  - A normal transfer goes device → router → device, sharing the router's
    airtime with everything else on the network.
  - Wi-Fi Direct is one hop between the devices, often several times faster.
  - It also works with no router at all, e.g. outdoors, or when one device is
    on mobile data.
- **How:**
  1. Start every transfer on the current link.
  2. If the file is over a threshold (e.g. 50 MB) and both devices advertise
     Wi-Fi Direct, negotiate the upgrade over the existing encrypted link.
  3. Form the Wi-Fi Direct group and run QUIC over it, with the same keys and
     the same end-to-end encryption.
  4. Resume the transfer on the new link at the byte where it stopped.
  5. If the upgrade fails or the link drops, keep going on the original link.
     The user never has to choose.
- **Where it works:**
  - **Android ⇄ Android:** `WifiP2pManager`.
  - **Android ⇄ Linux:** `wpa_supplicant` P2P over D-Bus. Driver support
    varies, so the laptop has to be tested.
  - **macOS: not available.** Apple doesn't expose Wi-Fi Direct; it uses its
    own AWDL, which has no public API for this. The Mac stays on the normal
    link (LAN QUIC) or a wired/USB link.
  - Where Wi-Fi Direct isn't possible, alternatives worth measuring:
    - Phone hotspot
    - USB tethering (`adb reverse` in the lab)
    - Wi-Fi Aware (NAN) on newer Android
- **Permissions:**
  - Android 13+: `NEARBY_WIFI_DEVICES`.
  - Older Android: location.
  - Ask on the first big transfer, with a one-line reason ("for faster
    transfers to nearby devices").
- **Test:**
  - A lab scenario sends a 1 GB file phone ⇄ laptop with and without the
    upgrade, and records the speed.
  - A fallback test cuts the Wi-Fi Direct link mid-transfer and checks the
    file finishes on the original link and arrives intact (checksum).

A feature counts as done when it has:
- handler unit tests
- a permission-matrix entry
- a 2-node test
- a lab scenario that passes on Mac ⇄ Linux ⇄ Android
- its row ticked in features.md
- a learnings journal entry

## 8. Phase 6: More devices

- **Android TV / Google TV / Fire TV:** an `:app-tv` module. Fire TV has no
  Google Play Services, so no FCM there.
- **Wear OS:** a small `:app-wear` module.
- **iOS / watchOS / tvOS:** Swift bindings from the same UniFFI layer.
- **Windows:** the Tauri desktop app already builds there; the
  Windows-specific feature code is still to write.

## Known issues

- **Linux FCM:** the first fresh registration failed with
  `PHONE_REGISTRATION_ERROR` and worked after a restart. Watch whether it
  happens again.
- **The office network blocks all three server names.** FortiGuard blocks
  the "Dynamic DNS" category (sslip.io, work.gd, servehttp.com). A paid
  domain would pass; your call.
- **QUIC peers aren't authenticated yet.** Messages are still end-to-end
  encrypted. Fixed in Phase 4.
