// my-domain desktop backend — a thin Tauri adapter over the shared `mdcore`
// Engine. Platform specifics (machine id, hostname, interface enumeration,
// clipboard, session persistence) are implemented here; all networking,
// crypto, discovery, and messaging live in `mdcore`.

use std::net::IpAddr;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use mdcore::capabilities::Clipboard;
use mdcore::engine::{Engine, FirewallStatus, JoinInvite, JoinStatus, SavedSession, SessionInfo};
use mdcore::events::{CoreEvent, EventSink};
use mdcore::kv::FileKv;
use mdcore::model::{Identity, Peer};
use mdcore::platform::{IfaceMode, Platform};
use mdcore::CoreError;

mod account;
mod activity;
mod automations;
mod away_lock;
mod battery_log;
mod calls;
mod chat_history;
mod clip_history;
mod clip_image;
mod clip_privacy;
mod commands_run;
mod computer_audio;
mod computer_notifs;
mod device;
mod device_kind;
mod device_labels;
mod diagnostics;
mod ext_bridge;
mod fcm;
mod health;
mod hotspot;
mod insert_capture;
mod key_changes;
mod kvm;
mod kvm_keys;
mod kvm_os;
mod links_map;
mod location;
mod media_ctl;
mod mute_here;
mod nearby;
mod note_reminders;
mod notes;
mod notif_history;
mod notif_popup;
mod notify;
mod otp_codes;
mod page_layout;
mod peer_perms;
mod phone_audio;
mod phone_notifs;
mod phone_screen;
mod print;
mod quick_panel;
mod quiet;
mod read_later;
mod remote_download;
mod remote_input;
mod schedule;
mod screen_share;
mod search;
mod send_to;
mod settings_backup;
mod shortcut;
mod shortcuts;
mod sync_folders;
mod sysinfo;
mod timers;
mod transfers;
mod tray;
mod tray_battery;
mod updates;
mod wallpaper;
mod webhooks;
mod wifi_direct;
mod wifi_share;
mod wol;

pub(crate) type Eng = Engine<DesktopPlatform>;

/// The managed engine, reachable from other threads (md-ble's scanner).
#[derive(Clone)]
struct EngineHandle(AppHandle);

impl std::ops::Deref for EngineHandle {
    type Target = Eng;
    fn deref(&self) -> &Eng {
        self.0.state::<Eng>().inner()
    }
}

// ---------------------------------------------------------------------------
// Platform implementation
// ---------------------------------------------------------------------------

/// System clipboard via arboard.
///
/// The handle is kept ALIVE for the process lifetime, not recreated per call.
/// On Linux this is mandatory: X11/Wayland clipboards are served by the *owning*
/// process, so a transient handle would lose the data the instant it's dropped
/// (right after `set`) — which is exactly why pasting on Linux failed. A
/// persistent owner keeps serving the selection. macOS/Windows copy into a
/// system pasteboard, so this is merely an optimization there.
struct DesktopClipboard {
    inner: std::sync::Mutex<Option<arboard::Clipboard>>,
}

impl DesktopClipboard {
    fn new() -> Self {
        DesktopClipboard {
            inner: std::sync::Mutex::new(arboard::Clipboard::new().ok()),
        }
    }
}

/// Wayland with no clipboard arboard can reach (no data-control protocol
/// and no XWayland): `wl-paste` / `wl-copy` instead, when installed.
#[cfg(target_os = "linux")]
fn wl_clipboard_fallback() -> bool {
    md_portal::Session::wayland() && md_portal::service::caps().1.wl_clipboard
}

impl Clipboard for DesktopClipboard {
    fn get(&self) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        if g.is_none() {
            *g = arboard::Clipboard::new().ok();
        }
        #[cfg(target_os = "linux")]
        if g.is_none() && wl_clipboard_fallback() {
            return md_portal::service::wl_paste();
        }
        g.as_mut().and_then(|c| c.get_text().ok())
    }
    fn set(&self, text: &str) {
        let mut g = self.inner.lock().unwrap();
        if g.is_none() {
            *g = arboard::Clipboard::new().ok();
        }
        #[cfg(target_os = "linux")]
        if g.is_none() && wl_clipboard_fallback() {
            md_portal::service::wl_copy(text);
            return;
        }
        if let Some(c) = g.as_mut() {
            let _ = c.set_text(text.to_owned());
        }
    }
    /// A password copied from a password manager, or a copy from an
    /// excluded app (`clip_privacy`).
    fn keep_private(&self) -> bool {
        clip_privacy::keep_private_now()
    }
}

struct DesktopPlatform {
    /// Session store: `<app config dir>/session.json` (same file and format as
    /// before, so existing saved sessions keep working). `None` if the OS
    /// gave us no config dir.
    kv: Option<FileKv>,
    /// Debug builds keep the session secrets here instead of the OS keychain
    /// (`<app config dir>/dev-secrets.json`, owner-only). Every rebuild of an
    /// unsigned dev app looks like a new app to macOS, which then asks for
    /// the login password before each keychain read. Release builds (signed,
    /// with a stable identity) use the keychain. `None` in release builds.
    dev_secrets: Option<FileKv>,
    clipboard: DesktopClipboard,
    /// Set by the FCM receiver thread once Firebase issues a token; reported to
    /// the registry on refresh so siblings can signal this device.
    fcm_token: Arc<std::sync::Mutex<Option<String>>>,
}

/// Make a secrets file readable by this user only.
fn owner_only(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The OS secure store (macOS Keychain, Secret Service on Linux, Windows
/// Credential Manager) entry for one of the core's secrets. Lab instances on
/// one machine (`TEST=...`) each get their own.
fn keyring_entry(key: &str) -> Option<keyring::Entry> {
    let service = match test_suffix() {
        Some(suffix) => format!("my-domain-{suffix}"),
        None => "my-domain".to_string(),
    };
    keyring::Entry::new(&service, key)
        .map_err(|e| eprintln!("[secrets] keyring unavailable: {e}"))
        .ok()
}

/// Debug builds' secrets file (see `DesktopPlatform::dev_secrets`); `None`
/// in release builds, which use the OS secure store.
fn dev_secrets_file(app: &AppHandle) -> Option<FileKv> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let dir = app.path().app_config_dir().ok()?;
    let name = match test_suffix() {
        Some(suffix) => format!("dev-secrets-{suffix}.json"),
        None => "dev-secrets.json".to_string(),
    };
    Some(FileKv::new(dir.join(name)))
}

/// A secret, from the debug builds' file (`dev`) or the OS secure store.
fn read_secret(dev: Option<&FileKv>, key: &str) -> Option<String> {
    if let Some(file) = dev {
        return file.get(key);
    }
    keyring_entry(key)?.get_password().ok()
}

/// Keep (or, with `None`, forget) a secret where `read_secret` finds it.
fn write_secret(dev: Option<&FileKv>, key: &str, value: Option<&str>) {
    if let Some(file) = dev {
        let result = match value {
            Some(v) => file.set(key, v).and_then(|()| owner_only(file.path())),
            None => file.remove(key),
        };
        if let Err(e) = result {
            eprintln!("[secrets] {key}: {e}");
        }
        return;
    }
    let Some(entry) = keyring_entry(key) else {
        return;
    };
    let result = match value {
        Some(v) => entry.set_password(v),
        None => match entry.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        },
    };
    if let Err(e) = result {
        eprintln!("[secrets] {key}: {e}");
    }
}

/// One of the app's own secrets (the webhook token), kept where the core's
/// are: the OS secure store, or the debug builds' file.
pub(crate) fn app_secret(app: &AppHandle, key: &str) -> Option<String> {
    read_secret(dev_secrets_file(app).as_ref(), key)
}

pub(crate) fn set_app_secret(app: &AppHandle, key: &str, value: Option<&str>) {
    write_secret(dev_secrets_file(app).as_ref(), key, value)
}

