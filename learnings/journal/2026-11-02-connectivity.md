# 2026-11-02: Connectivity, overnight

What changed, why, and the Rust behind it. Work items are tracked in
[docs/PENDING.md](../../docs/PENDING.md) (section 2) and
[docs/DONE.md](../../docs/DONE.md).

## The goal

Make two of your devices connect **in more places, faster, and with less
battery**: across the internet, through routers, when a phone switches
networks, with nobody pressing "Scan".

## What we built, in the order devices try it

| Step | What | File |
|---|---|---|
| Discover | mDNS / DNS-SD on the LAN, continuous | [mdns.rs](../../crates/mdcore/src/mdns.rs) |
| Rung 0 | Dial every address at once: IPv6, router mapping, LAN IPv4 | [transport/quic.rs](../../crates/mdcore/src/transport/quic.rs) `connect_any` |
| Rung 3 | Swap QUIC addresses through a signal; the responder dials back if needed | [engine/ladder.rs](../../crates/mdcore/src/engine/ladder.rs) |
| Relay | Comes up after the quick rungs; hole punch / WebRTC upgrade it | [engine/ladder.rs](../../crates/mdcore/src/engine/ladder.rs) |
| Always | Signals over a WebSocket to the server; FCM only to wake a sleeping phone | [engine/background.rs](../../crates/mdcore/src/engine/background.rs) |

And around it:
- **NAT type detection:** two STUN servers. Same answer means an easy NAT;
  different answers mean symmetric.
- **UPnP port mapping:** the router forwards our QUIC port.
- **Network-change handling, plus QUIC connection migration.**
- **Phone-friendly keep-alive:** 20 s / 60 s instead of 5 s / 20 s.
- **Link quality:** RTT and loss, rated "Excellent", "Good" or "Weak
  connection".
- **Relay fair use:** a token bucket that slows heavy senders and never
  drops their frames.

## Decisions worth remembering

- **Race, don't queue.** Trying addresses one by one makes a dead address
  cost a full timeout. `connect_any` starts every dial at once in a tokio
  `JoinSet` and keeps the first handshake that completes. Node analogy:
  `Promise.any([...])`. In Rust, dropping the losing futures cancels them
  for free.
- **When both sides dial, agree on a winner without talking.** Two QUIC
  connections between the same pair is "glare". Each side keeps the
  connection whose *client* has the lower node id. It's the same rule on
  both ends, so they converge with no extra message. Messages in flight on
  the losing connection can still be lost; request ids and retries (Phase 4
  RPC) fix that properly.
- **Slow down, never drop, in the relay.** Relayed frames aren't
  retransmitted, so dropping one loses a message. The token bucket instead
  stops *reading* the sender's socket for a moment, and TCP backpressure
  slows the sender down automatically.
- **Push notifications are a wake-up call, not a pipe.** Desktop FCM
  registrations kept getting rejected by Google. A device that's running
  can just hold one WebSocket to the server, which is simpler and instant.
  FCM stays only for waking a phone that isn't running the app.
- **Fakes for everything external.** `FakeStun`, `FakeRouter` (UPnP),
  `FakeRelay` and `FakeRegistry` mean no test ever touches the internet or
  your real router. The opt-in
  [live_server.rs](../../crates/mdcore/tests/live_server.rs) is the one test
  that does, on purpose.

## Bugs that only real devices found

1. **The Android APK shipped a day-old Rust core.** Gradle's JNI merge kept
   saying UP-TO-DATE. Lesson: verify what ships. We grepped the `.so`
   inside the APK for strings from new code. The merge now always re-runs.
2. **The QUIC registry race** and **the if-let lock deadlock.** See
   [06-async-tokio.md](../06-async-tokio.md), "Gotchas".
3. **Desktop FCM registration rejected** (`PHONE_REGISTRATION_ERROR`), which
   led to the WebSocket signal channel.

## Verified on real hardware

- The phone on mobile data measured an easy NAT and published a global IPv6
  address.
- Your home router accepted UPnP mappings for both desktops. The Oracle
  server got a QUIC reply from the laptop through the public mapping, so the
  laptop is reachable from the internet without hole punching.
- Two engines logged in to the real HTTPS server chatted through its relay
  over `wss://`.
