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
