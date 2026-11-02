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

mod device;
mod fcm;
mod notify;

pub(crate) type Eng = Engine<DesktopPlatform>;

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
        if let Some(file) = &self.dev_secrets {
            return file.get(key);
        }
        keyring_entry(key)?.get_password().ok()
    }

    fn secret_set(&self, key: &str, value: Option<&str>) {
        if let Some(file) = &self.dev_secrets {
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
            CoreEvent::Ring { from } => {
                let (app, from) = (self.app.clone(), from.clone());
                std::thread::spawn(move || device::on_ring(&app, &from));
            }
            CoreEvent::OpenUrl { url, .. } => {
                let (app, url) = (self.app.clone(), url.clone());
                std::thread::spawn(move || device::on_open_url(&app, &url));
            }
            CoreEvent::Action { name, args, .. } => {
                let (app, name, args) = (self.app.clone(), name.clone(), args.clone());
                std::thread::spawn(move || device::on_action(&app, &name, &args));
            }
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
            CoreEvent::FileReceived { from, name, .. } => {
                let (app, from, name) = (self.app.clone(), from.clone(), name.clone());
                std::thread::spawn(move || {
                    let device = notify::device_name(&app, &from);
                    notify::post_if_away(&app, "my-domain", &format!("Saved {name} from {device}"));
                });
            }
            CoreEvent::Notification {
                from,
                title,
                body,
                app: source,
            } => {
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
fn send_file(node_id: String, path: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.send_file(&node_id, &path)
}

/// Stop a transfer, either way.
#[tauri::command]
#[specta::specta]
fn cancel_file(id: String, engine: State<Eng>) {
    engine.cancel_file(&id)
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
        firewall_check,
        fcm_selftest,
        report_firewall,
        on_signal,
        request_apps,
        subscribe_apps,
        request_status,
        ring_device,
        open_url_on,
        send_file,
        cancel_file,
        pick_file,
        run_action,
        clipboard_history,
        clear_clipboard_history,
        copy_to_clipboard,
        reveal_file,
        open_file,
        notifications_enabled,
        set_notifications_enabled
    ])
}

/// Write `ui/src/bindings.ts`.
fn export_bindings(commands: &tauri_specta::Builder<tauri::Wry>) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");
    if let Err(e) = commands.export(specta_typescript::Typescript::default(), path) {
        eprintln!("[specta] failed to export TypeScript bindings: {e}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let commands = specta_builder();
    // Debug builds keep the UI's TypeScript bindings current.
    #[cfg(debug_assertions)]
    export_bindings(&commands);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Before the Engine: its first events may already notify.
            app.manage(notify::Prefs::load(&handle));
            let fcm_token: Arc<std::sync::Mutex<Option<String>>> =
                Arc::new(std::sync::Mutex::new(None));
            let platform = DesktopPlatform {
                kv: handle.path().app_config_dir().ok().map(FileKv::in_dir),
                dev_secrets: cfg!(debug_assertions)
                    .then(|| handle.path().app_config_dir().ok())
                    .flatten()
                    .map(|dir| {
                        let name = match test_suffix() {
                            Some(suffix) => format!("dev-secrets-{suffix}.json"),
                            None => "dev-secrets.json".to_string(),
                        };
                        FileKv::new(dir.join(name))
                    }),
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
            Ok(())
        })
        .invoke_handler(commands.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod bindings {
    /// `cargo test -p my-domain export_bindings` regenerates the bindings.
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::specta_builder());
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/bindings.ts");
        let ts = std::fs::read_to_string(path).expect("bindings written");
        assert!(ts.contains("sendMessage"), "commands are exported");
    }
}
