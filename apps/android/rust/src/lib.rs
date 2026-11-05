// my-domain Android core: the shared `mdcore` Engine, exported to Kotlin with
// UniFFI (see learnings/decisions/ADR-0003-uniffi.md).
//
// The Kotlin side is generated from this file at build time
// (`uniffi-bindgen generate --library`), so the two can't drift apart:
// - `MdEngine`: the process-wide engine, with typed methods.
// - `EventListener`: Kotlin receives every core event as it happens
//   (no polling), as `(name, json)`: the desktop's event names
//   (`CoreEvent::name`) with the kind-tagged JSON (`CoreEvent::to_json`).
// - `MdError`: failures arrive in Kotlin as exceptions.
//
// Platform specifics (device id, name and Wi-Fi IP passed from Kotlin, the
// single-interface multicast strategy) live here; all networking, crypto,
// discovery and messaging live in `mdcore`.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, OnceLock};

use mdcore::engine::Engine;
use mdcore::error::CoreError;
use mdcore::events::{CoreEvent, EventSink};
use mdcore::kv::FileKv;
use mdcore::platform::{detect_local_ip, IfaceMode, Platform};

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------------
// Platform implementation
// ---------------------------------------------------------------------------

struct AndroidPlatform {
    device_id: String,
    name: String,
    /// Updated on network change; shared so `nativeNetworkChanged` can mutate it.
    wifi_ip: Arc<Mutex<String>>,
    /// Set from Kotlin via `nativeSetFcmToken` once Firebase issues it.
    fcm_token: Arc<Mutex<Option<String>>>,
    /// Session store in the app's private files dir (`Context.filesDir`).
    kv: Option<FileKv>,
    /// Secrets (session token, derived key), kept by Kotlin in the Android
    /// Keystore, so the login survives the app being killed.
    secrets: Arc<dyn SecretStore>,
}

impl AndroidPlatform {
    fn iface_v4(&self) -> Ipv4Addr {
        self.wifi_ip
            .lock()
            .unwrap()
            .parse()
            .unwrap_or_else(|_| detect_local_ip().parse().unwrap_or(Ipv4Addr::UNSPECIFIED))
    }
}

impl Platform for AndroidPlatform {
    fn device_id(&self) -> String {
        self.device_id.clone()
    }
    fn device_name(&self) -> String {
        self.name.clone()
    }
    fn platform_kind(&self) -> &'static str {
        "android"
    }
    fn iface_mode(&self) -> IfaceMode {
        IfaceMode::Single(self.iface_v4())
    }
    fn local_ip(&self) -> String {
        let i = self.iface_v4();
        if i.is_unspecified() {
            detect_local_ip()
        } else {
            i.to_string()
        }
    }
    fn kv_get(&self, key: &str) -> Option<String> {
        self.kv.as_ref()?.get(key)
    }

    fn kv_set(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                log::warn!("kv: failed to save {key}: {e}");
            }
        }
    }

    fn secret_get(&self, key: &str) -> Option<String> {
        self.secrets.get(key.to_string())
    }

    fn secret_set(&self, key: &str, value: Option<&str>) {
        self.secrets.set(key.to_string(), value.map(str::to_string));
    }

    fn fcm_token(&self) -> Option<String> {
        self.fcm_token.lock().unwrap().clone()
    }
    // clipboard: handled in Kotlin (NetService), not through the core.
}

// ---------------------------------------------------------------------------
// Types crossing the boundary
// ---------------------------------------------------------------------------

/// A failure, thrown in Kotlin as `MdException.<Variant>`.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum MdError {
    #[error("not logged in")]
    NotLoggedIn,
    #[error("set your encryption key first")]
    NoKey,
    #[error("peer not found")]
    PeerNotFound,
    #[error("{0}")]
    NotConnected(String),
    #[error("{0}")]
    Failed(String),
}

impl From<CoreError> for MdError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NotLoggedIn => MdError::NotLoggedIn,
            CoreError::NoKey => MdError::NoKey,
            CoreError::PeerNotFound => MdError::PeerNotFound,
            CoreError::NoLink(m) => MdError::NotConnected(m),
            other => MdError::Failed(other.to_string()),
        }
    }
}

type MdResult<T> = Result<T, MdError>;

/// A serde unit-variant enum as its wire string ("relay", "registry", ...).
fn as_str(v: impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Receives every core event: `name` is e.g. `message-received`,
/// `peer-connected`, `peers-updated`; `payload_json` is the event as a JSON
/// object with a `kind` field (`message_received`, ...) and its fields.
/// Called on core threads: hop to the main thread before touching UI.
#[uniffi::export(callback_interface)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, name: String, payload_json: String);
}

/// The app's secure store for the session token and the derived device key
/// (Kotlin: AES-GCM under an Android Keystore key). Called on core threads.
#[uniffi::export(callback_interface)]
pub trait SecretStore: Send + Sync {
    fn get(&self, key: String) -> Option<String>;
    /// `None` deletes the secret.
    fn set(&self, key: String, value: Option<String>);
}

/// Sends one whole frame over a Bluetooth connection (Kotlin: `BleLink`,
/// which splits it into packets). Called on core threads.
#[uniffi::export(callback_interface)]
pub trait BleOut: Send + Sync {
    fn send(&self, frame: Vec<u8>);
}

