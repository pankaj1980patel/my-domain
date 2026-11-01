# 00 — Node.js → Rust, for this repo

This is not a Rust tutorial. It covers only the Rust ideas this codebase
actually uses, in the order you will meet them, each mapped to something you
already know from Node/TypeScript. For depth, read *The Rust Book*
(doc.rust-lang.org/book) alongside.

---

## 1. Tooling: cargo is npm

| Node | Rust | In this repo |
|---|---|---|
| `npm` / `pnpm` | `cargo` | `cargo build`, `cargo test`, `cargo run -p my-domain-server` |
| `package.json` | `Cargo.toml` | [crates/mdcore/Cargo.toml](../crates/mdcore/Cargo.toml) |
| `package-lock.json` | `Cargo.lock` | one at the repo root, shared by all crates |
| npm package | **crate** | `mdcore`, `serde`, `tokio`, … |
| npmjs.com | crates.io | |
| pnpm workspace | **Cargo workspace** | root [Cargo.toml](../Cargo.toml) `[workspace] members = [...]` |
| `node_modules/` + `dist/` | `target/` | one `target/` at the root for everything |
| `npm run lint` (eslint) | `cargo clippy` | runs in CI with warnings treated as errors |
| prettier | `cargo fmt` (rustfmt) | CI fails if code isn't formatted |
| jest / vitest | `cargo test` (built in) + `cargo nextest` (faster runner) | |
| `npx tsc --noEmit` | `cargo check` | type-checks without producing a binary, fast |

**Feature flags**: Cargo has a feature system that npm lacks. A crate can declare
optional pieces that are compiled in only when asked for. `mdcore` has a
`webrtc` feature that pulls in the `str0m` crate; turn it off and that code
doesn't exist in the binary at all. Later, feature flags pick which
capabilities a TV or watch build includes.

```toml
# crates/mdcore/Cargo.toml
[features]
default = ["webrtc"]
webrtc = ["dep:str0m"]
```
```rust
#[cfg(feature = "webrtc")]      // this module only exists when the feature is on
pub mod rtc;
```
Node has no real equivalent; the closest is a bundler stripping dead code
behind `if (process.env.FEATURE)`.

---

## 2. Compiled, typed, no runtime

Node runs your JS on V8 with a garbage collector. Rust compiles ahead of time
to a native binary, has no garbage collector and no runtime VM. Consequences
you'll feel:

- **Errors move from runtime to compile time.** The compiler refuses code that
  could use freed memory, race between threads, or ignore an error. It feels
  strict at first. Each refusal is a bug you would otherwise find in production.
- **The same core runs everywhere.** The `mdcore` crate compiles into the macOS
  app, the Linux app, an Android `.so`, and later iOS. Node can't go to Android
  or a watch like that.
- **Cross-compiling** is a first-class concept: `--target aarch64-linux-android`.

---

## 3. Variables, mutability, types

```rust
let name = String::from("pixel");     // immutable by default (like const)
let mut count = 0;                    // `mut` = can change (like let)
count += 1;
let port: u16 = 45678;                // explicit type: unsigned 16-bit
```
Node: `const name = "pixel"; let count = 0;`

Number types are explicit: `u8 u16 u32 u64` (unsigned), `i32 i64` (signed),
`f64`, `usize` (pointer-sized, used for lengths). A port is a `u16` because ports
range from 0 to 65535. That's the type system encoding a fact.

Two string types, which confuses everyone at first:
- `String`: an owned, growable string on the heap. You own it and can change it.
- `&str`: a *borrowed view* of some string data. It's cheap and read-only.
Functions usually **take** `&str` (they only need to look at the text) and
**return** `String` (they hand you something new that you now own).

```rust
// crates/mdcore/src/crypto.rs
pub fn derive_key(passphrase: &str, username: &str) -> Option<[u8; 32]>
```

---

## 4. Ownership and borrowing, the core idea

