# 2026-11-01 — Phase 0: monorepo, workspace, CI, device lab

## What changed
| Commit | Change |
|---|---|
| `Bump server pointer; add features.md` | The umbrella repo pointed `server` at a commit two behind its `main` |
| `Remove submodules…` | Dropped the six gitlinks and `.gitmodules` |
| `Import <x> repo history into <prefix>` ×6 | `git subtree add`, history preserved (109 commits) |
| `Turn the repo into a Cargo workspace monorepo` | Root `Cargo.toml`, one `Cargo.lock`, path deps → `workspace = true`, Gradle `targetDirectory`, scripts adapted |
| `cargo fmt --all` | Formatting only (159 files had drift). Kept separate so `git blame` can skip it |
| `Add workspace CI…` | fmt, clippy `-D warnings`, nextest on Linux and macOS, Android APK, cargo-deny |
| `Add device-lab scripts` | `scripts/lab/*`: sync, linux, android, setup-arch |
| `Add learnings/` | This folder |

## Why in this order
1. **Baseline first.** Before moving anything, `cargo check` passed for core,
   server, desktop and the Android crate. If something broke later, the move
   was the cause.
2. **Secrets before history.** `deployment.dev.md` contained a live
   `JWT_SECRET`. It was redacted *before* it was ever committed, because once a
   secret is in git history, removing it means rewriting history.
3. **Formatting in its own commit.** Mixing a 159-file reformat with real changes
   makes review impossible. Put the hash in `.git-blame-ignore-revs` to hide it
   from blame.

## Rust/Cargo concepts met
- **Workspace members and `workspace = true` dependencies.** See
  [02-repo-layout.md](../02-repo-layout.md).
- **Profiles only in the root.** Android's `opt-level = "z"` became
  `[profile.release.package.mydomain_net]`.
- **Lockfile seeding.** In a workspace, member `Cargo.lock` files are ignored.
  Copying the desktop's lock (the biggest dependency graph) to the root before the
  first build kept already-tested versions, instead of letting Cargo resolve
  everything fresh to the newest versions.
- **`[workspace.lints]`.** Declares `clippy::unwrap_used = "warn"`. Members haven't
  opted in yet (`[lints] workspace = true`). That happens in Phase 1, once the
  ~80 `unwrap()`s are being removed, or CI (`-D warnings`) would fail on day one.

## Gotchas
- **Submodule `.git` can be a file, not a folder.** `core` had a `.git` *file*
  (`gitdir: ../.git/modules/core`), a relative pointer into the umbrella repo.
  Moving the folder broke the pointer. Fix: write an absolute `gitdir:` and set
  `core.worktree` in `.git/modules/core/config`. The other five had real `.git`
  folders. **Don't delete `.git/modules/core`**: the backup copy of core depends on it.
- **`deploy.sh --locked` in remote mode** synced only `apps/server/`, which no
  longer has its own `Cargo.lock`. It now syncs the whole workspace and builds
  with `-p my-domain-server`.
- **Server CI was inside the server folder** (`apps/server/.github/`), where
  GitHub ignores it. Moved to the root as `server-release.yml`.
- **`sudo` over `ssh` with a heredoc.** `ssh -t host bash -s <<EOF` can't prompt for
  a password, because stdin is the heredoc and not a terminal. The pacman step in
  `setup-arch.sh` is a separate `ssh -t` call.

## Not done yet (needs the owner)
- `ssh-copy-id pankaj@192.168.0.176` (installing an SSH key on another machine is
  an action the owner must do or explicitly allow).
- Run `scripts/lab/setup-arch.sh` (installs packages with sudo on the laptop).
- Rotate the server's `JWT_SECRET`, and change the laptop password.
- Push the `monorepo` branch and archive the six old GitHub repos.

## Commands used
```bash
git subtree add --prefix=crates/mdcore <path-to-core-repo> main
cargo check --workspace
cargo fmt --all
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked
cargo test --workspace --locked
```

## Update — device lab live (same day)
- The owner installed the SSH key and ran `setup-arch.sh`. The laptop now has JDK 17, SDK 35, NDK 30.0.14904198 and all four Rust Android targets.
- `scripts/lab/linux.sh test`: the whole workspace builds and tests pass on Linux (Manjaro).
- `scripts/lab/android.sh install`: **BUILD SUCCESSFUL in 13m 17s**. That's a cold build: the Rust core compiled for 4 ABIs, and the x86/x86_64 ones also compile C code with clang. The APK was installed on the I2405 and launched. Later builds reuse `target/` and are much faster.
- Added a live log. Every lab command appends to `~/my-domain-lab.log` on the laptop. Follow it with `tail -f` there, or with `scripts/lab/watch.sh` from the Mac.
- Gotcha: `node` on the laptop fails with `libabsl_raw_hash_set.so… not found`. That's a partial upgrade on Manjaro; `sudo pacman -Syu` fixes it. It only matters for running the desktop app on Linux.

## Update — laptop system upgrade (same day)
- `node` was broken by a **partial upgrade**: `nodejs` 26 had been upgraded on its own and needed glibc 2.44 and a newer abseil than the rest of the system had. On Arch/Manjaro, never upgrade one package alone (`pacman -S pkg`); always `pacman -Syu`.
- The first `-Syu` failed during dependency resolution (nothing was changed): `lib32-audit` was pinned to `audit=4.1.4`. The `lib32-libcap → lib32-pam → lib32-audit` chain (plus `lib32-libnsl`, `lib32-libtirpc`) had been dropped from Manjaro's repos, and nothing installed used them. Removed with `pacman -Rs` after a `--print` dry run.
- Faster downloads: Manjaro has no mirror in India. Response-time ranking (`pacman-mirrors --country …`) isn't download speed, so the candidates were measured with `curl -w %{speed_download}`. The global CDN mirrors ran at ~2.4 MB/s against 0.4 MB/s for the previous first mirror (Bangladesh). The mirrorlist is now in speed order, and `ParallelDownloads = 10`. Backups: `*.bak-mydomain` in `/etc`.
- Interrupting pacman is safe **only during the download phase**. Downloads resume from the package cache; killing it mid-install can leave a broken system.
- The new kernel 6.12.112 needed `linux612-headers` so the `openrazer-driver` DKMS module could build.
- Left for the owner: `/etc/default/grub.pacnew` and `/etc/lightdm/lightdm.conf.pacnew` (new default configs to merge by hand), and a **reboot** into the new kernel.
