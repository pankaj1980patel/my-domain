# 08 — Testing

## The idea: pin behaviour first, then change code

Phase 1's rule: **before refactoring, write tests that describe what the code
does today**. These are *characterization tests*. They don't say the behaviour
is right, only that it's what currently happens. Then:
- a refactor that keeps every test green didn't change behaviour;
- a deliberate fix flips a test from "documents the bug" to "asserts the fix",
  and the commit says so.

You'll see that pattern in the history. For example
`known_gap_claimed_chunk_count_is_preallocated` became
`oversized_chunk_counts_are_dropped` in the commit that fixed it.

## The pyramid in this repo

```
            ▲   device lab: same tests on the real phone (scripts/lab/android.sh coretest)
           ▲▲▲  integration: 2+ real engines on loopback (crates/mdcore/tests/*.rs)
        ▲▲▲▲▲▲▲ unit: pure functions, in the same file (#[cfg(test)] mod tests)
```

| Layer | Where | Speed | What it proves |
|---|---|---|---|
| Unit | bottom of each `.rs` file | ms | One function's logic: STUN parsing, chunk reassembly, `merge_beacon`, crypto |
| Integration | `crates/mdcore/tests/` | seconds | Whole engines talking: real sockets, threads, crypto |
| Device | the phone, via adb | ~10 s + build | Same binaries behave the same on Android/ARM64 |

**Node analogy:** unit = a jest test for a pure helper; integration = supertest
against a real server on an ephemeral port; device = running the suite inside
the real deployment environment.

## Running tests

```bash
cargo test -p mdcore                     # all core tests (unit + integration)
cargo test -p mdcore --lib               # unit tests only
cargo test -p mdcore --test messaging    # one integration file
cargo test -p mdcore stun                # only tests whose name contains "stun"
cargo test -p mdcore -- --nocapture      # show println! output

scripts/lab/linux.sh test                # whole workspace on the Arch laptop
scripts/lab/android.sh coretest          # core tests ON the phone
scripts/lab/android.sh coretest flood    # ...filtered by name
```

## `md-testkit`: how engines become testable

[crates/md-testkit/src/lib.rs](../crates/md-testkit/src/lib.rs)

The engine is generic over `Platform` and reports through `EventSink`
(see [00-node-to-rust.md](00-node-to-rust.md) §6). So a test supplies:

- **`MockPlatform`**: device id/name, everything on `127.0.0.1`, an in-memory
  key-value store, an in-memory clipboard (`MemClipboard`), and a
  `RecordingNotifier`.
- **`RecordingSink`**: stores every `CoreEvent`. `wait_until(timeout, pick)`
  blocks until an event matches. Under the hood it uses a **`Condvar`**: the
  engine thread calls `notify_all()` after pushing an event, and the waiting test
  thread wakes and re-checks. No `sleep` loops, so tests are as fast as the code.
- **`TestNode`**: starts an engine, logs in offline with a shared passphrase, and
  keeps handles to the sink, clipboard and notifier. `TestNode::pair(&a, &b)`
  makes them know each other, as discovery would.

```rust
let (a, b) = pair("chat-udp");                       // two engines, paired
let used = a.engine.send(&b.node_id(), "hi").unwrap();
assert_eq!(used, "UDP");
let msg = b.sink.wait_for_message("hi");              // blocks ≤ 5 s
assert!(msg.ok);
```

### Why tests can run in parallel
- Every engine binds **ephemeral ports** (`0.0.0.0:0`: the OS picks a free one).
- The discovery socket uses `SO_REUSEPORT`, so many engines share 45678.
- Each test uses unique node ids (`format!("{test}-a")`).

### Why `md-testkit` is its own crate
It depends on `mdcore` (it implements `mdcore::Platform`), and `mdcore`'s tests
depend on it, as a **dev-dependency**. Cargo allows that cycle for
integration tests (`tests/*.rs`), which see `mdcore` as an external crate, the
same way testkit does. Later, `md-net`, `md-core` and the apps reuse it too.

**Gotcha:** unit tests *inside* `mdcore` (`#[cfg(test)] mod tests`) can't use
`md-testkit`. There, `mdcore` is compiled a second time, in test mode, and its
types count as different from the ones testkit sees. Engine-level tests
therefore live in `tests/`.

## Running the tests on the phone

[scripts/lab/remote/android-coretest.sh](../scripts/lab/remote/android-coretest.sh):
1. `cargo test --target aarch64-linux-android --no-run --message-format=json`
   compiles the test binaries without running them. `--message-format=json`
   lets us find their paths.
