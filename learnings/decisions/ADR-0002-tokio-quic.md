# ADR-0002 — Move the core to tokio, with QUIC as the main transport

- **Status:** Accepted (2026-11-01). Implemented in Phase 2.
- **Phase:** 2

## Context
`mdcore` uses blocking OS threads: a fixed set of listener loops plus a new
thread per connection and, for TCP, per message. Measured problems:
- A new TCP connection (or UDP socket) for **every message**.
- Connection loops wake every 200 ms even when idle, which costs battery.
- Background threads have no shutdown, so engines can't be stopped cleanly or
  restarted in tests.
- Large messages over the hole-punched UDP path are split into 1 KB chunks with
  **no retransmission**, so one lost chunk loses the whole message.
- An unused binary channel mux (`wire/`) shows streaming was always the plan,
  but reliability, ordering and flow control would all have to be written by hand.

The coming features (screen share, file transfer, remote input, many peers) need
reliable, multiplexed, flow-controlled streams.

## Options
1. **Keep threads and fix things incrementally.** Least rewrite. But we'd hand-roll
   reliability and congestion control for screen/file streams, which is very
   hard to get right.
2. **tokio + QUIC (`quinn`).** tokio is the standard async runtime (the server
   already uses it). QUIC gives encrypted, reliable, ordered streams, many per
   connection, plus unreliable datagrams for low-latency input, with congestion
   control built in. It runs on UDP, so it works over the **same hole-punched
   socket** we already create.
3. **tokio + WebRTC for everything.** WebRTC data channels also multiplex, but
   ICE/DTLS/SCTP is heavier and harder to debug. We keep it as a fallback rung only.

## Decision
Option 2. QUIC is the one data plane. TLS 1.3 certificates are derived from the
per-device identity keys (ADR-0005), so the handshake also **authenticates the
peer**, which closes the current spoofing hole (unauthenticated WS `Hello` and
LAN beacons). Each service gets its own stream: control/RPC, file, screen.
Remote input uses QUIC datagrams.

New connect ladder: LAN QUIC → public QUIC → STUN + punch, then QUIC on the
same socket → WebRTC data channel → WS relay through the server (new; fills the
"no TURN" gap).

## Consequences
- ➕ Removes per-message connections, lossy chunking, 200 ms polling loops and the
  hand-written mux.
- ➕ `async`/`.await` reads like Node, which suits a Node background.
- ➕ Cancellation (`CancellationToken`) and a controllable clock in tests
  (`tokio::time::pause`) make networking code testable.
- ➖ A large rewrite of `transport/`, `nat/` and the engine loops. Phase 1 writes
  behaviour tests *first* so the rewrite is checked against them.
- ➖ Bigger binary than plain threads. Acceptable for phone, desktop and TV. For
  watches, `md-proto` stays I/O-free so a minimal build is possible.
