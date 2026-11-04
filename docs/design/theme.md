# Themes and design tokens

The reference is the owner's brand kit,
[`my-domain-brand-theme-kit.html`](my-domain-brand-theme-kit.html). It is one
self-contained page; open it in a browser. Where this doc and the kit
disagree, the kit wins.

Both apps use the same tokens and the same five themes:

- Desktop: `apps/desktop/ui/src/styles/tokens.css`
- Android: `apps/android/app/src/main/java/com/mydomain/android/ui/Theme.kt`

A theme's id is the same in both apps: `cosmic-blue`, `aurora`, `ocean`,
`midnight` or `light`.

## The five themes

The kit has five coordinated palettes. Cosmic Blue is the default. The kit
recommends offering Minimal Midnight to people who want less glow, Aurora as
the expressive option, and Cosmic Light as the light mode. Ocean Blue is in the
kit as well, so both apps offer it too.

| Theme | id | Character | Canvas |
|---|---|---|---|
| Cosmic Blue (default) | `cosmic-blue` | Dark-first, vivid and trustworthy | Star field, navy veil, blue glow |
| Aurora Purple | `aurora` | Creative, premium and atmospheric | Star field, violet glow |
| Ocean Blue | `ocean` | Calm and connected | Star field, aqua glow |
| Minimal Midnight | `midnight` | Quiet and professional, with less glow | Flat near-black: no star field, no glow, no blur |
| Cosmic Light | `light` | Soft, airy and accessible | Flat cloud white with a faint cyan glow |

## Tokens

Each token is one role. Components use roles, never raw colours, so changing
the theme changes every screen. **Bold** values are copied from the kit as it
is. The other values are derived from it: a translucent version of a kit
colour for glass, a darker shade for text on light backgrounds, or a value
filled in where the kit's palette has no colour for that role.

| Role | CSS var | Compose `T.` | Cosmic Blue | Aurora | Ocean | Midnight | Light |
|---|---|---|---|---|---|---|---|
| Background | `--bg` | `bg` | **#050B18** | **#090817** | **#061321** | **#080B12** | **#F4F8FF** |
| Surface (panels) | `--surface` | `surface` | **#0B1630** at 72% | **#17152E** at 72% | **#0D2439** at 72% | **#111827** | **#FFFFFF** at 86% |
| Raised surface | `--surface-2` | `surface2` | **#101D35** at 86% | #201D3D | #12304A | **#1C2738** | #E7F0FF |
| Border | `--border` | `border` | **#203454** | #2E2A52 | #1D3D5A | **#29364A** | **#DCE8F7** |
| Control edge | `--control-border` | `controlBorder` | #5D7598 | #6F679E | #4F7A9C | #64758F | #73859E |
| Text | `--text` | `text` | **#F4F8FF** | **#F7F4FF** | **#F2FAFF** | **#F8FAFC** | **#10213D** |
| Secondary text | `--text-2` | `text2` | **#C6D5EA** | #D6D0EA | #C4DBE8 | #CBD5E1 | #33476A |
| Muted text | `--text-3` | `text3` | **#94A3B8** | **#A8A2C2** | **#8AA8BC** | **#94A3B8** | #56677F |
| Primary | `--primary` | `primary` | **#1455D9** | **#713BDB** | **#0759A6** | **#2563EB** | **#1769E8** |
| Primary gradient | `--gradient` | `brush` | **Cosmic Flow** | **Aurora** | #0759A6, #0A8FC4, #00B8D9 | flat #2563EB | #1769E8, #38BDF8, #713BDB |
| Primary scrim | `--primary-scrim` | `primaryScrim` | #050B18 at 40% | #050B18 at 32% | #050B18 at 34% | none | #050B18 at 38% |
| Accent (interaction) | `--accent` | `accent` | **#22C7F2** | **#42D9F5** | **#00B8D9** | **#38BDF8** | #0284C7 |
| Accent as text | `--accent-fg` | `accentFg` | #22C7F2 | #42D9F5 | #00B8D9 | #38BDF8 | #0369A1 |
| Secondary accent | `--accent-soft` | `accentSoft` | **#64BFFF** | **#9B75FF** | **#A5E8FF** | #38BDF8 | #38BDF8 |
| Highlight | `--highlight` | `highlight` | **#FFC857** | **#E879F9** | #FFC857 | #FFC857 | **#F5B942** |
| Highlight as text (tagline) | `--highlight-fg` | `highlightFg` | #FFC857 | #E879F9 | #FFC857 | #FFC857 | #8A5900 |
| Success | `--success` | `success` | **#34D399** | #34D399 | **#55E6C1** | #34D399 | #047857 |
| Warning | `--warning` | `warning` | **#FFC857** | #FFC857 | #FFC857 | #FFC857 | #9A4508 |
| Danger | `--danger` | `danger` | **#FB7185** | #FB7185 | #FB7185 | #FB7185 | #BE123C |