In Node, objects are shared freely and the GC frees them when no one points at
them. Rust has no GC. Instead, **every value has exactly one owner**, and the
value is freed when its owner goes out of scope.

```rust
let a = String::from("hi");
let b = a;              // ownership MOVES to b
// println!("{a}");     // compile error: a was moved
```
To let someone use a value without taking it, you **borrow** it:
- `&T`: shared borrow, read-only. You can have many at once.
- `&mut T`: exclusive borrow, read-write. Only one at a time, and no `&T` alongside it.

That rule ("many readers **or** one writer") is what lets the compiler rule out
data races. In Node you never think about it because JS runs on one thread.
Rust code here uses many threads, so the rule matters.

`.clone()` makes a deep copy when you really need a second owned value. You'll
see a lot of `.clone()` in [engine.rs](../crates/mdcore/src/engine.rs). Some of
it is necessary; some is a sign of the "pass everything everywhere" style the
Phase 1 refactor cleans up.

---

## 5. No null, no exceptions: `Option` and `Result`

Rust has no `null`/`undefined` and no `throw`. Instead there are two enums:

```rust
enum Option<T> { Some(T), None }          // "maybe a value" — replaces null
enum Result<T, E> { Ok(T), Err(E) }       // "value or error" — replaces throw
```

| Node | Rust |
|---|---|
| `user?.name ?? "anon"` | `user.map(\|u\| u.name).unwrap_or("anon".into())` |
| `try { const x = f(); } catch (e) { … }` | `match f() { Ok(x) => …, Err(e) => … }` |
| `const x = await f();` inside an async fn that rethrows | `let x = f()?;`: the **`?` operator** returns the error to the caller early |
| `x!` (TS non-null assertion) | `.unwrap()`, which **panics** (crashes the thread) if it's `None`/`Err` |

The `?` operator is what you'll use most. Inside a function that returns
`Result`, `?` means "if this failed, return the error now; otherwise give me the
value". It's like a `try/catch` that rethrows, but visible at every call.

```rust
// crates/mdcore/src/crypto.rs — every step can fail; `?` bails out with None
let nonce = STANDARD.decode(&env.nonce).ok()?;
let ct    = STANDARD.decode(&env.ciphertext).ok()?;
```

**Gotcha in this codebase:** there are about 80 `.lock().unwrap()` calls. A
`Mutex` (section 9) becomes *poisoned* if a thread panics while holding it, and
then every later `.unwrap()` on it panics too, so one bug takes down the engine.
Phase 1 replaces these. CI already warns on `unwrap` (`clippy::unwrap_used` in
the root `Cargo.toml`).

Errors are currently plain `String`s (`Result<T, String>`). That's like
`throw "something went wrong"`: it works, but callers can't tell error kinds
apart. Phase 1 moves to a typed error enum built with the `thiserror` crate, the
equivalent of custom `class NotLoggedInError extends Error`.

---

## 6. Structs, impl, traits: classes and interfaces, split apart

Rust has no classes. Data and behaviour are declared separately:

```rust
pub struct Peer {                 // data only (like a TS interface/type for an object)
    pub node_id: String,
    pub ip: String,
    pub tcp_port: u16,
}

impl Peer {                       // methods, attached afterwards
    pub fn addr(&self) -> String { format!("{}:{}", self.ip, self.tcp_port) }
}
```
`&self` is `this`, borrowed read-only. `&mut self` is a mutable `this`. A method
without `self` is a static method: `Peer::new(...)`.

A **trait** is an interface: a set of methods a type promises to have.

```rust
// crates/mdcore/src/platform.rs (shortened)
pub trait Platform: Send + Sync + 'static {
    fn device_id(&self) -> String;
    fn device_name(&self) -> String;
    fn kv_get(&self, _key: &str) -> Option<String> { None }   // default method body
    fn clipboard(&self) -> Option<&dyn Clipboard> { None }
}
```
Node/TS: `interface Platform { deviceId(): string; kvGet?(key: string): string | undefined }`

