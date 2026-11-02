# 02 — Repo layout: why each folder exists

## Then → Now

**Then (until 2026-11-01):** an umbrella repo with six **git submodules** (core,
server, desktop, android, ios, mac), each a separate GitHub repo. Changing one
feature meant committing in `core`, then in `desktop` and `android`, then
committing again in the umbrella repo to "bump the pointers". Pointers went
stale; the `server` one was two commits behind when we started.

**Now:** one repo, one Cargo **workspace**. The six repos were imported with
`git subtree add`, which keeps all 109 commits of history: `git log
crates/mdcore` still shows the core's original commits. See
[decisions/ADR-0001-monorepo.md](decisions/ADR-0001-monorepo.md).

## Current tree

```
my-domain/
├── Cargo.toml          workspace root: member list, shared dependency versions, lints, profiles
├── Cargo.lock          ONE lockfile for every Rust crate
├── deny.toml           cargo-deny rules (vulnerable/unknown dependencies)
├── crates/             reusable Rust libraries (no UI, no platform code)
│   └── mdcore/         today's shared core; split into md-proto/md-crypto/md-net/md-core… in Phases 1–2
├── apps/               things you ship or run
│   ├── server/         registry + signaling HTTP server (axum, MongoDB)
│   ├── desktop/        Tauri app: src/ = web UI, src-tauri/ = Rust side
│   ├── android/        Gradle project: app/ = Kotlin, rust/ = the JNI crate
│   └── apple/          ios/ and mac/ placeholders (future Swift apps via UniFFI)
├── docs/               product and protocol docs: features.md, SIGNALING.md
├── learnings/          you are here
├── scripts/lab/        real-device testing over SSH/adb (see scripts/lab/README.md)
└── .github/            CI workflows + a reusable "install Linux deps" action
```

## Why `crates/` vs `apps/`

This is a common Rust monorepo convention, and it matches the Node one of
`packages/` vs `apps/`:
- **`crates/`** holds libraries other code depends on. They must not depend on
  any app, and they don't know about Tauri, Android or MongoDB. That rule is what
  keeps the core portable to TV, watch and iOS later.
- **`apps/`** holds leaf programs that depend on crates. Nothing depends on an app.

Dependency direction is always `apps → crates`, never the reverse.

## Why one workspace

| Benefit | Node equivalent |
|---|---|
| One `Cargo.lock`, so every crate uses the same `serde`/`tokio` version | a single pnpm lockfile |
| One `target/` build cache: a dependency built for the server is reused by the desktop | shared `node_modules` store |
| `cargo test --workspace` tests everything in one command | `pnpm -r test` |
| `[workspace.dependencies]`: declare a version once, members use `{ workspace = true }` | pnpm `catalog:` |
| `[workspace.lints]`: one lint policy | a root `.eslintrc` |

**Gotcha — profiles move to the root.** Cargo ignores `[profile.*]` sections in
member crates. Android's size-optimised release profile (`opt-level = "z"`,
`lto = true`) used to live in `apps/android/rust/Cargo.toml`. It's now a
*per-package override* in the root `Cargo.toml`:
```toml
[profile.release.package.mydomain_net]
opt-level = "z"
```
Per-package overrides can't set `lto`, so the Android `.so` is temporarily a
little bigger. Phase 3 adds a dedicated `release-android` profile when the
Gradle build is reworked for UniFFI.

**Gotcha — the Gradle Rust plugin and the shared `target/`.** The Android Gradle
plugin used to find the built `.so` in `apps/android/rust/target`. In a
workspace Cargo writes to `<repo>/target`, so
[apps/android/app/build.gradle.kts](../apps/android/app/build.gradle.kts) now sets
`targetDirectory = "${rootDir}/../../target"`.

## Files that are deliberately NOT committed

Listed in the root [.gitignore](../.gitignore):
- `.env`, `fcm-sa.json`: server secrets (Mongo URI, JWT secret, Firebase service account).
- `google-services*.json`: Firebase client config.
- `target/`, `node_modules/`: build output.

`apps/server/deployment.dev.md` was untracked because it contained the real
`JWT_SECRET`. The secret was replaced with a placeholder before the file was
committed. **The real secret must be rotated on the server**, because it sat in
plain text on disk and was shared in a conversation.

## Where new things go

| You're adding… | Put it in |
|---|---|
| A wire type / RPC message | `crates/md-proto` (Phase 1+) |
| A feature (battery, media, …) | `crates/md-features/<feature>/` + platform impls in each app |
| A Kotlin screen | `apps/android/app-phone/…` (after Phase 3's module split) |
| A desktop screen | `apps/desktop/ui/src/screens/…` (SolidJS) |
| A server route | `apps/server/src/` |
| A real-device test | `scripts/lab/scenarios/` |
| An explanation of a decision | `learnings/decisions/ADR-XXXX-*.md` |
