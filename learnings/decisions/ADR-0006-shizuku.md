# ADR-0006 — Shizuku for privileged Android actions

- **Status:** Accepted (2026-11-01). Implemented in Phase 5, wave E.
- **Phase:** 5

## Context
Several requested features are blocked for normal Android apps:
- Wi-Fi on/off: blocked since Android 10.
- Bluetooth on/off: blocked since Android 13.
- Airplane mode, mobile data, hotspot, force-stopping apps: system-only.

## Options
1. **Root.** Can do everything, but most phones aren't rooted and rooting breaks
   banking apps and OTA updates.
2. **Device Owner mode.** Powerful, but needs a factory reset to provision. Meant
   for company-managed phones.
3. **Shizuku.** A small app the user starts once through *wireless debugging*. It
   then runs a process with ADB-shell privileges that other apps can call through
   an API. No root, no reset. It must be restarted after a reboot (or
   auto-started on rooted devices).
4. **No privileged access.** Toggles just open the matching Settings panel
   (`Settings.Panel.ACTION_WIFI` etc.) for the user to tap.

## Decision
Shizuku (owner's choice), with option 4 as the **automatic fallback** whenever
Shizuku isn't running or permission is denied. Every feature that needs it
declares tier `R` in features.md, and its capability advertisement reports
whether it's currently available, so the remote UI greys it out instead of failing.

## Consequences
- ➕ Wi-Fi, Bluetooth, airplane mode and similar work on stock phones.
- ➖ One-time setup for the user, plus re-start after reboot. The app shows a setup
  guide and a status indicator.
- ➖ Shizuku APIs can change. Calls are isolated in one Kotlin module.
