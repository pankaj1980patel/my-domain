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

    // Notes (`notes.json`) beside the session file, in the app's files.
    fn data_file(&self, name: &str) -> Option<std::path::PathBuf> {
        Some(self.kv.as_ref()?.path().with_file_name(name))
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
    /// An `audio-frame` event, many a second: the PCM as bytes (s16le,
    /// `channels` interleaved, at `rate` Hz), so the player never parses
    /// JSON or base64 per frame. Never comes as `on_event`.
    fn on_audio_frame(&self, from: String, seq: u64, rate: u32, channels: u8, pcm: Vec<u8>);
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

/// What waking a sleeping computer did (`wake_device`).
/// A computer's request for a photo or a scan (insert from phone,
/// `mdcore::features::insert_capture`).
#[derive(uniffi::Record)]
pub struct InsertRequestInfo {
    /// The request's id: pass it to `insert_opened`, `insert_send`,
    /// `insert_cancel`.
    pub id: String,
    /// The computer asking (node id).
    pub from: String,
    /// "photo" or "scan".
    pub mode: String,
}

#[derive(uniffi::Record)]
pub struct WakeResult {
    /// Where this phone sent the magic packet (`ip:port`).
    pub sent: Vec<String>,
    /// Your other devices asked to send it too, on its network (node ids).
    pub relays: Vec<String>,
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
    /// "off", "target", "battery", "wifi", "charging", "retry", or ""
    /// (sending / done).
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

/// A note in the notepad shared with your other devices (`notes_list`,
/// and `notes_changed` events with the same fields under `notes`).
#[derive(uniffi::Record)]
pub struct NoteItem {
    pub id: String,
    pub title: String,
    /// Plain text, up to 64 KB.
    pub body: String,
    pub pinned: bool,
    /// Lines starting with "[ ] " / "[x] " are drawn as checkboxes.
    pub checklist: bool,
    pub color: Option<String>,
    /// Unix ms.
    pub created_at: i64,
    /// Unix ms.
    pub updated_at: i64,
    /// The device that last changed it (node id) and its name then.
    pub by: String,
    pub by_name: String,
    /// A conflict copy: the note it split from.
    pub conflict_of: Option<String>,
    /// A reminder: when (unix ms) ...
    pub remind_at: Option<i64>,
    /// ... and marked Done (on any device).
    pub remind_done: bool,
}

impl From<mdcore::model::Note> for NoteItem {
    fn from(n: mdcore::model::Note) -> Self {
        NoteItem {
            id: n.id,
            title: n.title,
            body: n.body,
            pinned: n.pinned,
            checklist: n.checklist,
            color: n.color,
            created_at: i64::try_from(n.created_at).unwrap_or(i64::MAX),
            updated_at: i64::try_from(n.updated_at).unwrap_or(i64::MAX),
            by: n.by,
            by_name: n.by_name,
            conflict_of: n.conflict_of,
            remind_at: n.remind_at.map(|t| i64::try_from(t).unwrap_or(i64::MAX)),
            remind_done: n.remind_done,
        }
    }
}

/// A countdown timer shared with your other devices (`timers_list`, and
/// `timers_changed` events with the same fields under `timers`).
#[derive(uniffi::Record)]
pub struct TimerItem {
    pub id: String,
    pub label: String,
    /// How long it was set for, in ms.
    pub duration_ms: i64,
    /// "running" or "paused".
    pub state: String,
    /// Running: when it ends (unix ms; past: ended, ringing until
    /// dismissed). Paused: 0.
    pub ends_at: i64,
    /// Paused: what's left, in ms.
    pub remaining_ms: i64,
    /// The devices (node ids) that ring when it ends.
    pub ring_on: Vec<String>,
    /// Unix ms.
    pub created_at: i64,
    pub updated_at: i64,
    /// The device that last changed it (node id) and its name then.
    pub by: String,
    pub by_name: String,
}

impl From<mdcore::model::Timer> for TimerItem {
    fn from(t: mdcore::model::Timer) -> Self {
        let ms = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        TimerItem {
            id: t.id,
            label: t.label,
            duration_ms: ms(t.duration_ms),
            state: t.state,
            ends_at: ms(t.ends_at),
            remaining_ms: ms(t.remaining_ms),
            ring_on: t.ring_on,
            created_at: ms(t.created_at),
            updated_at: ms(t.updated_at),
            by: t.by,
            by_name: t.by_name,
        }
    }
}

/// A link in the "Read later" list shared with your other devices
/// (`read_later_list`, and `read_later_changed` events with the same fields
/// under `items`).
#[derive(uniffi::Record)]
pub struct ReadLaterEntry {
    /// Made from the link, the same on every device.
    pub id: String,
    pub url: String,
    /// The title the share gave, else the host (nothing is fetched).
    pub title: String,
    pub host: String,
    pub read: bool,
    /// Unix ms, when it was (last) saved.
    pub saved_at: i64,
    /// The name of the device it was saved on.
    pub saved_on: String,
    /// Unix ms.
    pub updated_at: i64,
}

impl From<mdcore::model::ReadLaterItem> for ReadLaterEntry {
    fn from(i: mdcore::model::ReadLaterItem) -> Self {
        ReadLaterEntry {
            id: i.id,
            url: i.url,
            title: i.title,
            host: i.host,
            read: i.read,
            saved_at: i64::try_from(i.saved_at).unwrap_or(i64::MAX),
            saved_on: i.saved_on,
            updated_at: i64::try_from(i.updated_at).unwrap_or(i64::MAX),
        }
    }
}

/// A link downloaded on one of your devices at another's request
/// (`remote_downloads`; `remote_download` events carry the same fields
/// under `item`). `here`: this phone downloads it.
#[derive(uniffi::Record)]
pub struct RemoteDownloadEntry {
    /// Also its transfer id (file_progress and the others, `cancel_file`).
    pub id: String,
    pub node_id: String,
    pub here: bool,
    pub url: String,
    pub host: String,
    pub name: String,
    /// Bytes, 0 while not known.
    pub size: i64,
    pub done: i64,
    /// requested, checking, asking, queued, downloading, done, failed,
    /// cancelled or refused.
    pub state: String,
    pub error: Option<String>,
    pub path: Option<String>,
    /// Unix ms.
    pub started_at: i64,
    pub updated_at: i64,
}

impl From<mdcore::model::RemoteDownloadItem> for RemoteDownloadEntry {
    fn from(i: mdcore::model::RemoteDownloadItem) -> Self {
        let n = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
        RemoteDownloadEntry {
            id: i.id,
            node_id: i.node_id,
            here: i.here,
            url: i.url,
            host: i.host,
            name: i.name,
            size: n(i.size),
            done: n(i.done),
            state: i.state,
            error: i.error,
            path: i.path,
            started_at: n(i.started_at),
            updated_at: n(i.updated_at),
        }
    }
}

/// What this phone does when another of your devices asks it to download
/// a link: `policy` "ask" (the first time for each device), "always" or
/// "never"; `max_mb` 0 = no limit; `allowed`: devices allowed without
/// asking.
#[derive(uniffi::Record)]
pub struct RemoteDownloadPrefs {
    pub policy: String,
    pub allow_lan: bool,
    pub max_mb: i64,
    pub allowed: Vec<String>,
}

impl From<mdcore::model::RemoteDownloadSettings> for RemoteDownloadPrefs {
    fn from(s: mdcore::model::RemoteDownloadSettings) -> Self {
        RemoteDownloadPrefs {
            policy: s.policy,
            allow_lan: s.allow_lan,
            max_mb: i64::try_from(s.max_mb).unwrap_or(i64::MAX),
            allowed: s.allowed,
        }
    }
}

impl From<RemoteDownloadPrefs> for mdcore::model::RemoteDownloadSettings {
    fn from(s: RemoteDownloadPrefs) -> Self {
        mdcore::model::RemoteDownloadSettings {
            policy: s.policy,
            allow_lan: s.allow_lan,
            max_mb: u64::try_from(s.max_mb).unwrap_or(0),
            allowed: s.allowed,
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

/// "Add a device": a one-time join link as a QR code, and as 6 digits.
#[derive(uniffi::Record)]
pub struct JoinInvite {
    pub link: String,
    pub code: String,
    /// Seconds it stays valid.
    pub expires_in: u32,
    /// Modules per side.
    pub qr_size: u32,
    /// Row-major, 1 = dark, `qr_size * qr_size` of them (no margin).
    pub qr_modules: Vec<u8>,
}

/// Comparing security keys with one of your devices (mdcore
/// `KeyVerification`), its QR code as modules.
#[derive(uniffi::Record)]
pub struct KeyVerify {
    pub node_id: String,
    pub name: String,
    /// This phone's short fingerprint, and the other device's.
    pub own_fingerprint: String,
    pub their_fingerprint: String,
    /// 12 groups of 5 digits: the same on both devices.
    pub safety_number: String,
    pub verified: bool,
    /// Its key changed and that isn't settled.
    pub key_changed: bool,
    /// What the QR code holds (`mydomain://verify?…`).
    pub code: String,
    pub qr_size: u32,
    /// Row-major, 1 = dark, `qr_size * qr_size` of them (no margin).
    pub qr_modules: Vec<u8>,
}

/// What came of a scanned verify code: the device now verified, or why
/// not ("not_a_code", "not_for_this_device", "unknown_device",
/// "key_changed", "mismatch").
#[derive(uniffi::Record)]
pub struct VerifyResult {
    pub node_id: Option<String>,
    pub problem: Option<String>,
}

/// Has the new device joined: "waiting", "joined" (with its name), "none".
#[derive(uniffi::Record)]
pub struct JoinStatus {
    pub state: String,
    pub name: Option<String>,
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
        let guard = self.listener.lock().unwrap();
        let Some(l) = guard.as_ref() else {
            return;
        };
        if let CoreEvent::AudioFrame {
            from,
            seq,
            rate,
            channels,
            pcm,
        } = ev
        {
            use base64::Engine as _;
            if let Ok(pcm) = base64::engine::general_purpose::STANDARD.decode(pcm.as_bytes()) {
                l.on_audio_frame(from, seq, rate, channels, pcm);
            }
            return;
        }
        l.on_event(ev.name().to_string(), ev.to_json().to_string());
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

    // --- adding a device by QR code / 6-digit code (mdcore `join`) ---

    /// Start adding a device: a fresh single-use invite, valid 5 minutes.
    pub fn join_invite(&self) -> MdResult<JoinInvite> {
        let inv = self.inner.join_invite()?;
        let qr = mdcore::qr::QrCode::encode(inv.link.as_bytes())
            .ok_or_else(|| MdError::Failed("the join link is too long for a QR code".into()))?;
        Ok(JoinInvite {
            link: inv.link,
            code: inv.code,
            expires_in: inv.expires_in,
            qr_size: qr.size as u32,
            qr_modules: qr.modules.iter().map(|&d| u8::from(d)).collect(),
        })
    }

    pub fn join_status(&self) -> MdResult<JoinStatus> {
        let s = self.inner.join_status()?;
        Ok(JoinStatus {
            state: s.state,
            name: s.name,
        })
    }

    /// The invite was closed: withdraw it.
    pub fn join_cancel(&self) -> MdResult<()> {
        Ok(self.inner.join_cancel()?)
    }

    /// Sign in with a scanned `mydomain://join?…` link (the account and
    /// device key come with it).
    pub fn join_with_link(&self, link: String) -> MdResult<()> {
        Ok(self.inner.join_with_link(&link)?)
    }

    /// Sign in with the 6-digit code another device shows.
    pub fn join_with_code(&self, server_url: String, code: String) -> MdResult<()> {
        Ok(self.inner.join_with_code(&server_url, &code)?)
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

    /// The device's network changed, and to what (`net`: "wifi",
    /// "ethernet", "mobile" or "offline"): links lost are dialled again at
    /// once (then with backoff), nothing is dialled while offline, and
    /// links made on mobile data keep alive sooner. `wifi_ip`: the Wi-Fi
    /// IPv4, or "" without one (the address is then worked out again
    /// instead of keeping the old Wi-Fi one).
    pub fn network_changed_on(&self, wifi_ip: String, net: String) {
        *self.wifi_ip.lock().unwrap() = if wifi_ip.parse::<Ipv4Addr>().is_ok() {
            wifi_ip.clone()
        } else {
            Ipv4Addr::UNSPECIFIED.to_string()
        };
        self.inner.network_changed_on(Some(&wifi_ip), &net);
    }

    /// A push from `from` arrived `waited_ms` ago (before the engine was
    /// ready): the link it brings is timed from then.
    pub fn push_received(&self, from: String, waited_ms: u64) {
        self.inner.push_received(&from, waited_ms);
    }

    /// The app's UI went to the background (`true`) or came back. In the
    /// background, idle links are closed to save battery (they re-open on
    /// demand, and pushes still arrive).
    pub fn set_background(&self, background: bool) {
        self.inner.set_background(background);
    }

    /// The phone's battery saver: `heavy_allowed` false holds back synced
    /// folder rounds and the photo backup; `metered_cap` (bytes, 0 = none)
    /// keeps bigger synced files waiting while on a metered network.
    pub fn set_power_policy(&self, heavy_allowed: bool, metered_cap: u64) {
        self.inner.set_power_policy(heavy_allowed, metered_cap);
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

    // --- check connection ---

    /// The connection report for a device, as JSON (the core's
    /// `Diagnosis`): the link, both ends' addresses and networks, round
    /// trips measured now, recent connect attempts and the relay server.
    /// Blocks for a few seconds: call off the main thread.
    pub fn diagnose(&self, node_id: String) -> MdResult<String> {
        let d = self.inner.diagnose(&node_id)?;
        serde_json::to_string(&d).map_err(|e| MdError::Failed(e.to_string()))
    }

    /// "How your devices are connected", as JSON (the core's `LinksMap`):
    /// this phone's link to each of your devices (or why there is none, per
    /// path tried), and the links your other devices reported between
    /// themselves. Quick: measures nothing. Changes come as
    /// `peer_connected` / `peer_disconnected` / `links_map_changed` events.
    pub fn links_map(&self) -> MdResult<String> {
        serde_json::to_string(&self.inner.links_map()).map_err(|e| MdError::Failed(e.to_string()))
    }

    /// Devices whose security key changed (reinstalled, data cleared, or
    /// someone else using their id), as JSON (the core's `KeyChange`
    /// list, newest first): their direct connections are refused until the
    /// user trusts the new key. A new one comes as a `key_changed` event
    /// (once per key).
    pub fn key_changes(&self) -> MdResult<String> {
        serde_json::to_string(&self.inner.key_changes()).map_err(|e| MdError::Failed(e.to_string()))
    }

    /// The user trusts `node_id`'s new key: the one whose fingerprint they
    /// were shown (`new_fingerprint`); only that key is remembered. Fails
    /// when no change is recorded or its key changed again since.
    pub fn trust_new_key(&self, node_id: String, new_fingerprint: String) -> MdResult<()> {
        Ok(self.inner.trust_new_key(&node_id, &new_fingerprint)?)
    }

    /// "Not now": keep refusing `node_id`'s new key, stop asking about it.
    pub fn dismiss_key_change(&self, node_id: String) -> bool {
        self.inner.dismiss_key_change(&node_id)
    }

    /// Remove `node_id` for good, for that install: a signed revocation goes
    /// to all your devices, which drop it and refuse it from then on (even
    /// with the passphrase). Undone with `allow_device_again`.
    pub fn remove_device(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.remove_device(&node_id)?)
    }

    /// "Allow again": take a removal back, on all your devices.
    pub fn allow_device_again(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.allow_device_again(&node_id)?)
    }

    /// The removed devices as JSON (the core's `RemovedDevice` list, the
    /// most recently removed first). Changes come as a
    /// `revocations_changed` event.
    pub fn removed_devices(&self) -> MdResult<String> {
        serde_json::to_string(&self.inner.removed_devices())
            .map_err(|e| MdError::Failed(e.to_string()))
    }

    /// Start changing the passphrase on all your devices
    /// (`mdcore::features::rekey`): the new key goes to each trusted
    /// device, sealed to it. Returns the change as JSON (the core's
    /// `KeyRotation`); progress comes as `key_rotation_changed` events.
    pub fn start_key_rotation(&self, new_passphrase: String) -> MdResult<String> {
        let r = self.inner.start_key_rotation(&new_passphrase)?;
        serde_json::to_string(&r).map_err(|e| MdError::Failed(e.to_string()))
    }

    /// The passphrase change under way (or the last one) as JSON, and
    /// whether this phone needs the new passphrase typed in.
    pub fn key_rotation(&self) -> MdResult<String> {
        serde_json::to_string(&self.inner.key_rotation())
            .map_err(|e| MdError::Failed(e.to_string()))
    }

    /// Finish: every device switches (again: stop waiting for the rest).
    pub fn finish_key_rotation(&self) -> MdResult<()> {
        self.inner.finish_key_rotation()?;
        Ok(())
    }

    /// Cancel before finishing: every device drops the new key.
    pub fn cancel_key_rotation(&self) -> MdResult<()> {
        self.inner.cancel_key_rotation()?;
        Ok(())
    }

    /// Clear a finished change, or the "type the new passphrase" notice.
    pub fn dismiss_key_rotation(&self) {
        self.inner.dismiss_key_rotation();
    }

    /// How strong a passphrase being typed is: 0 (too short) to 4.
    pub fn passphrase_strength(&self, passphrase: String) -> u8 {
        self.inner.passphrase_strength(&passphrase)
    }

    /// This phone's own security key as a short fingerprint (Settings, Add
    /// a device): what your other devices show for it.
    pub fn own_fingerprint(&self) -> String {
        self.inner.own_fingerprint()
    }

    /// Both fingerprints, the safety number and the QR code to compare with
    /// `node_id`; none until it has connected with a security key.
    pub fn key_verification(&self, node_id: String) -> Option<KeyVerify> {
        let v = self.inner.key_verification(&node_id)?;
        let qr = mdcore::qr::QrCode::encode(v.code.as_bytes());
        Some(KeyVerify {
            qr_size: qr.as_ref().map_or(0, |q| q.size as u32),
            qr_modules: qr
                .map(|q| q.modules.iter().map(|&d| u8::from(d)).collect())
                .unwrap_or_default(),
            node_id: v.node_id,
            name: v.name,
            own_fingerprint: v.own_fingerprint,
            their_fingerprint: v.their_fingerprint,
            safety_number: v.safety_number,
            verified: v.verified,
            key_changed: v.key_changed,
            code: v.code,
        })
    }

    /// The user compared and the keys match (`verified`), or takes it back.
    pub fn set_device_verified(&self, node_id: String, verified: bool) -> MdResult<()> {
        Ok(self.inner.set_device_verified(&node_id, verified)?)
    }

    /// A scanned verify code (`mydomain://verify?…`, opened by the camera
    /// app): marks its device verified when it was made for this phone by
    /// one of your devices and both keys match.
    pub fn verify_scanned(&self, text: String) -> VerifyResult {
        let r = self.inner.verify_scanned(&text);
        VerifyResult {
            node_id: r.node_id,
            problem: r.problem,
        }
    }

    /// The phone's Bluetooth can't link now (`reason`: "off",
    /// "not_paired", "out_of_range", "unsupported"), or can again (none):
    /// the map shows it for devices not linked by Bluetooth.
    pub fn set_ble_state(&self, reason: Option<String>) {
        self.inner.set_ble_state(reason.as_deref());
    }

    /// What this phone can do for big file transfers (1: host a Wi-Fi
    /// Direct group, 2: join one, 4: start a local hotspot; 0 nothing).
    pub fn set_wifi_direct_caps(&self, caps: u64) {
        self.inner.set_wifi_direct_caps(caps);
    }

    /// "Use Wi-Fi Direct for big files when nearby" (on unless turned off).
    pub fn set_wifi_direct_enabled(&self, on: bool) {
        self.inner.set_wifi_direct_enabled(on);
    }

    /// The group (or hotspot) for `wifi_direct_request` `sid` is up, named
    /// `ssid` with `passphrase`; `host_ip`: this phone's address on it, ""
    /// if not known. Never logged.
    pub fn wifi_direct_hosted(
        &self,
        sid: String,
        ssid: String,
        passphrase: String,
        host_ip: String,
    ) {
        self.inner
            .wifi_direct_hosted(&sid, &ssid, &passphrase, &host_ip);
    }

    /// How transfers with `node_id` go now: "wifi_direct", "hotspot", or
    /// none.
    pub fn wifi_direct_via(&self, node_id: String) -> Option<String> {
        self.inner.wifi_direct_via(&node_id)
    }

    /// Joined the group for `sid`; its owner is at `host_ip`.
    pub fn wifi_direct_joined(&self, sid: String, host_ip: String) {
        self.inner.wifi_direct_joined(&sid, &host_ip);
    }

    /// Couldn't host or join for `sid` (no permission, Wi-Fi off, ...): the
    /// transfer stays on its link.
    pub fn wifi_direct_failed(&self, sid: String, reason: String) {
        self.inner.wifi_direct_failed(&sid, &reason);
    }

    /// Transfers with `node_id` go over Wi-Fi Direct right now.
    pub fn wifi_direct_active(&self, node_id: String) -> bool {
        self.inner.wifi_direct_active(&node_id)
    }

    /// Start a speed test with a device (`bytes` each way, at most 32 MB).
    /// Returns its id; progress and the result come as `speed_test` events.
    pub fn speed_test(&self, node_id: String, bytes: u32) -> MdResult<String> {
        Ok(self.inner.speed_test(&node_id, bytes)?)
    }

    /// Stop a running speed test; false if it isn't running.
    pub fn cancel_speed_test(&self, id: String) -> bool {
        self.inner.cancel_speed_test(&id)
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

    /// Send verification code `code` (found in a notification of `app`,
    /// from `issuer`, may be empty) to those of `nodes` that are your
    /// computers and take codes: only the code, app, issuer and expiry go,
    /// end-to-end encrypted, over a live link (else it waits two minutes at
    /// most). Returns the computers it went or will go to; throws if none
    /// of them takes codes, or it doesn't look like a code.
    pub fn send_verification_code(
        &self,
        code: String,
        app: String,
        issuer: String,
        nodes: Vec<String>,
    ) -> MdResult<Vec<String>> {
        Ok(self
            .inner
            .send_verification_code(&code, &app, &issuer, &nodes)?)
    }

    /// Your computers as JSON, for "choose computers":
    /// `[{"node": id, "supported": bool}]` (`supported` false: its app is
    /// too old for verification codes).
    pub fn verification_code_computers_json(&self) -> String {
        let list: Vec<serde_json::Value> = self
            .inner
            .verification_code_computers()
            .into_iter()
            .map(|(node, supported)| serde_json::json!({"node": node, "supported": supported}))
            .collect();
        serde_json::Value::Array(list).to_string()
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

    /// Producer: share a notification of package `pkg` with everything it
    /// has (`notif_json`: title, body, app, key, actions, in_use, conv,
    /// conv_title, group_chat, sender, messages, avatar, picture,
    /// sensitive, silent, ts; see mdcore `features::notif_rich`). Each
    /// subscriber gets what it understands and asked for.
    pub fn share_app_notification_rich(&self, pkg: String, notif_json: String) {
        self.inner.share_app_notification_rich(&pkg, &notif_json);
    }

    /// Producer: app `pkg`'s icon, a small JPEG in base64 (16 KB at most).
    /// False when it isn't one.
    pub fn set_app_icon(&self, pkg: String, jpeg_b64: String) -> bool {
        self.inner.set_app_icon(&pkg, &jpeg_b64)
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

    /// Send several files and folders (paths) as one batch: one row, its
    /// files inside (`transfer_batch` events, and `file_*` events with
    /// `batch` set). Returns how many files go.
    pub fn send_batch(&self, node_id: String, paths: Vec<String>) -> MdResult<u32> {
        let b = self.inner.send_batch(&node_id, &paths)?;
        Ok(u32::try_from(b.count).unwrap_or(u32::MAX))
    }

    /// Pause (`paused`) or resume a transfer or a batch, either way (a
    /// `file_paused` or `transfer_batch` event follows). Receiving, only
    /// from a device that knows how (`transfer_batches_supported`).
    pub fn pause_file(&self, id: String, paused: bool) -> MdResult<()> {
        Ok(self.inner.pause_file(&id, paused)?)
    }

    /// Send a failed transfer again: a batch's files that didn't go, a
    /// file sent from here, or a link asked of a device. Returns the id to
    /// follow (the same for a batch).
    pub fn retry_file(&self, id: String) -> MdResult<String> {
        Ok(self.inner.retry_file(&id)?)
    }

    /// `node_id`'s transfers to here can be paused from here.
    pub fn transfer_batches_supported(&self, node_id: String) -> bool {
        self.inner.transfer_batches_supported(&node_id)
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
    /// state}` (`state` "playing" / "paused"), or "" for nothing, plus
    /// when known `duration` / `position` (ms, as of now), `can_next` /
    /// `can_prev` / `can_seek`, `volume` / `volume_max`, `art` / `icon`
    /// (base64 JPEG/PNG, small), `session` (the session shown) and
    /// `sessions` (`[{id, app, title, state}]` when several apps play).
    /// Call it on every change; devices that looked at this phone lately
    /// get a `now-playing` event (throttled, pictures sent once), and
    /// status replies carry it.
    pub fn set_now_playing(&self, media_json: String) {
        self.inner.set_now_playing(&media_json);
    }

    /// Whether device `node_id` takes media commands (`media_ctl`).
    pub fn media_supported(&self, node_id: String) -> bool {
        self.inner.media_supported(&node_id)
    }

    /// Control what plays on `node_id`: `action` "play", "pause",
    /// "play_pause", "next", "previous", "seek" (`value`: ms),
    /// "volume_up", "volume_down", "volume_set" (`value`: level on its
    /// `volume_max`) or "session" (`session`: the one to show). Needs a
    /// live link (else it starts connecting and errs).
    pub fn media_command(
        &self,
        node_id: String,
        action: String,
        value: Option<i64>,
        session: Option<String>,
    ) -> MdResult<()> {
        Ok(self
            .inner
            .media_command(&node_id, &action, value, session.as_deref())?)
    }

    /// Ask `node_id` to keep this device posted on what it plays for the
    /// next few minutes; false when there's no live link (nothing sent).
    pub fn media_watch(&self, node_id: String) -> MdResult<bool> {
        Ok(self.inner.media_watch(&node_id)?)
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

    /// `send_text`, marked `sensitive` (a password copied from a password
    /// manager): the device keeps it out of its history and notices.
    pub fn send_text_marked(
        &self,
        node_id: String,
        text: String,
        mode: String,
        sensitive: bool,
    ) -> MdResult<String> {
        Ok(self
            .inner
            .send_text_marked(&node_id, &text, &mode, sensitive)?)
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

    /// Whether `node_id` may `perm` ("control", "screen", "kvm") here,
    /// always (per-device permissions, all off until allowed). On the
    /// phone: "screen", a computer allowed to see this screen when you
    /// share it (Android still asks every time).
    pub fn peer_permission(&self, node_id: String, perm: String) -> bool {
        let p = self.inner.peer_permissions(&node_id);
        match perm.as_str() {
            "control" => p.control,
            "screen" => p.screen,
            "kvm" => p.kvm,
            _ => false,
        }
    }

    /// Allow (`on`) `node_id` to do `perm` here, or not; off ends what it
    /// allowed at once.
    pub fn set_peer_permission(&self, node_id: String, perm: String, on: bool) -> MdResult<()> {
        Ok(self.inner.set_peer_permission(&node_id, &perm, on)?)
    }

    /// The owner's answer to a `permission_ask` event: "once", "always"
    /// or "deny".
    pub fn permission_answer(&self, node_id: String, perm: String, answer: String) -> MdResult<()> {
        Ok(self.inner.permission_answer(&node_id, &perm, &answer)?)
    }

    /// The computers asking to see this phone's camera now (its owner
    /// allows each one first).
    pub fn camera_watchers(&self) -> Vec<String> {
        self.inner.camera_watchers()
    }

    /// Send a camera picture (a JPEG) to those of `nodes` watching: the
    /// computers its owner allowed. Returns how many it went to.
    pub fn camera_frame_to(&self, jpeg: Vec<u8>, nodes: Vec<String>) -> MdResult<u32> {
        let sent = self.inner.camera_frame_to(&jpeg, &nodes)?;
        Ok(u32::try_from(sent).unwrap_or(u32::MAX))
    }

    /// Tell a computer the camera is on for it (`on`; which camera, the
    /// torch, `None` where there's none), or off and why (`reason`:
    /// "declined", "stopped", "locked", "no_permission", "busy", "error").
    /// Only to a computer that said `camera_view`; over a live link only.
    pub fn camera_offer(
        &self,
        node_id: String,
        on: bool,
        front: bool,
        torch: Option<bool>,
        reason: Option<String>,
    ) -> MdResult<()> {
        let state = mdcore::features::camera::CameraState {
            on,
            front,
            torch,
            reason,
        };
        Ok(self.inner.camera_offer(&node_id, &state)?)
    }

    /// Insert from phone: the computers' requests for a photo or a scan
    /// that are waiting now (each ends by itself after 3 minutes).
    pub fn insert_requests(&self) -> Vec<InsertRequestInfo> {
        self.inner
            .insert_requests()
            .into_iter()
            .map(|r| InsertRequestInfo {
                id: r.id,
                from: r.from,
                mode: r.mode,
            })
            .collect()
    }

    /// The capture screen for request `id` is up (the computer says so).
    pub fn insert_opened(&self, id: String) -> MdResult<()> {
        Ok(self.inner.insert_opened(&id)?)
    }

    /// Send the photo (a JPEG) or the scan (a PDF) at `path` to the
    /// computer that asked, as the answer to request `id`.
    pub fn insert_send(&self, id: String, path: String) -> MdResult<()> {
        Ok(self.inner.insert_send(&id, &path)?)
    }

    /// Cancel request `id` and tell the computer why (`reason`:
    /// "cancelled", "no_permission", "error"). Nothing to do for a request
    /// that already ended.
    pub fn insert_cancel(&self, id: String, reason: String) -> MdResult<()> {
        self.inner.insert_cancel(&id, &reason);
        Ok(())
    }

    /// Whether computer `node_id` prints for your phones (it said `print`
    /// in its status or link start; kept across restarts). The print
    /// dialog lists only those.
    pub fn print_supported(&self, node_id: String) -> bool {
        self.inner.print_supported(&node_id)
    }

    /// Ask computer `node_id` for its printers: they arrive as the
    /// `printers` event (`on` false: it doesn't print for your phones).
    /// Only to a computer that prints for your phones.
    pub fn request_printers(&self, node_id: String) -> MdResult<()> {
        Ok(self.inner.request_printers(&node_id)?)
    }

    /// Send the PDF at `path` to computer `node_id` to print on `printer`
    /// (`duplex`: "none", "long" or "short"; `paper`: the printer's own
    /// name for it, "" for its default; `orientation`: "portrait" or
    /// "landscape"). Returns the job's id, its transfer's id: the `file-*`
    /// events say how sending goes, `print-status` how printing goes.
    #[allow(clippy::too_many_arguments)]
    pub fn print_file(
        &self,
        node_id: String,
        path: String,
        printer: String,
        copies: u32,
        color: bool,
        duplex: String,
        paper: String,
        orientation: String,
        title: String,
    ) -> MdResult<String> {
        let options = mdcore::features::print::PrintOptions {
            printer,
            copies,
            color,
            duplex,
            paper,
            orientation,
            title,
        };
        Ok(self.inner.print_file(&node_id, &path, &options)?)
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

    /// The switch "Use nearby mode for media" (on by default).
    pub fn nearby_media(&self) -> bool {
        self.inner.nearby_media()
    }

    pub fn set_nearby_media(&self, on: bool) {
        self.inner.set_nearby_media(on);
    }

    /// How a media stream with `node_id` stands, as JSON (the core's
    /// `NearbyStatus`: near, low_latency, path, latency_ms, before_ms,
    /// rtt_ms). Changes come as `nearby-media` events.
    pub fn nearby_status(&self, node_id: String) -> MdResult<String> {
        serde_json::to_string(&self.inner.nearby_status(&node_id))
            .map_err(|e| MdError::Failed(e.to_string()))
    }

    /// What arrived from audio source `node_id`, as JSON (the core's
    /// `AudioRxStats`: frames, recovered, duplicates, concealed).
    pub fn audio_rx_stats(&self, node_id: String) -> MdResult<String> {
        serde_json::to_string(&self.inner.audio_rx_stats(&node_id))
            .map_err(|e| MdError::Failed(e.to_string()))
    }

    /// Which way audio with `node_id` goes: "auto" (a network link first,
    /// else Bluetooth), "wifi" or "bluetooth". Kept per device.
    pub fn audio_route(&self, node_id: String) -> String {
        self.inner.audio_route(&node_id)
    }

    pub fn set_audio_route(&self, node_id: String, route: String) {
        self.inner.set_audio_route(&node_id, &route);
    }

    /// Which way audio with `node_id` went just now: "bluetooth",
    /// "network" or "" (the "via Bluetooth" badge).
    pub fn audio_via(&self, node_id: String) -> String {
        self.inner.audio_via(&node_id)
    }

    /// `node_id` reads Opus audio, so its audio can go over Bluetooth.
    pub fn audio_opus(&self, node_id: String) -> bool {
        self.inner.audio_opus(&node_id)
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
    /// network, and sent by your other devices linked now on its network
    /// (works if it's set to wake for network access).
    pub fn wake_device(&self, node_id: String) -> MdResult<WakeResult> {
        let r = self.inner.wake_device(&node_id)?;
        Ok(WakeResult {
            sent: r.sent,
            relays: r.relays,
        })
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

    // --- device labels shared with your other devices ---

    /// Whether the nicknames, icons and colours you give your devices are
    /// shared with your other devices (and theirs taken here). On until
    /// turned off.
    pub fn labels_share(&self) -> bool {
        self.inner.labels_share()
    }

    pub fn set_labels_share(&self, on: bool) {
        self.inner.set_labels_share(on);
    }

    /// A label set on this phone (all none: cleared): sent to your other
    /// devices while sharing is on. Labels set on them arrive as
    /// `labels_changed` events.
    pub fn set_shared_label(
        &self,
        node_id: String,
        nickname: Option<String>,
        icon: Option<String>,
        color: Option<String>,
    ) -> MdResult<()> {
        Ok(self.inner.set_shared_label(
            &node_id,
            nickname.as_deref(),
            icon.as_deref(),
            color.as_deref(),
        )?)
    }

    /// A label this phone had before labels were shared (at start): kept
    /// unless the core already has one for the device. True if added.
    pub fn seed_label(
        &self,
        node_id: String,
        nickname: Option<String>,
        icon: Option<String>,
        color: Option<String>,
    ) -> bool {
        self.inner.seed_label(
            &node_id,
            nickname.as_deref(),
            icon.as_deref(),
            color.as_deref(),
        )
    }

    // --- what kind of device each is (mdcore features::device_kind) ---

    /// This phone's own kind: `os` "android", `form` phone / tablet /
    /// foldable / tv, its model and maker for display. Said to your other
    /// devices (status, `addr_update`, registry) so they draw its icon.
    pub fn set_device_kind(&self, os: String, form: String, model: String, maker: String) {
        self.inner.set_device_kind(mdcore::model::DeviceKind {
            os,
            form,
            model,
            maker,
            icon: String::new(),
        });
    }

    /// Every device's kind that's known, as JSON: node id -> `{os, form,
    /// model, maker, icon}`. Changes arrive as `device_kind` events
    /// (`{node, device}`).
    pub fn device_kinds_json(&self) -> String {
        serde_json::to_string(&self.inner.device_kinds()).unwrap_or_else(|_| "{}".into())
    }

    // --- first-run setup and the health check (mdcore features::onboarding,
    // features::health, features::app_info) ---

    /// This app's version (BuildConfig.VERSION_NAME): said to your other
    /// devices for their health check. Call once at start.
    pub fn set_app_version(&self, version: String) {
        self.inner.set_app_version(&version);
    }

    /// The health check as JSON (the core's `HealthReport`: `checks`
    /// worst first, `worst`, `bad`, `warn`). `facts_json` is what the app
    /// read (`PlatformFacts`: `permissions` [{id, features, state,
    /// critical}], `disk_free_mb`, `disk_total_mb`). `check_server` asks
    /// the server (blocks up to 5 s: call off the main thread).
    pub fn health_json(&self, facts_json: String, check_server: bool) -> MdResult<String> {
        let facts: mdcore::features::health::PlatformFacts = serde_json::from_str(&facts_json)
            .map_err(|e| MdError::Failed(format!("health facts: {e}")))?;
        serde_json::to_string(&self.inner.health(&facts, check_server))
            .map_err(|e| MdError::Failed(e.to_string()))
    }

    /// Where first-run setup stands on this phone, as JSON (`step`,
    /// `finished`, `skipped_all`, `skipped`, `features`).
    pub fn onboarding_json(&self) -> String {
        serde_json::to_string(&self.inner.onboarding()).unwrap_or_else(|_| "{}".into())
    }

    /// A tap in first-run setup: "next", "back", "skip", "goto" (with
    /// `step`), "skip_all", "finish", "restart". Returns the new state as
    /// JSON.
    pub fn onboarding_act(&self, action: String, step: Option<String>) -> MdResult<String> {
        let s = self.inner.onboarding_act(&action, step.as_deref())?;
        serde_json::to_string(&s).map_err(|e| MdError::Failed(e.to_string()))
    }

    /// The feature groups picked in setup ("clipboard", "notifications",
    /// "files", "media", "find"). Returns the new state as JSON.
    pub fn set_onboarding_features(&self, groups: Vec<String>) -> String {
        serde_json::to_string(&self.inner.set_onboarding_features(&groups))
            .unwrap_or_else(|_| "{}".into())
    }

    /// The health check's "Test push": the server pushes a ping back to
    /// this phone; when it arrives (as a signal) the push line turns green.
    /// Returns the ping's id.
    pub fn fcm_selftest(&self) -> MdResult<String> {
        Ok(self.inner.fcm_selftest()?)
    }

    /// Name this phone (kept, told to the server). Returns the name as kept.
    pub fn set_device_name(&self, name: String) -> MdResult<String> {
        Ok(self.inner.set_device_name(&name)?)
    }

    // --- notes shared with your other devices (mdcore features::notes) ---

    /// Every note, pinned first, then the most recently changed. Notes
    /// changed on your other devices arrive as `notes_changed` events.
    pub fn notes_list(&self) -> Vec<NoteItem> {
        self.inner.notes().into_iter().map(Into::into).collect()
    }

    /// Save a note (`id` none: a new one; a deleted one comes back, which
    /// is how Undo works). Goes to your other devices. Answers with it as
    /// saved. `remind_at` (unix ms, none: no reminder) and `remind_done`:
    /// its reminder (snooze saves a new time, Done saves true).
    #[allow(clippy::too_many_arguments)]
    pub fn note_save(
        &self,
        id: Option<String>,
        title: String,
        body: String,
        pinned: bool,
        checklist: bool,
        color: Option<String>,
        remind_at: Option<i64>,
        remind_done: bool,
    ) -> MdResult<NoteItem> {
        let remind_at = remind_at
            .and_then(|t| u64::try_from(t).ok())
            .filter(|t| *t > 0);
        Ok(self
            .inner
            .save_note(
                id.as_deref(),
                &title,
                &body,
                pinned,
                checklist,
                color.as_deref(),
                remind_at,
                remind_done,
            )?
            .into())
    }

    /// One note (none: no such note, or deleted).
    pub fn note_get(&self, id: String) -> Option<NoteItem> {
        self.inner.note(&id).map(Into::into)
    }

    /// Delete a note on all your devices.
    pub fn note_delete(&self, id: String) -> MdResult<()> {
        Ok(self.inner.delete_note(&id)?)
    }

    // --- timers shared with your other devices (mdcore features::timers) ---

    /// Every timer shown: running ones by when they end (ended ones
    /// first), then paused ones. Timers started or changed on your other
    /// devices arrive as `timers_changed` events.
    pub fn timers_list(&self) -> Vec<TimerItem> {
        self.inner.timers().into_iter().map(Into::into).collect()
    }

    /// Start a timer for `duration_ms` (a second to a week), ringing on
    /// `ring_on` (node ids) when it ends. Goes to your other devices.
    pub fn timer_start(
        &self,
        label: String,
        duration_ms: i64,
        ring_on: Vec<String>,
    ) -> MdResult<TimerItem> {
        let ms = u64::try_from(duration_ms).map_err(|_| MdError::Failed("how long?".into()))?;
        Ok(self.inner.start_timer(&label, ms, &ring_on)?.into())
    }

    /// "pause", "resume", "cancel", "dismiss" (stop it ringing) or
    /// "restart" a timer, on every device.
    pub fn timer_action(&self, id: String, action: String) -> MdResult<TimerItem> {
        let t = match action.as_str() {
            "pause" => self.inner.pause_timer(&id)?,
            "resume" => self.inner.resume_timer(&id)?,
            "cancel" => self.inner.cancel_timer(&id)?,
            "dismiss" => self.inner.dismiss_timer(&id)?,
            "restart" => self.inner.restart_timer(&id)?,
            _ => return Err(MdError::Failed("what to do?".into())),
        };
        Ok(t.into())
    }

    /// Add `ms` to a timer (one that ended starts again for that long).
    pub fn timer_add(&self, id: String, ms: i64) -> MdResult<TimerItem> {
        let ms = u64::try_from(ms).map_err(|_| MdError::Failed("how long?".into()))?;
        Ok(self.inner.add_timer_time(&id, ms)?.into())
    }

    /// Ring on these devices when the timer ends.
    pub fn timer_set_ring(&self, id: String, ring_on: Vec<String>) -> MdResult<TimerItem> {
        Ok(self.inner.set_timer_ring(&id, &ring_on)?.into())
    }

    /// Device `node_id` shares timers (its app is new enough).
    pub fn timers_supported(&self, node_id: String) -> bool {
        self.inner.timers_supported(&node_id)
    }

    // --- read later, shared with your other devices (features::read_later) ---

    /// Every saved link, the most recently saved first. Links changed on
    /// your other devices arrive as `read_later_changed` events.
    pub fn read_later_list(&self) -> Vec<ReadLaterEntry> {
        self.inner
            .read_later()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Save a link to read later (`text`: the link, or a share's text with
    /// one in it; `title` none or blank: the link's host). Saving one that's
    /// there puts it back on top, unread. Goes to your other devices.
    pub fn read_later_save(&self, text: String, title: Option<String>) -> MdResult<ReadLaterEntry> {
        let url = mdcore::features::read_later::find_url(&text)
            .ok_or_else(|| CoreError::InvalidInput("That isn't a web link.".into()))?;
        Ok(self.inner.save_read_later(&url, title.as_deref())?.into())
    }

    /// Mark a saved link read, or unread again, on all your devices.
    pub fn read_later_mark(&self, id: String, read: bool) -> MdResult<ReadLaterEntry> {
        Ok(self.inner.mark_read_later(&id, read)?.into())
    }

    /// Delete a saved link on all your devices.
    pub fn read_later_delete(&self, id: String) -> MdResult<()> {
        Ok(self.inner.delete_read_later(&id)?)
    }

    // --- download on <device> (features::remote_download) ---

    /// Ask `node_id` to download the first web link in `text`. What follows
    /// comes as `remote_download` events and the file_* events (same id).
    pub fn remote_download_request(
        &self,
        node_id: String,
        text: String,
    ) -> MdResult<RemoteDownloadEntry> {
        Ok(self.inner.request_remote_download(&node_id, &text)?.into())
    }

    /// Device `node_id` downloads links for your other devices.
    pub fn remote_download_supported(&self, node_id: String) -> bool {
        self.inner.remote_download_supported(&node_id)
    }

    /// Answer a download waiting on this phone (state "asking").
    pub fn remote_download_answer(&self, id: String, allow: bool, always: bool) -> MdResult<()> {
        Ok(self.inner.answer_remote_download(&id, allow, always)?)
    }

    /// Stop a download either way. False if it isn't one.
    pub fn remote_download_cancel(&self, id: String) -> bool {
        self.inner.cancel_remote_download(&id)
    }

    /// Downloads either way since the app started, the newest first.
    pub fn remote_downloads(&self) -> Vec<RemoteDownloadEntry> {
        self.inner
            .remote_downloads()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    pub fn remote_download_settings(&self) -> RemoteDownloadPrefs {
        self.inner.remote_download_settings().into()
    }

    pub fn set_remote_download_settings(&self, prefs: RemoteDownloadPrefs) -> MdResult<()> {
        Ok(self.inner.set_remote_download_settings(prefs.into())?)
    }
}