/// Optional test suffix (`TEST=foo npm run tauri dev`) so several instances on
/// one machine register as distinct devices instead of clobbering the same
/// machine-uid row. Trimmed; empty/unset => no suffix.
fn test_suffix() -> Option<String> {
    std::env::var("TEST")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl Platform for DesktopPlatform {
    fn device_id(&self) -> String {
        let base = machine_uid::get().unwrap_or_default();
        match test_suffix() {
            Some(suffix) => format!("{base}-{suffix}"),
            None => base,
        }
    }

    fn device_name(&self) -> String {
        let base = gethostname::gethostname().to_string_lossy().to_string();
        match test_suffix() {
            Some(suffix) => format!("{base} [{suffix}]"),
            None => base,
        }
    }

    fn platform_kind(&self) -> &'static str {
        "desktop"
    }

    fn shares_input(&self) -> bool {
        true
    }

    fn iface_mode(&self) -> IfaceMode {
        let mut v4_addrs = Vec::new();
        if let Ok(ifaces) = if_addrs::get_if_addrs() {
            for iface in ifaces {
                if iface.is_loopback() {
                    continue;
                }
                if let IpAddr::V4(v4) = iface.ip() {
                    v4_addrs.push(v4);
                }
            }
        }
        IfaceMode::All { v4_addrs }
    }

    fn kv_get(&self, key: &str) -> Option<String> {
        self.kv.as_ref()?.get(key)
    }

    fn secret_get(&self, key: &str) -> Option<String> {
        read_secret(self.dev_secrets.as_ref(), key)
    }

    fn secret_set(&self, key: &str, value: Option<&str>) {
        write_secret(self.dev_secrets.as_ref(), key, value)
    }

    fn kv_set(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                eprintln!("[kv] failed to save {key}: {e}");
            }
        }
    }

    // Notes (`notes.json`) beside the session file, in the app data dir.
    fn data_file(&self, name: &str) -> Option<std::path::PathBuf> {
        Some(self.kv.as_ref()?.path().with_file_name(name))
    }

    fn fcm_token(&self) -> Option<String> {
        self.fcm_token.lock().unwrap().clone()
    }

    fn clipboard(&self) -> Option<&dyn Clipboard> {
        Some(&self.clipboard)
    }
}

// ---------------------------------------------------------------------------
// Event sink — bridge core events to the existing Tauri event names/payloads.
// ---------------------------------------------------------------------------

struct TauriSink {
    app: AppHandle,
}

