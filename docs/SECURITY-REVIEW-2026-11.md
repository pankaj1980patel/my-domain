# Security review, November 2026

Scope: everything that crosses a trust boundary, meaning wire messages from paired devices, peer-influenced filesystem writes, network fetches, OS commands, remote-control surfaces, secrets, the browser-extension bridge, and the server (`apps/server`).

Threat model: a paired device may be compromised, lost or buggy. The relay server and the network are untrusted for confidentiality and integrity. A web page may be hostile.

Status values: **fixed** (a regression test is named), **partly fixed**, and **open** (the reason is given).

Line numbers refer to the tree after the fixes.

Totals: 3 high, 21 medium, 24 low. Fixed: 2 high, 14 medium and 6 low. Partly fixed: 1 medium and 1 low.

## High

| # | Where | Scenario | Status |
|---|-------|----------|--------|
| H1 | `crates/mdcore/src/crypto.rs:25-43`, `transport/mod.rs:382` (`handle_incoming`), `transport/relay.rs:571-598`, `engine/ble.rs:143-156` | Envelopes carry no sender, recipient, time or counter in their authenticated data. Control messages are not de-duplicated. Over the relay, the sender is whatever the server says; over BLE, it is an unauthenticated `Hello`. So the relay server (or anyone who recorded traffic) can replay an old ciphertext, such as `input` text plus Enter, `open_url`, `action: lock` or `media_cmd`, and attribute it to any device. It decrypts and is acted on. | **Open.** The fix changes the wire format: envelope v2 with AAD = `v2\|sender\|recipient`, plus a `ts` and a random `mid` in the plaintext, a ±60 s window, and a per-peer seen-`mid` cache. Relay and BLE frames would also need a device-key signature checked against the pinned key. Desktop and Android must ship this together, so it is not a contained fix. |
| H2 | `apps/server/src/pair.rs:181` (`create`) | `/pair/create` lets a caller choose its own `code` and answers 409 when another account's live pairing already uses it. Creates were not limited. An attacker with a free account could walk all 10^6 codes in minutes, then `/pair/claim` the hit. That gives a session on the victim's account and the victim's `code_box`, which opens with the now-known code and yields the device key. | **Fixed.** Chosen codes are limited to 5 per account and 120 server-wide per minute, counted before the code is checked. Live pairings are capped at 10k. Test: `pair::tests::chosen_codes_cannot_be_walked_to_find_another_accounts_pairing`. |
| H3 | `apps/server/src/relay.rs:118` (`Queue`) | Each connection's outgoing queue was unbounded. A device that stops reading, plus a sender within its 4 MiB/s budget (or `/signal` posts, which are not budgeted), grows server memory without limit. | **Fixed.** Queued bytes are counted per connection, capped at 16 MiB. Past the cap the sender gets `offline` and signals fall back to FCM. Test: `relay::tests::a_device_that_stops_reading_cannot_make_the_server_hold_frames_without_end`. |

## Medium

