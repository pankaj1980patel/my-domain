# Accessibility: desktop app

This covers the desktop UI (`apps/desktop/ui/src`). The target is **WCAG 2.2 AA**.
It records what was checked, what was fixed, and what is still open. Shared
keyboard helpers are in `src/lib/a11y.ts`. Words that only a screen reader
hears are the `a11y.*` keys in `src/i18n/en.ts`. The CSS is the block headed
"Accessibility" at the end of `src/styles/app.css`, plus the tokens in
`src/styles/tokens.css`.

## What was checked

Every screen and component was read:
`App.tsx`, `screens/*.tsx`, `screens/device/*.tsx` and `components/*.tsx`.
Each was checked for:

- **Keyboard.** Every control can be reached and used with Tab, Shift+Tab,
  Enter, Space and Esc. Focus is always visible. Nothing traps focus except
  dialogs that are meant to, and those hand focus back when they close.
- **Semantics.** Real buttons and links, headings in order (one `h1` per
  screen), lists marked up as lists, labelled form fields, switches, progress
  bars, live regions and dialog roles.
- **Contrast.** Each theme's tokens were measured with the WCAG formula. For
  translucent panels, the panel colour was first laid over `--bg`. Text needs
  4.5:1. Large text, control edges and focus rings need 3:1.
- **Motion.** `prefers-reduced-motion` is respected.
- **Images and icons.** Decorative ones are hidden from screen readers.
  Meaningful ones have names.
- **Canvas and graphs.** The phone or computer screen view and the battery
  graph have text alternatives.

## What was fixed

### Keyboard and focus

- **Skip to content.** A "Skip to content" link is the first Tab stop. It
  stays off screen until it has focus, then moves focus to `<main id="main">`.
- **Changing screens.** Choosing a place in the sidebar moves focus to the new
  screen's content. Focus also moves there when the old screen's element
  disappears. Focus is no longer left in the sidebar or dropped on `<body>`.
  The window title now names the screen ("Transfers · my-domain").
- **Device cards.** Before, each card was a focusable `<article>` with a click
  handler and a Connect button inside it. That put one interactive element
  inside another, and Space did nothing. Now the device's name is a real
  `<button>` inside an `h2`. A click anywhere on the card still opens it. The
  whole card shows the focus ring (`:has()`), and the status and unread count
  are read out with the name.
- **Settings version line.** It was a `<p>` with a click handler, so the 7
  presses that unlock Developer only worked with a mouse. It is now a button
  that looks the same.
- **Theme picker.** It is a proper radio group. Tab reaches only the chosen
  theme. The arrow keys, Home and End move through the themes and choose as
  they go.
- **Dialogs (focus kept inside, given back on close):**
  - **Command palette:** already kept focus inside and gave it back. Now it
    also announces how many commands match.
  - **Send to…:** now keeps Tab inside the dialog and gives focus back on
    close, whether by Esc, Cancel, a click outside or a send.
  - **Phone/computer screen in full screen:** now a modal dialog. Tab stays
    inside, "Exit full screen" takes focus, and Esc or Exit gives focus back
    to "Full screen". Stopping the stream from full screen moves focus to
    Show/Stop.
  - **Incoming call:** Answer takes focus. Before, `autofocus` was set on an
    element added after page load, so it did nothing. Focus goes back where it
    was when the call is answered, declined or missed. The box isn't modal, so
    Tab can still leave it.
  - **"Looking for this computer" banner:** Stop takes focus, Esc dismisses
    the banner, and focus goes back afterwards.
- **Inline forms.** These forms replace the button that opens them. Each now
  moves focus into the form when it opens and back to a button when it
  closes, so focus is never lost to the page. They aren't modal, so Tab
  leaves them normally.
  - **Device label editor:** focus goes to the nickname. Esc, Cancel or Save
    closes it and gives focus back to Edit.
  - **Lost mode:** focus goes to the message. Esc or Cancel gives focus back
    to "Lost mode…". Once lost mode is on, focus goes to Stop.
  - **Change device key:** focus goes to the first field and comes back to the
    button.
  - **Add or edit a keyboard shortcut:** focus goes to the form. Esc cancels
    (unless keys are being recorded) and focus comes back to "Add a shortcut".
- **Chat thread.** It is a focusable `role="log"`, so it can be scrolled from
  the keyboard.
