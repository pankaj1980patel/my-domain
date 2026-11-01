# 2026-11-01 — Phase 1, part 1: tests first, then security fixes

## What changed
| Commit | Change |
|---|---|
| `Add md-testkit and 2-node behaviour tests` | New crate `crates/md-testkit`; 10 integration tests; `Engine::upsert_peer`; `Debug` derives |
| `mdcore unit tests…` | 23 unit tests; two "known gap" tests recording bugs |
| `security: cap punch reassembly…; require matching STUN txid` | Fixes 1 and 2 below |
| `refactor: Engine holds one NetCtx…` | Removed the copy of 16 `Arc` fields (behaviour unchanged, tests green) |
| `security: rate-limit undecryptable-message notices` | Fix 3 |
| `security: harden the TCP receiver` | Fix 4 |
| `security: LAN beacons can't re-point registry peers; WebRTC won't clobber…` | Fixes 5, 6, 7 |
| `security: cap LAN-learned peers at 256` | Fix 8 |

## The fixes, and what each one teaches

1. **Reassembly preallocation.** A punch datagram declares how many chunks its
   message has, and the code allocated that many slots *before* receiving them.
   One 12-byte packet saying "65535 chunks" allocated 65535 slots, times 256
   message ids. **Lesson:** never size an allocation from an untrusted number
   without a cap. Rust prevents memory *unsafety*, not memory *exhaustion*.
2. **STUN transaction id ignored.** Any datagram shaped like a STUN success
   response was accepted as "our public address". Now it must echo our random
   12-byte id, and stray packets on the shared socket are skipped until the real
   answer arrives (tested with a fake STUN server thread on loopback).
3. **Undecryptable spam.** Each junk envelope became a chat entry. Dropping them
   silently would hide a real problem (a mistyped passphrase), so now there's one
   notice per sender IP per minute, and every failure is counted
   (`Engine::decrypt_failures()`).
4. **TCP receiver.** It used `read_to_end` with no limit, no timeout, and a thread
   per connection. Now: a 4 MiB cap via `.take(MAX + 1)` (reading one byte past
   the cap is how you *detect* "too big"), a 10 s read timeout, at most 128
   connections being read at once, and a back-off on accept errors.
5. **LAN beacon hijack.** An unauthenticated beacon could re-point a
   registry-learned peer to the attacker's IP. The rule now lives in a pure
   function, `merge_beacon`, and is unit-tested.
6. **WebRTC replaced live links.** It now only registers if the slot is free, the
   same rule the punch path already followed. *Not yet covered by a test:* it
   needs the signaling seam.
7. **Unencoded query parameter.** `?exclude=` is now built with ureq's `.query()`.
8. **Unbounded peer map.** LAN-learned peers are capped at 256.

## Rust concepts met

- **`Condvar`** in `RecordingSink`: wait without sleeping in a loop. Node analogy:
  awaiting a Promise that `emit` resolves.
- **`HashMap::entry(...)` + `Entry::Vacant`**: "insert only if absent" in one
  lookup while holding the lock (rtc fix). It avoids the
  check-then-insert race you'd get from `contains_key` followed by `insert`.
- **`bool::then`**: `merge_beacon(..).then(|| snapshot)` returns `Some(snapshot)`
  only when something changed.
- **`Read::take(n)`**: wraps a reader so it stops after `n` bytes, like piping a
  Node stream through a byte-limit transform.
- **`AtomicUsize::fetch_add` / `fetch_sub`**: a lock-free counter for in-flight
  connections. `AcqRel` ordering pairs the increment with the decrement across threads.
- **Dev-dependency cycles**: `mdcore` ⇄ `md-testkit` (see [08-testing.md](../08-testing.md)).

## Gotchas
- **Perl `s|…|…|` with `\|` inside.** Escaping the delimiter turned it into regex
  *alternation*, which matched an empty string and pasted the replacement at the
  top of `punch.rs`. Restored from git and redone with an exact-match editor.
  Lesson: for multi-line code edits, use exact string replacement, not regex.
- **rustfmt splits method chains** (`self` on one line, `.peers` on the next), so a
  regex looking for `self.peers` misses them. Match `self\s+\.peers`.
- **The NDK needs `CC_<target>` as well as the linker** for crates that compile C.
- **Sourcing a bash helper from zsh** prints a harmless `BASH_SOURCE` warning, and
  a `|` inside a quoted remote command got split by the remote shell. Grep on the
  Mac side instead.

## Verified
`cargo test -p mdcore`: 32 unit + 14 integration = **46 tests**, green on the
Mac, the Manjaro laptop, and the I2405 phone (Android 16, ARM64).

## Next (Phase 1, part 2)
Typed errors (`thiserror`), `parking_lot` locks, a `Transport` enum, `CoreConfig`,
splitting `engine.rs`, `Serialize` on `CoreEvent` (delete both apps'
hand-written JSON), the key-value store in core, and `Registry`/`Signaler`
traits with in-memory test doubles. That unlocks tests for the connect ladder,
punch and WebRTC.
