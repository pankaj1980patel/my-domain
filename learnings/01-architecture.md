# 01 — Architecture (as of Phase 0, before the rebuild)

This describes the code **as it is today**, so you can read it with the source
open. When later phases change the architecture, this file gets a "Then → Now"
section instead of being rewritten, so the history stays readable.

---

## The big picture

```
              ┌──────────────────────── registry server (apps/server) ───────────────────────┐
              │  axum HTTP + MongoDB:  /auth/*   /devices/*   /signal  ──► FCM push            │
              └───────▲───────────────────────────▲──────────────────────────────┬───────────┘
                      │ HTTPS (ureq)              │                              │ FCM data msg
                      │                           │                              ▼
┌─ desktop app (Tauri) ────────────────┐      ┌─ android app ─────────────────────────────────┐
│  src/main.js  (vanilla JS UI)        │      │  Kotlin / Compose UI, NetService, NotifListener │
│      │ invoke("send_message") ▲ emit │      │      │ RustNet.nativeSend()   ▲ nativePollEvents│
│  src-tauri/src/lib.rs (Tauri cmds)   │      │  rust/src/lib.rs  (JNI functions)              │
│      │                         │     │      │      │                        │               │
│  ┌───▼─────────── mdcore::Engine ────┴──┐   │  ┌───▼──────────── mdcore::Engine ─────────┐  │
│  │ discovery · transports · crypto ·    │◄──┼─►│ (same crate, compiled for Android)       │  │
│  │ features · signaling · connect ladder│ P2P  └──────────────────────────────────────────┘  │
│  └──────────────────────────────────────┘   └────────────────────────────────────────────────┘
```

Three ideas carry the whole design:

1. **One shared core.** All networking, crypto and feature logic is in one Rust
   crate, [crates/mdcore](../crates/mdcore). Each app is a thin shell that adapts
   it to its OS. The same `.rs` files run on macOS, Linux and Android.
2. **The server only does introductions.** It stores which devices exist and
   their addresses, and it can wake a device through push (FCM). Messages never
   pass through it, and it can't read them.
3. **Peers talk directly**, over whichever path works: the same LAN, a public
   address, a hole-punched UDP path, or WebRTC.

---

## Then → Now (after Phase 1)

The table below describes the original layout. Since Phase 1:
- `engine.rs` became the folder [crates/mdcore/src/engine/](../crates/mdcore/src/engine):
  `mod.rs` (shared state + `start`), `session.rs` (login, key), `peers.rs`,
  `messaging.rs` (chat, clipboard), `sharing.rs` (notifications, calls, apps),
  `signaling.rs` (firewall checks, `on_signal`), `ladder.rs` (connect ladder),
  `background.rs` (clipboard watcher, IP watcher, heartbeat).
- New: `config.rs` (`CoreConfig`, `ConnectRungs`), `kv.rs` (`FileKv`), a real
  `error.rs` (`CoreError`), and a `Signaler` trait in `signal.rs`.
- `CoreEvent` serializes itself. The apps no longer build event JSON by hand.
- Undecryptable messages are rate-limited (step 5 below now shows one notice
  per sender per minute instead of one per message).

## The core, `mdcore`, file by file