impl EventSink for TauriSink {
    fn emit(&self, ev: CoreEvent) {
        // Event names and payload shapes are defined (and contract-tested) in
        // mdcore::events, so this adapter no longer hand-builds JSON per event.
        // Debug builds also log every event, so the device lab can verify
        // delivery from the log without looking at the window.
        // A phone's audio, many frames a second: played here, never logged
        // or handed to the window.
        if let CoreEvent::AudioFrame {
            from,
            seq,
            rate,
            channels,
            pcm,
        } = &ev
        {
            phone_audio::on_frame(from, *seq, *rate, *channels, pcm);
            return;
        }
        // Share mouse and keyboard: the controlling computer's input, many
        // events a second and keys typed: played here, never logged or
        // handed to the window.
        if let CoreEvent::KvmInput { events, .. } = &ev {
            kvm::on_input(events.clone());
            return;
        }
        // Wi-Fi Direct: the core asks this computer to host a group (or
        // leave it). It carries a passphrase: never logged or handed to the
        // window.
        if let CoreEvent::WifiDirectRequest {
            sid,
            node_id,
            method,
            op,
            ssid,
            passphrase,
            host_ip,
        } = &ev
        {
            wifi_direct::on_request(
                &self.app,
                sid,
                node_id,
                method,
                op,
                ssid.as_deref(),
                passphrase.as_deref(),
                host_ip.as_deref(),
            );
            return;
        }
        // A verification code from your phone: never logged, never in
        // Activity or any history (`activity_record` skips it too); shown
        // here (`otp_codes`) and in the window only.
        if let CoreEvent::VerificationCode(c) = &ev {
            otp_codes::on_code(&self.app, c);
            let _ = self.app.emit(ev.name(), ev.ui_payload());
            return;
        }
        if cfg!(debug_assertions) {
            let payload = ev.ui_payload().to_string();
            let short: String = payload.chars().take(200).collect();
            eprintln!("[event] {} {short}", ev.name());
        }
        // Ring / open-link need the desktop to act, not just the UI. Off the
        // core's thread: these look up peers and talk to the OS.
        activity_record(&self.app, &ev);
        match &ev {
            // A stream moved to (or off) a nearby link: the phone's audio
            // here may buffer less.
            CoreEvent::NearbyMedia(s) => phone_audio::on_nearby(&self.app, s),
            // The tray's device submenus follow the peer list and link state,
            // and automations wait for devices to connect or disconnect.
            CoreEvent::PeersUpdated { .. }
            | CoreEvent::PeerConnected { .. }
            | CoreEvent::PeerDisconnected { .. } => {
                if let CoreEvent::PeerDisconnected { node_id } = &ev {
                    phone_audio::on_disconnected(&self.app, node_id);
                    computer_audio::on_disconnected(&self.app, node_id);
                }
                // A phone back: it hears this computer's subscription and
                // settings again (an older phone forgets them on restart).
                if let CoreEvent::PeerConnected { node_id, .. } = &ev {
                    let (app, node) = (self.app.clone(), node_id.clone());
                    std::thread::spawn(move || phone_notifs::resubscribe(&app, &node));
                }
                tray::refresh(&self.app);
                tray_battery::update(&self.app);
                automations::on_link(&self.app);
                quiet::on_link(&self.app);
            }
            // Which of your devices are phones: lock when they go away,
            // where the copy shortcut sends, and the tray's phone actions.
            // Automations watch battery and charging, and the battery graph
            // keeps a sample; webhooks list their actions.
            CoreEvent::DeviceStatus { from, status } => {
                away_lock::on_status(&self.app, from, status);
                automations::on_status(&self.app, from, status);
                battery_log::on_status(&self.app, from, status);
                quiet::on_status(&self.app, from, status);
                webhooks::on_status(&self.app, from, status);
                shortcut::saw_status(&self.app, from, status);
                shortcuts::on_status(&self.app, from, status);
                tray::on_status(&self.app, from, status);
                tray_battery::on_status(&self.app, from, status);
            }
            CoreEvent::Ring { from } => {
                let (app, from) = (self.app.clone(), from.clone());
                std::thread::spawn(move || device::on_ring(&app, &from));
            }
            CoreEvent::OpenUrl { url, .. } => {
                let (app, url) = (self.app.clone(), url.clone());
                std::thread::spawn(move || device::on_open_url(&app, &url));
            }
            CoreEvent::Action { from, name, args } => {
                let (app, from, name, args) =
                    (self.app.clone(), from.clone(), name.clone(), args.clone());
                std::thread::spawn(move || device::on_action(&app, &from, &name, &args));
            }
            // The phone's touchpad: queued to one thread, so the order holds.
            CoreEvent::Input { events, .. } => remote_input::play(&self.app, events.clone()),
            // Share mouse and keyboard: capture this computer's input, or
            // play another's, or let go of everything.
            CoreEvent::KvmChanged { state } => kvm::on_changed(&self.app, state),
            CoreEvent::ScreenWatch { from, on } => {
                let (app, from, on) = (self.app.clone(), from.clone(), *on);
                std::thread::spawn(move || screen_share::on_watch(&app, &from, on));
            }
            // A device asked for what it isn't allowed: the window comes
            // up for the owner's answer.
            CoreEvent::PermissionAsk { .. } => peer_perms::on_ask(&self.app),
            // A phone shares its screen with this computer: bring its page up.
            CoreEvent::ScreenOffer { from, on } => {
                let (app, from, on) = (self.app.clone(), from.clone(), *on);
                std::thread::spawn(move || phone_screen::on_offer(&app, &from, on));
            }
            // A phone plays its audio here (or stopped): off the core's
            // thread (it opens the output, posts a notice).
            CoreEvent::AudioOffer { from, on, .. } => {
                let (app, from, on) = (self.app.clone(), from.clone(), *on);
                std::thread::spawn(move || phone_audio::on_offer(&app, &from, on));
            }
            // A phone plays this computer's sound (or stopped, or said no):
            // off the core's thread (it starts the capture, posts a notice).
            CoreEvent::AudioListen { from, on } => {
                let (app, from, on) = (self.app.clone(), from.clone(), *on);
                std::thread::spawn(move || computer_audio::on_listen(&app, &from, on));
            }
            CoreEvent::Dnd { from, on } => quiet::on_dnd(&self.app, from, *on),
            // Print from the phone: its printers, and a job to print (the
            // transfer's row ends as received; the PDF is deleted after).
            CoreEvent::PrintersRequested { from } => print::on_printers_requested(&self.app, from),
            CoreEvent::PrintJob {
                id,
                from,
                name,
                path,
                size,
                options,
            } => {
                transfers::on_received(&self.app, id, from, name, path, *size);
                print::on_job(&self.app, id, from, name, path, options);
            }
            // Labels set on your other devices: take them here.
            CoreEvent::LabelsChanged { labels } => device_labels::apply_shared(&self.app, labels),
            // A device asks to sync a folder: say so.
            CoreEvent::SyncPairs { pairs } => sync_folders::on_pairs(&self.app, pairs),
            // A reminder set, snoozed or Done on another device.
            CoreEvent::NotesChanged { .. } => note_reminders::changed(&self.app),
            // A timer started or changed on another device: ring it here
            // when it ends, if it rings here.
            CoreEvent::TimersChanged { .. } => timers::changed(&self.app),
            // OS notifications for what arrives while the window is in the
            // background (off the core's thread too: peer lookup, the OS).
            // Kept in the chat history too (`chat_history`).
            CoreEvent::MessageReceived(m) if m.ok => {
                if let Some(node) = &m.node_id {
                    chat_history::on_received(&self.app, node, m.id.as_deref(), &m.text);
                }
                let app = self.app.clone();
                let (node, from, text) = (m.node_id.clone(), m.from.clone(), m.text.clone());
                std::thread::spawn(move || {
                    let title = match &node {
                        Some(node) => notify::device_name(&app, node),
                        None => from,
                    };
                    notify::post_if_away(&app, &title, &text);
                });
            }
            CoreEvent::MessageDelivered { id, .. } => chat_history::on_delivered(&self.app, id),
            CoreEvent::CodeCopied { from, app: src, .. } => {
                let (app, from, src) = (self.app.clone(), from.clone(), src.clone());
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    let body = match src {
                        Some(s) if !s.is_empty() => {
                            format!("A code from {s} on {device} is on your clipboard")
                        }
                        _ => format!("A code from {device} is on your clipboard"),
                    };
                    notify::post(&app, "Code copied", &body);
                });
            }
            // Text another device sent: the core already put a clipboard
            // one on the clipboard; say so (and keep it in the clipboard
            // history), or show a "notify" one.
            // One marked sensitive (a password sent anyway) is neither kept
            // nor shown.
            CoreEvent::TextShared {
                from,
                text,
                mode,
                sensitive,
            } => {
                if mode == "clipboard" && !sensitive {
                    clip_history::on_received(&self.app, from, text);
                }
                let app = self.app.clone();
                let (from, text, mode, sensitive) =
                    (from.clone(), text.clone(), mode.clone(), *sensitive);
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    let short: String = text.chars().take(200).collect();
                    if mode == "clipboard" && sensitive {
                        notify::post(
                            &app,
                            &format!("Copied from {device}"),
                            "A password is on your clipboard.",
                        );
                    } else if mode == "clipboard" {
                        notify::post(&app, &format!("Copied from {device}"), &short);
                    } else {
                        notify::post(&app, &format!("From {device}"), &short);
                    }
                });
            }
            // Every transfer is kept in the Transfers list (`transfers`),
            // both ways, before the window hears of it.
            CoreEvent::FileProgress {
                id,
                node_id,
                name,
                done,
                total,
                incoming,
                resumed,
                batch,
            } => transfers::on_progress(
                &self.app,
                id,
                node_id,
                name,
                *done,
                *total,
                *incoming,
                *resumed,
                batch.as_deref(),
            ),
            CoreEvent::FilePaused { id, paused, .. } => {
                transfers::on_paused(&self.app, id, *paused);
            }
            // A folder or several files: one notice when they're all in.
            CoreEvent::TransferBatch { item } => {
                let ended_here = item.incoming && item.state == "done";
                let new = transfers::get_state(&self.app, &item.id);
                transfers::on_batch(&self.app, item);
                if ended_here && new.as_deref() != Some("done") {
                    let (app, item) = (self.app.clone(), item.clone());
                    std::thread::spawn(move || {
                        let device = notify::device_name(&app, &item.node_id);
                        let what = if item.kind == "folder" {
                            format!("the folder {}", item.name)
                        } else {
                            format!("{} files", item.count)
                        };
                        notify::post_if_away(
                            &app,
                            "my-domain",
                            &format!("Saved {what} from {device}"),
                        );
                    });
                }
            }
            CoreEvent::FileSent { id, node_id, name } => {
                transfers::on_sent(&self.app, id, node_id, name);
            }
            // The transfer rows with that device say "via Wi-Fi Direct".
            CoreEvent::WifiDirect {
                node_id,
                active,
                via,
            } => {
                transfers::on_wifi_direct(&self.app, node_id, *active, via);
            }
            CoreEvent::FileFailed {
                id,
                node_id,
                name,
                incoming,
                reason,
            } => transfers::on_failed(&self.app, id, node_id, name, *incoming, reason),
            // Photo backups and synced folders' files arrive by themselves,
            // often many: no notice each; nor a batch's files (the batch
            // has one).
            CoreEvent::FileReceived {
                id,
                from,
                name,
                path,
                size,
                sync,
                ..
            } if *sync || is_backup(path) || transfers::in_batch(&self.app, id) => {
                transfers::on_received(&self.app, id, from, name, path, *size);
            }
            // A picture copied on another device: on this clipboard too.
            CoreEvent::FileReceived {
                id,
                from,
                name,
                path,
                size,
                clipboard: true,
                ..
            } => {
                transfers::on_received(&self.app, id, from, name, path, *size);
                clip_history::on_picture(
                    &self.app,
                    clip_history::ClipDirection::In,
                    from,
                    std::path::Path::new(path),
                );
                let (app, from, path) = (self.app.clone(), from.clone(), path.clone());
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    match clip_image::put_on_clipboard(std::path::Path::new(&path)) {
                        Ok(()) => notify::post_if_away(
                            &app,
                            "my-domain",
                            &format!("Copied a picture from {device}"),
                        ),
                        Err(why) => notify::post(
                            &app,
                            &format!("Couldn't copy the picture from {device}"),
                            &why,
                        ),
                    }
                });
            }
            // A link downloaded here ("Download on <device>"): its own
            // notice came with its `RemoteDownload` event.
            CoreEvent::FileReceived {
                id,
                from,
                name,
                path,
                size,
                ..
            } if transfers::is_link(&self.app, id) => {
                transfers::on_received(&self.app, id, from, name, path, *size);
            }
            CoreEvent::RemoteDownload { item } => remote_download::on_event(&self.app, item),
            CoreEvent::FileReceived {
                id,
                from,
                name,
                path,
                size,
                ..
            } => {
                transfers::on_received(&self.app, id, from, name, path, *size);
                let (app, from, name) = (self.app.clone(), from.clone(), name.clone());
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    notify::post_if_away(&app, "my-domain", &format!("Saved {name} from {device}"));
                });
            }
            // Insert from phone: the photo or scan this computer asked for
            // (Downloads, the clipboard, maybe pasted where the user is).
            CoreEvent::InsertReceived {
                id,
                from,
                mode,
                name,
                path,
                size,
            } => insert_capture::on_received(&self.app, id, from, mode, name, path, *size),
            CoreEvent::InsertEnded {
                node_id, reason, ..
            } => insert_capture::on_ended(&self.app, node_id, reason),
            // A computer has no camera to take one with: say so at once.
            CoreEvent::InsertRequest { id, .. } => {
                if let Some(engine) = self.app.try_state::<Eng>() {
                    engine.insert_cancel(id, "unsupported");
                }
            }
            // One of your devices plays, pauses, skips or seeks here.
            CoreEvent::MediaCommand {
                from,
                action,
                value,
                ..
            } => media_ctl::on_command(&self.app, from, action, *value),
            CoreEvent::KeyChanged { node_id, .. } => {
                let (app, node) = (self.app.clone(), node_id.clone());
                std::thread::spawn(move || key_changes::on_changed(&app, &node));
            }
            CoreEvent::BatteryAlert { from, alert, level } => {
                let (app, from, alert, level) =
                    (self.app.clone(), from.clone(), alert.clone(), *level);
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    let (title, body) = match alert.as_str() {
                        "low" => (
                            format!("{device}: battery low"),
                            format!("{level}% left. Time to charge it."),
                        ),
                        "charged" => (
                            format!("{device} is charged to {level}%"),
                            "You can unplug it now.".to_string(),
                        ),
                        _ => (
                            format!("{device} is charged"),
                            "The battery is full.".to_string(),
                        ),
                    };
                    notify::post(&app, &title, &body);
                });
            }
            // Gone on the phone: marked dismissed in the history before the
            // window hears of it (it drops it from the active list).
            CoreEvent::NotificationRemoved { from, key } => {
                notif_history::on_dismissed(&self.app, from, key);
                phone_notifs::on_removed(&self.app, from, key);
            }
            // An app icon the phone was asked for: kept.
            CoreEvent::NotificationIcon {
                from,
                pkg,
                hash,
                data,
            } => phone_notifs::on_icon(&self.app, from, pkg, hash, data),
            // Swiped away on a phone showing this computer's notifications:
            // closed here too, if it's one of ours still on screen.
            CoreEvent::NotificationAction { key, index: -1, .. } => {
                computer_notifs::on_dismiss(&self.app, key);
            }
            // Kept (the list and the history) before the window hears of
            // it, private mode applied; a muted app's get no further, and
            // the window gets what's shown here, never more.
            CoreEvent::Notification {
                from,
                title,
                body,
                app: source,
                key,
                actions,
                extra,
            } => {
                let Some(arrived) = phone_notifs::on_notification(
                    &self.app,
                    from,
                    source.as_deref(),
                    title,
                    body,
                    key.as_deref(),
                    actions,
                    extra,
                ) else {
                    return;
                };
                let n = &arrived.notif;
                let shown = serde_json::json!({
                    "from": n.from, "title": n.title, "body": n.body, "app": n.app,
                    "key": n.key, "private": n.private,
                });
                if arrived.pop_up {
                    let app = self.app.clone();
                    std::thread::spawn(move || {
                        notif_popup::show(&app, &arrived.notif, arrived.icon_path)
                    });
                }
                let _ = self.app.emit(ev.name(), shown);
                return;
            }
            _ => {}
        }
        let _ = self.app.emit(ev.name(), ev.ui_payload());
    }
}