The desktop app implements `Platform` with real OS calls. Android implements it
with Android calls. **Tests will implement it with a fake** (`MockPlatform`,
Phase 1). That's the whole trick for keeping one core across many platforms,
and the most important pattern in this repo.

`Send + Sync + 'static` on the trait means "safe to move to and share between
threads, and holds no short-lived references". The engine runs on background
threads, so whatever platform you give it must be thread-safe. The compiler
enforces this.

---

## 7. Generics and `dyn`: two ways to be polymorphic

```rust
pub struct Engine<P: Platform> { platform: Arc<P>, /* … */ }   // generic
pub sink: Arc<dyn EventSink>,                                    // trait object
```
- **Generics** (`Engine<P>`): the compiler makes a separate copy of the code for
  each concrete `P`. Calls are direct and fast, and the type is fixed when you
  compile. Like a TS generic, but it really produces specialised code.
- **`dyn Trait`**: a pointer plus a lookup table. The concrete type is chosen at
  runtime. This is the closest thing to how *every* JS method call works.

Rule of thumb in this repo: the platform is generic (fixed per app), while the
event sink and capabilities are `dyn` (they're optional and swapped at runtime).

---

## 8. Enums are tagged unions, and `match` is an exhaustive switch

Rust enums can carry data in each variant, like TS discriminated unions:

```rust
// crates/mdcore/src/events.rs (first three of many variants)
pub enum CoreEvent {
    PeersUpdated { peers: Vec<Peer> },               // struct-like variant
    MessageReceived(IncomingMessage),                // tuple-like variant wrapping a struct
    PeerConnected { node_id: String, transport: TransportKind },
    // …
}
```
TS: `type CoreEvent = { kind: "peersUpdated", peers: Peer[] } | { kind: "messageReceived", msg: IncomingMessage } | …`

`match` is a `switch` that the compiler checks for **completeness**: add a new
variant and every `match` that doesn't handle it becomes a compile error. With
about 100 features coming, this is how we make sure no handler is forgotten.

```rust
match event {
    CoreEvent::PeersUpdated { peers } => render(peers),
    CoreEvent::MessageReceived(msg) => show(msg),
    CoreEvent::PeerConnected { node_id, .. } => mark_online(node_id),  // `..` = ignore other fields
    _ => {}                                    // `_` = "everything else" (opts out of the completeness check)
}
```

---

## 9. Concurrency: the biggest mental shift from Node

**Node** has one JS thread plus an event loop (libuv). You never share memory
between threads, so you never lock anything.

**Rust (today's `mdcore`)** uses real OS threads, several running at once:
discovery listener, TCP listener, UDP listener, WS listener, clipboard watcher,
heartbeat, one thread per connection. Shared state needs two tools:

| Tool | Meaning | Node analogy |
|---|---|---|
| `Arc<T>` | *Atomically Reference Counted* pointer: many owners across threads; freed when the last one drops | a JS object reference, but counted explicitly |
| `Mutex<T>` | Only one thread at a time may touch the `T` inside; `.lock()` waits for your turn | no analogy (JS never needs it) |
| `Arc<Mutex<T>>` | "shared, mutable, thread-safe" | a module-level `let` that every callback mutates |
| `mpsc::channel` | a queue between threads: many senders, one receiver | `EventEmitter` / a message queue |
| `AtomicBool` | a bool that's safe to flip from any thread without a lock | |

Look at `NetCtx` in [engine.rs](../crates/mdcore/src/engine.rs): 16 fields, most
`Arc<Mutex<…>>`, and the same set is copied into `Engine` and into a hand-written
`Clone` impl. Phase 1 folds them into one `Shared` struct behind one `Arc`.

**Where we're going (Phase 2): async with tokio.** tokio is Rust's libuv: an
event loop that runs many lightweight *tasks* on a few threads. Rust's
`async fn` / `.await` will feel almost exactly like Node's. The big difference
is that tokio can use **several threads**, so shared state still needs
`Arc`/`Mutex`, just much less of it. See
[decisions/ADR-0002-tokio-quic.md](decisions/ADR-0002-tokio-quic.md).

| Node | tokio |
|---|---|
| `async function f() {}` | `async fn f() {}` |
| `await p` | `p.await` |
| `Promise.all([a, b])` | `tokio::join!(a, b)` |
| `Promise.race` | `tokio::select!` |
| `setTimeout` | `tokio::time::sleep` |
| `AbortController` | `CancellationToken` |
| `net.createServer` | `tokio::net::TcpListener` |

---

## 10. Modules and visibility

```rust
// crates/mdcore/src/lib.rs
pub mod crypto;        // loads src/crypto.rs (like `export * as crypto from "./crypto"`)
pub mod transport;     // loads src/transport/mod.rs (like an index.ts)
pub use engine::Engine;  // re-export: users write mdcore::Engine
```
Everything is **private by default**. `pub` exports it, `pub(crate)` makes it
visible inside this crate only (like an internal module that isn't in the
package's `exports`). `use` is `import`.

---

## 11. Macros: the `!` functions

`println!`, `format!`, `vec![]`, `json!({...})` are **macros**: code that writes
code at compile time. Two kinds you'll see:
- Function-like: `format!("{}:{}", ip, port)` is a type-checked template literal.
- Derive: `#[derive(Serialize, Deserialize, Clone, Debug)]` on a struct makes the
  compiler generate JSON (de)serialization, cloning and debug printing.
  `serde` is the JSON library. `#[derive(Serialize)]` is what gives you
  `JSON.stringify` for free, but type-checked.

```rust
#[derive(Serialize, Deserialize)]
pub struct Envelope { pub nonce: String, pub ciphertext: String }
let json = serde_json::to_string(&env)?;          // JSON.stringify
let env: Envelope = serde_json::from_str(&json)?; // JSON.parse + schema validation in one step
```
That last line *validates* the shape too, roughly what you'd use `zod` for in Node.

---

## 12. Lifetimes (just enough)

You'll occasionally see `'a`, `'l` or `'static`, e.g. in
[apps/android/rust/src/lib.rs](../apps/android/rust/src/lib.rs):
`fn …<'l>(env: JNIEnv<'l>, …)`. A lifetime is a compile-time label for "how long
this borrowed reference stays valid". Here it says the Java string handles are
only valid during this one JNI call. `'static` means "valid for the whole
program". Mostly the compiler infers lifetimes. When it asks for one, it's
asking you to prove a borrowed reference won't outlive the data it points to.

---

## 13. `unsafe` and FFI

`unsafe { … }` turns off some compiler checks so you can call C, or other
languages through a C interface, which is called **FFI**. The Android app calls
into Rust through JNI, a C interface, and that's why its functions look like:

```rust
#[no_mangle]                                   // keep this exact symbol name so Java can find it
pub extern "system" fn Java_com_mydomain_android_RustNet_nativeSend(...)
```
Hand-written FFI is the riskiest code in the repo. Phase 3 replaces it with
**UniFFI**, which generates the Kotlin (and later Swift) bindings from a
single Rust definition. See [decisions/ADR-0003-uniffi.md](decisions/ADR-0003-uniffi.md).

---

## 14. Tests live next to the code

```rust
// at the bottom of any .rs file
#[cfg(test)]                 // compiled only for `cargo test`
mod tests {
    use super::*;            // import everything from the parent module
    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let env = encrypt(&key, b"hi").unwrap();
        assert_eq!(decrypt(&key, &env).unwrap(), b"hi");
    }
}
```
Unit tests sit in the same file and can see private functions. Integration tests
go in a crate's `tests/` folder and only see the public API, like a consumer
would. The single existing test is in
[crates/mdcore/src/wire/frame.rs](../crates/mdcore/src/wire/frame.rs). Phase 1 adds
many more. See 08-testing.md (written in Phase 1).
