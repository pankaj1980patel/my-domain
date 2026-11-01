# my-domain

Control and share your devices with each other: phone ⇄ laptop ⇄ desktop, and
later TV and watch. Devices talk **directly and end-to-end encrypted**. A small
registry server only introduces them.

Devices are grouped into **domains**. Any device can be shared inside or outside
a domain with per-feature permission flags (`off` / `ask` / `on`). See the full
feature catalogue in [docs/features.md](docs/features.md).

> **New here, or new to Rust?** Start with [learnings/](learnings/README.md). It
> explains every folder and decision, with Node.js comparisons.

## Layout

| Path | What | Status |
|---|---|---|
| [crates/mdcore](crates/mdcore) | Shared Rust core: discovery, transports, crypto, features | ✅ working; being split and rebuilt (see plan) |
| [apps/server](apps/server) | Registry + signaling server (axum, MongoDB, FCM) | ✅ working |
| [apps/desktop](apps/desktop) | Desktop app, macOS / Linux / Windows (Tauri) | ✅ working |
| [apps/android](apps/android) | Android app (Compose + Rust core) | ✅ working |
| [apps/apple](apps/apple) | iOS / native macOS | 🚧 placeholder |
| [docs](docs) | Feature catalogue, signaling protocol | |
| [learnings](learnings) | Why everything is the way it is | |
| [scripts/lab](scripts/lab) | Real-device testing (Mac ⇄ Arch laptop ⇄ Android phone) | |

This used to be six git submodules. It's now one Cargo workspace, with every
repo's history preserved ([ADR-0001](learnings/decisions/ADR-0001-monorepo.md)).

## Build and test

```bash
cargo check --workspace          # type-check everything
cargo test --workspace           # run all Rust tests
cargo run -p my-domain-server    # run the server (needs apps/server/.env)

cd apps/desktop && npm install && npm run tauri dev   # desktop app
scripts/lab/android.sh install   # build the APK on the lab laptop and install it on the phone
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, tests on Linux and macOS, the
Android build, and a dependency audit on every PR.

## How it works today

1. **Accounts**: log in (username/password → JWT) and set an encryption passphrase.
2. **Discovery**: devices register their endpoints with the server and fetch the
   user's other devices. LAN discovery (UDP multicast) also works with no server.
3. **Connecting**: same LAN → direct; otherwise a ladder of direct WebSocket →
   reverse WebSocket via push signal → UDP hole punch → WebRTC.
4. **E2EE**: passphrase → Argon2id → XChaCha20-Poly1305. The server never sees
   plaintext or keys.

Details: [learnings/01-architecture.md](learnings/01-architecture.md). The
signaling protocol is in [docs/SIGNALING.md](docs/SIGNALING.md).
