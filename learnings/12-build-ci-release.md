# 12 — Build, CI, device lab, release

## Building locally (Mac)

| Goal | Command |
|---|---|
| Type-check everything (fast) | `cargo check --workspace` |
| Run all Rust tests | `cargo test --workspace` |
| Lint exactly like CI | `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` |
| Format | `cargo fmt --all` |
| Run the server | `cargo run -p my-domain-server` (needs `apps/server/.env`) |
| Run the desktop app | `cd apps/desktop && npm install && npm run tauri dev` |
| Build only one crate | `cargo build -p mdcore` (`-p` = package, like `pnpm --filter`) |

`--workspace` means "every member". Without it, cargo acts on the crate in your
current directory.

## CI: [.github/workflows/ci.yml](../.github/workflows/ci.yml)

Runs on every push to `main` and every PR. Four jobs, in parallel:

| Job | What | Why |
|---|---|---|
| `lint` | `cargo fmt --check`, `cargo clippy` with `-D warnings` | Formatting is never discussed in review; warnings never pile up |
| `test` (ubuntu + macos) | `cargo nextest run --workspace` + doctests | Primary targets are Linux and macOS, so both run every test |
| `android` | Installs JDK 17, Android SDK + the NDK pinned in `build.gradle.kts`, Rust Android targets, then `./gradlew assembleDebug` | Proves the APK, including the Rust `.so` for 4 ABIs, still builds |
| `deny` | `cargo deny check advisories bans sources` | Fails on crates with known vulnerabilities, or crates from unknown registries/git |

**Why `-D warnings` only in CI:** locally, warnings shouldn't stop you from
running code mid-refactor. In CI, a warning blocks the merge.

**Why nextest:** it runs each test in its own process (one hanging network test
can't block the others), runs them in parallel, and gives clearer output. Our
tests will open real sockets, so isolation matters.

**[.github/actions/linux-deps](../.github/actions/linux-deps/action.yml):** the
desktop crate links GTK/WebKit (Tauri) and ALSA (pulled in by the FCM receiver
library). Even `cargo clippy --workspace` compiles it, so every Linux job
installs these first. A *composite action* is GitHub's way to reuse steps,
like a shared npm script.

## Server release: [.github/workflows/server-release.yml](../.github/workflows/server-release.yml)

Builds the server's Linux x86_64 binary when server code changes on `main` (or a
`v*` tag), and uploads it as an artifact. [apps/server/deploy.sh](../apps/server/deploy.sh)
can download that artifact (`BIN_SOURCE=gh`), cross-compile, or build on the VM,
then installs it under systemd on the Oracle VM.

## Device lab: [scripts/lab/](../scripts/lab/README.md)

```
Mac (dev, macOS node) ──ssh/rsync──▶ Arch laptop (Linux node + Android build host) ──USB──▶ phone
```
- The Mac has no Android NDK and isn't the place to build Android. The Arch
  laptop is, and the phone is plugged into it. So `android.sh` syncs the working
  tree to the laptop, runs Gradle there, and runs `adb` there too.
- `sync.sh` uses `rsync`, not `git push/pull`, so **uncommitted** changes get
  tested on real devices before you commit.
- `setup-arch.sh` installs the toolchain once: JDK 17 (the Android Gradle plugin
  in this project doesn't support the laptop's default JDK 26), Android SDK and
  NDK, and the Rust Android targets.

Probe results for the laptop (2026-11-01): Manjaro, x86_64, 12 cores, 15 GB RAM.
Rust and `adb` were present, and the phone I2405 was attached and authorized. The
Android SDK/NDK were missing.
