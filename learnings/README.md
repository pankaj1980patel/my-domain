# learnings/

This folder explains **why** my-domain is built the way it is. You should be
able to read it start to finish and understand every folder, file and decision
as if you had written the project by hand.

It is written for a developer who knows **Node.js/TypeScript** well and is new
to **Rust**. Every Rust idea is shown next to its closest Node equivalent.

## How to read it

| Order | File | What you get | Status |
|---|---|---|---|
| 1 | [00-node-to-rust.md](00-node-to-rust.md) | The Rust you need for this repo, mapped to Node | ✅ |
| 2 | [01-architecture.md](01-architecture.md) | How the app works today, end to end, with one message traced through the code | ✅ |
| 3 | [02-repo-layout.md](02-repo-layout.md) | Why each folder exists and what goes where | ✅ |
| 4 | [12-build-ci-release.md](12-build-ci-release.md) | Cargo workspace, CI jobs, device lab, deploy | ✅ |
| 5 | [glossary.md](glossary.md) | Every term (NAT, STUN, FFI, crate, …) in one place | ✅ |
| 7 | [03-networking.md](03-networking.md) | NAT, STUN, hole punching, QUIC, the connect ladder | ✅ |
| – | 04-crypto-identity.md | Device keys, certificates, pinning, replay protection | Phase 4 |
| – | 05-domains-permissions.md | Domains, grants, flags, the `ask` flow, audit log | Phase 4 |
| 8 | [06-async-tokio.md](06-async-tokio.md) | tokio vs the Node event loop; our sync/async bridge | ✅ (growing) |
| – | 07-ffi-bridges.md | UniFFI, Tauri commands, why hand-written JNI was removed | Phase 3 |
| 6 | [08-testing.md](08-testing.md) | Test pyramid, md-testkit, running tests on the phone, writing good tests | ✅ |
| – | 09-android.md | Services, permission tiers, Accessibility, MediaProjection, Shizuku | Phase 3/5 |
| – | 10-desktop.md | Tauri, SolidJS, IPC, OS integrations | Phase 3 |
| – | 11-server.md | axum, MongoDB schema, JWT, FCM | Phase 4 |

## Two kinds of record

- **[decisions/](decisions/)**: *Architecture Decision Records* (ADRs). One file
  per big decision, covering the context, the options considered, what was
  chosen and the consequences. ADRs are never edited after the fact. If a
  decision changes, a new ADR supersedes the old one. This is how you find out
  *why* something is the way it is, years later.
- **[journal/](journal/)**: one entry per chunk of work, written as the work
  happens. Each entry covers what changed, why, which Rust concepts showed up,
  what went wrong, and the exact commands used. Reading the journal in order
  replays the project's history.

## Conventions

- Links point at real files, e.g. [crates/mdcore/src/engine.rs](../crates/mdcore/src/engine.rs).
- "Node:" blocks show the closest JavaScript/TypeScript equivalent.
- "Gotcha:" blocks record something that surprised us or broke.
