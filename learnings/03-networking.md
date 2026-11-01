# 03 — Networking: how two devices find and reach each other

The hardest problem in this project isn't Rust, it's the internet. This file
explains the networking from first principles, then shows where each idea
lives in the code.

---

## 1. Why "just connect to the other device" doesn't work

Your laptop at home has an address like `192.168.1.20`. That's a **private**
address: thousands of homes use the same one. The internet only sees your
router's single **public** address, say `203.0.113.7`. The router does
**NAT** (Network Address Translation):

```
laptop 192.168.1.20:5000  ──►  router rewrites to 203.0.113.7:61234  ──►  internet
```

When a reply comes back to `203.0.113.7:61234`, the router remembers the
mapping and forwards it to the laptop. But **an unsolicited packet from
outside has no mapping, so the router drops it.** That's why your phone on
mobile data can't simply dial your laptop at home. The laptop has no public
address you could even dial.

Node analogy: running `app.listen(3000)` on your laptop doesn't make it
reachable from the internet. You'd need port forwarding or a tunnel (ngrok).
my-domain does the equivalent automatically.

## 2. The toolbox

| Technique | What it does | Where |
|---|---|---|
| **LAN discovery** | Shout on the local network: "I'm here, these are my ports" (UDP multicast `239.255.42.98:45678` + broadcast + a /24 sweep) | [discovery.rs](../crates/mdcore/src/discovery.rs) |
| **Registry** | A server both devices *can* reach stores each device's addresses | [apps/server](../apps/server), [registry.rs](../crates/mdcore/src/registry.rs) |
| **Signaling** | Small messages relayed through the server (and woken by FCM push) to coordinate a connection | [signal.rs](../crates/mdcore/src/signal.rs), [engine/signaling.rs](../crates/mdcore/src/engine/signaling.rs) |
| **STUN** | Ask a public server "what address do you see me as?" to learn our public `IP:port` | [nat/stun.rs](../crates/mdcore/src/nat/stun.rs) |
| **Hole punching** | Both sides send UDP to each other's public address *at the same time*, so each router creates a mapping that lets the other's packets in | [nat/punch.rs](../crates/mdcore/src/nat/punch.rs), [transport/punch.rs](../crates/mdcore/src/transport/punch.rs) |
| **QUIC** | Reliable, encrypted, multiplexed streams over UDP | [transport/quic.rs](../crates/mdcore/src/transport/quic.rs) |
| **WebRTC** | Browsers' P2P stack (ICE/DTLS/SCTP). A second, independent way to traverse NATs | [transport/rtc.rs](../crates/mdcore/src/transport/rtc.rs) |
| **Relay** | When nothing direct works, forward traffic through the server over an ordinary WebSocket | [transport/relay.rs](../crates/mdcore/src/transport/relay.rs), [apps/server/src/relay.rs](../apps/server/src/relay.rs) |

## 3. Hole punching, step by step

```
 laptop (behind NAT A)                server                phone (behind NAT B)
   │  STUN: "what's my address?" ─► stun.l.google.com          │
   │  ◄─ "203.0.113.7:61234"                                    │
   │── PunchOffer{candidates} ──────► /signal ─► FCM push ────► │
   │                                              STUN, gather   │
   │ ◄──────────────────────────── PunchAnswer{candidates} ─────│
   │══ UDP "mdpunch1" ═══════════════════════════════════════► ✗ (NAT B drops: no mapping yet)
   │ ✗ ◄═══════════════════════════════════════ UDP "mdpunch1" ═│ (but this created NAT B's mapping)
   │══ UDP "mdpunch1" ═══════════════════════════════════════► ✓ now both mappings exist
   │◄═ ping/pong both ways (confirm_round_trip) ═══════════════►│
   │══════════════ QUIC handshake on the SAME socket ══════════►│
```

The key subtlety: **the socket that did STUN must be the socket that punches,
and the socket QUIC then uses.** The NAT mapping belongs to that socket's local
port. That's why `QuicLink::run_over_socket` builds a fresh QUIC endpoint *on
the punched socket* instead of using the engine's main QUIC port.

When it fails: some NATs ("symmetric NAT", common on mobile carriers and
corporate networks) pick a different public port for every destination, so
the address STUN reported is useless for reaching the peer. Punching then fails
and the ladder moves on to WebRTC (which tries more candidate pairs) and,
eventually, the relay.

## 4. QUIC, and why it replaced three things

QUIC (RFC 9000, the transport under HTTP/3) runs over UDP but gives you what
TCP gives and more:

| Need | Before Phase 2 | With QUIC |
|---|---|---|
| Reliable delivery | TCP: a new connection per message. Punched UDP: lost chunk = lost message | Retransmission built in |
| Big messages | Punch path capped at 1 MiB | Streams of any size (we cap frames at 4 MiB) |
| Several things at once (chat + file + screen) | An unused hand-written mux (`wire/`) | Many independent streams per connection |
| Encryption | Only our E2EE envelope | TLS 1.3 on every connection, *plus* our envelope inside |
| Works through NAT holes | Only raw UDP | It's UDP, so it runs on a punched socket |
| Congestion control | None (could flood a slow link) | Built in |
| Dead-peer detection | Our own heartbeat | Keep-alive + idle timeout (5 s / 20 s) |

**Keep-alive matters for NAT:** routers forget idle UDP mappings, often within
30 seconds. A punched hole that goes quiet closes, and the peer can't get back in.
QUIC sends a tiny ping every 5 s (`KEEP_ALIVE` in quic.rs) to hold it open.

### Our framing on a QUIC stream
```
[ u32 big-endian length ][ JSON WsFrame ]   e.g.  {"type":"hello","node_id":"…","name":"…"}
                                                   {"type":"msg","nonce":"…","ciphertext":"…"}
```
QUIC streams are byte streams, like TCP, so message boundaries need a length
prefix. The JSON frames are the same ones the WebSocket link uses, so a QUIC
link plugs into the engine exactly like WS, punch and WebRTC: it registers an
outgoing queue in `ctx.ws_conns`.

### Security note (until Phase 4)
Each engine generates a throwaway self-signed certificate, and the dialer
accepts *any* certificate (`AcceptAnyCert`). The QUIC connection is encrypted,
but **the peer isn't authenticated** yet. A host on your network could pretend
to be your phone, though it still can't read or forge messages, which are
end-to-end encrypted with your key. Phase 4 derives certificates from
per-device identity keys and pins them, so the TLS handshake itself proves who
is on the other end.

## 5. The connect ladder today

[engine/ladder.rs](../crates/mdcore/src/engine/ladder.rs) tries the cheapest
option first. Each rung can be switched off (`CoreConfig::rungs`):

| Rung | Try | Server needed? |
|---|---|---|
| 0 | **Direct QUIC** to the peer's advertised `ip:quic_port` | no |
| 1 | UDP ping on the same LAN | no |
| 2 | Direct WebSocket (peer says it accepts inbound) | no |
| 3 | Ask the peer, via a signal, to open its WebSocket for us | signal |
| 4 | **STUN + hole punch, then QUIC on the punched socket** | signal + STUN |
| 5 | WebRTC data channel | signal (+ STUN) |
| 6 | **Relay** through the server's WebSocket `/relay` | yes (always works if the server is reachable) |

And without calling `connect` at all: **`Engine::send` dials QUIC on first
use** (`auto_quic`) and reuses the link. A failed dial is remembered for 30 s
so an unreachable peer doesn't cost a timeout on every message.

## 6. The relay (rung 6)

Some networks block UDP entirely (corporate Wi-Fi, some hotels), and two
symmetric NATs defeat hole punching. The relay always works if both devices
can reach the server, because it's just an outgoing HTTPS-style connection.

```
laptop ──ws──►  server /relay  ◄──ws── phone
   {"to":"phone","data":"<E2EE frame>"}  ──►  {"from":"laptop","data":"<E2EE frame>"}
```
- **Auth:** the first frame carries the session token; the server verifies it
  (`verify_token`, same check as the HTTP API).
- **Isolation:** frames are only forwarded between devices of the *same user*.
  A test (`never_forwards_to_another_users_device`) proves it on the real server
  code, and `different_accounts_cannot_reach_each_other_through_the_relay` proves
  it end to end against the fake.
- **Privacy:** `data` is never parsed by the server. It's the same encrypted
  frame the direct links carry. A client test scans everything the relay
  forwarded and asserts no plaintext appears.
- **Liveness:** a peer counts as connected only when its `Hello` arrives
  *through the relay*, which proves it's actually connected there. Each side
  answers the first `Hello` it gets, so both confirm even if one joined late.
- **Cost:** every byte crosses the server twice, so it's the last rung, not the first.
- **Limit today:** `ws://` only. A server behind `https://` needs TLS on the
  WebSocket (`wss://`), a small addition when the server moves to HTTPS.

## 7. How it's tested

All of this runs in tests, on loopback, on macOS, Linux and the Android phone:
[crates/mdcore/tests/connect_ladder.rs](../crates/mdcore/tests/connect_ladder.rs)
enables one rung at a time and checks the transport that came up and the exact
signals exchanged. See [08-testing.md](08-testing.md).

Loopback can't fake a real NAT, though. The plan is Linux network namespaces:
two engines, each behind its own simulated NAT router (`ip netns` + `iptables
MASQUERADE`), with a STUN server and the registry "on the internet" namespace.
**Each device has to be its own process** to live in its own namespace. The
engine opens new sockets later from background threads (e.g. for punching),
so pinning threads of one test process to namespaces isn't reliable. That's
why this test arrives with `mdctl`, the headless engine binary, in Phase 3.