Cosmic Light differs from the kit in a few places, for contrast (WCAG 2.2
AA, measured in [`docs/accessibility.md`](../accessibility.md)):

- Muted text is #56677F, not the kit's #61738D, so it reaches 4.5:1 on the
  raised surface.
- The accent is #0284C7, not the kit's #38BDF8. Switches, the battery line
  and selection rings need 3:1 against white, and #38BDF8 is only 2.1:1. The
  kit's #38BDF8 stays in the gradient, the glow and `accentSoft`.
- Success, warning, danger and the tagline gold are darker than before
  (#059669, #B45309, #E11D48 and #9A6400 were just under 4.5:1 on the raised
  surface).

The two newer tokens:

- **Control edge** (`--control-border`, `controlBorder`): the edge of text
  fields and of an off switch, 3:1 on every surface. Card and panel borders
  are decorative and keep `--border`, so the glass look doesn't change. On
  Android it is also Material's `outline`.
- **Primary scrim** (`--primary-scrim`, `primaryScrim`): a thin navy veil
  laid over the gradient on primary buttons only, so their white label
  keeps 4.5:1 at the gradient's lightest stop. Progress bars keep the plain
  gradient. Minimal Midnight's flat blue doesn't need one.

Supporting tokens:

- `--on-primary`, `onPrimary`: white, on primary and gradient fills.
- `--on-accent`, `onAccent`: dark navy (#041321), on cyan fills such as
  badges. The kit's chips use this pairing.
- `--accent-bg`, `accentBg`: a faint wash of the accent, behind selected items.
- `--border-strong`: a stronger hairline for switches and secondary buttons.
- `--btn-secondary-bg` / `--btn-secondary-border`: the kit's secondary button
  (#152744 / #2A456A in Cosmic Blue).
- `--surface-solid`, `bar`: an opaque panel for bars, dialogs and banners.
- `*-soft` status colours: tinted backgrounds for status and error boxes.
- `--toast-bg` / `--toast-fg`: the kit's toast (#EAF7FF on #071525).

### Gradients (kit)

| Name | Value | Use |
|---|---|---|
| Cosmic Flow | `linear-gradient(120deg, #1455D9, #22C7F2 55%, #713BDB)` | Primary buttons, progress (Cosmic Blue) |
| Stellar Gold | `linear-gradient(110deg, #FF9F43, #FFC857, #FFF0A6)` | Rare celebratory moments (not used yet) |
| Deep Space | `linear-gradient(130deg, #050B18, #0B1630, #183B69)` | Hero surfaces (not used yet) |
| Aurora | `linear-gradient(115deg, #312E81, #713BDB, #E879F9)` | Primary buttons (Aurora theme) |

### Type

- Font: **Inter**, bundled in both apps so it works offline (SIL Open Font
  License 1.1).
  - Desktop: the `@fontsource-variable/inter` npm package, imported once in
    `src/index.tsx` (`--font: "Inter Variable", Inter, ui-sans-serif, ...`).
    Licence: `apps/desktop/ui/public/licenses/Inter-OFL.txt`.
  - Android: static TTFs (400, 500, 600, 700, 800) from the npm package
    `inter-font` 3.19.0, in `res/font/inter_*.ttf`, used by every style in
    the Material `Typography`. Licence:
    `apps/android/app/src/main/assets/licenses/Inter-OFL.txt`. It can't sit
    next to the fonts, because `res/font` only accepts font files.
- Body: 15 px with 1.55 line height.
- Titles: extra-bold (800). Page titles are 26 px on desktop and 24 sp on
  Android, with slightly tight letter spacing.
- Eyebrow labels: 12 px, weight 800, uppercase, 0.16 em letter spacing, in
  the accent colour. The kit uses these above hero titles.
- Code and IDs: `ui-monospace, SFMono-Regular, Menlo, monospace`.

### Shape and elevation

| Token | Value | Use |
|---|---|---|
| `--r-md` / `shapes.small` | 12 | Buttons, inputs, nav items, inner panels |
| `--r-lg` / `shapes.medium` | 18 | Cards and tiles (the kit's `--radius`) |
| `--r-xl` / `shapes.extraLarge` | 28 | Hero surfaces: the sign-in card |
| `--r-full` | 999 | Chips, pills, switches, badges |
| `--r-logo` | 22% | The logo's rounded-square mask |
| `--shadow-3` | `0 18px 50px #0008` | Floating surfaces: sign-in card, toasts, banners |
| `--shadow-logo` | `0 8px 35px #0008` | The logo |

## Components

- **Primary button**: filled with the theme gradient under the primary
  scrim, white label, extra-bold, 12 px corners. Use one per view, for the
  main action.
- **Secondary button**: a deep panel fill with a hairline border.
- **Ghost button**: transparent with a border. The border turns to the
  accent on hover.
- **Card / tile**: glass surface, 1 px border, 18 px corners, no shadow.
  A clickable card's border turns to the accent on hover.
- **Input**: raised surface, a control-edge border and 12 px corners. On
  focus, the border turns to the focus colour (desktop: a 2 px `--focus`
  ring; Android: Material's 2 dp accent border).
- **Switch**: accent track when on; a control-edge outline when off.
- **Chip / badge**: fully round. An accent fill takes dark navy text.
- **Navigation**: the active item shows a faint accent wash and accent text.
- **Status dot**: green when connected, with a soft glow. The glow is off in
  Minimal Midnight.
- **Focus ring**: 2 px solid accent, offset 3 px.

## Rules from the kit, in plain words

- Use deep navy for the main background, never pure black.
- Use cyan (the accent) for focus, selection and active navigation.
- Save gold (the highlight) for important moments and small accents. Never use
  it as a large fill.
- Status colours: green means connected or healthy, amber means warning, red
  means error.
- Use gradients sparingly: primary actions, hero surfaces and emphasis only.
- Text on dark surfaces is near-white. Text on light surfaces is dark navy.
- Never put small text directly on bright cyan or gold. Badges on cyan use
  dark navy text.
- Check contrast on every real component before shipping.
- Logo: the galaxy image, used as it is. Do not recolour, crop or redraw it.
  - Dark themes: the dark galaxy (`public/logo.png`, Android `logo_mark`),
    shown as a rounded square (22% corner radius) with a soft drop shadow.
  - Cosmic Light: the light galaxy, the owner's spiral on a transparent
    background (`public/logo-light.png`, Android
    `drawable-nodpi/logo_mark_light.png`; master at
    [`logo-galaxy-light-1024.png`](logo-galaxy-light-1024.png)). It has no
    frame: no rounded square, just a faint blue drop shadow.
  - Desktop picks the logo with `logoSrc()` (`components/Brand.tsx`); Android
    with `logoRes()` (`ui/Brand.kt`).
- Wordmark: "My-Domain", extra-bold, with "Domain" in the accent colour
  (`--accent-fg`). Below it, the tagline "YOUR DEVICES. YOUR DOMAIN." in gold
  (`--highlight-fg`), small, bold and uppercase. Both appear on the sign-in
  screen in both apps, and in the desktop sidebar's brand area. Use the
  `Wordmark` and `Tagline` components; don't retype them. Other copy keeps
  the lowercase app name "my-domain".
- Icons: simple monochrome glyphs (line or solid), tinted by role: cyan for
  interaction, gold for highlights, green for status. No emoji in the UI.

## Using the tokens

### Desktop (SolidJS)

- Styles use only `var(--token)`. `src/styles/app.css` contains no raw
  colours. A new colour means a new token in every theme block of
  `tokens.css`.
- The theme lives in `src/lib/theme.ts`:

  ```ts
  import { theme, setTheme, THEMES } from "./lib/theme";
  setTheme("light");   // applies now and is remembered (localStorage "md.theme")
  theme();             // current id; a Solid signal
  ```

- `initTheme()` runs in `index.tsx` before the first render, so the screen
  never flashes the wrong theme.
- `<ThemePicker />` (`src/components/ThemePicker.tsx`) is a self-contained
  picker. Settings shows it under "Appearance".

### Android (Compose)

- `MdTheme { ... }` wraps the app (`MainActivity`). It builds the Material 3
  `ColorScheme`, `Typography` and `Shapes` from the current theme and draws the
  canvas: star field, veil and glow.
- In composables, read tokens through `T`: `T.text`, `T.accent`,
  `T.brush`, and so on. Read shapes through `MaterialTheme.shapes`. Don't use
  `Color(0x...)` outside `Theme.kt`.
- Switch theme with `ThemeStore.set(prefs, ThemeId.Light)`. It is stored in
  the `md` SharedPreferences under `theme`, and the whole app re-themes at
  once. `ThemePicker(prefs)` is the ready-made picker, shown in Settings under
  "Appearance".