- **Visible focus.**
  - The global `:focus-visible` outline uses `--focus` (3:1 or better on every
    theme).
  - Text fields and the lost-mode message showed focus with a 1 px accent
    border and a glow at 12 % alpha. In Cosmic Light that was 2.1:1. They now
    show a 2 px `--focus` ring with the glow kept outside it.
  - The screen view's focus ring used `--accent` with a hard-coded fallback
    colour. It now uses `--focus`.

### Semantics

- **Sidebar.** The places are now a list (`<ul>`). The current one has
  `aria-current="page"`. The count badges are hidden from screen readers and
  replaced by words ("Devices, 2 connected", "Transfers, 1 under way").
- **Brand.** Removed `aria-label` from a plain `<span>`, where it is not
  allowed. The wordmark text is already read out.
- **Headings.**
  - Device cards and empty states used `h3` straight under the page's `h1`.
    They are now `h2`, with the same look.
  - The "device not found" state had no heading. It now has a hidden `h1`.
- **Toasts.** There are now two live regions that are always on the page.
  Errors go in `role="alert"` (assertive). Everything else goes in
  `role="status"` (polite). Before, one polite region carried errors too.
- **Disabled controls with a reason.** A disabled button can't take focus, so
  its tooltip never shows from the keyboard. `Gate` now also gives the reason
  as hidden text ("Unavailable: …") next to the control.
- **Progress bars.**
  - Transfers already used `role="progressbar"`. It now also has a name ("Sending
    or receiving photo.jpg") and `aria-valuetext` ("42%, 12 MB of 30 MB ·
    3 MB/s · 6 s").
  - The Files tile's progress bars had no role. They now have the same
    progressbar role and values.
- **Device info.**
  - The bars repeat the value next to them, so they are hidden from screen
    readers.
  - The bars and the battery graph sat directly inside a `<dl>` row, which
    isn't valid there. They now sit in their own `<dd>`.
- **Labels.**
  - The change-key fields had only placeholders. They now have names.
  - The Developer screen's four fields now have names (in English, like the
    rest of that screen).
  - The sign-in device key is linked to its hint with `aria-describedby`.
    Empty required fields get `aria-invalid` after a failed attempt.
- **Label in name (2.5.3).** Accessible names now start with the visible
  label:
  - "Mute" is named "Mute {app}: …".
  - "Unmute" is named "Unmute {app}: …".
  - "Send again" is named "Send again to {name}'s clipboard".
- **Icons and symbols.**
  - The clipboard log's ↑/↓ arrows get `role="img"`, so their `aria-label` is
    read.
  - The Wi-Fi list's "▂▄▆█" and 🔒 are hidden from screen readers. In their
    place it says "signal 3 of 4, secured".
  - The webhook token's 24 "•" are read as "Hidden". Show/Hide is a toggle
    button (`aria-pressed`).
  - The palette's "↑↓ to move" hint was already hidden. Decorative SVGs (the
    device icons) were already `aria-hidden`. The audio/status dots and the
    ring pulse were already hidden.
- **Device label editor.** The form is named by its heading. The "no icon"
  choice, which shows only a letter, is named "None".

### Canvas and graphs