/// What the Activity timeline keeps of an event that no history keeps:
/// links coming and going, calls, rings, links opened here, screens and
/// sounds shared (`activity.rs`).
fn activity_record(app: &AppHandle, ev: &CoreEvent) {
    let on_off = |on: bool| if on { "on" } else { "off" };
    match ev {
        CoreEvent::PeerConnected { node_id, transport } => activity::record(
            app,
            "link_up",
            Some(node_id),
            "",
            &format!("{transport:?}").to_lowercase(),
        ),
        CoreEvent::PeerDisconnected { node_id } => {
            activity::record(app, "link_down", Some(node_id), "", "")
        }
        // Verification codes are never kept anywhere: not here either.
        CoreEvent::VerificationCode(_) | CoreEvent::VerificationCodeGone { .. } => {}
        CoreEvent::CallNotification {
            from,
            caller,
            state,
            ..
        } => activity::record(app, "call", Some(from), caller, state),
        CoreEvent::Ring { from } => activity::record(app, "ring", Some(from), "", ""),
        CoreEvent::OpenUrl { from, url, .. } => {
            activity::record(app, "open_url", Some(from), url, "")
        }
        CoreEvent::ScreenWatch { from, on } => {
            activity::record(app, "screen_watch", Some(from), "", on_off(*on))
        }
        CoreEvent::ScreenOffer { from, on } => {
            activity::record(app, "screen_offer", Some(from), "", on_off(*on))
        }
        CoreEvent::PermissionAsk { from, perm } => {
            activity::record(app, "permission_ask", Some(from), perm, "")
        }
        CoreEvent::AudioOffer { from, on, .. } => {
            activity::record(app, "audio_offer", Some(from), "", on_off(*on))
        }
        CoreEvent::AudioListen { from, on } => {
            activity::record(app, "audio_listen", Some(from), "", on_off(*on))
        }
        _ => {}
    }
}

/// A file saved by the photo backup (`<downloads>/Phone backup/...`).
fn is_backup(path: &str) -> bool {
    std::path::Path::new(path)
        .components()
        .any(|c| c.as_os_str() == mdcore::features::backup::FOLDER)
}

// ---------------------------------------------------------------------------
// Tauri commands (thin wrappers over the Engine)
// ---------------------------------------------------------------------------

#[tauri::command]
#[specta::specta]
fn get_identity(engine: State<Eng>) -> Identity {
    engine.identity()
}

#[tauri::command]
#[specta::specta]
fn get_peers(engine: State<Eng>) -> Vec<Peer> {
    engine.get_peers()
}

#[tauri::command]
#[specta::specta]
fn get_saved_session(engine: State<Eng>) -> SavedSession {
    engine.saved_session()
}

/// Device-lab test account, from `MD_DEV_SERVER` / `MD_DEV_USER` /
/// `MD_DEV_PASS` / `MD_DEV_KEY` (see scripts/lab/test-creds.env). The login
/// screen fills itself in and logs in when these are set. Debug builds only:
/// a release build never reads credentials from the environment.
#[derive(serde::Serialize, specta::Type)]
struct DevCredentials {
    server: String,
    user: String,
    pass: String,
    key: String,
}

#[tauri::command]
#[specta::specta]
fn dev_credentials() -> Option<DevCredentials> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Some(DevCredentials {
        server: var("MD_DEV_SERVER")?,
        user: var("MD_DEV_USER")?,
        pass: var("MD_DEV_PASS")?,
        key: var("MD_DEV_KEY")?,
    })
}

/// Live links with their measured quality (RTT, loss, "Excellent"/"Good"/
/// "Weak connection").
#[tauri::command]
#[specta::specta]
fn link_stats(engine: State<Eng>) -> Vec<mdcore::transport::quality::LinkStats> {
    engine.link_stats()
}

#[tauri::command]
#[specta::specta]
fn is_ready(engine: State<Eng>) -> bool {
    engine.is_ready()
}

#[tauri::command]
#[specta::specta]
fn session_info(engine: State<Eng>) -> SessionInfo {
    engine.session_info()
}

#[tauri::command]
#[specta::specta]
fn auth_register(
    server_url: String,
    username: String,
    password: String,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.auth_register(&server_url, &username, &password)
}

#[tauri::command]
#[specta::specta]
fn auth_login(
    server_url: String,
    username: String,
    password: String,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.auth_login(&server_url, &username, &password)
}

#[tauri::command]
#[specta::specta]
fn set_encryption_key(passphrase: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.set_encryption_key(&passphrase)
}

#[tauri::command]
#[specta::specta]
fn update_encryption_key(
    new_passphrase: String,
    password: String,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.update_encryption_key(&new_passphrase, &password)
}

#[tauri::command]
#[specta::specta]
fn generate_key(engine: State<Eng>) -> String {
    engine.generate_key()
}

/// Run a blocking engine call (it talks to the server) off the main thread.
async fn off_main<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Eng) -> Result<T, CoreError> + Send + 'static,
) -> Result<T, CoreError> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<Eng>()))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

/// "Add a device": a one-time QR code (and 6-digit code), valid 5 minutes.
#[tauri::command]
#[specta::specta]
async fn join_invite(app: AppHandle) -> Result<JoinInvite, CoreError> {
    off_main(app, |e| e.join_invite()).await
}

/// Has the new device joined yet ("waiting", "joined" + name, "none")?
#[tauri::command]
#[specta::specta]
async fn join_status(app: AppHandle) -> Result<JoinStatus, CoreError> {
    off_main(app, |e| e.join_status()).await
}

/// The QR was closed: withdraw the invite.
#[tauri::command]
#[specta::specta]
async fn join_cancel(app: AppHandle) -> Result<(), CoreError> {
    off_main(app, |e| e.join_cancel()).await
}

/// Sign in with the 6-digit code another device shows.
#[tauri::command]
#[specta::specta]
async fn join_with_code(app: AppHandle, server_url: String, code: String) -> Result<(), CoreError> {
    off_main(app, move |e| e.join_with_code(&server_url, &code)).await
}

/// Sign in with a pasted `mydomain://join?…` link.
#[tauri::command]
#[specta::specta]
async fn join_with_link(app: AppHandle, link: String) -> Result<(), CoreError> {
    off_main(app, move |e| e.join_with_link(&link)).await
}

#[tauri::command]
#[specta::specta]
fn logout(engine: State<Eng>) {
    engine.logout()
}

#[tauri::command]
#[specta::specta]
fn refresh_from_server(engine: State<Eng>) -> Result<(), CoreError> {
    engine.refresh_from_server()
}

#[tauri::command]
#[specta::specta]
fn scan_lan(engine: State<Eng>) {
    engine.scan_lan()
}

#[tauri::command]
#[specta::specta]
fn send_message(
    node_id: String,
    text: String,
    id: Option<String>,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    // The UI picks the id so it can match the delivery receipt to its bubble.
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Kept before it goes, so a fast receipt finds it in the history.
    chat_history::on_sending(&app, &node_id, &id, &text);
    let result = engine.send_with_id(&node_id, &text, &id);
    chat_history::on_send_result(&app, &id, &result);
    result
}

#[tauri::command]
#[specta::specta]
fn enable_clipboard_sync(engine: State<Eng>) {
    engine.enable_clipboard_sync()
}

#[tauri::command]
#[specta::specta]
fn disable_clipboard_sync(engine: State<Eng>) {
    engine.disable_clipboard_sync()
}

/// Recent clipboard values from your devices, newest first, as JSON
/// (`[{text, from?, ts}]`; the same list the `clipboard-history` event
/// carries).
#[tauri::command]
#[specta::specta]
fn clipboard_history(engine: State<Eng>) -> String {
    serde_json::to_string(&engine.clipboard_history()).unwrap_or_else(|_| "[]".into())
}

#[tauri::command]
#[specta::specta]
fn clear_clipboard_history(engine: State<Eng>) {
    engine.clear_clipboard_history()
}

/// Put a value (e.g. from the history) on this computer's clipboard.
#[tauri::command]
#[specta::specta]
fn copy_to_clipboard(text: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.copy_to_clipboard(&text)
}

#[tauri::command]
#[specta::specta]
fn clipboard_sync_enabled(engine: State<Eng>) -> bool {
    engine.clipboard_sync_enabled()
}

