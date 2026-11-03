# 2026-11-03: Brand themes from the owner's kit

The owner made a brand and UI kit,
[docs/design/my-domain-brand-theme-kit.html](../../docs/design/my-domain-brand-theme-kit.html).
Both apps now follow it. The tokens, the themes and the kit's rules are in
[docs/design/theme.md](../../docs/design/theme.md).

## What changed

- **Five themes instead of one.** The kit's palettes are Cosmic Blue (the
  default), Aurora, Ocean Blue, Minimal Midnight and Cosmic Light. Each one is
  a block of CSS custom properties under `<html data-theme="...">` on desktop,
  and a `Tokens` value in `Theme.kt` on Android. The ids match across apps.
- **The theme is a setting.** Settings has a new "Appearance" tile. The choice
  is stored on the device: `localStorage` on desktop, SharedPreferences on
  Android.
- **Restyled, not restructured.** Components already read only tokens, so
  restyling meant changing tokens and a few rules: gradient primary buttons,
  12 px buttons and inputs, 18 px cards, and cyan for focus, selection and
  active navigation. No screen layout moved.

## The Node comparison

- CSS custom properties work like a JS object you swap at runtime:
  `[data-theme="light"] { --bg: ... }` re-points every `var(--bg)` at once,
  with no re-render. Setting `document.documentElement.dataset.theme` is the
  whole switch.
- On Android, `ThemeStore.current` is a Compose `mutableStateOf`. Think of it
  as a tiny observable store, like a Solid signal or a MobX observable. When
  it changes, `MdTheme` recomposes with the new `ColorScheme`, and every
  composable that reads `T.something` updates.
