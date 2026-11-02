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

mod fcm;

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
    clipboard: DesktopClipboard,
    /// Set by the FCM receiver thread once Firebase issues a token; reported to
    /// the registry on refresh so siblings can signal this device.
    fcm_token: Arc<std::sync::Mutex<Option<String>>>,
}

/// Optional test suffix (`TEST=foo npm run tauri dev`) so several instances on
/// one machine register as distinct devices instead of clobbering the same
/// machine-uid row. Trimmed; empty/unset => no suffix.
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
        keyring_entry(key)?.get_password().ok()
    }

    fn secret_set(&self, key: &str, value: Option<&str>) {
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
fn send_message(node_id: String, text: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.send(&node_id, &text)
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
        subscribe_apps
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
        .setup(move |app| {
            let handle = app.handle().clone();
            let fcm_token: Arc<std::sync::Mutex<Option<String>>> =
                Arc::new(std::sync::Mutex::new(None));
            let platform = DesktopPlatform {
                kv: handle.path().app_config_dir().ok().map(FileKv::in_dir),
                clipboard: DesktopClipboard::new(),
                fcm_token: fcm_token.clone(),
            };
            let sink: Arc<dyn EventSink> = Arc::new(TauriSink {
                app: handle.clone(),
            });
            match Engine::start(platform, sink) {
                Ok(engine) => {
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