#[tauri::command]
#[specta::specta]
fn get_clipboard(node_id: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.get_clipboard(&node_id)
}

#[tauri::command]
#[specta::specta]
fn add_manual_peer(
    name: String,
    ip: String,
    quic_port: u16,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.add_manual_peer(&name, &ip, quic_port)
}

#[tauri::command]
#[specta::specta]
fn remove_peer(node_id: String, engine: State<Eng>) {
    engine.remove_peer(&node_id)
}

#[tauri::command]
#[specta::specta]
fn share_notification(title: String, body: String, app: Option<String>, engine: State<Eng>) {
    engine.share_notification(&title, &body, app.as_deref())
}

#[tauri::command]
#[specta::specta]
fn share_call_notification(
    caller: String,
    number: Option<String>,
    state: String,
    engine: State<Eng>,
) {
    engine.share_call_notification(&caller, number.as_deref(), &state)
}

#[tauri::command]
#[specta::specta]
fn share_call_history(entries_json: String, engine: State<Eng>) {
    engine.share_call_history(&entries_json)
}

// --- signaling / connection setup ---

#[tauri::command]
#[specta::specta]
fn connect(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.connect(&node_id)
}

/// Close the link to a device now (it comes back when needed).
#[tauri::command]
#[specta::specta]
fn disconnect(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.disconnect(&node_id)
}

/// Close the link to a device and connect afresh.
#[tauri::command]
#[specta::specta]
fn reconnect(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.reconnect(&node_id)
}

#[tauri::command]
#[specta::specta]
fn firewall_check(engine: State<Eng>) -> Result<FirewallStatus, CoreError> {
    engine.firewall_check()
}

/// Round-trip FCM self-test: pushes a ping to our own token via the server. The
/// pong arrives through the FCM receiver (see `fcm.rs`) as an `fcm-event`.
/// If FCM rejects our token (expired/revoked registration), register a new one;
/// the frontend retries the self-test a few seconds later.
#[tauri::command]
#[specta::specta]
fn fcm_selftest(
    app: AppHandle,
    engine: State<Eng>,
    fcm: State<fcm::FcmToken>,
) -> Result<String, CoreError> {
    match engine.fcm_selftest() {
        Err(e) if e.to_string().contains("token invalid") => {
            fcm::reregister(app, fcm.0.clone());
            Err(CoreError::Other(
                "fcm token rejected; re-registering".into(),
            ))
        }
        r => r,
    }
}

/// Run + persist this device's inbound firewall status (background) so siblings
/// see our reachability via `GET /devices`. Called on startup after login.
#[tauri::command]
#[specta::specta]
fn report_firewall(engine: State<Eng>) {
    engine.report_firewall()
}

/// Feed an inbound signal (when a desktop signal channel — e.g. SSE — is wired).
#[tauri::command]
#[specta::specta]
fn on_signal(from: String, payload: String, engine: State<Eng>) {
    engine.on_signal(&from, &payload)
}

// --- app-notification pub/sub (consumer side) ---

#[tauri::command]
#[specta::specta]
fn request_apps(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.request_apps(&node_id)
}

#[tauri::command]
#[specta::specta]
fn subscribe_apps(node_id: String, apps_json: String, app: AppHandle) -> Result<(), CoreError> {
    // With this computer's settings (private mode, only while not in use).
    phone_notifs::subscribe(&app, &node_id, &apps_json)
}

/// Press one of a shared phone notification's buttons (`index`, in the order
/// the notification listed them), reply to it (`text`, for a `reply` button),
/// or dismiss it (`index` -1). Returns the transport, or "queued".
#[tauri::command]
#[specta::specta]
fn notification_action(
    node_id: String,
    key: String,
    index: i32,
    text: Option<String>,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    let via = engine.notification_action(&node_id, &key, i64::from(index), text.as_deref())?;
    // Still in the history, marked dismissed.
    if index < 0 {
        notif_history::on_dismissed(&app, &node_id, &key);
    }
    phone_notifs::on_acted(&app, &node_id, &key, index);
    Ok(via)
}

// --- quick device actions (find my device, status, open a link) ---
//
// Each returns the transport used, or "queued" when the device isn't
// reachable yet (it's delivered once it is).

/// Ask a device for its battery/charging; the answer arrives as
/// `device-status-event`.
#[tauri::command]
#[specta::specta]
fn request_status(node_id: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.request_status(&node_id)
}

/// Make a device ring.
#[tauri::command]
#[specta::specta]
fn ring_device(node_id: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.ring(&node_id)
}

/// Open a web link (http/https only) on a device.
#[tauri::command]
#[specta::specta]
fn open_url_on(node_id: String, url: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.open_url(&node_id, &url)
}

/// Open a link on a device in a chosen app (an Android package such as
/// "com.google.android.youtube"); without it, the device's browser.
#[tauri::command]
#[specta::specta]
fn open_url_in_app(
    node_id: String,
    url: String,
    app: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.open_url_in(&node_id, &url, &app)
}

/// Search for `query` in `app` ("youtube", "spotify" or "web") on a device
/// that lists the `search` action (its `search_apps` say which it has).
#[tauri::command]
#[specta::specta]
fn search_on(
    node_id: String,
    app: String,
    query: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.search_in_app(&node_id, &app, &query)
}

// --- notifications ---

/// Whether OS notifications are shown (Settings, "Show notifications").
#[tauri::command]
#[specta::specta]
fn notifications_enabled(prefs: State<notify::Prefs>) -> bool {
    prefs.enabled()
}

#[tauri::command]
#[specta::specta]
fn set_notifications_enabled(on: bool, prefs: State<notify::Prefs>) {
    prefs.set_enabled(on)
}

/// Whether pop-ups are held while your phone is in Do Not Disturb.
#[tauri::command]
#[specta::specta]
fn follow_phone_dnd(prefs: State<notify::Prefs>) -> bool {
    prefs.follow_dnd()
}

#[tauri::command]
#[specta::specta]
fn set_follow_phone_dnd(on: bool, app: AppHandle, prefs: State<notify::Prefs>) {
    prefs.set_follow_dnd(on);
    // The indicator follows; turned off mid-DND: say what was held.
    quiet::changed(&app);
    notify::dnd_changed(&app);
}

/// Whether your phone may see where this computer is (find my computer).
#[tauri::command]
#[specta::specta]
fn share_location(pref: State<location::LocationPref>) -> bool {
    pref.allowed()
}

/// Let your phone see where this computer is, or stop it.
#[tauri::command]
#[specta::specta]
fn set_share_location(on: bool, app: AppHandle, pref: State<location::LocationPref>) {
    pref.set_allowed(on);
    device::refresh_status(&app);
}

/// This computer can tell where it is at all (macOS; Linux with GeoClue;
/// Windows).
#[tauri::command]
#[specta::specta]
fn location_available() -> bool {
    location::available()
}

/// Your phones on Do Not Disturb right now (node ids; a live link and a
/// fresh word from each, see `quiet`), whether or not this computer stays
/// quiet with them.
#[tauri::command]
#[specta::specta]
fn dnd_devices(app: AppHandle) -> Vec<String> {
    quiet::phones(&app)
}

// --- your phone sees this screen, and you see its ---

/// Control another computer from this one (its screen tile): `events_json`
/// is a list of input events (`{"kind": "point", "x", "y"}`, `{"kind":
/// "click", "button"}`, `{"kind": "scroll", "dx", "dy"}`, `{"kind": "text",
/// "text"}`, `{"kind": "key", "key"}`; see mdcore `features::input`).
#[tauri::command]
#[specta::specta]
fn send_input(node_id: String, events_json: String, engine: State<Eng>) -> Result<(), CoreError> {
    let events: Vec<mdcore::model::InputEvent> = serde_json::from_str(&events_json)
        .map_err(|e| CoreError::InvalidInput(format!("input events: {e}")))?;
    engine.send_input(&node_id, events)
}

/// Send the picture on this clipboard to a device's clipboard. Returns the
/// transfer id (progress as `file-progress`). Kept in the clipboard history.
#[tauri::command]
#[specta::specta]
fn send_clipboard_image(
    node_id: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    let path = clip_image::copied_to_png().map_err(CoreError::InvalidInput)?;
    let id = engine.send_clipboard_image(&node_id, &path.to_string_lossy())?;
    clip_history::on_picture(&app, clip_history::ClipDirection::Out, &node_id, &path);
    Ok(id)
}

/// Start (`on`) or stop watching a device's screen: snapshots arrive as
/// `screen-frame` events. Renew every 10 s while it shows (a watch ends by
/// itself after 25 s).
#[tauri::command]
#[specta::specta]
fn screen_watch(node_id: String, on: bool, engine: State<Eng>) -> Result<(), CoreError> {
    engine.screen_watch(&node_id, on)
}

// --- files ---

