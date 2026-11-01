# ADR-0001 — Merge the six submodule repos into one monorepo

- **Status:** Accepted (2026-11-01)
- **Phase:** 0

## Context
The project was an umbrella repo with six git submodules: core, server,
desktop, android, ios, mac. Most changes touch the core and at least one app,
so each change needed 2–3 commits in different repos plus a "bump pointer"
commit in the umbrella repo. Pointers drifted: `server` was two commits behind.
There was CI for the server only. The desktop and Android crates already pointed
at the core with a relative `path = "../../core"`, so they only built when
checked out inside the umbrella repo anyway.

## Options
1. **Keep submodules.** No migration cost. The ceremony, drift and per-repo CI stay.
2. **Monorepo with history preserved (`git subtree add`).** Every repo's commits
   are re-parented under a prefix (`apps/server`, …). One commit can change the
   core and both apps atomically. One CI.
3. **Monorepo, fresh history (copy files).** Simpler, but loses `git log` / `blame`
   for 109 commits.

## Decision
Option 2. Layout: `crates/` for libraries, `apps/` for programs, one root Cargo
workspace (see [02-repo-layout.md](../02-repo-layout.md)).

How it was done:
```bash
git checkout -b monorepo
# move the submodule working copies aside (kept as a backup outside the repo)
git rm --cached core server desktop android ios mac && git rm .gitmodules
git subtree add --prefix=crates/mdcore <backup>/core main
git subtree add --prefix=apps/server   <backup>/server main
# … desktop, android, apple/ios, apple/mac
```
Untracked local files (`.env`, `fcm-sa.json`, `deploy.sh`, Docker build files,
`google-services-android.json`) were then copied into the new paths. Secrets stay
git-ignored.

## Consequences
- ➕ One PR = one feature, end to end. One lockfile, one CI, one build cache.
- ➕ `git log -- crates/mdcore` still shows the original core history.
- ➖ The old GitHub repos need archiving, with a README pointing here. **That needs
  the owner's approval and hasn't been done yet.**
- ➖ Workspace rules apply: profiles only in the root, and a shared `target/`
  (Gradle plugin reconfigured).
- The local backup of the original working copies is at
  `../my-domain-local-backup/` (outside the repo). Delete it once you're happy.