2. The linker and C compiler come from the NDK
   (`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`, `CC_aarch64_linux_android`).
   **Gotcha:** setting only the linker isn't enough. Crates with C code (built
   through the `cc` crate) failed with `failed to find tool
   "aarch64-linux-android-clang"` until `CC_<target>` was set too.
3. `adb push` each binary to `/data/local/tmp`, the only place `adb shell` may
   execute files, run it, then delete it.

These are the same tests and the same assertions, running on ARM64 Android 16.
They catch platform differences, like socket behaviour, that a Mac-only CI would miss.

## Writing a good test here

- **Name it as a sentence** that states the behaviour:
  `beacon_cannot_repoint_a_registry_peer_to_another_ip`.
- **Regression tests explain the old bug** in a comment, so nobody "simplifies"
  the fix away later.
- **Prefer a pure function plus a unit test** over a thread plus a sleep. The LAN
  trust rule was pulled out of the receive loop into `merge_beacon(&mut map,
  &beacon, src) -> bool` precisely so it could be tested in microseconds.
- **Assert the negative too.** `assert_no_message("secret", …)` proves plaintext
  never leaks, not just that *something* arrived.
- **Test through real sockets when the bug lives there.** The TCP size-cap test
  pads a *valid* envelope with 4 MiB of JSON whitespace. Before the fix that was
  delivered (serde accepts trailing whitespace); after, it's rejected. That
  proves the cap, not the JSON parser, did the rejecting.

## Testing the connect ladder: `SignalHub` + `ConnectRungs`

Rungs 3–5 need a *signal* to reach the other device. In production that's an
HTTP POST to the server, which forwards it as an FCM push. In tests:

- **`SignalHub`** (md-testkit) implements the core's `Signaler` trait. `send(from,
  to, signal)` looks up the target engine and calls its `on_signal` **on a new
  thread**, like a push arriving asynchronously. It also logs each signal type,
  so a test can assert the exact handshake: `["punch_offer", "punch_answer"]`.
- **`CoreConfig::rungs`** enables one rung at a time. Two engines on `127.0.0.1`
  are always "on the same LAN", so without this, rung 1 would win every time and
  the others would never run.
- **`test_config()`** sets `stun_servers: vec![]`, so tests never call Google's
  STUN server. Loopback host candidates are enough.

The hub holds **`Weak`** references to engines. Each engine holds the hub (as
its signaler), so strong `Arc`s both ways would form a reference cycle that is
never freed. Node has a GC that collects cycles; Rust's `Arc` doesn't.

```rust
let (a, b, hub) = pair_on_hub("rung-punch", ConnectRungs { punch: true, ..ConnectRungs::none() });
a.engine.connect(&b.node_id()).unwrap();
assert_eq!(a.wait_connected(&b, LADDER_WAIT).as_deref(), Some("punch"));
assert_eq!(hub.kinds(), vec!["punch_offer", "punch_answer"]);
```

`..ConnectRungs::none()` is **struct update syntax**: "every other field from
this value". It's like `{ ...ConnectRungs.none(), punch: true }` in JS.

## Current inventory (Phase 1, 2026-11-01)

| File | Tests | Covers |
|---|---|---|
| `crypto.rs` | 7 | round trip, key derivation, wrong key, nonce freshness, tampering, malformed input |
| `error.rs` | 2 | error text unchanged for the UIs; serializes as its message |
| `events.rs` | 4 | **JSON contract** with desktop JS and Android Kotlin |
| `nat/stun.rs` | 6 | address parsing, rejects junk, transaction-id check, fake STUN server |
| `transport/punch.rs` | 11 | chunking, reorder, duplicates, loss, 1 MiB cap (both directions), reassembly cap |
| `transport/mod.rs` | 1 | transport choice (`best_proto`) |
| `discovery.rs` | 8 | LAN targets, beacon trust rules, LAN peer cap |
| `wire/frame.rs` | 1 | v2 frame round trip |
| `tests/messaging.rs` | 16 | chat over UDP/TCP/WS, typed errors, wrong key + flood, clipboard, notifications, TCP hardening |
| `tests/connect_ladder.rs` | 6 | each rung alone: LAN UDP, signaled WS, **hole punch**, **WebRTC**, all-off, unknown peer |

**60 tests, green on macOS, Linux (Manjaro) and Android 16 (ARM64).** The ladder
suite passed 10 consecutive runs (flakiness check).

Not yet covered:
- The WebRTC "lost race" path (a session opening while another link owns the
  slot). It needs a hand-crafted SDP offer.
- Real NAT traversal through actual NAT routers: Linux network namespaces in Phase 2.
- Login, registry refresh and firewall checks (HTTP to the server). A `Registry`
  trait comes with the server work.
