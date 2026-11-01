# What's pending

Status as of **2026-11-01**, in priority order. What's already built is in
[DONE.md](DONE.md). The full plan for every phase lives in the rebuild plan
and in [learnings/](../learnings/README.md).

## 1. Needs a decision or an action from you

| Item | Why |
|---|---|
| **Deploy the latest server to `oracle-dev`** (`apps/server/deploy.sh`) | The live server (152.67.7.50:6969) is still the **Jun 28** build. It has no `/relay` endpoint (returns 404) and doesn't store `quic_port`. Relaying through the server won't work until it's deployed. |
| **Rotate the server `JWT_SECRET`** | The old value was in a committed file before it was redacted. Changing it logs every device out once. |
| **Change the laptop password** | It was shared in chat. |
| **Dirty NTFS partition** on the laptop (`/dev/nvme0n1p6`) | Was already dirty before this work; left untouched. Fix it from Windows (`chkdsk`) or with `ntfsfix`. |
| **Archive the six old GitHub repos** | They're merged into this monorepo with full history. |
| **Merge `monorepo` → `main`** (open a PR) | CI runs only on pushes to `main` and on PRs, so this branch hasn't been through CI yet. |

## 2. Connectivity improvements (do these first)

In priority order. Each item makes devices connect in more places, faster,
or with less battery.

1. **Serve the server on port 443 with HTTPS.** *(in progress, waiting on you)*
   - Done:
     - The latest server is deployed on `oracle-dev` (with `/relay`).
     - Caddy is installed with
       [apps/server/Caddyfile](../apps/server/Caddyfile) for
       `152-67-7-50.sslip.io`.
     - The server reads the real client IP from the proxy.
     - Clients support `https://` and `wss://`.
   - **Blocked:** Oracle Cloud's VCN Security List doesn't allow inbound
     TCP 80/443 yet, so Let's Encrypt can't verify the domain. Open them in
     the console. Caddy then gets the certificate by itself.
   - After that:
     - Switch the apps' default server to `https://152-67-7-50.sslip.io`.
     - Test the relay over `wss://` on the real devices.
     - Later, close `:6969` to the public.
2. ~~Detect NAT type up front~~: **done**, see [DONE.md](DONE.md).
3. ~~Re-check the network when it changes~~: **done**, see [DONE.md](DONE.md).
4. **IPv6.**
   - Many mobile networks (Jio, Airtel) give phones a public IPv6 address
     with no NAT, so QUIC can connect directly, with no punch and no relay.
   - Store **several addresses per device** in the registry (LAN IPv4, global
     IPv6, public address from STUN) and dial them in parallel; the first to
     answer wins.
   - Bind the QUIC endpoint dual-stack (IPv4 + IPv6).
5. **Ask the home router to open a port** (UPnP-IGD, NAT-PMP, PCP).
   - If the router agrees, the device becomes reachable from anywhere on a
     known public port. Publish that address as one more candidate.
   - Renew the mapping before it expires, and remove it on exit.
6. **QUIC connection migration.**
   - When the phone moves from Wi-Fi to mobile data, keep the live QUIC
     connection by rebinding the endpoint to the new network (quinn
     `Endpoint::rebind`) instead of letting it time out and reconnecting
     from scratch.
7. **Standard LAN discovery (mDNS / DNS-SD).**
   - Advertise `_mydomain._udp.local` with node id and ports. macOS has
     Bonjour built in, Android has NSD, Linux has Avahi.
   - It replaces the custom multicast and the /24 sweep (254 packets every
     ~10 s, which some routers flag). Keep the sweep only as a last fallback.
8. **Keep-alive that suits phones.**
   - The fixed 5 s keep-alive and 20 s idle timeout cost a phone battery.
   - Instead: about 25 s keep-alive (just under typical router timeouts),
     idle links closed while the screen is off, and wake-up by push when
     needed.
   - Tune it per platform and per link type: the relay and LAN can afford
     less chatter.
9. **Connection quality.**
   - Read RTT, packet loss and throughput from each QUIC connection (quinn
     stats).
   - Use them to choose between paths, e.g. prefer LAN over relay even when
     both are up.
   - Show it in plain words ("Excellent", "Weak connection") in the device
     card, with details on the Developer screen.
10. **Better relay.**
    - Relays in more than one region; clients pick the closest by measured
      RTT.
    - Move the relay from WebSocket to QUIC, so it has no head-of-line
      blocking and copes better with lossy mobile networks.
    - A per-account bandwidth cap, plus metrics on relay usage.
11. **Bluetooth LE.**
    - Find nearby devices and send small commands (ring, media keys, wake,
      "connect to my hotspot") when there is no shared Wi-Fi or no internet
      at all.
    - It can also be the out-of-band channel for pairing and for starting
      Wi-Fi Direct.
12. **Clean up old paths.**
    - Once QUIC covers every case, delete:
      - the one-off TCP connection and single-UDP-datagram sending
      - the WebSocket rungs (2 and 3)
      - the UDP/TCP "directed transport" setting
    - That's less code and fewer ways to fail. The ladder tests show what can
      safely go.

## 3. Phase 2 leftovers (networking)

