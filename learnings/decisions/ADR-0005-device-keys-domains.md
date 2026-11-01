# ADR-0005 — Per-device identity keys and domains, built before new features

- **Status:** Accepted (2026-11-01). Implemented in Phase 4.
- **Phase:** 4

## Context
Today every device of a user derives **one shared key** from a passphrase
([crates/mdcore/src/crypto.rs](../../crates/mdcore/src/crypto.rs), salt =
username). The product ([docs/features.md](../../docs/features.md)) is built
around **domains**: sharing a device with a person or group, with per-feature
flags (`off` / `ask` / `on`), expiry and an audit log. A shared key can't express
"this device may see my battery but not my SMS", or "revoke Alice only". It also
can't prove which device sent a message.

## Options
1. **Features first, retrofit permissions later.** Ships visible things sooner,
   but every feature would need reworking once identities and grants exist.
2. **Foundation first.** Per-device keys, domains, RPC with a permission check,
   and an audit log, then features on top. It breaks the current passphrase
   scheme, which is fine now because there are no users outside the owner.

## Decision
Option 2 (owner's choice). Design:
- Each install generates an **Ed25519** signing key and an **X25519** key, stored
  in the OS keystore: macOS Keychain, Android Keystore, or Secret Service on Linux.
- The device's QUIC/TLS certificate is derived from its key, and peers **pin**
  each other's keys (ADR-0002).
- A **domain** has an owner key that signs **membership certificates** for devices.
- **Grants live on the target device and are enforced there.** It never trusts the
  caller or the server. The server stores domains, members, public keys and
  invites, never message contents.
- Every RPC passes through: verify cert → look up grant → `ask` prompt if needed
  → handler → audit log.
- Pairing your own devices: QR code or short code, with a SAS (short
  authentication string) comparison.

## Consequences
- ➕ Fine-grained, revocable sharing. Peers authenticated. Replay protection
  through QUIC.
- ➕ Each feature only declares its flag; enforcement is central and covered by one
  permission test matrix.
- ➖ Existing installs must re-pair once.
- ➖ Key storage differs per OS, so a `KeyStore` capability trait is added to `Platform`.
