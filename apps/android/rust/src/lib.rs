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

/// This device as the core sees it.
#[derive(uniffi::Record)]
pub struct DeviceIdentity {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub quic_port: u16,
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
}