/// Adapts the Kotlin sender to the core's `BleSender`.
struct BleOutSender(Box<dyn BleOut>);

impl mdcore::engine::BleSender for BleOutSender {
    fn send(&self, frame: Vec<u8>) {
        self.0.send(frame);
    }
}

/// This device as the core sees it.
#[derive(uniffi::Record)]
pub struct DeviceIdentity {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub quic_port: u16,
}

/// A folder this phone lets your other devices browse and download from.
#[derive(uniffi::Record)]
pub struct SharedDir {
    /// What the other devices see it as, e.g. "Pictures".
    pub name: String,
    /// Where it is on this phone.
    pub path: String,
}

/// One photo or video for a gallery page (`gallery_reply`). `path` stays
/// on the phone; `thumb` is a small JPEG (up to 48 KB, else left out).
#[derive(uniffi::Record)]
pub struct GalleryMedia {
    /// MediaStore row id.
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// Unix seconds (0 if unknown).
    pub taken: u64,
    pub width: u32,
    pub height: u32,
    /// Milliseconds (videos).
    pub duration: u64,
    pub video: bool,
    pub path: String,
    pub thumb: Option<Vec<u8>>,
}

/// A new photo or video for the backup (`backup_enqueue`).
#[derive(uniffi::Record)]
pub struct BackupMedia {
    /// Names it for good (e.g. "<MediaStore id>:<date added>").
    pub key: String,
    pub path: String,
    /// Unix seconds.
    pub taken: u64,
    /// "YYYY-MM" in local time: the folder it goes into on the computer.
    pub month: Option<String>,
}

/// Where the photo backup stands (`backup_state`, and `backup-state`
/// events with the same fields).
#[derive(uniffi::Record)]
pub struct BackupInfo {
    pub enabled: bool,
    pub target: String,
    pub charging_only: bool,
    pub since: u64,
    pub queued: u32,
    pub sent: u32,
    /// "off", "target", "wifi", "charging", "retry", or "" (sending / done).
    pub waiting_for: String,
    pub current: Option<String>,
    pub done: u64,
    pub total: u64,
    pub last_error: Option<String>,
    pub last_sent_at: u64,
}

/// A synced folder (`sync_pairs`, and `sync-pairs` events with the same
/// fields under `pairs`).
#[derive(uniffi::Record)]
pub struct SyncPairInfo {
    /// The same id on both devices.
    pub id: String,
    /// The other device (node id).
    pub peer: String,
    pub name: String,
    /// The folder on this phone ("" while an invitation waits here).
    pub path: String,
    /// "both", "send" or "receive": this phone's part.
    pub mode: String,
    pub paused: bool,
    /// "invited", "pending", "paused", "error", "syncing", "in_sync" or
    /// "waiting".
    pub state: String,
    pub syncing: u32,
    pub files: u32,
    pub conflicts: u32,
    pub skipped: u32,
    /// Files kept on this phone under another name: theirs differs from
    /// another file's only by letter case (or by how an accent is
    /// written), which the folder's storage can't tell apart.
    pub renamed: u32,
    /// Unix seconds, 0 = never.
    pub last_synced: u64,
    pub error: Option<String>,
}

impl From<mdcore::model::SyncPair> for SyncPairInfo {
    fn from(p: mdcore::model::SyncPair) -> Self {
        SyncPairInfo {
            id: p.id,
            peer: p.peer,
            name: p.name,
            path: p.path,
            mode: p.mode,
            paused: p.paused,
            state: p.state,
            syncing: p.syncing,
            files: p.files,
            conflicts: p.conflicts,
            skipped: p.skipped,
            renamed: p.renamed,
            last_synced: p.last_synced,
            error: p.error,
        }
    }
}

/// Battery alerts this phone sends your other devices.
#[derive(uniffi::Record)]
pub struct BatteryAlertConfig {
    pub enabled: bool,
    /// Percent, 1-99.
    pub low: u8,
    /// Tell them when charging reaches this percent (80, 90 or 100), or 0
    /// for off.
    pub charged: u8,
}

/// A known device of ours.
#[derive(uniffi::Record)]
pub struct PeerInfo {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub quic_port: u16,
    /// Where we learned about it: "lan", "registry" or "manual".
    pub source: String,
    /// Live link right now.
    pub connected: bool,
    /// The live link's transport ("quic", "relay", "punch", "webrtc"), if any.
    pub link: Option<String>,
}

/// One live link's measured quality.
#[derive(uniffi::Record)]
pub struct LinkQuality {
    pub node_id: String,
    /// "excellent", "good", "weak", or "unknown" (not measured).
    pub quality: String,
    pub rtt_ms: Option<u32>,
    pub loss_pct: Option<f32>,
}

/// Who is signed in, and where.
#[derive(uniffi::Record)]
pub struct Session {
    pub username: Option<String>,
    pub server_url: Option<String>,
    pub has_key: bool,
}

#[derive(uniffi::Record)]
pub struct FirewallStatus {
    pub outbound_ok: bool,
    pub inbound_blocked: bool,
}

// ---------------------------------------------------------------------------
// Events: forward everything to the Kotlin listener
// ---------------------------------------------------------------------------