| File | Job | Node analogy |
|---|---|---|
| [lib.rs](../crates/mdcore/src/lib.rs) | Declares the modules and re-exports the public API | `index.ts` |
| [engine.rs](../crates/mdcore/src/engine.rs) | `Engine<P>`: owns all state, starts the background threads, exposes the public methods. ~1,300 lines; split in Phase 1 | the main service class |
| [platform.rs](../crates/mdcore/src/platform.rs) | The `Platform` trait each app implements (device id, storage, push token, clipboard) | an injected interface (dependency injection) |
| [capabilities.rs](../crates/mdcore/src/capabilities.rs) | Optional abilities: `Clipboard`, `Notifier` | optional interfaces |
| [events.rs](../crates/mdcore/src/events.rs) | `CoreEvent` enum + `EventSink` trait: how the core talks back to the UI | `EventEmitter` |
| [model.rs](../crates/mdcore/src/model.rs) | Wire structs: `Peer`, `Envelope`, `Plaintext`, `WsFrame`, `Beacon` | TS types for JSON payloads |
| [crypto.rs](../crates/mdcore/src/crypto.rs) | Passphrase → Argon2id → 32-byte key; XChaCha20-Poly1305 encrypt/decrypt | `crypto.subtle` |
| [discovery.rs](../crates/mdcore/src/discovery.rs) | LAN discovery: UDP multicast beacons on `239.255.42.98:45678`, broadcast, and a /24 sweep | mDNS/bonjour-ish |
| [registry.rs](../crates/mdcore/src/registry.rs) | Blocking HTTP client for the server (ureq) | `fetch` wrapper |
| [signal.rs](../crates/mdcore/src/signal.rs) | `Signal` enum: messages sent through the server to set up connections | |
| [transport/mod.rs](../crates/mdcore/src/transport/mod.rs) | Send/receive over UDP/TCP, choosing a transport (`best_proto`), handling incoming messages | the socket layer |
| [transport/ws.rs](../crates/mdcore/src/transport/ws.rs) | Persistent WebSocket per peer (tungstenite) | `ws` package |
| [transport/punch.rs](../crates/mdcore/src/transport/punch.rs) | Data over a hole-punched UDP path, with chunking for big messages | |
| [transport/rtc.rs](../crates/mdcore/src/transport/rtc.rs) | WebRTC data channel (str0m), behind the `webrtc` feature | `RTCPeerConnection` |
| [nat/stun.rs](../crates/mdcore/src/nat/stun.rs), [nat/punch.rs](../crates/mdcore/src/nat/punch.rs) | Ask a STUN server "what's my public address?", then punch | |
| [features/](../crates/mdcore/src/features) | Small JSON control messages (clipboard, notification, call, apps, ping), each in its own file, dispatched by `"type"` | an Express router keyed on `type` |
| [wire/](../crates/mdcore/src/wire), [service.rs](../crates/mdcore/src/service.rs), [services/](../crates/mdcore/src/services) | Scaffolding for a binary multiplexed protocol and streaming services; **nothing uses it yet** | |

---

## One chat message, end to end

The user types "hi" on the desktop, sends it to their phone, and the phone shows it.

**Desktop, outgoing**
1. [apps/desktop/src/main.js](../apps/desktop/src/main.js) calls
   `invoke("send_message", { nodeId, text })`. Tauri's `invoke` is IPC from the
   web view to Rust, much like `ipcRenderer.invoke` in Electron.
2. [apps/desktop/src-tauri/src/lib.rs](../apps/desktop/src-tauri/src/lib.rs):
   the `#[tauri::command] fn send_message` wrapper calls `engine.send(&node_id, &text)`.
3. `Engine::send` in [engine.rs](../crates/mdcore/src/engine.rs) picks a transport
   with `proto_for(node_id)`, which calls `best_proto` in
   [transport/mod.rs](../crates/mdcore/src/transport/mod.rs):
   - `"WS"` if a persistent link exists. WS, punch and WebRTC links all register
     in the `ws_conns` map, so they all count as "WS" here.
   - otherwise the user's chosen directed transport, `"UDP"` (default) or `"TCP"`.
4. `transport::send_payload`:
   - Wraps the text as `Plaintext { from, text }` and serializes it to JSON.
   - Encrypts it with the shared key, giving `Envelope { nonce, ciphertext }`, both base64.
   - **WS**: wraps it as `WsFrame::Msg` and pushes it into that peer's channel. The
     connection's own thread writes it to the socket.
   - **UDP**: sends one datagram. **TCP**: opens a new connection, writes, and closes.
     (A new connection per message is one of the inefficiencies Phase 2 removes.)

