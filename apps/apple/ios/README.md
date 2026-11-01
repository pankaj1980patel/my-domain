# my-domain · iOS

Native iOS client for the **my-domain** LAN messenger.

This is a placeholder submodule. The iOS app will implement the same LAN feature
set as the [desktop](https://github.com/pankaj1980patel/my-domain-desktop) app:

- **Discovery** — browse/advertise on the LAN. On iOS the idiomatic approach is
  Bonjour via `NWBrowser` / `NWListener` (Network framework), or raw UDP multicast
  to the shared group `239.255.42.98:45678` to interoperate with the desktop app.
- **Messaging** — send/receive messages to peers over **UDP** and **TCP** using
  `NWConnection`.

## Wire compatibility with the desktop app

To talk to desktop peers, match its formats:

- **Beacon** (UDP multicast `239.255.42.98:45678`, every ~2s):
  `{"node_id": "...", "name": "...", "tcp_port": <u16>, "udp_port": <u16>}`
- **Message** (TCP body, or single UDP datagram to the peer's advertised port):
  `{"from": "...", "text": "..."}`

## Getting started

```bash
# Create the Xcode project here, e.g.
#   File > New > Project > App  (SwiftUI)
```

> Requires the **Local Network** privacy entitlement (`NSLocalNetworkUsageDescription`)
> and a `NSBonjourServices` entry in `Info.plist` for discovery to work on iOS 14+.