/// The listener is replaceable: NetService can be recreated in the same
/// process (the engine lives as long as the process).
#[derive(Default)]
struct ListenerSink {
    listener: Mutex<Option<Box<dyn EventListener>>>,
}

impl EventSink for ListenerSink {
    fn emit(&self, ev: CoreEvent) {
        if let Some(l) = self.listener.lock().unwrap().as_ref() {
            l.on_event(ev.name().to_string(), ev.to_json().to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

/// The process-wide engine. `MdEngine.start` returns the running one if it
/// was already started (e.g. by a push that woke the app).
#[derive(uniffi::Object)]
pub struct MdEngine {
    inner: Engine<AndroidPlatform>,
    sink: Arc<ListenerSink>,
    wifi_ip: Arc<Mutex<String>>,
}

static ENGINE: OnceLock<Arc<MdEngine>> = OnceLock::new();
static FCM_TOKEN: OnceLock<Arc<Mutex<Option<String>>>> = OnceLock::new();

/// Shared FCM-token cell, so the token can arrive before or after `start`.
fn fcm_token_cell() -> &'static Arc<Mutex<Option<String>>> {
    FCM_TOKEN.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// The running engine, if `MdEngine.start` has been called in this process.
#[uniffi::export]
pub fn running_engine() -> Option<Arc<MdEngine>> {
    ENGINE.get().cloned()
}

/// Hand the Firebase token to the core (before or after `start`). With a
/// running, logged-in engine it re-registers so the server stores it.
#[uniffi::export]
pub fn set_fcm_token(token: String) {
    *fcm_token_cell().lock().unwrap() = (!token.is_empty()).then_some(token);
    if let Some(e) = ENGINE.get() {
        let _ = e.inner.refresh_from_server();
    }
}

/// A fresh random passphrase.
#[uniffi::export]
pub fn generate_key() -> String {
    mdcore::crypto::generate_key()
}

#[uniffi::export]
impl MdEngine {
    /// Start networking (or return the running engine) and attach `listener`.
    /// Signed in last time (and the token is still good): starts signed in.
    #[uniffi::constructor]
    pub fn start(
        device_id: String,
        name: String,
        wifi_ip: String,
        data_dir: String,
        listener: Box<dyn EventListener>,
        secrets: Box<dyn SecretStore>,
    ) -> MdResult<Arc<MdEngine>> {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("mydomain_net"),
        );
        if let Some(e) = ENGINE.get() {
            e.set_listener(listener);
            return Ok(e.clone());
        }
        let wifi = Arc::new(Mutex::new(wifi_ip));
        let platform = AndroidPlatform {
            device_id,
            name: if name.is_empty() {
                "android".into()
            } else {
                name
            },
            wifi_ip: wifi.clone(),
            fcm_token: fcm_token_cell().clone(),
            kv: (!data_dir.is_empty()).then(|| FileKv::in_dir(&data_dir)),
            secrets: Arc::from(secrets),
        };
        let sink = Arc::new(ListenerSink::default());
        *sink.listener.lock().unwrap() = Some(listener);
        let inner = Engine::start(platform, sink.clone() as Arc<dyn EventSink>)?;
        log::info!("engine started");
        let engine = Arc::new(MdEngine {
            inner,
            sink,
            wifi_ip: wifi,
        });
        // Lost a race with another start: use the winner.
        Ok(ENGINE.get_or_init(|| engine).clone())
    }

    /// Replace the event listener (e.g. a recreated service).
    pub fn set_listener(&self, listener: Box<dyn EventListener>) {
        *self.sink.listener.lock().unwrap() = Some(listener);
    }

    pub fn identity(&self) -> DeviceIdentity {
        let id = self.inner.identity();
        DeviceIdentity {
            node_id: id.node_id,
            name: id.name,
            ip: id.ip,
            quic_port: id.quic_port,
        }
    }

    // --- account ---

    pub fn auth_login(
        &self,
        server_url: String,
        username: String,
        password: String,
    ) -> MdResult<()> {
        Ok(self.inner.auth_login(&server_url, &username, &password)?)
    }

    pub fn auth_register(
        &self,
        server_url: String,
        username: String,
        password: String,
    ) -> MdResult<()> {
        Ok(self
            .inner
            .auth_register(&server_url, &username, &password)?)
    }

    pub fn set_encryption_key(&self, passphrase: String) -> MdResult<()> {
        Ok(self.inner.set_encryption_key(&passphrase)?)
    }

    pub fn update_encryption_key(&self, new_passphrase: String, password: String) -> MdResult<()> {
        Ok(self
            .inner
            .update_encryption_key(&new_passphrase, &password)?)
    }

    /// DEV / LAN-only: skip the registry login (see `Engine::dev_login`).
    pub fn dev_login(&self, username: String, passphrase: String) -> MdResult<()> {
        Ok(self.inner.dev_login(&username, &passphrase)?)
    }

    pub fn logout(&self) {
        self.inner.logout();
    }

    pub fn session(&self) -> Session {
        let s = self.inner.session_info();
        Session {
            username: s.username,
            server_url: s.server_url,
            has_key: s.has_key,
        }
    }

    /// Signed in to the server and holding a key. (An offline `dev_login`
    /// has a key but no server session, so it isn't "ready".)
    pub fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    // --- Bluetooth LE (the GATT server lives in Kotlin: BleLink) ---

    /// A computer connected over Bluetooth: returns the link id for
    /// `ble_frame` / `ble_closed`. Our `Hello` goes out through `out` at once.
    pub fn ble_opened(&self, out: Box<dyn BleOut>) -> u64 {
        self.inner.ble_opened(Arc::new(BleOutSender(out)))
    }

    /// A whole frame arrived over Bluetooth link `id`.
    pub fn ble_frame(&self, id: u64, frame: Vec<u8>) {
        self.inner.ble_frame(id, &frame);
    }

    /// Bluetooth link `id` ended.
    pub fn ble_closed(&self, id: u64) {
        self.inner.ble_closed(id);
    }

    // --- devices and links ---

    /// Register this device and fetch the others from the server.
    pub fn refresh(&self) -> MdResult<()> {
        Ok(self.inner.refresh_from_server()?)
    }

    pub fn peers(&self) -> Vec<PeerInfo> {
        let live: std::collections::HashMap<String, String> = self
            .inner
            .links()
            .into_iter()
            .map(|(node, kind)| (node, as_str(kind)))
            .collect();
        self.inner
            .get_peers()
            .into_iter()
            .map(|p| PeerInfo {
                connected: live.contains_key(&p.node_id),
                link: live.get(&p.node_id).cloned(),
                source: as_str(p.source),
                node_id: p.node_id,
                name: p.name,
                ip: p.ip,
                quic_port: p.quic_port,
            })
            .collect()
    }

    /// Add a device by address (Developer screen). Listed as `manual:<ip>`
    /// until the first QUIC link reveals its real node id.
    pub fn add_manual_peer(&self, name: String, ip: String, quic_port: u16) -> MdResult<()> {
        Ok(self.inner.add_manual_peer(&name, &ip, quic_port)?)
    }

    /// The device's network changed (`wifi_ip`: its new Wi-Fi IPv4, or "").
    pub fn network_changed(&self, wifi_ip: String) {
        if wifi_ip.parse::<Ipv4Addr>().is_ok() {
            *self.wifi_ip.lock().unwrap() = wifi_ip.clone();
        }
        self.inner.network_changed(Some(&wifi_ip));
    }

    /// The app's UI went to the background (`true`) or came back. In the
    /// background, idle links are closed to save battery (they re-open on
    /// demand, and pushes still arrive).
    pub fn set_background(&self, background: bool) {
        self.inner.set_background(background);
    }

    pub fn scan_lan(&self) {
        self.inner.scan_lan();
    }

    /// Connect along the ladder, in the background; progress arrives as
    /// `connect-progress` events, success as `peer-connected`.
    pub fn connect(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.connect(&node_id)?)
    }

    /// Close the link now; it comes back when either side sends something.
    pub fn disconnect(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.disconnect(&node_id)?)
    }

    /// Close the link and connect afresh.
    pub fn reconnect(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.reconnect(&node_id)?)
    }

