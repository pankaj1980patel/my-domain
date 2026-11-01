# What's done

Status as of **2026-11-01**. Everything below is committed on the `monorepo`
branch, and was built and tested unless a line says otherwise. What's still
left is in [PENDING.md](PENDING.md).

## At a glance

| | |
|---|---|
| Tests | **84** core tests (37 unit tests and 47 integration tests), plus **3** server tests |
| Where the core tests pass | macOS (dev Mac), Linux (Manjaro laptop), **Android phone** (I2405, Android 16, ARM64, run on the device) |
| Where the server tests pass | macOS and Linux |
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
| 0 | Direct QUIC | no | [connect_ladder.rs](../crates/mdcore/tests/connect_ladder.rs) |
| 1 | UDP ping on the same LAN | no | connect_ladder.rs |
| 2 | Direct WebSocket | no | connect_ladder.rs |
| 3 | Ask the peer (via a signal) to open its WebSocket | signal | connect_ladder.rs |
| 4 | STUN + hole punch, then QUIC on the punched socket | signal + STUN | connect_ladder.rs |
| 5 | WebRTC | signal | connect_ladder.rs |
| 6 | Relay through the server | yes | [relay.rs](../crates/mdcore/tests/relay.rs) |

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