- **Screen view (`<canvas>`).**
  - When it only shows a picture, it is `role="img"` named "{name}'s screen,
    view only". Before, it was named just "Its screen".
  - When it takes clicks and typing for another computer, it is
    `role="application"`, so screen readers pass keys through. It is named
    "{name}'s screen" and described by the existing instructions ("Click,
    right-click or scroll…").
  - Phone taps and swipes are mouse/touch only. The phone's Back, Home and
    Recents buttons below the picture work from the keyboard.
- **Battery graph (SVG).** It used to be labelled only "Battery over the last
  24 hours". Its name now sums up the data, for example: "Battery over the
  last 24 hours: 92% to 41% now, lowest 38%, highest 92%. Charging now. About
  6 h left at this rate." The hover tooltip and the axis captions are hidden
  from screen readers because the summary covers them.

### Contrast (token changes in `tokens.css`)

The four dark themes (Cosmic Blue, Aurora, Ocean, Midnight) already passed for
text. Measured on `--surface`, `--card-bg` and `--surface-2`:

- `--text-3` is at least 5.6:1.
- `--accent-fg` is at least 5.9:1.
- The status colours are at least 5.2:1.

The failures were control edges, primary buttons, and Cosmic Light text.

| Token | Theme | Old → new | Contrast before → after |
|---|---|---|---|
| `--control-border` (new) | Cosmic Blue | `--border` #203454 → #5d7598 | input/select edge: 1.4–1.5 → 3.6–3.9:1 |
| | Aurora | `--border` #2e2a52 → #6f679e | 1.2–1.4 → 3.2–3.6:1 |
| | Ocean | `--border` #1d3d5a → #4f7a9c | 1.3–1.5 → 3.1–3.7:1 |
| | Midnight | `--border` #29364a → #64758f | 1.2–1.5 → 3.2–3.8:1 |
| | Cosmic Light | `--border` #dce8f7 → #73859e | 1.1–1.2 → 3.3–3.7:1 |
| `--primary-scrim` (new) | Cosmic Blue | none → rgba(5,11,24,.40) | white label at the gradient's lightest stop #22c7f2: 2.0 → 4.5+:1 |
| | Aurora | none → .32 | at #e879f9: 2.5 → 4.5+:1 |
| | Ocean | none → .34 | at #00b8d9: 2.4 → 4.5+:1 |
| | Midnight | transparent | flat #2563eb: 5.2:1 already |
| | Cosmic Light | none → .38 | at #38bdf8: 2.1 → 4.5+:1 |
| `--text-3` | Cosmic Light | #61738d → #56677f | on `--surface-2`: 4.21 → 5.03:1 |
| `--success` | Cosmic Light | #059669 → #047857 | on white: 3.74 → 5.44:1; on `--surface-2`: 3.28 → 4.78:1 |
| `--warning` | Cosmic Light | #b45309 → #9a4508 | on `--surface-2`: 4.38 → 5.67:1 |
| `--danger` | Cosmic Light | #e11d48 → #be123c | on `--surface-2`: 4.09 → 5.48:1; on its soft fill: 3.99 → 5.34:1 |
| `--highlight-fg` | Cosmic Light | #9a6400 → #8a5900 | on `--surface-2`: 4.36 → 5.22:1 |
| `--accent` | Cosmic Light | #38bdf8 → #0284c7 | switch-on track, chart line, hover edges on white: 2.12 → 4.06:1 (badge text `--on-accent` on it: 4.58:1) |

How the new tokens are used:

- **`--control-border`** is the edge of text fields, the lost-mode message,
  the schedule and screen-settings selects, and the inside of an off switch’s
  track. Decorative card and panel borders keep `--border`, which WCAG
  doesn't require to reach 3:1, so the glass look is unchanged.
- **`--primary-scrim`** is a thin veil laid over the brand gradient on primary
  buttons only. Progress bars keep the plain gradient.
  - Primary buttons used to brighten on hover, which took them back under
    4.5:1. They now show a ring on hover instead.

### Motion

- The tokens already set `--fast` and `--normal` to 0 under
  `prefers-reduced-motion: reduce`.
- Some animations use fixed durations: the 1 s "connecting" pulse, the
  0.25 s progress-bar transition, and the incoming-call ring pulse. These
  were missed before. A global reduced-motion rule at the end of `app.css`
  now stops every animation and transition.

## Known remaining gaps

- **Phone control.** Taps and swipes on a phone's screen need a pointer. The
  phone's Back, Home, Recents and the Controls tile's keys are keyboard
  alternatives. There is no keyboard way to tap a point on the phone's
  screen.
- **Battery graph tooltip.** The time and level for each point show on mouse
  hover only. The text summary covers the trend but not each reading.
- **Toast timing.** Toasts disappear after 3.5 s (6 s for errors) and don't
  pause on hover or focus. Screen readers hear them through the live
  regions, and the same events are also kept in Activity. Sighted keyboard
  users with slow reading may miss one.
- **Translucent panels over the star field.** Contrast was measured with the
  panels over the plain `--bg`. A bright star right behind small muted text
  could locally lower it. The veil (`--bg-veil`) and the panel blur keep this
  small, but it can't be checked with a formula.
- **Target size (2.5.8).** The "?" Help buttons are 20×20 px. They meet the
  2.2 AA spacing exception (nothing else within 24 px). They are below the
  24 px size on their own.
- **Native selects.** Selects without a class (Settings, call audio,
  shortcuts, phone audio) use WebKit's own drawing. Their edge contrast is
  the platform's.
- **Windows High Contrast / forced colours.** Not tested. The custom switches
  use `appearance: none` and may need a `forced-colors` rule.
- **Not tested with assistive technology.** No screen reader (VoiceOver,
  NVDA, Orca) has been run on the Tauri webviews (WKWebView, WebView2,
  WebKitGTK). The checks so far were by reading the code and measuring.
- **Android mirrors.** `tokens.css` says the Android app (`ui/Theme.kt`) uses
  the same values. The Cosmic Light changes and the two new tokens above
  haven't been carried over to it, or to `docs/design/theme.md`, yet.