    /// Quality of every live link (RTT and loss, rated by the core).
    pub fn link_stats(&self) -> Vec<LinkQuality> {
        self.inner
            .link_stats()
            .into_iter()
            .map(|s| LinkQuality {
                quality: as_str(s.quality),
                rtt_ms: s.rtt_ms,
                loss_pct: s.loss_pct,
                node_id: s.node_id,
            })
            .collect()
    }

    pub fn firewall_check(&self) -> MdResult<FirewallStatus> {
        let fs = self.inner.firewall_check()?;
        Ok(FirewallStatus {
            outbound_ok: fs.outbound_ok,
            inbound_blocked: fs.inbound_blocked,
        })
    }

    // --- messaging ---

    /// Send a message over the live link (a QUIC link is opened first if
    /// there is none). Returns the transport used, e.g. "QUIC".
    pub fn send(&self, node_id: String, text: String) -> MdResult<String> {
        Ok(self.inner.send(&node_id, &text)?)
    }

    /// `send` with the message id chosen by the app. When the peer has it, a
    /// `message-delivered` event carries the same `id`. Returns the
    /// transport used, or "queued" when there is no link yet (the core sends
    /// it once connected, and resends until acknowledged).
    pub fn send_with_id(&self, node_id: String, text: String, id: String) -> MdResult<String> {
        Ok(self.inner.send_with_id(&node_id, &text, &id)?)
    }

    // --- signaling ---

    /// An inbound FCM signal (from the FirebaseMessagingService).
    pub fn on_signal(&self, from: String, payload: String) {
        self.inner.on_signal(&from, &payload);
    }

    // --- shared notifications and calls ---

    pub fn share_notification(&self, title: String, body: String, app: Option<String>) {
        self.inner.share_notification(&title, &body, app.as_deref());
    }

    /// A verification code the host found in a notification, for your
    /// connected devices; those with a clipboard copy it. Throws if it
    /// doesn't look like a code.
    pub fn share_code(&self, code: String, app: Option<String>) -> MdResult<()> {
        Ok(self.inner.share_code(&code, app.as_deref())?)
    }

    pub fn share_call_notification(&self, caller: String, number: Option<String>, state: String) {
        self.inner
            .share_call_notification(&caller, number.as_deref(), &state);
    }

