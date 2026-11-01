# Glossary

**ADR (Architecture Decision Record):** a short document recording one decision:
context, options, choice, consequences. Lives in [decisions/](decisions/).

**Arc:** *Atomically Reference Counted* pointer. Lets several threads own the same
value; the value is freed when the last owner drops it.

**Argon2id:** a deliberately slow password-hashing function. It turns a
passphrase into a key in a way that's expensive to brute-force.

**Cargo / crate / workspace:** Rust's npm, package, and monorepo respectively.

**cdylib:** a crate compiled as a C-compatible shared library (`.so` / `.dylib`
/ `.dll`) that another language loads. The Android `.so` is a cdylib.

**Clippy:** Rust's linter.

**Connect ladder:** the ordered list of ways to reach a peer, cheapest first.
See [01-architecture.md](01-architecture.md).

**Domain:** in my-domain, a named set of devices that trust each other, with
per-feature permission grants. See [docs/features.md](../docs/features.md).

**E2EE (end-to-end encryption):** only the two endpoints can read a message.
Servers in between see ciphertext only.

**EventSink:** the trait the core uses to push events (message received, peer
connected, …) to the app's UI layer.

**FCM (Firebase Cloud Messaging):** Google's push service. The server uses it to
wake a device and deliver connection signals.

**FFI (Foreign Function Interface):** calling code across languages, here Kotlin
→ Rust. **JNI** is the Java/Kotlin flavour. **UniFFI** generates the glue.

**Grant / flag:** permission for a device or domain to use one feature on a
target device (`off` / `ask` / `on`).

**Hole punching:** two devices behind NAT routers send UDP packets to each
other's public address at the same time, so each router opens a path for the
other's packets.

**JWT:** a signed token the server issues at login; clients send it on every request.

**Monorepo:** all components in one git repository.

**Mutex:** a lock. Only one thread at a time can access the data inside.
*Poisoned* means a thread panicked while holding it.

**NAT:** what your home router does. Many devices share one public IP, so
devices outside can't connect in without help.

**Nonce:** a "number used once". Every encrypted message needs a fresh one.

**Panic:** Rust's crash. It unwinds the current thread. Caused by `.unwrap()` on
`None`/`Err`, out-of-bounds indexing, and so on.

**QUIC:** a transport protocol on top of UDP with reliable, ordered, encrypted,
multiplexed streams (HTTP/3 runs on it). Adopted in Phase 2 via the `quinn` crate.

**Serde:** Rust's (de)serialization framework. `#[derive(Serialize, Deserialize)]`.

**Shizuku:** an Android app that lets other apps use ADB-level system APIs
without root. We use it for Wi-Fi, Bluetooth and airplane-mode toggles.

**Signal (connection signal):** a small message relayed through the server to set
up a direct connection (WS request, punch candidates, WebRTC offer/answer).

**STUN:** a tiny protocol for asking a public server "what IP:port do you see me
coming from?". The first step of hole punching.

**Tauri:** a framework for desktop apps with a web UI and a Rust backend
(Electron's role, with a much smaller footprint).

**tokio:** Rust's async runtime (libuv's role).

**TURN / relay:** a server that forwards traffic when no direct path works.

**Trait:** Rust's interface.

**UniFFI:** a Mozilla tool that generates Kotlin/Swift bindings from Rust code.