/// Where received files are saved (set once at startup).
struct DownloadDir(std::path::PathBuf);

/// The folder incoming files go to: the user's Downloads (the app data dir
/// if the OS has none). Dev instances started with `TEST=...` each get their
/// own subfolder, so two of them on one machine don't save over each other.
fn download_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let base = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .ok()?;
    let dir = match test_suffix() {
        Some(suffix) if cfg!(debug_assertions) => base.join(format!("my-domain-{suffix}")),
        _ => base,
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[files] can't create {}: {e}", dir.display());
    }
    Some(dir)
}

/// Run an action a device listed in its status (`actions`).
#[tauri::command]
#[specta::specta]
fn run_action(
    node_id: String,
    name: String,
    args_json: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.run_action(&node_id, &name, &args_json)
}

/// Send a file to a device. Returns the transfer id; progress arrives as
/// `file-progress`, then `file-sent` or `file-failed`.
#[tauri::command]
#[specta::specta]
fn send_file(
    node_id: String,
    path: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    let id = engine.send_file(&node_id, &path)?;
    // Kept with the transfer, for "Show in Finder" and "Send again".
    transfers::on_sending(&app, &id, &node_id, &path);
    Ok(id)
}

/// Send a whole folder to a device; it's recreated under the device's
/// downloads. Returns how many files it holds; each one is a transfer with
/// the usual `file-*` events.
#[tauri::command]
#[specta::specta]
fn send_folder(node_id: String, path: String, engine: State<Eng>) -> Result<u32, CoreError> {
    engine.send_folder(&node_id, &path)
}

/// Stop a transfer, either way.
#[tauri::command]
#[specta::specta]
fn cancel_file(id: String, engine: State<Eng>) {
    engine.cancel_file(&id)
}

/// Send text to a device: onto its clipboard (`mode` "clipboard") or shown
/// as a notification ("notify"). It gets a `text-shared` event.
#[tauri::command]
#[specta::specta]
fn send_text(
    node_id: String,
    text: String,
    mode: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    let how = engine.send_text(&node_id, &text, &mode)?;
    if mode == "clipboard" {
        clip_history::on_sent(&app, &node_id, &text);
    }
    Ok(how)
}

// --- browsing another device's shared folders ---

/// Ask a device what's in `path` of its shared folder `root` (`root` ""
/// lists its shared folders), from entry `offset` on. Returns the request
/// id; the answer is a `folder-listing` event with the same `req`.
#[tauri::command]
#[specta::specta]
fn list_folder(
    node_id: String,
    root: String,
    path: String,
    offset: u32,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.list_folder(&node_id, &root, &path, u64::from(offset))
}

/// Fetch a file from a device's shared folder into Downloads. Returns the
/// transfer id; it then shows up like a file the device sent
/// (`file-progress`, `file-received` / `file-failed`).
#[tauri::command]
#[specta::specta]
fn download_file(
    node_id: String,
    root: String,
    path: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.download_file(&node_id, &root, &path)
}

/// The folders this computer lets your other devices browse.
#[tauri::command]
#[specta::specta]
fn shared_folders(engine: State<Eng>) -> Vec<mdcore::model::SharedFolder> {
    engine.shared_folders()
}

// --- a phone's gallery, battery alerts, keep awake ---

/// Ask a phone for `limit` of its photos and videos (newest first) from
/// `offset`, with thumbnails if `thumbs`. Returns the request id; the
/// answer is a `gallery-listing` event with the same `req`, then a
/// `gallery-thumb` per item (`jpeg`: base64).
#[tauri::command]
#[specta::specta]
fn list_gallery(
    node_id: String,
    offset: u32,
    limit: u32,
    thumbs: bool,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.list_gallery(&node_id, u64::from(offset), limit, thumbs)
}

/// Fetch a photo or video from a phone's gallery (`id` from the listing,
/// `name` for the progress) into Downloads. Returns the transfer id.
#[tauri::command]
#[specta::specta]
fn download_gallery_item(
    node_id: String,
    id: String,
    name: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.download_gallery_item(&node_id, &id, &name)
}

/// Whether this computer tells your other devices when its battery is low
/// or charged, the low level and the charged level (0 = when full).
#[tauri::command]
#[specta::specta]
fn battery_alerts(engine: State<Eng>) -> mdcore::model::BatteryAlertSettings {
    engine.battery_alerts()
}

#[tauri::command]
#[specta::specta]
fn set_battery_alerts(
    settings: mdcore::model::BatteryAlertSettings,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.set_battery_alerts(settings)
}

/// Keep a device from sleeping for `minutes` (0 = stop), if its status
/// lists "keep_awake".
#[tauri::command]
#[specta::specta]
fn keep_awake(node_id: String, minutes: u32, engine: State<Eng>) -> Result<String, CoreError> {
    engine.keep_awake(&node_id, minutes)
}

/// The native "choose a file" dialog; `None` if the user closed it. Async so
/// it doesn't run on (and block) the main thread the dialog needs.
#[tauri::command]
#[specta::specta]
async fn pick_file(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_file(move |file| {
        let _ = tx.send(file.and_then(|f| f.into_path().ok()));
    });
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

/// The native "choose a folder" dialog; `None` if the user closed it. Async
/// for the same reason as `pick_file`.
#[tauri::command]
#[specta::specta]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder.and_then(|f| f.into_path().ok()));
    });
    tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .ok()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

/// A file the UI may open or reveal: one this app saved, in the downloads
/// folder or a synced folder (`sync_folders::openable_file`).
fn received_file(
    path: &str,
    dir: &DownloadDir,
    engine: &Eng,
) -> Result<std::path::PathBuf, CoreError> {
    let mut roots = vec![dir.0.clone()];
    roots.extend(sync_folders::synced_roots(&engine.sync_pairs()));
    sync_folders::openable_file(path, &roots)
}

