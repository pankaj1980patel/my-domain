# 06 — Async Rust and tokio (as used here)

## Node's model vs Rust's

**Node:** one JS thread, one event loop (libuv). `async`/`await` is built in.
Everything I/O is non-blocking by default. You can't accidentally block other
requests... until you run a big `for` loop or `fs.readFileSync`.

**Rust:** the language has `async fn` and `.await`, but **no built-in event
loop**. You choose a *runtime* library to run async code. **tokio** is the
standard one, the libuv of Rust. Two big differences from Node:

1. **tokio can use several threads** (a "multi-thread runtime"). Tasks may move
   between threads, so anything they share must be thread-safe (`Send`, `Arc`,
   `Mutex`). The compiler enforces this.
2. **Futures are lazy.** `let f = do_io();` does nothing until something
   `.await`s it or `spawn`s it. In Node, calling an async function starts it
   immediately.

| Node | tokio |
|---|---|
| `async function f() {}` / `await p` | `async fn f() {}` / `p.await` |
| the event loop | `tokio::runtime::Runtime` |
| fire and forget: `doIt()` | `tokio::spawn(do_it())` (returns a `JoinHandle`) |
| `Promise.all` / `Promise.race` | `tokio::join!` / `tokio::select!` |
| `setTimeout` / `AbortSignal.timeout(3000)` | `tokio::time::sleep` / `tokio::time::timeout(dur, fut)` |
| `EventEmitter` / a queue between parts | `tokio::sync::mpsc` channels |
| worker_threads for CPU/blocking work | `spawn_blocking` / `block_in_place` |

## Where we are: a hybrid (Phase 2, in progress)

`mdcore` started as plain OS threads (see [01-architecture.md](01-architecture.md)).
Rewriting everything at once would be risky, so tokio comes in piece by piece:

```
            synchronous world (std threads)          │        async world (tokio runtime, 2 threads)
                                                     │
 Engine::send ──► ensure_link ──► QuicLink::connect ─┼─► rt.block_on(handshake)  ◄── blocks the caller ≤ 3 s
                                                     │
 engine queues a frame: std::mpsc::Sender ───────────┼─► bridge thread ─► tokio mpsc ─► writer task ─► QUIC stream
                                                     │
 handle_incoming (locks, sink, features) ◄───────────┼── block_in_place ◄── reader task ◄── QUIC stream
```

The three bridging tools, and why each is used:

- **`Handle::block_on(future)`**: run a future to completion *from a normal
  thread* and wait for the result. The engine's API (`send`, `connect`) is
  still synchronous, so dialing QUIC blocks the caller until the handshake is
  done. **Rule:** never call it from *inside* the runtime. tokio panics,
  because a worker thread waiting on itself would deadlock. That's why
  `ensure_link` (auto-dial) only runs on API threads, never from network handlers.
- **`block_in_place(closure)`**: from *inside* an async task, run blocking
  code without stalling the other tasks. tokio hands this worker's other tasks
  to another thread first. Used to call the synchronous `handle_incoming`.
- **A bridge thread**: links register a `std::sync::mpsc::Sender<String>` in
  `ws_conns` (the type every transport shares). A small thread does a blocking
  `recv()` on that and forwards into a `tokio::sync::mpsc` channel the async
  writer awaits. It's temporary and goes away when `ws_conns` switches to tokio
  channels.

## Who owns the runtime (and a panic we designed around)

```rust
pub struct Engine<P> { …, _quic: QuicNode }      // owns the Runtime
pub struct NetCtx<P> { …, pub quic: QuicLink }   // just a Handle + Endpoint (cheap clones)
```
Every background task holds a `NetCtx` clone. If `NetCtx` owned the runtime:
1. the accept-loop task would keep its own runtime alive forever (a reference cycle), and
2. if the last clone were dropped *inside* a runtime task, tokio would panic
   ("cannot drop a runtime in a context where blocking is not allowed").

So the `Engine` (dropped on an ordinary thread) owns it, and everything else
holds a `Handle`. **Field order in `QuicNode` matters:** Rust drops struct fields
in declaration order, so `link` (the endpoint) closes *before* `_runtime` shuts down.

## Gotchas met so far
- **quinn needs a runtime context to bind a socket.** `runtime.enter()` returns
  a guard that makes "the current runtime" available on this thread. Hold it
  while calling `Endpoint::server(...)`.
- **A std `UdpSocket` handed to async code must be non-blocking:**
  `socket.set_nonblocking(true)` before `Endpoint::new(...)`. Otherwise it blocks
  a worker thread.
- **`read_exact` on a quinn stream returns quinn's own error type**, not
  `io::Error`, so `?` couldn't convert it. Fixed with `.map_err(io::Error::other)`.
- **A lock guard in an `if let` lives for the whole block.** This deadlocked
  the signal-channel keeper (2026-11-02):

  ```rust
  // BAD: the guard from `ctx.relay.lock()` is a temporary in the scrutinee,
  // and Rust keeps it alive until the end of the `if let` block, so
  // ensure_relay (which locks ctx.relay again) waits forever.
  if let (Some(t), true) = (target, ctx.relay.lock().is_none()) {
      ensure_relay(&ctx, ..);
  }
  // GOOD: copy the answer out first; the guard dies at the `;`.
  let connected = ctx.relay.lock().is_some();
  if !connected { ensure_relay(&ctx, ..); }
  ```
  Node has no equivalent: there is one thread, so there are no locks to hold.
  In Rust a `Mutex` guard unlocks when it's dropped, and "when" depends on
  the scope of the temporary it lives in. Rule of thumb: never call anything
  that might lock again while a guard can still be alive. Read into a
  variable first.
- **Two maps that must agree have to change under one lock.** The QUIC link
  registry wrote `ws_conns` under its lock, then the QUIC connection map
  after releasing it. A second connection registering in between saw a link
  with no QUIC record and orphaned it. The phone's different timing exposed
  it; the Mac never did. Run concurrency tests on more than one machine.

## What's next
Moving the remaining loops (TCP/UDP/WS listeners, discovery, heartbeat) into
tokio tasks with a `CancellationToken`, so dropping an engine stops *everything*
(today the blocking listeners can't be interrupted). Then `ws_conns` becomes a
tokio channel, and the bridge thread disappears.