- **Signaled QUIC as rung 3** (replaces the signaled WebSocket rung).
  - We send `QuicOffer { sid, our addresses }` through the server.
  - The peer dials us over QUIC **and** replies `QuicAnswer { its addresses }`,
    which we dial too.
  - The first connection to succeed wins. If both succeed, keep the one
    opened by the lower `node_id`, the same tie-break the hole punch uses.
  - It fixes two gaps in today's rungs:
    - **Stale or missing addresses:** the live server returns no
      `quic_port`, and a peer may have changed networks since it last
      checked in. The signal carries its current LAN and public addresses.
    - **One-way reachability:** when only one side accepts incoming
      connections (a firewalled Mac, a phone on mobile data), dialing from
      both ends still connects.
  - Later, remove the WebSocket rungs (2 and 3); QUIC covers them.
  - New ladder: direct QUIC → LAN UDP → **signaled QUIC** → hole punch +
    QUIC → WebRTC → relay.
  - Test: a ladder test with `SignalHub` where only one side can be dialed.
- **Relay right away, upgrade in the background.** Today the rungs run one
  after another, so a peer that's hard to reach takes up to about 66 s to
  connect, or to fail. Planned instead (the approach Tailscale and WebRTC
  use):
  1. **Connect fast:** if the quick rungs (0–3) don't succeed within a few
     seconds, bring up the **relay as a temporary link**. Messages flow
     immediately.
  2. **Keep looking for a direct path:** the hole punch and WebRTC keep
     trying in the background, and are retried every so often (e.g. on a
     network change, or every 30–60 s) while the peer is on the relay.
  3. **Upgrade without a hiccup:** when a direct path comes up, move traffic
     to it and close the relay link. Nothing in flight is lost, because the
     old link is closed only after the new one has carried a confirmation
     both ways.
  4. **Fall back the same way:** if the direct path drops, switch back to
     the relay instead of disconnecting.
  - Better still: **race the rungs instead of queueing them**, like "Happy
    Eyeballs" in browsers. Start direct QUIC, signaled QUIC and the relay
    together, use whichever works first, and keep upgrading to the cheapest
    working path.
  - Cost: the relay only carries traffic until something better is found,
    so server load stays low. Large transfers should wait a moment for an
    upgrade, or trigger the Wi-Fi Direct fast path (Phase 5, wave C).
  - The UI shows the path in plain words: "Connected via relay, looking for
    a faster route…", then "Connected directly".
  - Tests:
    - Peer reachable only via the relay → connects within a few seconds.
    - Direct path appears later → traffic moves over, no message lost.
    - Direct path drops → falls back to the relay.
  - Needs the latest server deployed, because the live one has no `/relay`.
- **Move the remaining loops to tokio.** The TCP/UDP/WS listeners,
  discovery and the heartbeat still run on their own threads. Moving them to
  tokio tasks with a `CancellationToken` makes shutdown clean and immediate.
  Probably best done with Phase 3, because the UniFFI bridge wants an async
  engine anyway.
- **Switch `ws_conns` to tokio channels.** This removes the thread that sits
  between the threaded code and the async code (the "bridge thread").
- **Relay over `wss://` (TLS).** Today the relay only works with an `http://`
  server.
- **Test the engine against the real server's relay.** Today the engine has
  only been tested against `FakeRelay`. Both follow the same protocol, but
  the two have never been connected.

## 4. Phase 3: Bridges and apps

- **UniFFI bridge** (`md-ffi`) for Android. Why:
  [ADR-0003](../learnings/decisions/ADR-0003-uniffi.md).
  - It replaces the hand-written JNI and the 1-second polling. Events are
    pushed through Kotlin callbacks instead.
  - It also removes Kotlin's own copies of the clipboard and reconnect logic.
- **Android FCM receiver** (`FirebaseMessagingService` → `onSignal`). Today
  the phone can't be woken up by a push.
- **Desktop UI in SolidJS + TypeScript**, with typed commands (`tauri-specta`)
  and native OS notifications. Why:
  [ADR-0004](../learnings/decisions/ADR-0004-solidjs.md).
- **`mdctl`**, a command-line engine with no UI (`daemon`, `peers`, `call`,
  `scenario run`). Linux machines can run my-domain without a desktop, and
  it drives the cross-device tests.
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
- **Desktop:** a new SolidJS app replaces `apps/desktop/src/index.html`,
  `main.js` and `styles.css` (about 1,100 lines of vanilla JS).
- **Android:** new Compose screens replace `MainScreen.kt`,
  `LoginScreen.kt` and `SettingsDialog.kt`.

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
- **Localisation-ready:** all strings in resource files, so other languages
  (Hindi first?) can be added.
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
| A: quick wins | Battery and device info, ring device, open link, launch app, flashlight, DND/ringer, volume, clipboard history, notification actions |
| B: media and phone | Media control for any app, call log, live call alert, dial and speaker, answer/end, contacts, SMS, OTP → desktop clipboard |
| C: files | File push (resumable), browse, download, gallery, photo backup, **Wi-Fi Direct fast path** for big transfers (below) |
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
- **Stale FCM thread:** after re-registering, the old FCM receiver thread
  keeps running until its next reconnect. It's harmless but wastes a thread.
- **Unchecked phone step:** after logging in on the phone, nobody has yet
  confirmed that `files/session.json` exists, which would show the session
  survives an app restart.
- **QUIC peers aren't authenticated yet.** Messages are still end-to-end
  encrypted. Fixed in Phase 4.
