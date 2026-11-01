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

## 2. Phase 2 leftovers (networking)

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

## 3. Phase 3: Bridges and apps

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

## 4. Phase 4: Identity, domains, permissions

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

## 5. Phase 5: Feature waves (see [features.md](features.md))

| Wave | Features |
|---|---|
| A: quick wins | Battery and device info, ring device, open link, launch app, flashlight, DND/ringer, volume, clipboard history, notification actions |
| B: media and phone | Media control for any app, call log, live call alert, dial and speaker, answer/end, contacts, SMS, OTP → desktop clipboard |
| C: files | File push (resumable), browse, download, gallery, photo backup |
| D: screen and control | Screen share, remote input, phone as touchpad/keyboard, presentation remote |
| E: connectivity (Shizuku, see [ADR-0006](../learnings/decisions/ADR-0006-shizuku.md)) | Wi-Fi scan/toggle/connect, Bluetooth, airplane mode, mobile data, hotspot |
| F: find, security, automation | Locate, lost mode, wipe, triggers, scenes, schedules, webhooks |

A feature counts as done when it has:
- handler unit tests
- a permission-matrix entry
- a 2-node test
- a lab scenario that passes on Mac ⇄ Linux ⇄ Android
- its row ticked in features.md
- a learnings journal entry

## 6. Phase 6: More devices

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