/// Open a map at a device's reported position (OpenStreetMap in the
/// browser). Only coordinates come in, so the page can't open any link.
#[tauri::command]
#[specta::specta]
fn open_map(lat: f64, lon: f64, app: AppHandle) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return Err(CoreError::InvalidInput("not a position".into()));
    }
    let url = format!(
        "https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=17/{lat:.6}/{lon:.6}"
    );
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Show a received file in Finder / Explorer / the file manager.
#[tauri::command]
#[specta::specta]
fn reveal_file(
    path: String,
    app: AppHandle,
    dir: State<DownloadDir>,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let file = received_file(&path, &dir, &engine)?;
    app.opener()
        .reveal_item_in_dir(file)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Open a received file with its default app.
#[tauri::command]
#[specta::specta]
fn open_file(
    path: String,
    app: AppHandle,
    dir: State<DownloadDir>,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let file = received_file(&path, &dir, &engine)?;
    app.opener()
        .open_path(file.to_string_lossy(), None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

/// Every command the UI can call. The same list registers the handlers and
/// generates the TypeScript bindings (`ui/src/bindings.ts`), so a renamed
/// command or field breaks the UI build instead of failing at runtime.
fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        get_identity,
        get_peers,
        get_saved_session,
        dev_credentials,
        link_stats,
        is_ready,
        session_info,
        auth_register,
        auth_login,
        set_encryption_key,
        update_encryption_key,
        generate_key,
        join_invite,
        join_status,
        join_cancel,
        join_with_code,
        join_with_link,
        logout,
        refresh_from_server,
        scan_lan,
        send_message,
        enable_clipboard_sync,
        disable_clipboard_sync,
        clipboard_sync_enabled,
        get_clipboard,
        add_manual_peer,
        remove_peer,
        share_notification,
        share_call_notification,
        share_call_history,
        connect,
        disconnect,
        reconnect,
        open_map,
        firewall_check,
        fcm_selftest,
        report_firewall,
        on_signal,
        request_apps,
        subscribe_apps,
        notification_action,
        notif_history::notification_history,
        notif_history::clear_notification_history,
        notif_history::muted_apps,
        notif_history::set_muted_apps,
        phone_notifs::phone_notifications,
        phone_notifs::forget_phone_notification,
        phone_notifs::phone_notif_prefs,
        phone_notifs::set_phone_notif_prefs,
        chat_history::chat_history,
        chat_history::clear_chat_history,
        chat_history::chat_history_enabled,
        chat_history::set_chat_history_enabled,
        clip_history::clip_history,
        clip_history::clear_clip_history,
        clip_history::clip_history_enabled,
        clip_history::set_clip_history_enabled,
        clip_privacy::clip_privacy,
        clip_privacy::set_clip_hide_concealed,
        clip_privacy::set_clip_excluded_apps,
        clip_history::clip_picture,
        clip_history::copy_clip_picture,
        clip_history::send_clip_picture,
        clip_history::save_clip_picture,
        request_status,
        ring_device,
        open_url_on,
        open_url_in_app,
        search_on,
        send_file,
        send_folder,
        cancel_file,
        transfers::transfer_list,
        transfers::transfer_pause,
        transfers::transfer_retry,
        transfers::send_batch,
        transfers::transfer_clear_finished,
        transfers::transfer_reveal,
        send_text,
        list_folder,
        download_file,
        shared_folders,
        pick_file,
        pick_folder,
        run_action,
        clipboard_history,
        clear_clipboard_history,
        copy_to_clipboard,
        reveal_file,
        open_file,
        notifications_enabled,
        set_notifications_enabled,
        list_gallery,
        download_gallery_item,
        battery_alerts,
        set_battery_alerts,
        keep_awake,
        follow_phone_dnd,
        set_follow_phone_dnd,
        share_location,
        set_share_location,
        wifi_share::current_wifi_name,
        wifi_share::share_wifi_to,
        location_available,
        wol::wake_setting,
        wol::set_wake_setting,
        wol::can_wake,
        wol::wake_device,
        wifi_direct::wifi_direct_setting,
        wifi_direct::set_wifi_direct,
        wifi_direct::set_phone_hotspot,
        print::print_setting,
        print::set_print_allow,
        print::set_print_ask,
        print::print_answer,
        hotspot::hotspot_consent,
        tray_battery::tray_battery_setting,
        tray_battery::set_tray_battery,
        away_lock::away_lock,
        away_lock::set_away_lock,
        shortcuts::list_shortcuts,
        shortcuts::check_shortcut_keys,
        shortcuts::save_shortcut,
        shortcuts::delete_shortcut,
        shortcuts::pause_shortcuts,
        schedule::list_schedules,
        schedule::add_schedule,
        schedule::update_schedule,
        schedule::delete_schedule,
        schedule::run_schedule_now,
        schedule::local_actions,
        schedule::run_local_action,
        automations::list_automations,
        automations::add_automation,
        automations::update_automation,
        automations::delete_automation,
        battery_log::battery_history,
        commands_run::list_shell_commands,
        commands_run::add_shell_command,
        commands_run::update_shell_command,
        commands_run::delete_shell_command,
        commands_run::test_shell_command,
        webhooks::webhook_settings,
        webhooks::set_webhooks_enabled,
        webhooks::set_webhook_port,
        webhooks::new_webhook_token,
        device_labels::get_device_labels,
        device_kind::device_kinds,
        device_kind::own_device_kind,
        health::health_check,
        health::open_permission_page,
        health::request_permission,
        health::open_downloads,
        health::onboarding_state,
        health::onboarding_act,
        health::set_onboarding_features,
        health::set_device_name,
        device_labels::set_device_label,
        device_labels::labels_share,
        device_labels::set_labels_share,
        page_layout::get_page_layout,
        page_layout::set_page_layout,
        notes::notes_list,
        notes::note_save,
        notes::note_delete,
        read_later::read_later_list,
        read_later::read_later_save,
        read_later::read_later_save_clipboard,
        read_later::read_later_mark,
        read_later::read_later_delete,
        read_later::read_later_open,
        remote_download::remote_download_request,
        remote_download::remote_download_copied_link,
        remote_download::remote_download_targets,
        remote_download::remote_download_answer,
        remote_download::remote_downloads,
        remote_download::remote_download_settings,
        remote_download::set_remote_download_settings,
        note_reminders::note_remind_done,
        note_reminders::note_remind_at,
        timers::timers_list,
        timers::timer_start,
        timers::timer_action,
        timers::timer_add,
        timers::timer_set_ring,
        timers::timer_devices,
        dnd_devices,
        quiet::phone_quiet,
        peer_perms::peer_permissions,
        peer_perms::all_permissions,
        peer_perms::set_peer_permission,
        peer_perms::permission_answer,
        peer_perms::permission_notice,
        peer_perms::dismiss_permission_notice,
        kvm::kvm_settings,
        kvm::kvm_place,
        kvm::kvm_set_enabled,
        kvm::kvm_set_allowed,
        kvm::kvm_set_clip,
        kvm::kvm_set_swap,
        kvm::kvm_set_hotkey,
        kvm::kvm_stop,
        screen_watch,
        phone_screen::phone_touch,
        phone_screen::phone_camera_watch,
        phone_screen::phone_camera_control,
        media_ctl::media_command,
        media_ctl::media_watch,
        media_ctl::media_supported,
        otp_codes::verification_codes,
        otp_codes::copy_verification_code,
        otp_codes::dismiss_verification_code,
        otp_codes::otp_auto_copy,
        otp_codes::set_otp_auto_copy,
        insert_capture::insert_request,
        insert_capture::insert_cancel,
        insert_capture::insert_supported,
        insert_capture::insert_settings,
        insert_capture::set_insert_paste,
        phone_audio::phone_audio_status,
        phone_audio::phone_audio_outputs,
        phone_audio::phone_audio_start,
        phone_audio::phone_audio_stop,
        phone_audio::phone_audio_set_volume,
        phone_audio::phone_audio_set_output,
        computer_audio::computer_audio_status,
        computer_audio::computer_audio_offer,
        computer_audio::computer_audio_stop,
        mute_here::mute_here,
        mute_here::set_mute_here,
        nearby::audio_route,
        nearby::set_audio_route,
        nearby::audio_via,
        computer_notifs::computer_notifs_state,
        computer_notifs::computer_notifs_set_enabled,
        computer_notifs::computer_notifs_set_muted,
        send_input,
        send_clipboard_image,
        shortcut::send_clipboard,
        calls::calls_status,
        calls::calls_devices,
        calls::calls_set_enabled,
        calls::calls_set_device,
        calls::calls_set_answer_audio,
        calls::calls_audio_devices,
        calls::calls_connect,
        calls::calls_disconnect,
        calls::call_answer,
        calls::call_decline,
        calls::call_hang_up,
        calls::call_dial,
        calls::call_redial,
        calls::call_dtmf,
        calls::call_mute,
        calls::call_set_audio,
        calls::call_audio_to_phone,
        calls::call_audio_to_mac,
        send_to::take_send_request,
        send_to::send_paths,
        send_to::file_manager_menu,
        send_to::set_file_manager_menu,
        sync_folders::sync_pairs,
        sync_folders::sync_create,
        sync_folders::sync_accept,
        sync_folders::sync_remove,
        sync_folders::sync_pause,
        sync_folders::sync_now,
        sync_folders::sync_max_size_mb,
        sync_folders::sync_set_max_size_mb,
        sync_folders::sync_open_folder,
        search::search,
        activity::activity_feed,
        activity::set_activity_hidden,
        activity::clear_activity,
        activity::restore_activity,
        quick_panel::quick_panel_enabled,
        quick_panel::set_quick_panel_enabled,
        quick_panel::quick_panel_hide,
        quick_panel::quick_panel_open_main,
        quick_panel::quick_panel_action,
        quick_panel::quick_panel_clips,
        diagnostics::diagnose,
        diagnostics::speed_test,
        diagnostics::cancel_speed_test,
        links_map::links_map,
        nearby::nearby_media,
        nearby::set_nearby_media,
        nearby::nearby_status,
        wallpaper::share_wallpaper,
        wallpaper::set_share_wallpaper,
        wallpaper::wallpapers,
        wallpaper::own_wallpaper,
        key_changes::key_changes,
        key_changes::trust_new_key,
        key_changes::dismiss_key_change,
        key_changes::own_fingerprint,
        key_changes::key_verification,
        key_changes::set_device_verified,
        key_changes::remove_device,
        key_changes::allow_device_again,
        key_changes::removed_devices,
        key_changes::start_key_rotation,
        key_changes::key_rotation,
        key_changes::finish_key_rotation,
        key_changes::cancel_key_rotation,
        key_changes::dismiss_key_rotation,
        key_changes::passphrase_strength,
        account::change_account_password,
        account::sign_out_all_devices,
        account::account_sessions,
        account::sign_out_session,
        account::signed_out_elsewhere,
        account::sign_in_again,
        account::account_password_problem,
        settings_backup::settings_backup_save,
        settings_backup::settings_restore_open,
        settings_backup::settings_restore_apply,
        settings_backup::restart_app,
        updates::update_status,
        updates::update_check,
        updates::update_install,
        updates::update_later,
        updates::set_update_auto,
        ext_bridge::browser_extension_status,
        ext_bridge::browser_extension_install,
        ext_bridge::browser_extension_remove
    ])
}

