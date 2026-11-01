# ADR-0003 — UniFFI for the mobile bridge, typed Tauri commands for desktop

- **Status:** Accepted (2026-11-01). Implemented in Phase 3.
- **Phase:** 3

## Context
- Android talks to the core through **31 hand-written JNI functions**
  ([apps/android/rust/src/lib.rs](../../apps/android/rust/src/lib.rs)). Everything
  crosses as JSON strings, with errors encoded as strings starting with `"ERROR: "`.
- Kotlin can't receive callbacks, so it **polls every second** for messages and
  events, and peer/connection events are dropped.
- Every new feature needs Rust JNI glue and Kotlin declarations written by hand,
  kept in sync by eye.
- iOS, watchOS and tvOS (future targets) would need the same again in Swift.
- Desktop has 34 hand-written `#[tauri::command]` wrappers. The JS calls them by
  string name, with no type checking across the boundary.

## Options
1. **Keep hand-written JNI.** Full control, but the costs above grow with each of
   the ~100 planned features, and there's no Swift path.
2. **UniFFI (Mozilla).** Describe the API once in Rust with `#[uniffi::export]`,
   and it generates idiomatic Kotlin *and* Swift: real classes, enums, typed
   errors as exceptions, `async` → Kotlin `suspend` / Swift `async`, and
   **callback interfaces** so Rust can call Kotlin. That callback support is what
   replaces polling.
3. **Diplomat / flutter_rust_bridge / cxx.** Diplomat is less mature for Kotlin;
   flutter_rust_bridge would force a Flutter UI rewrite; cxx targets C++.

For desktop: **`tauri-specta`** generates TypeScript types and typed command
functions from the Rust commands, so a renamed field breaks the TS build instead
of failing at runtime.

## Decision
UniFFI in a new `md-ffi` crate for Android (and later Apple). `tauri-specta` for
desktop.

## Consequences
- ➕ Deletes the JNI file, the 1 s polling loop, and the Kotlin duplicates of
  clipboard and reconnect logic.
- ➕ One API definition reaches Kotlin (phone, TV, Wear) and Swift (iOS, watch, tvOS).
- ➖ A build step generates bindings. CI checks the generated code is current.
- ➖ UniFFI's type system is a subset of Rust's. The FFI surface uses simple types
  and records.
