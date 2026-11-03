# my-domain — Feature Catalogue

What one device can see or do on another, and how access is granted.

---

## 1. Core concept: Domains

| Term | Meaning |
|---|---|
| **Device** | One install of my-domain (`node_id`), e.g. "Pixel 8", "MacBook". |
| **Domain** | A named set of devices that trust each other. Every account gets a default domain, **My Devices**. Users can create more, e.g. *Family*, *Office*, *Support*. |
| **Member** | A device that belongs to a domain. A device can be in many domains. |
| **Share** | Making a device available to another domain (yours or someone else's) or to a single device, without moving it. |
| **Grant** | `(target device) → (grantee domain or device) : {flags} + constraints`. |
| **Flag** | One permission per feature, e.g. `calls.read`, `phone.dial`, `wifi.manage`. |

### Flag levels
- `off`: denied silently.
- `ask`: the target device shows a prompt each time (allow once / allow for N minutes / deny).
- `on`: allowed without a prompt.

### Flag scopes
Most features split into `*.read` (view) and `*.control` / `*.manage` (change something). For example, `wifi.read` lets you see networks and `wifi.manage` lets you toggle Wi-Fi or connect.

### Constraints
A grant can be limited by any of these:
- Expiry: a date, or *for 1 hour*.
- Time-of-day window.
- LAN only.
- Only while the target is unlocked or charging.
- Rate limit.
- Requires the target user to be present (`ask` override).

### Presets
| Preset | Typical flags |
|---|---|
| **Owner** | Everything `on`, including destructive actions (wipe, reboot). |
| **Trusted** | Everything `on` except destructive actions, which are `ask`. |
| **Family** | `find.*`, `device.battery`, `notifications.mirror`, `files.push`, `find.ring`. |
| **Viewer** | `*.read` only. |
| **Guest** | `clipboard.push`, `files.push`, `chat`. |
| **Remote support** | `screen.view` + `input.control` on `ask`, expires in 1 h. |

### Rules
1. **The target device enforces every permission.** It never trusts a check made by the caller or the server.
2. Every request is written to an **audit log** on the target, which can be viewed from any Owner device.
3. A **visible indicator** (persistent notification or tray icon) shows while the screen, camera, mic or input is being accessed.
4. A **"Revoke all"** kill switch is available on every device.
5. Destructive actions (wipe, reboot, uninstall) can never be granted to a domain you don't own.

### Architecture implications
- **Per-device keys:**
  - Today all devices of one user share a single derived key (`core/src/crypto.rs`). Sharing outside the account needs per-device identity keypairs (X25519/Ed25519) and pairwise sessions, e.g. Noise.
  - Membership in a domain is a certificate signed by the domain owner.
- **Server:** stores domains, members and public keys. It never stores message contents. Grants are owned by the target device and can be mirrored to the server for UI display.
- **RPC layer:** request/response messages `{id, method, params}` → `{id, result | error}` on top of the existing control messages in `core/src/features/`, with a permission-check step before any handler runs.
- **Streaming:** screen, camera, audio and input need the v2 channel mux (`core/src/wire/`, `core/src/service.rs`) plus WebRTC media tracks.

---

## 2. How hard each feature is to build (Android)

| Tier | Meaning |
|---|---|
| **S** | Standard runtime permission (the user taps Allow). |
| **P** | Special access the user enables in Settings: Accessibility, Notification access, Device admin, Usage access, Overlay, Modify system settings, DND access, a default-app role (Dialer/SMS), or MediaProjection consent per session. |
| **R** | Privileged: ADB-granted permission via **Shizuku**, root, or Device Owner. Needed for most radio toggles on Android 10+. |

Desktop notes:
- macOS needs the *Accessibility* and *Screen Recording* permissions.
- Linux on Wayland uses xdg-desktop-portal.
- Windows has no special requirements.

Status: ✅ exists · 🟡 partial · ⬜ planned.
Targets: **A** = Android, **D** = Desktop (macOS/Linux/Windows).

---

## 3. Features

### 3.1 Messaging & sharing
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Chat | Encrypted text between devices | `chat` | A D | — | ✅ |
| Clipboard push | Auto-sync the clipboard to a peer | `clipboard.push` | A D | — | ✅ |
| Clipboard pull | Fetch a peer's clipboard on demand | `clipboard.read` | A D | — | ✅ |
| Clipboard history | Browse recent clipboard items across devices | `clipboard.read` | A D | — | ✅ (last 20, memory only) |
| Notification mirror | Show a peer's notifications | `notifications.mirror` | A→D | P | ✅ (desktop OS notifications) |
| App notification subscribe | Choose which apps' notifications to receive | `notifications.mirror` | A | P | ✅ |
| Notification actions | Reply, dismiss, or tap an action on a mirrored notification | `notifications.control` | A | P | ✅ |
| Send-to-device | Share sheet / context-menu target "Send to <device>" | `files.push` | A D | — | 🟡 (Android share sheet: files, text, links; no desktop context menu) |
| Send text | Put text on a device's clipboard or show it there | `clipboard.push` | A D | — | ✅ |
| Open link on device | Open a URL in the peer's browser | `apps.launch` | A D | — | ✅ (http/https only) |

### 3.2 Screen, camera & remote control
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Screen share (view) | Live view of the peer's screen | `screen.view` | A D | P (MediaProjection) | 🟡 (2026-11-03: snapshots about twice a second. Computer → phone behind the full-screen touchpad, tried from mdctl against the Mac; phone → computer in the desktop's "Its screen" tile, Android asks each time; macOS needs Screen Recording, Linux grim or ImageMagick; the phone views not tried live yet) |
| Remote control | Tap, swipe, type on the peer | `input.control` | A D | P (Accessibility) | 🟡 (2026-11-03: computer ← computer from the "Its screen" tile: click, right-click, scroll, type; not tried live yet. Controlling a phone: not started) |
| Screenshot | Grab a single frame | `screen.view` | A D | P | ⬜ |
| Screen record | Record the peer's screen to a file | `screen.record` | A D | P | ⬜ |
| Remote keyboard | Type into the phone from the desktop keyboard | `input.control` | A | P (Accessibility / IME) | ⬜ |
| Phone as touchpad / keyboard | Control the desktop cursor from the phone | `input.control` | D | S | ✅ (2026-11-03, tried on the Mac: move, clicks, two-finger scroll, typing, keys, three/four-finger swipes and buttons for them, full-screen pad, Point & click on the screen picture; macOS needs Accessibility, Linux an X11/XWayland session) |
| Phone as webcam | Use the phone camera as a desktop webcam | `camera.view` | A→D | S | ⬜ |
| Remote camera view | See the peer's front/back camera (indicator shown) | `camera.view` | A D | S | ⬜ |
| Remote mic listen | Listen to the peer's mic (`ask` by default, indicator shown) | `mic.listen` | A D | S | ⬜ |
| Presentation remote | Next/prev slide, laser pointer | `input.control` | D | S | 🟡 (next, previous, black screen; no laser pointer; macOS needs Accessibility) |
| Single-app mirror | Stream just one Android app to the desktop | `screen.view` | A | R | ⬜ |