/// Write `ui/src/bindings.ts`.
fn export_bindings(commands: &tauri_specta::Builder<tauri::Wry>) -> Result<(), String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");
    commands
        .export(specta_typescript::Typescript::default(), path)
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Started by a browser for the extension (`ext_bridge`): no window, just
    // pass its messages to the running app, then exit.
    if ext_bridge::host_requested(std::env::args_os()) {
        std::process::exit(ext_bridge::run_host());
    }
    let commands = specta_builder();
    // Debug builds keep the UI's TypeScript bindings current.
    #[cfg(debug_assertions)]
    if let Err(e) = export_bindings(&commands) {
        eprintln!("[specta] failed to export TypeScript bindings: {e}");
    }

    // `--send <path>...` (Explorer's Send to, the Linux file managers, a
    // terminal): hand the paths to my-domain if it's already running, else
    // start and ask where to send them (see `send_to`).
    let send_paths = send_to::from_command_line();
    if let Some(paths) = &send_paths {
        if send_to::hand_off(paths) {
            return;
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Automatic updates (`updates.rs`); checks only in release builds
        // signed with the updater key.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Before the Engine: its first events may already notify.
            app.manage(notify::Prefs::load(&handle));
            app.manage(quiet::Quiet::default());
            app.manage(activity::ActivityLog::load(&handle));
            app.manage(notif_history::NotifHistory::load(&handle));
            app.manage(phone_notifs::PhoneNotifs::load(&handle));
            app.manage(clip_history::ClipHistory::load(&handle));
            // Before the Engine: its clipboard watcher asks.
            clip_privacy::load(&handle);
            // Wayland: where the portals' restore tokens are kept, and a
            // first look at the portals and tools, off the UI's thread.
            #[cfg(target_os = "linux")]
            if let Ok(dir) = handle.path().app_config_dir() {
                md_portal::service::configure(dir);
                if md_portal::Session::wayland() {
                    std::thread::spawn(md_portal::service::caps);
                }
            }
            otp_codes::load(&handle);
            app.manage(chat_history::ChatHistory::load(&handle));
            app.manage(insert_capture::InsertPrefs::load(&handle));
            app.manage(transfers::TransferHistory::load(&handle));
            app.manage(location::LocationPref::load(&handle));
            app.manage(wol::WakePref::load(&handle));
            app.manage(wifi_direct::WifiDirectPref::load(&handle));
            app.manage(print::PrintPrefs::load(&handle));
            app.manage(hotspot::HotspotPref::load(&handle));
            app.manage(device_labels::DeviceLabels::load(&handle));
            app.manage(page_layout::PageLayouts::load(&handle));
            app.manage(away_lock::AwayLockState::load(&handle));
            app.manage(schedule::Schedules::load(&handle));
            app.manage(note_reminders::NoteReminders::load(&handle));
            app.manage(timers::TimerAlarm::default());
            app.manage(automations::Automations::load(&handle));
            app.manage(battery_log::BatteryLog::load(&handle));
            app.manage(commands_run::ShellCommands::load(&handle));
            app.manage(webhooks::Webhooks::load(&handle));
            app.manage(calls::CallsPrefs::load(&handle));
            app.manage(computer_notifs::ComputerNotifs::load(&handle));
            app.manage(quick_panel::QuickPanelPref::load(&handle));
            app.manage(tray_battery::TrayBatteryPref::load(&handle));
            app.manage(settings_backup::Pending::default());
            app.manage(updates::Updates::load(&handle));
            app.manage(ext_bridge::ExtBridge::load(&handle));
            let fcm_token: Arc<std::sync::Mutex<Option<String>>> =
                Arc::new(std::sync::Mutex::new(None));
            let platform = DesktopPlatform {
                kv: handle.path().app_config_dir().ok().map(FileKv::in_dir),
                dev_secrets: dev_secrets_file(&handle),
                clipboard: DesktopClipboard::new(),
                fcm_token: fcm_token.clone(),
            };
            let sink: Arc<dyn EventSink> = Arc::new(TauriSink {
                app: handle.clone(),
            });
            match Engine::start(platform, sink) {
                Ok(engine) => {
                    // Without a downloads folder the core declines offers.
                    let dir = download_dir(&handle);
                    if let Some(dir) = &dir {
                        engine.set_download_dir(&dir.to_string_lossy());
                        // Your other devices may browse and fetch from it.
                        let downloads = mdcore::model::SharedFolder {
                            name: "Downloads".into(),
                            path: dir.to_string_lossy().into_owned(),
                        };
                        if let Err(e) = engine.set_shared_folders(&[downloads]) {
                            eprintln!("[files] can't share Downloads: {e}");
                        }
                    }
                    app.manage(DownloadDir(dir.unwrap_or_default()));
                    // A phone's audio plays here (`phone_audio.rs`), and
                    // this computer's sound on a phone where it can be
                    // captured (`computer_audio.rs`); said in this
                    // computer's status as `audio_sink` / `audio_source`.
                    engine.set_audio_roles(true, computer_audio::supported());
                    // Labels given here before they were shared.
                    device_labels::seed(&handle, &engine);
                    // What kind of computer this is (its icon elsewhere).
                    device_kind::init(&handle, &engine);
                    // This app's version, for your other devices' health check.
                    engine.set_app_version(&handle.package_info().version.to_string());
                    // "Use nearby mode for media", as saved.
                    nearby::init(&handle, &engine);
                    // A mute left by a crash while the sound played on a
                    // phone (`mute_here.rs`).
                    mute_here::init(&handle);
                    app.manage(engine);
                    // "Share my wallpaper with my devices" (`wallpaper.rs`).
                    wallpaper::init(&handle);
                }
                Err(e) => {
                    eprintln!("failed to start networking: {e}");
                    return Err(Box::new(std::io::Error::other(e)));
                }
            }
            // Wi-Fi Direct for big files: what this computer can do (Windows
            // hosts a group), and the user's switch.
            wifi_direct::init(&handle);
            // Print from the phone: whether this computer prints for it.
            print::init(&handle);
            // Start the FCM receiver (needs the managed Engine for on_signal).
            app.manage(fcm::FcmToken(fcm_token.clone()));
            fcm::spawn(handle.clone(), fcm_token);
            device::spawn_status_reporter(handle.clone());
            media_ctl::spawn_now_playing(handle.clone());
            away_lock::spawn(handle.clone());
            quiet::spawn(handle.clone());
            schedule::spawn(handle.clone());
            note_reminders::spawn(handle.clone());
            timers::spawn(handle.clone());
            kvm::init(&handle);
            automations::spawn(handle.clone());
            battery_log::spawn(handle.clone());
            // Off by default: listens only once switched on in Settings.
            webhooks::spawn(handle.clone());
            // Calls on this Mac: off by default; reconnects to the chosen
            // phone when turned on (macOS only).
            calls::init(&handle);
            // This computer's notifications on your phone: off by default;
            // reads them only once switched on (Linux only).
            computer_notifs::init(&handle);
            // Reach phones over Bluetooth LE when there's no network path
            // (does nothing without Bluetooth).
            md_ble::central::start(EngineHandle(handle.clone()));
            // Keyboard shortcuts (Cmd/Ctrl+Shift+Y sends what you copied to
            // your phone, out of the box).
            shortcut::init(&handle);
            shortcuts::init(&handle);
            // "Send with my-domain" from Finder / Explorer / the file manager.
            send_to::init(&handle, send_paths);
            // A missing tray isn't fatal: the window still works.
            if let Err(e) = tray::init(&handle) {
                eprintln!("[tray] couldn't create the tray icon: {e}");
            }
            tray_battery::spawn(handle.clone());
            // Look for a new version 30 s from now, then every 6 h.
            updates::spawn(handle.clone());
            // The browser extension's host reaches the app here.
            ext_bridge::spawn(handle.clone());
            Ok(())
        })
        // Closing the window hides it to the tray; the tray's "Quit" exits.
        // The quick panel hides when it loses focus, and on a close.
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == quick_panel::LABEL {
                    api.prevent_close();
                    quick_panel::hide(window.app_handle());
                } else if window.label() == "main" && !tray::quitting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            tauri::WindowEvent::Focused(false) if window.label() == quick_panel::LABEL => {
                quick_panel::hide(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(commands.invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // Quitting with an update downloaded ("Install updates
            // automatically"): install it first (`updates.rs`).
            if let tauri::RunEvent::ExitRequested { api, .. } = &_event {
                if updates::on_exit_requested(_app) {
                    api.prevent_exit();
                }
            }
            // Quitting while this computer's sound plays on a phone: put
            // the speakers back as they were (`mute_here.rs`).
            if let tauri::RunEvent::Exit = _event {
                mute_here::restore(_app);
            }
            // Clicking the dock icon brings the hidden window back (macOS).
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                tray::show_window(_app);
            }
        });
}

#[cfg(test)]
mod bindings {
    /// `cargo test -p my-domain export_bindings` regenerates the bindings.
    #[test]
    fn export_bindings() {
        // Fails (rather than keeping the old file) if a command can't be
        // exported, e.g. one taking an i64, which TypeScript can't hold.
        super::export_bindings(&super::specta_builder()).expect("bindings exported");
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");
        let ts = std::fs::read_to_string(path).expect("bindings written");
        assert!(ts.contains("sendMessage"), "commands are exported");
    }
}
