# Signaling & NAT traversal — status and setup

This documents the FCM signaling + NAT-traversal work and what still needs
external setup or device testing. The **Rust across all crates compiles**; the
items marked ⚠️ can't be exercised in a headless dev box.

## Server (`server/`)

Endpoints added:
- `POST /devices/register` — now also accepts `fcm_token`, `ipv6`, `supports_ipv6`,
  `platform` (non-clobbering `$set` upsert).
- `POST /devices/state` — partial live update (`ws_open`, `inbound_blocked`,
  `reflexive_ip`, `reflexive_udp_port`).
- `POST /devices/probe` — server dials back to the device's **observed source IP**
  (SSRF-safe) on its advertised tcp/ws ports; records `inbound_blocked`.
- `POST /signal` — relays a typed signal to one of the caller's **own** sibling
  devices via FCM (`from` and `to` both verified against the caller's `user_id`).
- `GET /devices` — `DeviceOut` now exposes `ws_open`, `inbound_blocked`,
  `supports_ipv6`, `ipv6`, `reflexive_ip`, `reflexive_udp_port`. **`fcm_token` is
  never returned** (server secret).

### Server env (FCM)
FCM is optional; if unset, `/signal` returns 503 and everything else works.
- `FCM_SA_JSON` — service-account JSON inline, **or**
- `GOOGLE_APPLICATION_CREDENTIALS` — path to the service-account JSON file
- `FCM_PROJECT_ID` — optional override (defaults to the SA's `project_id`)

Project: `my-domain-c0c36`, sender `744722246836`. ⚠️ **You must drop the
service-account JSON on the server** (kept out of git) — `server/src/fcm.rs`
exchanges it for an OAuth token (cached ~55 min) and calls FCM HTTP v1.

## Core (`mdcore`)

- `signal.rs` — `Signal` enum (`StartWs`/`WsReady`, `PunchOffer`/`Answer`/`Ready`/
  `Failed`, `RelayOffer`/`Answer`, `Bye`) + `Candidate`.
- `registry.rs` — `send_signal`, `update_device_state`, `probe_inbound`,
  `health_ok`; `registry_register` now sends `fcm_token`/`platform`/`supports_ipv6`.
- `engine.rs` — `firewall_check()`, `update_ws_open()`, `send_signal()`,
  `on_signal()` (responder side: `StartWs`→open+`WsReady`, `WsReady`→dial),
  `connect()` (ladder: direct WS → FCM start-WS).
- `nat/stun.rs` — minimal STUN binding client (reflexive address).
- `nat/punch.rs` — UDP hole-punch primitive (simultaneous open).
- `platform.rs` — `Platform::fcm_token()` + `platform_kind()`.
- `wire/{frame,mux}.rs` — protocol-v2 binary frame (tested) + channel allocator.
- `services/{av,remotecontrol}.rs` — feature-gated (`av`, `remote-control`)
  `Service` seams.

## Adapters

- Desktop: commands `connect`, `firewall_check`, `update_ws_open`, `on_signal`;
  `platform_kind = "desktop"`.
- Android: JNI `nativeSetFcmToken`, `nativeOnSignal`, `nativeConnect`,
  `nativeFirewallCheck`, `nativePollEvents`, `nativeShare*`; settable FCM token;
  `platform_kind = "android"`.

## ⚠️ Pending external setup / device testing

1. **Firebase client side (FCM receive):**
   - Android: a `FirebaseMessagingService` (Kotlin) that obtains the token →
     `nativeSetFcmToken`, and on a data message → `nativeOnSignal(from, payload)`.
     Needs `google-services.json` in the android app.
   - Desktop: FCM client delivery is awkward off-mobile — wire the planned
     `GET /signal/stream` SSE channel instead (server endpoint + desktop reader),
     then feed `on_signal`.
2. **coturn (STUN/TURN host):** deploy coturn with `use-auth-secret`; server mints
   ephemeral creds into `RelayOffer`. The TURN relay client + `connect()` rungs for
   hole-punch/relay are not yet wired into the live data path.
3. **NAT end-to-end:** STUN + punch primitives exist but the full
   gather→offer→punch→promote flow over the engine's UDP socket needs multi-network
   device testing.
4. **Protocol v2 cutover:** frame/mux exist; messaging/clipboard/etc. still use v1.
   Cut over alongside the streaming services (coordinated wire change).
5. **AV / remote control:** `Service` seams + capability traits exist; real media
   (codecs, capture/playback, input injection, screen capture) is platform work.
6. **`core/` submodule + smoke test** both apps (see repo root instructions).
