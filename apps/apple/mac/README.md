# my-domain · macOS

Native macOS client for the **my-domain** LAN messenger.

This is a placeholder submodule. The macOS app will implement the same LAN feature
set as the [desktop](https://github.com/pankaj1980patel/my-domain-desktop) (Tauri) app,
but as a native SwiftUI/AppKit application.

- **Discovery** — UDP multicast to the shared group `239.255.42.98:45678`, or Bonjour
  via the Network framework (`NWBrowser` / `NWListener`).
- **Messaging** — send/receive over **UDP** and **TCP** with `NWConnection`.

## Wire compatibility with the desktop app

- **Beacon** (UDP multicast `239.255.42.98:45678`, every ~2s):
  `{"node_id": "...", "name": "...", "tcp_port": <u16>, "udp_port": <u16>}`
- **Message** (TCP body, or single UDP datagram to the peer's advertised port):
  `{"from": "...", "text": "..."}`

## Getting started

```bash
# Create the Xcode project here:
#   File > New > Project > App  (macOS, SwiftUI)
```

> Enable the **Outgoing/Incoming Connections** and **Local Network** entitlements
> in the App Sandbox for discovery and messaging to work.