### 3.3 Media
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Now playing | Title, artist, artwork, position of any playing app | `media.read` | A D | P (Notification access) | 🟡 (title, artist, album, app, playing/paused; no artwork or position; macOS: Spotify and Music only) |
| Media control | Play/pause/next/prev/seek **any app** (Spotify, YouTube, …) via MediaSession | `media.control` | A D | P | 🟡 (play/pause, next, previous on Android and Linux; no seek; not macOS) |
| Volume control | Media/ring/alarm/call volume | `media.control` | A D | S | 🟡 (media volume up/down; desktop output volume) |
| Ringer mode | Normal / vibrate / silent | `device.settings` | A | P (DND access) | ✅ (silent falls back to vibrate without DND access) |
| Play on device | Send a URL or file to play on the peer | `media.control` | A D | S | 🟡 (a link, opened in a chosen app on the phone; files: send, then open) |
| Deep-link play | Open Spotify/YouTube on the peer with a search or track | `apps.launch` | A D | S | ✅ (search in YouTube / Spotify / the web; a track link opens in its app) |
| Audio streaming | Route desktop audio to phone speakers or vice versa | `media.stream` | A D | S | ⬜ |
| Auto-pause on call | Pause desktop media when the phone rings | `calls.read` | A→D | S | ⬜ |

### 3.4 Apps
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| List apps | Installed apps with icons | `apps.read` | A D | S | 🟡 (Android list exists) |
| Launch app | Open an app on the peer | `apps.launch` | A D | S / P (background start) | 🟡 (Android only; tap-to-open notification in the background) |
| Foreground app | What's open right now | `apps.read` | A D | P (Usage access) | ⬜ |
| Usage stats | Screen time per app | `apps.read` | A | P | ⬜ |
| Install APK | Push an APK from the desktop and install it (user confirms) | `apps.manage` | A | P | ⬜ |
| Uninstall app | Prompts on the device | `apps.manage` | A | S | ⬜ |
| Force stop | Kill an app | `apps.manage` | A | R | ⬜ |
| Block app | Parental control: block an app during set hours | `apps.manage` | A | P (Accessibility / Device admin) | ⬜ |

