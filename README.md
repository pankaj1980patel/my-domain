# my-domain

A cross-platform, **end-to-end-encrypted** messenger. Devices log into a shared
**registry** server to find each other's endpoints (or discover each other on the
LAN), then message **directly** over UDP / TCP / WebSocket. This repository is the
umbrella **superproject**; each component lives in its own repo, wired in as a git
**submodule**.

| Component | Path | Repository | Status |
|-----------|------|-----------|--------|
| Registry server (axum + MongoDB) | [`server/`](server) | [my-domain-server](https://github.com/pankaj1980patel/my-domain-server) | ✅ working |
| Desktop (Tauri + Rust) | [`desktop/`](desktop) | [my-domain-desktop](https://github.com/pankaj1980patel/my-domain-desktop) | ✅ working |
| Android (Rust core + Compose) | [`android/`](android) | [my-domain-android](https://github.com/pankaj1980patel/my-domain-android) | ✅ working |
| macOS (native) | [`mac/`](mac) | [my-domain-mac](https://github.com/pankaj1980patel/my-domain-mac) | 🚧 placeholder |
| iOS (native) | [`ios/`](ios) | [my-domain-ios](https://github.com/pankaj1980patel/my-domain-ios) | 🚧 placeholder |

## How it works

1. **Accounts** — log in (username/password → JWT) and set an **encryption key**
   (a passphrase). Both are required before anything else is allowed.
2. **Discovery** — on login (and on network change) a device registers its endpoints
   with the server and fetches the user's other devices. No polling. **LAN discovery**
   (UDP multicast/broadcast/unicast beacons) is still available as a manual "Scan LAN".
3. **Messaging** — peer-to-peer over **UDP**, **TCP**, or a persistent LAN **WebSocket**
   (handy when one side's inbound is firewalled — either side can trigger the single
   bidirectional socket).
4. **E2EE** — message bodies are encrypted client-side (passphrase → Argon2id →
   XChaCha20-Poly1305). The server stores only endpoints; it never sees plaintext or keys.

### Wire formats

- **Registry**: REST (`/auth/*`, `/devices/*`) — see [`server/README.md`](server/README.md).
- **LAN beacon** (UDP `239.255.42.98:45678`): `{node_id,name,tcp_port,udp_port,ws_port,reply}`.
- **Message** (TCP body / UDP datagram / WS `msg` frame): the encrypted envelope
  `{nonce,ciphertext}` whose plaintext is `{from,text}`.

## Clone

Because the clients are submodules, clone recursively:

```bash
git clone --recurse-submodules https://github.com/pankaj1980patel/my-domain.git
# already cloned without --recurse-submodules?
git submodule update --init --recursive
```

## Working with submodules

```bash
# Pull latest for every client
git submodule update --remote --merge

# Make a change inside a client, then record the new pointer in the superproject
cd desktop && git add -A && git commit -m "…" && git push && cd ..
git add desktop && git commit -m "desktop: bump pointer" && git push
```

## Running the desktop client

```bash
cd desktop
npm install
npm run tauri dev
```

See [`desktop/README.md`](desktop/README.md) for details.