    /// `entries_json`: JSON array of call-log entries.
    pub fn share_call_history(&self, entries_json: String) {
        self.inner.share_call_history(&entries_json);
    }

    // --- app-notification pub/sub ---

    /// Producer: this device's shareable app list (JSON array of {pkg,label}).
    pub fn set_installed_apps(&self, apps_json: String) {
        self.inner.set_installed_apps(&apps_json);
    }

    /// Consumer: ask a producer for its app list (reply: an `apps-list` event).
    pub fn request_apps(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.request_apps(&node_id)?)
    }

    /// Consumer: the enabled packages (JSON array of strings) on a producer.
    pub fn subscribe_apps(&self, node_id: String, apps_json: String) -> MdResult<()> {
        Ok(self.inner.subscribe_apps(&node_id, &apps_json)?)
    }

    /// Producer: a captured app notification, forwarded to its subscribers.
    pub fn share_app_notification(&self, pkg: String, app: String, title: String, body: String) {
        self.inner.share_app_notification(&pkg, &app, &title, &body);
    }

    /// `share_app_notification`, plus the notification's `key` and its
    /// buttons (`actions_json`: `[{"title":"Reply","reply":true}, ...]`, in
    /// the notification's action order), so another device can press one
    /// (it arrives here as a `notification-action-event`).
    pub fn share_app_notification_with(
        &self,
        pkg: String,
        app: String,
        title: String,
        body: String,
        key: Option<String>,
        actions_json: String,
    ) {
        self.inner.share_app_notification_with(
            &pkg,
            &app,
            &title,
            &body,
            key.as_deref(),
            &actions_json,
        );
    }

    /// Producer: notification `key` from package `pkg`, shared with
    /// `share_app_notification_with`, went away here (swiped, opened, or
    /// cancelled by its app). Its subscribers drop it too.
    pub fn share_app_notification_removed(&self, pkg: String, key: String) {
        self.inner.share_app_notification_removed(&pkg, &key);
    }

    /// Press button `index` of notification `key` on `node_id` (with `text`
    /// for a reply button), or dismiss it with `index` -1. Returns the
    /// transport used, or "queued".
    pub fn notification_action(
        &self,
        node_id: String,
        key: String,
        index: i64,
        text: Option<String>,
    ) -> MdResult<String> {
        Ok(self
            .inner
            .notification_action(&node_id, &key, index, text.as_deref())?)
    }

    // --- quick device actions ---

    /// This device's status, as a JSON object (e.g.
    /// `{"battery":80,"charging":true}`), sent when another device asks.
    pub fn set_device_status(&self, status_json: String) {
        self.inner.set_device_status(&status_json);
    }

    /// Ask a device for its status (reply: a `device-status-event`). Returns
    /// the transport used, or "queued" (delivered when it is reachable).
    pub fn request_status(&self, node_id: String) -> MdResult<String> {
        Ok(self.inner.request_status(&node_id)?)
    }

    /// Run an action a device listed in its status (`actions`), e.g.
    /// `run_action(node, "flashlight", "{\"on\":true}")`; it gets an
    /// `action-event`. Returns the transport used, or "queued".
    pub fn run_action(&self, node_id: String, name: String, args_json: String) -> MdResult<String> {
        Ok(self.inner.run_action(&node_id, &name, &args_json)?)
    }

    /// Ring a device (find my device): it gets a `ring-event`. Returns the
    /// transport used, or "queued" (a sleeping phone is woken by push).
    pub fn ring(&self, node_id: String) -> MdResult<String> {
        Ok(self.inner.ring(&node_id)?)
    }

    /// Open a web link on a device (`open-url-event` there). Fails for a
    /// link that isn't http(s). Returns the transport used, or "queued".
    pub fn open_url(&self, node_id: String, url: String) -> MdResult<String> {
        Ok(self.inner.open_url(&node_id, &url)?)
    }

    /// `open_url`, in app `app` (a package, e.g. "com.google.android.youtube")
    /// if the device has it, else its browser.
    pub fn open_url_in(&self, node_id: String, url: String, app: String) -> MdResult<String> {
        Ok(self.inner.open_url_in(&node_id, &url, &app)?)
    }

    /// Search for `query` in `app` ("youtube", "spotify" or "web") on a
    /// device that lists the `search` action.
    pub fn search_in_app(&self, node_id: String, app: String, query: String) -> MdResult<String> {
        Ok(self.inner.search_in_app(&node_id, &app, &query)?)
    }

    // --- files ---

    /// Where files from your other devices are saved (an app-private dir;
    /// the app moves each one into the public Downloads). Until this is
    /// set, offers are declined.
    pub fn set_download_dir(&self, dir: String) {
        self.inner.set_download_dir(&dir);
    }

    /// Send the file at `path`. Returns the transfer id at once; progress
    /// arrives as `file-progress`, then `file-sent` or `file-failed`. Like a
    /// message, the offer waits for the device to be reachable.
    pub fn send_file(&self, node_id: String, path: String) -> MdResult<String> {
        Ok(self.inner.send_file(&node_id, &path)?)
    }

    /// Send the folder at `path`: each file inside goes as its own transfer
    /// (with the usual `file-*` events), and the other device recreates the
    /// folder under its downloads. Returns how many files it holds.
    pub fn send_folder(&self, node_id: String, path: String) -> MdResult<u32> {
        Ok(self.inner.send_folder(&node_id, &path)?)
    }

    /// How many folders `send_folder` is still sending; 0 once all their
    /// files have ended (the app then deletes the copies it sent from).
    pub fn folders_sending(&self) -> u32 {
        self.inner.folders_sending()
    }

    /// Stop a transfer, either way (a `file-failed` with reason
    /// "cancelled" follows; the other device is told).
    pub fn cancel_file(&self, id: String) {
        self.inner.cancel_file(&id);
    }

    // --- synced folders (mdcore features::folder_sync) ---

    /// Every synced folder here, invitations included.
    pub fn sync_pairs(&self) -> Vec<SyncPairInfo> {
        self.inner
            .sync_pairs()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Sync the folder at `path` with `node_id` (`mode`: "both", "send" or
    /// "receive", this phone's part). The other device is asked. Returns
    /// the pair id.
    pub fn sync_create(&self, node_id: String, path: String, mode: String) -> MdResult<String> {
        Ok(self.inner.sync_create(&node_id, &path, &mode)?)
    }

    /// Accept invitation `id`, keeping the folder at `path` on this phone
    /// (made if it isn't there).
    pub fn sync_accept(&self, id: String, path: String) -> MdResult<()> {
        Ok(self.inner.sync_accept(&id, &path)?)
    }

    /// Stop syncing on both devices, or decline an invitation. Files stay.
    pub fn sync_remove(&self, id: String) -> MdResult<()> {
        Ok(self.inner.sync_remove(&id)?)
    }

    /// Pause or resume a synced folder on this phone.
    pub fn sync_pause(&self, id: String, paused: bool) -> MdResult<()> {
        Ok(self.inner.sync_pause(&id, paused)?)
    }

    /// Compare with the other device now.
    pub fn sync_now(&self, id: String) -> MdResult<()> {
        Ok(self.inner.sync_now(&id)?)
    }

    /// Device `node_id`'s app can sync folders (it said so in its status).
    pub fn sync_supported(&self, node_id: String) -> bool {
        self.inner.sync_supported(&node_id)
    }

    // --- now playing ---

    /// What's playing on this phone, as JSON `{title, artist, album, app,
    /// state}` (`state` "playing" / "paused"), or "" for nothing. Call it on
    /// every change; devices that looked at this phone lately get a
    /// `now-playing` event, and status replies carry it.
    pub fn set_now_playing(&self, media_json: String) {
        self.inner.set_now_playing(&media_json);
    }

    // --- browsing shared folders ---

    /// The folders your other devices may browse and download from
    /// (replaces the previous set; empty shares nothing).
    pub fn set_shared_folders(&self, folders: Vec<SharedDir>) -> MdResult<()> {
        let folders: Vec<mdcore::model::SharedFolder> = folders
            .into_iter()
            .map(|f| mdcore::model::SharedFolder {
                name: f.name,
                path: f.path,
            })
            .collect();
        Ok(self.inner.set_shared_folders(&folders)?)
    }

    /// Ask a device what's in `path` of its shared folder `root` (`root` ""
    /// lists its shared folders), from entry `offset` on. Returns the
    /// request id; the answer is a `folder-listing` event with that `req`.
    pub fn list_folder(
        &self,
        node_id: String,
        root: String,
        path: String,
        offset: u64,
    ) -> MdResult<String> {
        Ok(self.inner.list_folder(&node_id, &root, &path, offset)?)
    }

    /// Fetch a file from a device's shared folder. Returns the transfer id;
    /// it then arrives like a file the device sent (`file-progress`, then
    /// `file-received` or `file-failed`).
    pub fn download_file(&self, node_id: String, root: String, path: String) -> MdResult<String> {
        Ok(self.inner.download_file(&node_id, &root, &path)?)
    }

    // --- sending text ---

    /// Send text to a device: onto its clipboard (`mode` "clipboard") or
    /// shown as a notification ("notify"); it gets a `text-shared` event.
    /// Returns the transport used, or "queued".
    pub fn send_text(&self, node_id: String, text: String, mode: String) -> MdResult<String> {
        Ok(self.inner.send_text(&node_id, &text, &mode)?)
    }

    // --- gallery ---

    /// This phone lists its gallery (`gallery-request` events). Until set,
    /// other devices are told "no photo gallery".
    pub fn set_gallery_enabled(&self, on: bool) {
        self.inner.set_gallery_enabled(on);
    }

    /// Answer `gallery-request` `req` from `node_id`: the page (newest
    /// first), the offset of the next page, or why there's none.
    pub fn gallery_reply(
        &self,
        node_id: String,
        req: String,
        items: Vec<GalleryMedia>,
        next: Option<u64>,
        error: Option<String>,
    ) {
        let items = items
            .into_iter()
            .map(|m| mdcore::model::GalleryItem {
                entry: mdcore::model::GalleryEntry {
                    id: m.id,
                    name: m.name,
                    mime: m.mime,
                    size: m.size,
                    taken: m.taken,
                    width: m.width,
                    height: m.height,
                    duration: m.duration,
                    video: m.video,
                },
                path: m.path,
                thumb: m.thumb,
            })
            .collect();
        self.inner
            .gallery_reply(&node_id, &req, items, next, error.as_deref());
    }

    /// Ask a device for a page of its photos and videos; the answer is a
    /// `gallery-listing` event with the returned `req`, then `gallery-thumb`s.
    pub fn list_gallery(
        &self,
        node_id: String,
        offset: u64,
        limit: u32,
        thumbs: bool,
    ) -> MdResult<String> {
        Ok(self.inner.list_gallery(&node_id, offset, limit, thumbs)?)
    }

    /// Fetch a listed photo or video. Returns the transfer id.
    pub fn download_gallery_item(
        &self,
        node_id: String,
        id: String,
        name: String,
    ) -> MdResult<String> {
        Ok(self.inner.download_gallery_item(&node_id, &id, &name)?)
    }

    // --- photo backup ---

    /// Turn the backup on or off, choose the computer, and whether to wait
    /// for charging (it always waits for unmetered Wi-Fi).
    pub fn backup_configure(
        &self,
        enabled: bool,
        target: String,
        charging_only: bool,
    ) -> MdResult<()> {
        Ok(self
            .inner
            .backup_configure(enabled, &target, charging_only)?)
    }

    /// On unmetered Wi-Fi? Charging? (Pushed whenever either changes.)
    pub fn backup_conditions(&self, unmetered: bool, charging: bool) {
        self.inner.backup_conditions(unmetered, charging);
    }

    /// New photos/videos; ones already sent or waiting are skipped.
    /// Returns how many were added.
    pub fn backup_enqueue(&self, items: Vec<BackupMedia>) -> u32 {
        let items = items
            .into_iter()
            .map(|m| mdcore::model::BackupItem {
                key: m.key,
                path: m.path,
                taken: m.taken,
                month: m.month,
            })
            .collect();
        self.inner.backup_enqueue(items)
    }

    pub fn backup_state(&self) -> BackupInfo {
        let s = self.inner.backup_state();
        BackupInfo {
            enabled: s.enabled,
            target: s.target,
            charging_only: s.charging_only,
            since: s.since,
            queued: s.queued,
            sent: s.sent,
            waiting_for: s.waiting_for,
            current: s.current,
            done: s.done,
            total: s.total,
            last_error: s.last_error,
            last_sent_at: s.last_sent_at,
        }
    }

    // --- battery alerts, keep awake ---

    pub fn battery_alerts(&self) -> BatteryAlertConfig {
        let s = self.inner.battery_alerts();
        BatteryAlertConfig {
            enabled: s.enabled,
            low: s.low,
            charged: s.charged,
        }
    }

    /// Turn the alerts off or on, and set the low level (1-99%) and the
    /// charged level (0 = off, or 50-100%).
    pub fn set_battery_alerts(&self, enabled: bool, low: u8, charged: u8) -> MdResult<()> {
        Ok(self
            .inner
            .set_battery_alerts(mdcore::model::BatteryAlertSettings {
                enabled,
                low,
                charged,
            })?)
    }

    /// Keep a device awake for `minutes` (0 = stop), if its status lists
    /// "keep_awake". Returns the transport used, or "queued".
    pub fn keep_awake(&self, node_id: String, minutes: u32) -> MdResult<String> {
        Ok(self.inner.keep_awake(&node_id, minutes)?)
    }

    // --- touchpad and keyboard for a computer ---

    /// Move a computer's pointer by `dx`, `dy` pixels. Moves add up and go
    /// out together (about 50 a second), so call it for every touch move;
    /// it only adds to what's pending. Fails when there's no link (and
    /// starts connecting): input is never queued.
    pub fn input_move(&self, node_id: String, dx: i32, dy: i32) -> MdResult<()> {
        Ok(self.inner.input_move(&node_id, dx, dy)?)
    }

    /// Scroll a computer by `dx`, `dy` pixels (positive `dy`: down, towards
    /// the end of the page). Batched like moves.
    pub fn input_scroll(&self, node_id: String, dx: i32, dy: i32) -> MdResult<()> {
        Ok(self.inner.input_scroll(&node_id, dx, dy)?)
    }

    /// Click "left", "right" or "middle".
    pub fn input_click(&self, node_id: String, button: String) -> MdResult<()> {
        Ok(self.inner.input_click(&node_id, &button)?)
    }

    /// Type `text` on a computer.
    pub fn input_text(&self, node_id: String, text: String) -> MdResult<()> {
        Ok(self.inner.input_text(&node_id, &text)?)
    }

    /// Press "enter", "backspace", "left", "right", "up", "down" or "esc".
    pub fn input_key(&self, node_id: String, key: String) -> MdResult<()> {
        Ok(self.inner.input_key(&node_id, &key)?)
    }

    /// The devices watching this phone's screen now: while there are any,
    /// send snapshots with `screen_frame`.
    pub fn screen_watchers(&self) -> Vec<String> {
        self.inner.screen_watchers()
    }

    /// Send a snapshot of this screen (a JPEG) to everyone watching;
    /// returns how many it went to.
    pub fn screen_frame(&self, jpeg: Vec<u8>) -> MdResult<u32> {
        let sent = self.inner.screen_frame(&jpeg)?;
        Ok(u32::try_from(sent).unwrap_or(u32::MAX))
    }

    /// `screen_frame`, only to those of `nodes` watching: the computers
    /// you chose to share this screen with.
    pub fn screen_frame_to(&self, jpeg: Vec<u8>, nodes: Vec<String>) -> MdResult<u32> {
        let sent = self.inner.screen_frame_to(&jpeg, &nodes)?;
        Ok(u32::try_from(sent).unwrap_or(u32::MAX))
    }

    /// Tell a computer this phone shares its screen with it (`on`: it
    /// starts watching), or stopped. Only to a computer whose status says
    /// `screen_view` (older ones would show it as chat). Fails when there's
    /// no link.
    pub fn screen_offer(&self, node_id: String, on: bool) -> MdResult<()> {
        Ok(self.inner.screen_offer(&node_id, on)?)
    }

    /// This phone sends its audio (`source`: Android 10 or later) or
    /// plays another's (`sink`). Said in its status (`audio_source`,
    /// `audio_sink`); off until set.
    pub fn set_audio_roles(&self, sink: bool, source: bool) {
        self.inner.set_audio_roles(sink, source);
    }

    /// Whether a computer said it plays a phone's audio (`audio_sink` in
    /// its status; kept across restarts).
    pub fn audio_sink(&self, node_id: String) -> bool {
        self.inner.audio_caps(&node_id).0
    }

    /// Whether a computer said it sends its sound (`audio_source` in its
    /// status, or it offered it; kept across restarts).
    pub fn audio_source(&self, node_id: String) -> bool {
        self.inner.audio_caps(&node_id).1
    }

    /// Play a computer's sound on this phone (`on`; renew every 10 s while
    /// playing, a listen ends by itself after 25 s), or stop (also the
    /// answer "no" to its offer). Its frames arrive as `audio-frame` events
    /// while this listens. Only to a computer whose status says
    /// `audio_source`; fails with no link (and starts connecting).
    pub fn audio_listen(&self, node_id: String, on: bool) -> MdResult<()> {
        Ok(self.inner.audio_listen(&node_id, on)?)
    }

    /// The computers listening to this phone's audio now: while there are
    /// any, capture and send with `audio_frame_to`.
    pub fn audio_listeners(&self) -> Vec<String> {
        self.inner.audio_listeners()
    }

    /// Tell a computer this phone plays its audio to it (`on`: it starts
    /// listening), or stopped (also the answer when you said no). Returns
    /// the rate to capture at for the link (48000 direct, 24000 relayed; 0
    /// when stopping). Only to a computer whose status says `audio_sink`;
    /// fails with no link, or over Bluetooth only.
    pub fn audio_offer(&self, node_id: String, on: bool) -> MdResult<u32> {
        Ok(self.inner.audio_offer(&node_id, on)?)
    }

    /// Send captured audio (s16le PCM, `channels` interleaved, at `rate`
    /// Hz; at most 100 ms of 48 kHz stereo) to those of `nodes` listening;
    /// returns how many it went to.
    pub fn audio_frame_to(
        &self,
        pcm: Vec<u8>,
        rate: u32,
        channels: u8,
        nodes: Vec<String>,
    ) -> MdResult<u32> {
        let sent = self.inner.audio_frame_to(&pcm, rate, channels, &nodes)?;
        Ok(u32::try_from(sent).unwrap_or(u32::MAX))
    }

    /// Send a picture copied on this phone (a PNG or JPEG file) to a
    /// device's clipboard. Returns the transfer id.
    pub fn send_clipboard_image(&self, node_id: String, path: String) -> MdResult<String> {
        Ok(self.inner.send_clipboard_image(&node_id, &path)?)
    }

    /// Whether a device ever said how to wake it from sleep (Wake-on-LAN).
    pub fn can_wake(&self, node_id: String) -> bool {
        self.inner.can_wake(&node_id)
    }

    /// Wake a sleeping computer: its magic packet, broadcast on this
    /// network (works from the same Wi-Fi, if it's set to wake for network
    /// access).
    pub fn wake_device(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.wake_device(&node_id)?)
    }

    /// Start (`on`) or stop watching a computer's screen: snapshots arrive
    /// as `screen_frame` events while it allows it. Renew every 10 s while
    /// the screen shows (a watch ends by itself after 25 s). Fails when
    /// there's no link (and starts connecting).
    pub fn screen_watch(&self, node_id: String, on: bool) -> MdResult<()> {
        Ok(self.inner.screen_watch(&node_id, on)?)
    }

    /// Put a computer's pointer at a spot on its screen: `x`, `y` from 0 to
    /// 10000 across its width and height, from the top left.
    pub fn input_point(&self, node_id: String, x: u16, y: u16) -> MdResult<()> {
        Ok(self.inner.input_point(&node_id, x, y)?)
    }

    /// A three- or four-finger swipe: `dir` is "left", "right", "up" or
    /// "down", the way the fingers moved.
    pub fn input_gesture(&self, node_id: String, fingers: u8, dir: String) -> MdResult<()> {
        Ok(self.inner.input_gesture(&node_id, fingers, &dir)?)
    }

    // --- Do Not Disturb ---

    /// This phone's Do Not Disturb state (read from the OS); your other
    /// devices are told when it changes.
    pub fn set_dnd(&self, on: bool) {
        self.inner.set_dnd(on);
    }
}