### 3.5 Phone & calls
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| View call log | Recent calls: number, name, type, duration | `calls.read` | A | S | 🟡 (handler exists, nothing sends it) |
| Incoming call alert | Live ringing / missed / ended | `calls.read` | A | S | 🟡 |
| Dial a number | Place a call on the phone from the desktop | `phone.dial` | A | S (`CALL_PHONE`, `TelecomManager.placeCall`) | 🟡 (opens the dialer with the number; you press Call) |
| Answer / reject / end | Control the active call | `phone.control` | A | S (`ANSWER_PHONE_CALLS`) | ⬜ |
| Speakerphone | Turn speaker on/off during a call | `phone.control` | A | P (Dialer role / Accessibility) | ⬜ |
| Mute in call | Mute the mic during a call | `phone.control` | A | P | ⬜ |
| DTMF keypad | Send tones to IVR menus | `phone.control` | A | P (Dialer role) | ⬜ |
| Call from desktop audio | Talk through the desktop mic/speaker | `phone.control` | A↔D | R / Bluetooth HFP | ⬜ |
| Block number | Add to the block list | `phone.control` | A | P (Dialer role) | ⬜ |

### 3.6 SMS
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Read SMS | View threads | `sms.read` | A | S (Play policy limits) | ⬜ |
| Send SMS | Compose from the desktop | `sms.send` | A | S | ⬜ |
| OTP forward | Detect an OTP and put it on the desktop clipboard | `sms.read` | A→D | S | ✅ (from notifications, no SMS permission; off by default) |

### 3.7 Contacts & calendar
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| View / search contacts | Name, numbers, email, photo | `contacts.read` | A | S | ⬜ |
| Add / edit contact | | `contacts.write` | A | S | ⬜ |
| Call / SMS from contact | One click from the contact card | `phone.dial` / `sms.send` | A | S | ⬜ |
| Calendar view / add | Events on the peer's calendars | `calendar.read` / `.write` | A | S | ⬜ |

### 3.8 Connectivity
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Wi-Fi status | On/off, SSID, signal, IP | `wifi.read` | A D | S | 🟡 (on, connected, signal; SSID on Linux, rarely on Android/macOS; no IP) |
| Wi-Fi scan | Nearby networks | `wifi.read` | A D | S (location) | ⬜ |
| Wi-Fi toggle | Turn Wi-Fi on/off | `wifi.manage` | A D | R (fallback: open settings panel) | ✅ (2026-11-03: directly, as the app targets Android 9; tried on Android 16) |
| Connect to network | Pick an SSID and enter its password remotely | `wifi.manage` | A D | S (suggestion API) / R (direct) | ⬜ |
| Share Wi-Fi credentials | Push saved credentials or a QR to another device | `wifi.manage` | A D | R | ⬜ |
| Hotspot toggle | Turn on tethering | `wifi.manage` | A | R | 🟡 (2026-11-03: state in the status; opens the tethering settings) |
| Bluetooth status | On/off, paired and connected devices | `bluetooth.read` | A D | S | 🟡 (on/off only) |
| Bluetooth toggle | | `bluetooth.manage` | A D | R on Android 13+ | ✅ (2026-11-03: the Accessibility helper taps the switch, tried on Android 16; directly where the phone allows old-target apps) |
| Connect / disconnect BT device | e.g. move earbuds to the desktop | `bluetooth.manage` | A D | R / Accessibility | ⬜ |
| Airplane mode view | | `radio.read` | A | S | ✅ (2026-11-03, in the status; not tried live) |
| Airplane mode toggle | | `radio.manage` | A | R | 🟡 (2026-11-03: opens the airplane mode settings) |
| Mobile data view / toggle | Including data usage | `radio.read` / `.manage` | A | S / R | 🟡 (2026-11-03: on/off in the status, the toggle opens the internet panel; no data usage) |
| NFC / location services / VPN | View, and toggle where possible | `radio.read` / `.manage` | A | S / R | ⬜ |

