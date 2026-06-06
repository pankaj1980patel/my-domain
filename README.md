# my-domain

A cross-platform **LAN messenger**: discover other devices on your local network
and exchange messages over **UDP** and **TCP**. This repository is the umbrella
**superproject** — each platform client lives in its own repository, wired in here
as a git **submodule**.

| Platform | Path | Repository | Status |
|----------|------|-----------|--------|
| Desktop (Tauri + Rust) | [`desktop/`](desktop) | [my-domain-desktop](https://github.com/pankaj1980patel/my-domain-desktop) | ✅ working |
| macOS (native) | [`mac/`](mac) | [my-domain-mac](https://github.com/pankaj1980patel/my-domain-mac) | 🚧 placeholder |
| iOS (native) | [`ios/`](ios) | [my-domain-ios](https://github.com/pankaj1980patel/my-domain-ios) | 🚧 placeholder |
| Android (native) | [`android/`](android) | [my-domain-android](https://github.com/pankaj1980patel/my-domain-android) | 🚧 placeholder |

## Shared wire protocol

All clients interoperate by speaking the same formats over the LAN:

- **Discovery beacon** — UDP multicast to `239.255.42.98:45678`, sent every ~2s:
  ```json
  { "node_id": "uuid", "name": "hostname", "tcp_port": 0, "udp_port": 0 }
  ```
  A peer that goes silent for 8s is dropped.
- **Message** — the JSON body of a TCP connection, or a single UDP datagram sent to
  the peer's advertised port:
  ```json
  { "from": "name", "text": "hello" }
  ```

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