**Phone, incoming**
5. A listener thread in the phone's `Engine` receives the bytes and calls
   `transport::handle_incoming`:
   - It decrypts with the key. **If decryption fails, the message is still sent to
     the UI marked as undecryptable.** That's a spam vector, fixed in Phase 1.
   - It parses `Plaintext`, then offers `msg.text` to the `ControlRegistry`. If
     the text is a JSON object whose `"type"` a feature claims (e.g.
     `"clipboard"`), that feature handles it and the function stops there.
   - Otherwise it emits `CoreEvent::MessageReceived` to the `EventSink`.
6. Android's `EventSink` ([apps/android/rust/src/lib.rs](../apps/android/rust/src/lib.rs),
   `InboxSink`) pushes the event as JSON into a queue (max 500 items).
7. Kotlin's `NetService` calls `RustNet.nativePollMessages()` **every second**,
   parses the JSON, and the Compose UI shows it. Phase 3 replaces this polling
   with a push callback through UniFFI.

The desktop's incoming path is the same, except its `TauriSink` calls
`app.emit("message-received", …)`, a Tauri event the JS hears through `listen(...)`.

---

## Connecting across the internet: the connect ladder

When two devices aren't on the same LAN, `run_connect_ladder` in
[engine.rs](../crates/mdcore/src/engine.rs) tries the cheapest option first and
moves on after each timeout:

| Rung | Try | Needs |
|---|---|---|
| 1 | UDP ping on the LAN | same network |
| 2 | Direct WebSocket to the peer's public address | the peer accepts inbound connections (`ws_open`, not `inbound_blocked`) |
| 3 | Ask the peer through the server to open a WS **back** to us (`Signal::StartWs`) | *we* accept inbound |
| 4 | UDP hole punch: both sides learn their public address from STUN, swap candidates through the server, then fire packets at each other at the same time | NATs that keep the same mapping (most home routers) |
| 5 | WebRTC data channel (str0m), offer/answer through the server | same as 4 with ICE |
| ✗ | Relay through a server (TURN) | **not implemented**; added in Phase 2 |

Rungs 3–5 need the **signal** to reach the other device. The server sends it as
an FCM push. Desktop receives FCM natively
([apps/desktop/src-tauri/src/fcm.rs](../apps/desktop/src-tauri/src/fcm.rs)). **Android
has no FCM receiver yet**, so a desktop can't start rungs 3–5 *toward* a phone.
Phase 3 adds a `FirebaseMessagingService`.

---

## Security model today, and why it changes

- All devices of a user derive **the same key** from one passphrase, salted with
  the username. It's simple, but:
  - anyone with the passphrase can read everything from every device;
  - you can't share *one* device with *another person* without giving them all of it;
  - nothing proves which device sent a message. The `node_id` in a WS `Hello` or
    a LAN beacon is taken on trust.
- No replay protection: a captured encrypted message can be sent again.

The domain/permission model in [docs/features.md](../docs/features.md) needs
per-device identities, so Phase 4 replaces this with per-device keys and signed
domain membership. See [decisions/ADR-0005-device-keys-domains.md](decisions/ADR-0005-device-keys-domains.md).

---

## The apps today

| App | UI | Bridge to core | Notes |
|---|---|---|---|
| Desktop ([apps/desktop](../apps/desktop)) | Vanilla JS, one 615-line `main.js` | ~35 hand-written `#[tauri::command]`s + `TauriSink` | Clipboard via `arboard`, native FCM receiver |
| Android ([apps/android](../apps/android)) | Jetpack Compose | ~30 hand-written JNI functions passing JSON strings | Polls the core every 1 s; Kotlin re-implements the clipboard handler and reconnect logic |
| Server ([apps/server](../apps/server)) | n/a | n/a | axum + MongoDB, JWT auth, FCM sender |
| Apple ([apps/apple](../apps/apple)) | n/a | n/a | READMEs only |