### 3.9 Device state & settings
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Battery | %, charging, temperature, health | `device.battery` | A D | — | ✅ (temperature and health on Android) |
| Storage / RAM / CPU | Usage overview | `device.read` | A D | — | 🟡 (storage and RAM free/total; no CPU) |
| Brightness / screen timeout / auto-rotate | | `device.settings` | A D | P (Modify system settings) | ⬜ |
| Do Not Disturb | Toggle and schedule | `device.settings` | A D | P | ⬜ |
| DND follows the phone | Computers hold my-domain's pop-ups while the phone is in Do Not Disturb (the OS state is only read, never changed) | `device.settings` | A→D | — | ✅ (2026-11-03; no permission needed; option on the computer, off by default; not tried live yet) |
| Flashlight | On/off | `device.settings` | A | S | ✅ |
| Lock screen | Lock now | `device.lock` | A D | P (Device admin) | 🟡 (desktop only: macOS display sleep, Linux logind / screen locker) |
| Wake / keep awake | Keep a device from sleeping for N minutes | `device.settings` | A D | S | ✅ (2026-11-03: 30 min / 1 hour / Let it sleep on the phone's page for a computer, tried on the Mac; macOS `caffeinate`, Linux `systemd-inhibit`, phone screen wake lock) |
| Wake-on-LAN | Wake a sleeping computer from the phone | `device.power` | A→D | — | 🟡 (2026-11-03: the computer says its network card in its status, the phone remembers it and shows "Wake it"; same network only; waking not tried yet) |
| Battery alerts | Tell the other devices once when the battery drops below a level (default 15%) or finishes charging | `device.battery` | A D | — | ✅ (2026-11-03; on/off switch on the phone, desktop command only) |
| Sleep / restart / shutdown | | `device.power` | A D | R on Android / S on desktop | ⬜ |
| Desktop volume / mute | | `media.control` | D | — | ✅ (macOS, Linux) |

### 3.10 Find & security
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Ring device | Full volume even when silent | `find.ring` | A D | P (DND access) | ✅ (phone: alarm stream at full volume; desktop: window + sound) |
| Locate | Current GPS location on a map | `find.locate` | A | S (background location) | ⬜ |
| Location history | Last known locations | `find.locate` | A | S | ⬜ |
| Lost mode | Lock with a message and contact number | `find.lost` | A | P (Device admin) | ⬜ |
| Remote wipe | Factory reset (Owner domain only) | `find.wipe` | A | P (Device admin) | ⬜ |
| SIM change alert | Notify when the SIM is swapped | `find.locate` | A | S | ⬜ |
| Failed-unlock alert | Alert after repeated wrong PINs | `find.lost` | A | P (Device admin) | ⬜ |

### 3.11 Files
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| File push | Send files or folders (chunked, resumable) | `files.push` | A D | — | 🟡 (files, resumable; no folders) |
| Browse files | Remote file explorer | `files.read` | A D | S / P (All files access) | 🟡 (folders each device shares; no screens yet) |
| Download / delete remote file | | `files.read` / `.write` | A D | P | 🟡 (download done, resumable; no delete) |
| Gallery | Browse photos and videos with thumbnails | `files.read` | A | S | 🟡 (2026-11-03: newest first, paged, 160 px thumbnails, download; desktop commands only, no grid screen yet) |
| Photo auto-backup | New phone photos copied to the desktop | `files.read` | A→D | S | ✅ (2026-11-03: unmetered Wi-Fi, optionally only while charging, resumable, never twice; switch in the phone's Settings; remembered per computer, so a new computer gets the recent photos it lacks) |
| Folder sync | Keep a folder in sync between devices | `files.write` | A D | P | ⬜ |

### 3.12 Automation
| Feature | What it does | Flag | Target | Tier | Status |
|---|---|---|---|---|---|
| Triggers | Battery low, call ringing, arrived home, Wi-Fi joined, device online/offline | `automation` | A D | varies | ⬜ |
| Actions | Any feature above, as a step | per-action flag | A D | varies | ⬜ |
| Scenes | e.g. *Meeting*: phone on DND, desktop media paused | `automation` | A D | varies | ⬜ |
| Scheduled actions | Run at a set time | `automation` | A D | — | ⬜ |
| Run command | Run a predefined shell script on the desktop (Owner only) | `desktop.exec` | D | — | ⬜ |
| Webhooks | Trigger actions from outside, e.g. Home Assistant | `automation` | A D | — | ⬜ |

### 3.13 Domain management
| Feature | What it does | Flag | Status |
|---|---|---|---|
| Create / rename / delete domain | | Owner | ⬜ |
| Invite device / user | QR code or link, with a preset | Owner | ⬜ |
| Grant editor | Per-device, per-flag `off` / `ask` / `on` plus constraints | Owner | ⬜ |
| Temporary share | Time-limited link, e.g. remote support for 1 h | Owner | ⬜ |
| Presence | Who is online, transport, last seen | any member | 🟡 (peer list) |
| Audit log | Who did what, when, from which device | Owner | ⬜ |
| Revoke all | Kill switch | Owner | ⬜ |
| Parental dashboard | Screen time, app blocks, location | Family preset | ⬜ |

---

## 4. Suggested build order
1. **Foundation**
   - Per-device keys and pairwise encryption.
   - Domain and grant model (server + target-side enforcement).
   - RPC layer with a permission check before each handler.
   - Audit log.
2. **Quick wins (S/P tier, small messages):**
   - Battery, ring device, open link, launch app.
   - Media now-playing and control.
   - Call log, contacts, dial, answer/end, SMS + OTP.
   - Flashlight, DND, file push.
   - Desktop system notifier.
3. **Streaming:** screen share, remote control, phone as webcam, audio, on the v2 mux plus WebRTC media.
4. **Privileged:** Wi-Fi, Bluetooth, airplane mode, mobile data, hotspot toggles via Shizuku, with a fallback that opens the settings panel.
5. **Automation:** triggers, scenes, webhooks.
