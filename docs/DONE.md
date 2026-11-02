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

## Clipboard history (2026-11-02)

- The core keeps the last 20 clipboard values (synced from a device,
  pulled, or copied here while sync is on), newest first, each with the
  device it came from; a repeat moves to the top. In memory only, so codes
  and passwords never reach the disk. `Engine::clipboard_history`,
  `clear_clipboard_history`, `copy_to_clipboard`; event
  `clipboard-history`.
- Desktop: "Recent" list in the Clipboard tile with Copy and Clear.
  Android keeps its clipboard in the app layer, so it has no history yet.
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
