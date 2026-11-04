// my-domain desktop backend — a thin Tauri adapter over the shared `mdcore`
// Engine. Platform specifics (machine id, hostname, interface enumeration,
// clipboard, session persistence) are implemented here; all networking,
// crypto, discovery, and messaging live in `mdcore`.

use std::net::IpAddr;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use mdcore::capabilities::Clipboard;
use mdcore::engine::{Engine, FirewallStatus, SavedSession, SessionInfo};
use mdcore::events::{CoreEvent, EventSink};
use mdcore::kv::FileKv;
use mdcore::model::{Identity, Peer};
use mdcore::platform::{IfaceMode, Platform};
use mdcore::CoreError;

mod automations;
mod away_lock;
mod battery_log;
mod calls;
mod clip_history;
mod clip_image;
mod commands_run;
mod device;
mod device_labels;
mod fcm;
mod location;
mod notif_history;
mod notify;
mod quiet;
mod remote_input;
mod schedule;
mod screen_share;
mod send_to;
mod shortcut;
mod sysinfo;
mod transfers;
mod tray;
mod webhooks;
mod wifi_share;

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

impl Clipboard for DesktopClipboard {
    fn get(&self) -> Option<String> {
        let mut g = self.inner.lock().unwrap();
        if g.is_none() {
            *g = arboard::Clipboard::new().ok();
        }
        g.as_mut().and_then(|c| c.get_text().ok())
    }
    fn set(&self, text: &str) {
        let mut g = self.inner.lock().unwrap();
        if g.is_none() {
            *g = arboard::Clipboard::new().ok();
        }
        if let Some(c) = g.as_mut() {
            let _ = c.set_text(text.to_owned());
        }
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
        if cfg!(debug_assertions) {
            let payload = ev.ui_payload().to_string();
            let short: String = payload.chars().take(200).collect();
            eprintln!("[event] {} {short}", ev.name());
        }
        // Ring / open-link need the desktop to act, not just the UI. Off the
        // core's thread: these look up peers and talk to the OS.
        match &ev {
            // The tray's device submenus follow the peer list and link state,
            // and automations wait for devices to connect or disconnect.
            CoreEvent::PeersUpdated { .. }
            | CoreEvent::PeerConnected { .. }
            | CoreEvent::PeerDisconnected { .. } => {
                tray::refresh(&self.app);
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
                tray::on_status(&self.app, from, status);
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
            CoreEvent::ScreenWatch { from, on } => {
                let (app, from, on) = (self.app.clone(), from.clone(), *on);
                std::thread::spawn(move || screen_share::on_watch(&app, &from, on));
            }
            CoreEvent::Dnd { from, on } => quiet::on_dnd(&self.app, from, *on),
            // OS notifications for what arrives while the window is in the
            // background (off the core's thread too: peer lookup, the OS).
            CoreEvent::MessageReceived(m) if m.ok => {
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
            CoreEvent::TextShared { from, text, mode } => {
                if mode == "clipboard" {
                    clip_history::on_received(&self.app, from, text);
                }
                let app = self.app.clone();
                let (from, text, mode) = (from.clone(), text.clone(), mode.clone());
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    let short: String = text.chars().take(200).collect();
                    if mode == "clipboard" {
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
            } => transfers::on_progress(
                &self.app, id, node_id, name, *done, *total, *incoming, *resumed,
            ),
            CoreEvent::FileSent { id, node_id, name } => {
                transfers::on_sent(&self.app, id, node_id, name);
            }
            CoreEvent::FileFailed {
                id,
                node_id,
                name,
                incoming,
                reason,
            } => transfers::on_failed(&self.app, id, node_id, name, *incoming, reason),
            // Photo backups arrive by themselves, often many: no notice each.
            CoreEvent::FileReceived {
                id,
                from,
                name,
                path,
                size,
                ..
            } if is_backup(path) => {
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
            } => {
                transfers::on_received(&self.app, id, from, name, path, *size);
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
            }
            // Kept in the history first (cheap, and before the window
            // hears of it); a muted app's get no further.
            CoreEvent::Notification {
                from,
                title,
                body,
                app: source,
                key,
                ..
            } => {
                let shown = notif_history::on_notification(
                    &self.app,
                    from,
                    source.as_deref(),
                    title,
                    body,
                    key.as_deref(),
                );
                if !shown {
                    return;
                }
                let app = self.app.clone();
                let (from, title, body, source) =
                    (from.clone(), title.clone(), body.clone(), source.clone());
                std::thread::spawn(move || {
                    let title = if !title.trim().is_empty() {
                        title
                    } else {
                        source.unwrap_or_else(|| notify::device_name(&app, &from))
                    };
                    notify::post_if_away(&app, &title, &body);
                });
            }
            _ => {}
        }
        let _ = self.app.emit(ev.name(), ev.ui_payload());
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
    engine: State<Eng>,
) -> Result<String, CoreError> {
    // The UI picks the id so it can match the delivery receipt to its bubble.
    match id {
        Some(id) => engine.send_with_id(&node_id, &text, &id),
        None => engine.send(&node_id, &text),
    }
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
fn subscribe_apps(node_id: String, apps_json: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.subscribe_apps(&node_id, &apps_json)
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

// --- the phone as touchpad and keyboard ---

/// Whether your phone may move the pointer and type here.
#[tauri::command]
#[specta::specta]
fn remote_input_allowed(engine: State<Eng>) -> bool {
    engine.remote_input_allowed()
}

#[tauri::command]
#[specta::specta]
fn set_remote_input_allowed(on: bool, app: AppHandle, engine: State<Eng>) {
    engine.set_remote_input_allowed(on);
    // The phone offers its touchpad only while "input" is listed.
    std::thread::spawn(move || device::refresh_status(&app));
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
/// transfer id (progress as `file-progress`).
#[tauri::command]
#[specta::specta]
fn send_clipboard_image(node_id: String, engine: State<Eng>) -> Result<String, CoreError> {
    let path = clip_image::copied_to_png().map_err(CoreError::InvalidInput)?;
    engine.send_clipboard_image(&node_id, &path.to_string_lossy())
}

/// Start (`on`) or stop watching a device's screen: snapshots arrive as
/// `screen-frame` events. Renew every 10 s while it shows (a watch ends by
/// itself after 25 s).
#[tauri::command]
#[specta::specta]
fn screen_watch(node_id: String, on: bool, engine: State<Eng>) -> Result<(), CoreError> {
    engine.screen_watch(&node_id, on)
}

/// Whether your other devices may watch this screen.
#[tauri::command]
#[specta::specta]
fn screen_allowed(engine: State<Eng>) -> bool {
    engine.screen_allowed()
}

#[tauri::command]
#[specta::specta]
fn set_screen_allowed(on: bool, app: AppHandle, engine: State<Eng>) {
    engine.set_screen_allowed(on);
    // The phone offers the screen only while "screen" is listed.
    std::thread::spawn(move || device::refresh_status(&app));
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

/// A received file, checked to be inside the downloads folder: the UI may
/// only open or reveal what this app saved.
fn received_file(path: &str, dir: &DownloadDir) -> Result<std::path::PathBuf, CoreError> {
    let bad = || CoreError::InvalidInput("not a received file".into());
    let file = std::fs::canonicalize(path).map_err(|_| bad())?;
    let dir = std::fs::canonicalize(&dir.0).map_err(|_| bad())?;
    if file.starts_with(&dir) && file.is_file() {
        Ok(file)
    } else {
        Err(bad())
    }
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
fn reveal_file(path: String, app: AppHandle, dir: State<DownloadDir>) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let file = received_file(&path, &dir)?;
    app.opener()
        .reveal_item_in_dir(file)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Open a received file with its default app.
#[tauri::command]
#[specta::specta]
fn open_file(path: String, app: AppHandle, dir: State<DownloadDir>) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let file = received_file(&path, &dir)?;
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
        clip_history::clip_history,
        clip_history::clear_clip_history,
        clip_history::clip_history_enabled,
        clip_history::set_clip_history_enabled,
        request_status,
        ring_device,
        open_url_on,
        open_url_in_app,
        search_on,
        send_file,
        send_folder,
        cancel_file,
        transfers::transfer_list,
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
        away_lock::away_lock,
        away_lock::set_away_lock,
        shortcut::send_shortcut,
        shortcut::set_send_shortcut,
        shortcut::set_send_shortcut_keys,
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
        device_labels::set_device_label,
        dnd_devices,
        quiet::phone_quiet,
        remote_input_allowed,
        set_remote_input_allowed,
        screen_allowed,
        set_screen_allowed,
        screen_watch,
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
        send_to::set_file_manager_menu
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
        .setup(move |app| {
            let handle = app.handle().clone();
            // Before the Engine: its first events may already notify.
            app.manage(notify::Prefs::load(&handle));
            app.manage(quiet::Quiet::default());
            app.manage(notif_history::NotifHistory::load(&handle));
            app.manage(clip_history::ClipHistory::load(&handle));
            app.manage(transfers::TransferHistory::load(&handle));
            app.manage(location::LocationPref::load(&handle));
            app.manage(device_labels::DeviceLabels::load(&handle));
            app.manage(away_lock::AwayLockState::load(&handle));
            app.manage(schedule::Schedules::load(&handle));
            app.manage(automations::Automations::load(&handle));
            app.manage(battery_log::BatteryLog::load(&handle));
            app.manage(commands_run::ShellCommands::load(&handle));
            app.manage(webhooks::Webhooks::load(&handle));
            app.manage(calls::CallsPrefs::load(&handle));
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
                    app.manage(engine);
                }
                Err(e) => {
                    eprintln!("failed to start networking: {e}");
                    return Err(Box::new(std::io::Error::other(e)));
                }
            }
            // Start the FCM receiver (needs the managed Engine for on_signal).
            app.manage(fcm::FcmToken(fcm_token.clone()));
            fcm::spawn(handle.clone(), fcm_token);
            device::spawn_status_reporter(handle.clone());
            device::spawn_now_playing(handle.clone());
            away_lock::spawn(handle.clone());
            quiet::spawn(handle.clone());
            schedule::spawn(handle.clone());
            automations::spawn(handle.clone());
            battery_log::spawn(handle.clone());
            // Off by default: listens only once switched on in Settings.
            webhooks::spawn(handle.clone());
            // Calls on this Mac: off by default; reconnects to the chosen
            // phone when turned on (macOS only).
            calls::init(&handle);
            // Reach phones over Bluetooth LE when there's no network path
            // (does nothing without Bluetooth).
            md_ble::central::start(EngineHandle(handle.clone()));
            // Cmd/Ctrl+Shift+Y sends what you copied to your phone.
            shortcut::init(&handle);
            // "Send with my-domain" from Finder / Explorer / the file manager.
            send_to::init(&handle, send_paths);
            // A missing tray isn't fatal: the window still works.
            if let Err(e) = tray::init(&handle) {
                eprintln!("[tray] couldn't create the tray icon: {e}");
            }
            Ok(())
        })
        // Closing the window hides it to the tray; the tray's "Quit" exits.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" && !tray::quitting() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(commands.invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
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