| # | Where | Scenario | Status |
|---|-------|----------|--------|
| M1 | `crates/mdcore/src/device_key.rs:165` (`key_in_cert`) | The device-key pin took the first byte-match of the Ed25519 SPKI header anywhere in the certificate. Someone holding the account key (for example a removed device) could plant a victim's header and key in the subject name of a certificate signed with their own key, and pass for that device. That defeats key-change detection and safety numbers. | **Fixed.** The key is read from the certificate's own SubjectPublicKeyInfo by walking the DER, and must be exactly the header plus 32 bytes. Test: `device_key::tests::a_certificate_for_another_key_cannot_pass_for_a_device_by_naming_its_key`. |
| M2 | Handlers in `features/*` | A device removed from the list (or never in it) could still act over a link that is still up. It could pull and set the clipboard, push files, browse and fetch shared folders, post notifications, open URLs, lock the screen, ring, run speed tests, and so on. | **Partly fixed.** `features::known()` gates clipboard, share, code, browse, gallery, files, notification, call history and call notification, device ring/open_url/action, diag and notif_apps. Tests: `messaging::a_removed_device_can_no_longer_act_here_over_a_link_still_up` and `messaging::a_removed_device_cannot_pull_the_clipboard`. **Still open:** a persistent revocation list (node id plus pinned key) checked in `device_key::check` and in LAN beacon merge (`discovery.rs:120-158`, `mdns.rs:146-152`), so a removed device holding the passphrase is not re-added on the LAN. The UI should also prompt for a passphrase rotation on removal. |
| M3 | `features/kvm.rs:1092` (`on_enter`), `:1058` (`on_layout`) | Any listed device could send `kvm_enter` on any edge and take full keyboard control (keys up to 0xFF). A `kvm_layout` with `at = u64::MAX` won every later layout change. | **Fixed.** Only the computer placed on that edge may enter, and layout times more than 24 h ahead are dropped. Test: `kvm::only_the_computer_placed_on_that_side_can_take_control`. Being controlled is still on by default (see M4). |
| M4 | `features/input.rs:117-120`, `features/screen.rs:74-77`, `features/kvm.rs:296` | Remote input, screen viewing and KVM are on until turned off (`!= Some("off")`). Combined with H1 and M2, this is the largest practical exposure. | **Open.** Changing the defaults, or asking per device on first use, is a product decision. Recommended: default off, plus a first-use prompt. |
| M5 | `crates/mdcore/src/join.rs:197-218` | The 6-digit code's `code_box` is sealed under default Argon2 over only 10^6 codes. The server, or anyone who obtains the box, can open it offline and get the account key. | **Open** (documented design). Recommended: heavier Argon2 parameters, or a PAKE between the two devices. The UI should present QR pairing as the stronger option. |
| M6 | `features/notes.rs:565` (`keep_copy`) | Conflict copies skipped `MAX_NOTES` and `MAX_TOTAL`. One message with many losing versions of a note (64 KB each) grew notes.json without limit. | **Fixed.** Copies must fit the caps. Test: `notes::tests::losing_versions_cannot_grow_the_notes_past_their_limits`. |
| M7 | `features/otp_relay.rs:416` (`expire_later`) | One OS thread, sleeping up to 5 minutes, was spawned per received code. A flood of codes exhausted threads. | **Fixed.** A single sweeper runs while codes exist. Test: `otp_relay::a_flood_of_codes_still_expires_each_one`. |
| M8 | `features/diag.rs:821` (`on_get`) | The `MAX_RUNNING` speed-test check ran before the entry was inserted, so a burst of `speed_get` started many 32 MB sends. The handler also had no device-list check. | **Fixed.** The check and the insert happen under one lock, and only listed devices are served. No dedicated test: the race is timing-dependent. `diag` tests pass. |
| M9 | `features/addr_update.rs`, `features/p2p_ping.rs` | Both took the sender from the body's `from`. Device A could rewrite device B's addresses, ports and capability flags (for example, making B look like a computer that takes OTP codes), and keep a dead link to B marked live. | **Fixed.** Both use `msg.sender()`, the link's own node. Test: `addr_update::a_device_cannot_rewrite_another_devices_address`. |
| M10 | `features/clipboard.rs` | `clipboard_request` answered any requester. A clipboard push had no size cap. | **Fixed.** Listed devices only, and pushes are capped at 256 KB. Pulling while auto-sync is off remains possible by design. Tests: `messaging::an_oversized_clipboard_push_is_ignored` and `messaging::a_removed_device_cannot_pull_the_clipboard`. |
| M11 | `features/notif_apps.rs` (`subscribe_apps`) | No device-list check, and no cap on the number or length of app names. The list is saved to kv, so it grows without limit. | **Fixed.** Listed devices only; package names only (`pkg_ok`), deduplicated, at most 2000. Test: `notif_apps::tests::a_subscription_keeps_only_distinct_package_names_up_to_the_cap`. |
| M12 | `features/{timers,notes,read_later,labels}.rs` handlers | `updated_at` (and timers' `ends_at`) were accepted as any u64. An entry stamped `u64::MAX` wins last-writer-wins for ever. A timer that never ends blocks `start_timer` ("at most 50"). | **Fixed.** Incoming entries more than 24 h ahead (`features::not_ahead`) are dropped, and timers ending past `MAX_DURATION` plus 24 h are dropped. Test: `timers::a_timer_stamped_far_in_the_future_is_left_out`. |
| M13 | `features/remote_download.rs` (`on_request`) | Unlimited `remote_download_request`s: each held a thread for up to 10 minutes and showed a prompt. | **Fixed.** At most 8 open requests per device; further ones are refused with "too many downloads are waiting". Test: `remote_download::a_device_cannot_pile_up_requests`. The HEAD probe still runs before asking, so the prompt can show name and size. It goes only to public addresses (SSRF-checked) and is noted as low. |
| M14 | `features/link_fetch.rs` (fetch loop) | A server sending one byte every 29 s held the single download slot for ever. | **Fixed.** Downloads slower than 1 KB/s over 60 s are given up (and retried with resume). Test: `link_fetch::tests::a_trickling_server_is_given_up_but_a_slow_one_is_not`. |
| M15 | `features/files.rs:1361` (`on_offer`), chunk write | An offer's `size` is any u64 and is accepted without asking. There is no free-space check, so a compromised device can fill the disk. | **Open.** It needs a platform free-space hook (core has no free-space call and no new dependencies were allowed). Recommended: a `Platform::free_space(dir)` check against `size` plus a reserve. |
| M16 | `features/folder_sync.rs` (`offer_dir`) | The sync receive path accepted any file for an active pair, not just files this side had asked for. The other side could stream unrequested files into the synced folder's state directory. | **Fixed.** Only a pending pull with the same transfer id and size is taken. Test: `folder_sync::a_synced_folder_takes_only_the_files_it_asked_for`. |
| M17 | `apps/server/src/auth.rs:243` (`login`), `:269` (`verify`) | Password guessing was unlimited, and Argon2 ran on async workers, so a flood stalled the server. | **Fixed.** 10 wrong passwords per username per 5 min; hashing runs in `spawn_blocking`; username is capped at 64 and password at 256; a missing user costs the same hashing work. Tests: `auth::guess_tests::*`. `/auth/verify` remains callable without a session, but is now rate-limited. |
| M18 | `apps/server/src/pair.rs:273` (`claim`) | 60 bad claims a minute from anywhere put every claim on the server, including QR claims, at 429. | **Open.** Recommended: skip the global limit for a well-formed 64-hex claim, and count only failed code claims toward it. |
| M19 | `apps/server/src/relay.rs` | No idle timeout, no per-account connection cap, and no `node_id` length limit (up to about 5 MB). | **Fixed.** 90 s idle timeout, 16 connections per account, `node_id` at most 128. Tests: `relay::tests::caps_connections_per_account` and `relay::tests::rejects_an_overlong_node_id`. |
| M20 | `apps/server/src/devices.rs:136-201, 277-306` | Device fields (`node_id`, `name`, `platform`, `fcm_token`, `ip`) have no length or format checks, and devices per account are unlimited. | **Open.** Recommended: length caps, parsing `ip` as `IpAddr`, and at most 32 devices per account. |
| M21 | `apps/server/src/signal.rs:29-149` | FCM sends have no rate or payload-size cap. The project's FCM quota can be used up. | **Open.** Recommended: reject payloads over 3.5 KB, and limit each user to about 60 signals a minute. |

## Low

| # | Where | Issue | Status |
|---|-------|-------|--------|
| L1 | `features/link_fetch.rs` `embedded_v4` / `is_public_ip` | IPv4-compatible `::a.b.c.d`, Teredo and local NAT64 counted as public. | **Fixed.** Test: `link_fetch::tests::ipv6_forms_that_carry_a_private_ipv4_are_not_public`. |
| L2 | `apps/desktop/src-tauri/src/ext_bridge.rs` `safe_name` | `CON.png` / `nul.jpg` from a page would open a device on Windows. | **Fixed.** Such names are prefixed with `_`, using `link_fetch::windows_reserved`. Test: `ext_bridge::tests` (safe_name). |
| L3 | `apps/desktop/src-tauri/src/commands_run.rs:352-405` | Any listed device can run any enabled command. There is no injection, but there is no per-device limit either. | Open. Recommended: an optional device list or an "ask each time" flag per command. |
| L4 | `apps/desktop/src-tauri/src/settings_backup.rs:92-104` | A restored backup brings `commands` and `automations` back enabled. | Open. Recommended: restore them disabled and show the full command lines in the preview. |
| L5 | `apps/desktop/src-tauri/src/hotspot.rs:459-487` | The hotspot password is passed in argv to `networksetup` and `nmcli`, so other local users can see it. On Windows the profile XML goes to a predictable `%TEMP%` name. | Open. Recommended: `nmcli --ask` with the password on stdin, and a random, owner-only temp file. |
| L6 | `apps/browser-extension/background.js:117-125` | The background `fetch(src, {credentials:"include"})` sends cookies, and can reach intranet URLs after a right-click. | Open. Recommended: drop `credentials:"include"`. |
| L7 | `apps/desktop/src-tauri/src/remote_input.rs` | An `eprintln!` Debug-printed `InputEvent::Text`, which could contain a typed password. | **Fixed.** Only the event kind is logged. Test: `remote_input::tests::the_log_names_the_kind_never_the_typed_text`. |
| L8 | `crates/mdcore/src/events.rs:19` | `CoreEvent` derives Debug, so `WifiDirectRequest.passphrase` would print with `{ev:?}`. Nothing does this today. | Open. Recommended: a manual Debug that hides it. |
| L9 | `features/otp_relay.rs`, `transport/mod.rs` | Codes are wiped in `VerificationCode` and `Pending`, but the decrypted plaintext and JSON copies are not. | Open. Either wipe after dispatch or soften the claim. |
| L10 | `features/signal_fwd.rs:89-97` | Forwarded signals trust the body's origin `from`, so a spoofed `Bye` can drop another device's link. | Open. Recommended: accept `Bye` only from that device's own link. |
| L11 | `features/files.rs` (`move_into` callers) | Received files carry no quarantine or Zone.Identifier mark, so Gatekeeper and SmartScreen are skipped. | Open. Recommended: set `com.apple.quarantine` / `Zone.Identifier` on desktop. |
| L12 | `features/files.rs:353` `safe_name` | Windows reserved names and bidi or format characters (U+202E) are not handled for received files. | Open. Recommended: reuse `link_fetch::windows_reserved` and `hidden_char`. |
| L13 | `features/files.rs:1479-1515` | After a restart, an offer from another device with a known id truncates the partial file. | Open. Recommended: refuse when the note names another sender. |
| L14 | `features/remote_download.rs:813` | Remote downloads share the partial-file namespace with transfers. | Open. Recommended: `dl-{id}.part`. |
| L15 | `features/print.rs:405`, `features/folder_sync.rs:2451` | Print PDFs are never swept if the app dies first. Folder-sync invites are not capped per device. | Open. |
| L16 | `features/folder_sync.rs:1931` | A two-way peer can trash a whole synced folder by omitting everything from its manifest. It is recoverable for 7 days. | Open (design). Recommended: a "large delete" pause. |
| L17 | `features/{notes,read_later}.rs` merge | Quadratic work per message (`fits`, `answer.contains`). | Open. |
| L18 | `features/nearby.rs`, `audio_share.rs`, `browse.rs` | Debug-build overflow panics on wire timestamps, sequence numbers and offsets. | **Fixed** (wrapping and saturating). Test: `nearby::tests::the_clock_offset_and_latency`. |
| L19 | `features/{timers,read_later,notes,links_map}.rs` | Receivers merge even when their own feature toggle is off. | Open. |
| L20 | `features/screen.rs:305` | `screen_frame` is emitted even when not watching. | Open. Recommended: track a `viewing` set, as camera does. |
| L21 | `apps/server/src/error.rs` | Mongo and FCM error text was returned to clients. | **Fixed.** It is logged, and clients get "internal error". |
| L22 | `apps/server/src/auth.rs` | Login timing revealed which usernames exist. | **Fixed.** Dummy hashing is done for a missing user. |
| L23 | `apps/server/src/auth.rs`, `main.rs:61` | JWTs (7 days) can't be revoked, and there is no minimum `JWT_SECRET` length. | Open. |
| L24 | `apps/server/src/pair.rs`, `relay.rs:203` | Unbounded in-memory maps: pairings, and relay buckets that are never cleared. | Partly fixed (pairings capped at 10k). Buckets are open. |

## Checked and fine

- **Network fetch**
  - Only http and https are allowed.
  - The resolver guard pins checked IPs, which defeats DNS rebinding, and every redirect is checked again.
  - Size caps are enforced while streaming, there is no gzip, and no proxy is taken from the environment.
  - `open_url` and Read later accept http(s) only.
- **Commands**
  - No shell is used on peer data.
  - `lp` gets `--` before the file and only locally listed printers.
  - AppleScript targets only fixed apps with integer values.
  - `netsh` XML is escaped.
  - `commands_run` runs only user-written text.
- **Extension bridge**
  - The Unix socket folder is 0700 and the socket 0600.
  - The Windows token comparison is constant-time.
  - Frames are capped at 1 MB, and requests are a closed enum.
  - Uploads have checked ids and a 25 MB cap.
  - Manifests are pinned to the extension ids.
- **File transfers**
  - `safe_name` and `safe_folder` strip traversal, drive letters and UNC paths.
  - `move_into` claims names with `create_new`, so no file is overwritten.
  - Folder-sync paths are checked with `rel_ok` and `inside()`, and symlinks are not followed.
  - Browse uses canonicalize plus a prefix check.
- **Server**
  - Every device, signal and relay route requires a JWT.
  - Every query is filtered by the token's `user_id`.
  - The relay only forwards within the same account.
  - Pairing codes come from OsRng without modulo bias, live 5 minutes and are single-use.
