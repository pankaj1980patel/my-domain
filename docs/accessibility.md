# Accessibility

The target in both apps is **WCAG 2.2 AA**. The desktop app comes first; the
Android app has its own section at the end ([Android](#android)).

## Desktop

This covers the desktop UI (`apps/desktop/ui/src`).
It records what was checked, what was fixed, and what is still open. Shared
keyboard helpers are in `src/lib/a11y.ts`. Words that only a screen reader
hears are the `a11y.*` keys in `src/i18n/en.ts`. The CSS is the block headed
"Accessibility" at the end of `src/styles/app.css`, plus the tokens in
`src/styles/tokens.css`.

### What was checked

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

### What was fixed

#### Keyboard and focus

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

#### Semantics

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

#### Canvas and graphs

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

#### Contrast (token changes in `tokens.css`)

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

#### Motion

- The tokens already set `--fast` and `--normal` to 0 under
  `prefers-reduced-motion: reduce`.
- Some animations use fixed durations: the 1 s "connecting" pulse, the
  0.25 s progress-bar transition, and the incoming-call ring pulse. These
  were missed before. A global reduced-motion rule at the end of `app.css`
  now stops every animation and transition.

### Known remaining gaps

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
- **Android mirrors.** Done: `ui/Theme.kt` and `docs/design/theme.md` now
  carry the Cosmic Light changes and the two new tokens (see
  [Android](#android)).

## Android

This covers the Android app's Compose UI
(`apps/android/app/src/main/java/com/mydomain/android/ui`), plus the
lost-mode screen in `LostMode.kt`. The target is the same, WCAG 2.2 AA, read
for TalkBack and touch. Shared helpers are in `ui/Components.kt`:
`reducedMotion()`, `Modifier.progressInfo(label, fraction)` and
`Modifier.announce()`. Words that only TalkBack hears are the `a11y_*`
strings in `res/values/strings.xml`.

### What was checked

Every screen, tile and component in `ui/*.kt` was read, along with the lost
mode screen. Each was checked for:

- **Names.** Icon-only buttons and meaningful icons have a content
  description; decorative ones have none (`null` or cleared).
- **Roles and states.** Switches, chosen options and selectable chips say
  what they are and whether they are on or chosen.
- **Grouping.** A row that is one thing (a device, a transfer, an activity
  entry) is one TalkBack stop, while its buttons stay their own stops.
- **Headings.** Screen titles, tile titles and sub-section titles are
  headings, so TalkBack's heading navigation works.
- **Touch targets.** 48 dp at least (Android's guideline, well above
  WCAG 2.5.8's 24 px).
- **Progress.** Bars report their value and say what they measure.
- **Live changes.** Status lines that change while the screen is open are
  live regions.
- **Canvas.** The battery graph and the computer's screen in the touchpad
  have text alternatives.
- **Contrast.** Measured as for the desktop. Android uses the same token
  colours, and its glass panels use the same alpha over the same `bg`.
- **Motion.** Android's "Remove animations" setting.

### What was fixed

#### Theme (`ui/Theme.kt`)

- **Same values as the desktop.** The Cosmic Light changes and the two new
  tokens from the desktop pass are now in `Theme.kt` (see the table under
  "Contrast" in the desktop section, and `docs/design/theme.md`):
  - `text3` #61738D → #56677F
  - `success` #059669 → #047857
  - `warning` #B45309 → #9A4508
  - `danger` #E11D48 → #BE123C
  - `highlightFg` #9A6400 → #8A5900
  - `accent` #38BDF8 → #0284C7
- **`controlBorder`** (#5D7598, #6F679E, #4F7A9C, #64758F, #73859E). It is:
  - the unfocused edge of every text field (was `border`, 1.1–1.5:1);
  - the outline of an off switch (was `borderStrong`);
  - the unchosen edge of the device-icon choices;
  - Material's `outline` colour (component edges).

  Card borders keep `border`, so the glass look is unchanged.
- **`primaryScrim`** (navy #050B18 at 40 / 32 / 34 / 0 / 38 %) is laid over
  the gradient on `PrimaryButton`. The white label is now 4.5:1 or better at
  the gradient's light stop. Before, it was 2.0–2.5:1 there.
- In Cosmic Light, switches (the accent track) and selection rings were
  2.1:1 against white. They are now 4.1:1, with the new `accent`.

#### Names, roles and states

- **Switch rows** (`SwitchRow`, used for every on/off setting). Before, the
  row and the switch inside it were two separate stops: the row had no role,
  and the switch had no label. Now the row is one `toggleable` with
  `Role.Switch` and is at least 48 dp tall. TalkBack reads "Keep clipboards
  in sync with my devices, switch, on", and a tap anywhere on the row
  toggles it.
- **Tiles.** A tile without an action was a *disabled* clickable `Surface`.
  TalkBack read the whole tile as one block, followed by "disabled". Now only
  a tile you can tap is clickable. Other tiles are plain surfaces, so each
  line and control is its own stop.
- **Chosen options say "selected".** These are buttons where the chosen one
  is drawn filled (`selected` on `PrimaryButton` and `SoftButton`):
  - the "Tell my other devices when charged to" level (Settings);
  - the automation form's trigger, device, target and action rows;
  - the ringer mode buttons on a phone's page.

  Each row is a `selectableGroup`.
- **Device icon and colour choices** (label editor) use `selectable` with
  `Role.RadioButton` in a `selectableGroup`. Their inner letter or slash is
  cleared, so TalkBack reads only the choice's name.
- **Unread badge** on a device card reads "3 unread messages", not "3".
- **Avatars** (the initial or icon) are decorative and hidden. The device's
  name is always next to them.
- **Directions.**
  - Transfers and clipboard history: the ↑/↓ glyphs are cleared and replaced
    by "Sent" or "Received", or "From …" or "To …".
  - Chat messages: each message is one stop, with "Sent" or "Received" as its
    state.
- **Automation-app key.** When hidden, it reads "Key hidden", not "bullet"
  sixteen times.
- **Small decorative glyphs** are hidden: the "›" on the Transfers tile and
  the "•" of Help bullet lists. Numbered list items keep their numbers.
- **Developer screen.** Back used a hard-coded "Back". It now uses the string
  resource.
- **Icon buttons** already had names: Back on every pushed screen and Help
  on Set up. The bottom navigation icons are rightly `null`, because the tab
  label names them.

#### Grouping (merged semantics)

Each of these is now one TalkBack stop. Its buttons (Cancel, Open, Copy, Send
again, Connect) remain their own stops.

- Device card: the name (a heading), the status and signal, and the unread
  count. The card was already one clickable `Surface`.
- Transfers row: direction, file name and device.
- Files tile row: name and state.
- Activity entry: what happened and when.
- Clipboard history entries (the device page and the history tile).
- Automation-app history entries.
- Browse rows.
- Info rows (Battery, Storage, Memory, Processor, Uptime, Do Not Disturb).

#### Headings

`heading()` is set on:

- `PageHead` titles (Your devices, Activity, Settings);
- the titles of the pushed screens: the device's name, Transfers,
  Automations, Help (and the topic shown), Set up this phone, Developer, the
  sign-in title, and lost mode's "This phone is lost";
- every `Tile` title;
- device names on the cards;
- the Transfers section titles;
- the sub-titles in the Automation apps tile and the clipboard history;
- Help topic headings.

#### Touch targets

- **Buttons.** `SoftButton` and `GhostButton` have a *minimum* height of 44
  or 36 dp now, instead of a fixed one, and Material keeps a 48 dp touch area
  around them. `PrimaryButton` draws its gradient on its own modifier, so it
  puts `minimumInteractiveComponentSize()` around a box instead. Every button
  is now a 48 dp target in layout, not only through Compose's touch-bounds
  expansion, and its label can grow with the font size instead of being
  clipped.
- **Device icon and colour choices** went from 44 and 40 dp to 48 dp. The
  swatch inside grew from 30 to 36 dp.
- **Folder rows** in the Browse tile are at least 48 dp tall, with
  `Role.Button`.
- **The Settings version line** (7 taps unlock Developer) is at least 48 dp
  tall, with `Role.Button`.
- **Switch rows** are at least 48 dp tall.

#### Progress

- Transfer bars (Transfers screen and the Files tile) carry
  `ProgressBarRangeInfo` and the file's name: "report.pdf, 40 percent".
- Info rows with a bar (battery, storage, memory, processor) are one stop
  with the label, the value and the range: "Storage, 12 GB free of 64 GB, 81
  percent". The bar shows how full it is.

#### Live regions

Polite unless marked otherwise:

- the error box on sign-in (`ErrorText`), which is **assertive**;
- the connecting progress line on a device page;
- the volume level;
- the located position;
- Browse's path, "Asking…" and its errors;
- a command's result line;
- the backup status;
- Accessibility helper on or off;
- the Shizuku state;
- "Sharing your screen with …";
- "Playing on …";
- the computer audio state and its last stop reason.

Toasts are Material snackbars. They are already announced, and their time
on screen follows Android's "Time to take action" setting.

#### Canvas and screen views

- **Battery graph.** Its description was "Battery over the last 24 hours".
  It now sums up the data: "Battery over the last 24 hours: 92% to 41% now,
  lowest 38%, highest 92%. Charging now." The "24 h ago" and "now" captions
  are hidden. The estimate line under the graph is read as normal.
- **Touchpad and the computer's screen** (`TouchpadUi.kt`). The pad is one
  stop that says what it is and that the click, key and swipe buttons do the
  same with TalkBack on. When the computer's screen shows on the pad, the
  description says so first. Before, the pad had no description, and only
  the picture had a name.

#### Motion

- The only app animation is the pulsing "Connecting" dot (`StatusPill`).
  `reducedMotion()` reads `Settings.Global.ANIMATOR_DURATION_SCALE`. When it
  is 0 ("Remove animations"), the dot stays still. The pull-to-refresh
  spinner is Material's and follows the system scale itself.

### Known remaining gaps

- **Not tested with TalkBack.** These checks were made by reading the code
  and measuring colours. Nothing has been run on a phone with TalkBack or
  Switch Access yet, and Accessibility Scanner hasn't been run either.
- **Touchpad gestures.** Moving the pointer and point-and-click need direct
  touch. With TalkBack on, the buttons cover clicks, keys and the three- and
  four-finger swipes. There is no TalkBack way to move the pointer to a
  given spot.
- **Battery graph points.** As on the desktop, the summary covers the trend,
  not each reading.
- **Chat.** New incoming messages aren't announced. A live region on the
  whole thread would read every message again. A polite announcement of the
  newest one is still to do.
- **Chosen-option buttons** say "selected", but their role stays "button",
  not "radio button".
- **Switch thumb.** In the dark themes, the white thumb on the cyan track is
  about 2:1. The state is still clear from the track against the panel
  (3:1 or better) and from where the thumb sits.
- **Waiting chat messages** are drawn at 70 % opacity, which lowers their
  contrast. The words "Sends when connected" say so.
- **Translucent panels over the star field.** As on the desktop, contrast
  was measured over the plain `bg`.
- **"Remove animations" is read once per screen.** Changing it while a
  screen is open takes effect when that screen is next opened.
- **Font scale.** Buttons now grow with the font size. Fixed-size bits do
  not: the 8 dp status dot, the 28 dp direction circle and the unread badge.
  They hold one glyph each. Long one-line texts (file names, keys) are
  ellipsized rather than wrapped.
