// Desktop FCM receiver.
//
// The desktop has no Firebase SDK (it is a native process, not a browser or an
// Android app), so we use `fcm_receiver_rs`, which registers with FCM the way
// an Android app would (GCM check-in + register3), then holds a persistent
// MTalk (mcs) connection to mtalk.google.com:5228 and delivers data messages.
//
// Android mode, not web mode: since late 2026, tokens from the crate's web
// (Chrome) registration are rejected by the FCM send API as UNREGISTERED the
// moment they're issued, while Android-style tokens are delivered. We register
// as the project's Firebase Android app.
//
// This mirrors the Android `FirebaseMessagingService` path: obtain a token,
// publish it to the registry (via the Engine's `fcm_token()` + a refresh), and
// feed inbound signals into `Engine::on_signal`, which drives the responder side
// of the connection ladder.
//
// Settings (env, else `<config>/fcm_config.json`, else the baked defaults):
//   MYDOMAIN_FCM_API_KEY     Firebase Android app's API key
//   MYDOMAIN_FCM_APP_ID      Firebase Android app id (1:NNN:android:xxxx)
//   MYDOMAIN_FCM_PROJECT_ID  Firebase project id
//   MYDOMAIN_FCM_SENDER_ID   project number (GCM sender id)
//   MYDOMAIN_FCM_PACKAGE     the Android app's package name

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use fcm_receiver_rs::client::{AndroidApp, FcmClient};
use fcm_receiver_rs::consts::{
    FCM_SOCKET_ADDRESS, KMCS_VERSION, K_CLOSE_TAG, K_DATA_MESSAGE_STANZA_TAG, K_HEARTBEAT_ACK_TAG,
    K_HEARTBEAT_PING_TAG,
};
use fcm_receiver_rs::messages::{
    create_login_request_raw, decode_data_message_stanza, ManualDataMessageStanza,
};

use crate::Eng;

/// Mirror an FCM lifecycle/message line to the frontend debug panel. `kind` is
/// one of "token" | "message" | "error" | "info"; the panel renders errors red.
fn emit_fcm(app: &AppHandle, kind: &str, from: &str, text: &str) {
    eprintln!("[fcm] {kind}: {text}");
    let _ = app.emit(
        "fcm-event",
        serde_json::json!({ "kind": kind, "from": from, "text": text }),
    );
}

/// Truncate a string for a one-line log entry.
fn preview(s: &str) -> String {
    if s.chars().count() > 160 {
        format!("{}…", s.chars().take(160).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Firebase Android app of the `my-domain-c0c36` project. None of these are
/// secrets: every Android build ships them in its google-services config.
/// Baking them means every desktop install can receive push signals out of the
/// box; a `fcm_config.json` or env var still overrides any of them per machine.
const DEFAULT_API_KEY: &str = "AIzaSyBF8ElZf7hu25ZWLTT9sf-QyuqQBVmBepA";
const DEFAULT_APP_ID: &str = "1:744722246836:android:f2d00a1c64624738ebfbe1";
const DEFAULT_PROJECT_ID: &str = "my-domain-c0c36";
const DEFAULT_SENDER_ID: &str = "744722246836";
const DEFAULT_PACKAGE: &str = "com.mydomain";
/// Signing-cert SHA-1 sent with the registration. FCM doesn't check it for a
/// Firebase app without SHA restrictions, so a fixed placeholder is enough.
const PACKAGE_CERT: &str = "0000000000000000000000000000000000000000";

#[derive(Clone)]
struct FcmConfig {
    api_key: String,
    app_id: String,
    project_id: String,
    sender_id: String,
    package: String,
}

impl FcmConfig {
    /// Env first, then `<config>/fcm_config.json`, then the defaults.
    fn load(config_dir: &Path) -> Option<FcmConfig> {
        let file: FileConfig = std::fs::read(config_dir.join("fcm_config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let pick = |env: &str, from_file: &str, default: &str| -> String {
            std::env::var(env)
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| Some(from_file.to_string()).filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| default.to_string())
        };

        let cfg = FcmConfig {
            api_key: pick("MYDOMAIN_FCM_API_KEY", &file.api_key, DEFAULT_API_KEY),
            app_id: pick("MYDOMAIN_FCM_APP_ID", &file.app_id, DEFAULT_APP_ID),
            project_id: pick(
                "MYDOMAIN_FCM_PROJECT_ID",
                &file.project_id,
                DEFAULT_PROJECT_ID,
            ),
            sender_id: pick("MYDOMAIN_FCM_SENDER_ID", &file.sender_id, DEFAULT_SENDER_ID),
            package: pick("MYDOMAIN_FCM_PACKAGE", &file.package, DEFAULT_PACKAGE),
        };
        if !cfg.app_id.contains(":android:") {
            // A web app id from an older fcm_config.json: web tokens are dead.
            eprintln!("[fcm] disabled: app_id must be a Firebase Android app id");
            return None;
        }
        Some(cfg)
    }

    fn android_app(&self) -> AndroidApp {
        AndroidApp {
            gcm_sender_id: self.sender_id.clone(),
            android_package: self.package.clone(),
            android_package_cert: PACKAGE_CERT.into(),
        }
    }
}

#[derive(Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    sender_id: String,
    #[serde(default)]
    package: String,
}

/// Saved registration. Named for Android mode, so a web-mode registration
/// saved by older builds (whose token is dead) is never reused.
const CREDS_FILE: &str = "fcm_creds_android.json";

/// Persisted registration so we keep the same FCM token (and MTalk identity)
/// across restarts — re-registering would orphan the old token on the server.
#[derive(Serialize, Deserialize)]
struct Creds {
    fcm_token: String,
    gcm_token: String,
    android_id: u64,
    security_token: u64,
    private_key_base64: String,
    auth_secret_base64: String,
}

/// The FCM token cell, managed by Tauri so commands can trigger `reregister`.
pub struct FcmToken(pub Arc<Mutex<Option<String>>>);

/// How long a receiver thread waits (in a sleep or a blocked MTalk read)
/// before checking whether it has been superseded.
const POLL: Duration = Duration::from_secs(1);

/// Registration generation. `reregister` bumps it; a receiver thread started
/// under an older value notices within `POLL` and exits, instead of listening
/// on a dead registration until its connection happens to drop.
struct Generation(AtomicU64);

impl Generation {
    const fn new() -> Self {
        Generation(AtomicU64::new(0))
    }

    fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn bump(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.current() == generation
    }

    /// Sleep for `total`, checking every `POLL`. False if superseded meanwhile.
    fn sleep(&self, generation: u64, total: Duration) -> bool {
        let until = Instant::now() + total;
        while self.is_current(generation) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(POLL));
        }
        false
    }
}

static GENERATION: Generation = Generation::new();

/// Throw away the saved registration and register a fresh token. Used when
/// FCM reports our token as unregistered (it expired or was revoked): the old
/// saved registration is kept as `fcm_creds.stale.json` for debugging.
pub fn reregister(app: AppHandle, token_cell: Arc<Mutex<Option<String>>>) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::rename(dir.join(CREDS_FILE), dir.join("fcm_creds.stale.json"));
    }
    *token_cell.lock().unwrap() = None;
    GENERATION.bump();
    emit_fcm(
        &app,
        "info",
        "",
        "FCM token rejected; registering a new one",
    );
    spawn(app, token_cell);
}

/// Start the FCM receiver in a background thread. No-op (with a log line) if the
/// required credentials are absent.
pub fn spawn(app: AppHandle, token_cell: Arc<Mutex<Option<String>>>) {
    let config_dir = match app.path().app_config_dir() {
        Ok(d) => {
            let _ = std::fs::create_dir_all(&d);
            d
        }
        Err(e) => {
            eprintln!("[fcm] disabled: no config dir: {e}");
            return;
        }
    };
    let Some(cfg) = FcmConfig::load(&config_dir) else {
        eprintln!(
            "[fcm] disabled: set MYDOMAIN_FCM_APP_ID and MYDOMAIN_FCM_VAPID_KEY \
             (env or {}/fcm_config.json) to receive push signals on desktop",
            config_dir.display()
        );
        return;
    };

    let generation = GENERATION.current();
    std::thread::spawn(move || {
        let creds = config_dir.join(CREDS_FILE);
        // Google's registration endpoint fails intermittently
        // (PHONE_REGISTRATION_ERROR, often several times in a row), so a
        // single attempt at startup left desktops without push for good.
        // Retry with backoff; a success is saved and reused on later starts.
        let mut wait = Duration::from_secs(30);
        loop {
            match run(
                app.clone(),
                token_cell.clone(),
                cfg.clone(),
                creds.clone(),
                generation,
            ) {
                Ok(()) => return,
                Err(e) => eprintln!("[fcm] registration failed: {e}; retrying in {wait:?}"),
            }
            if !GENERATION.sleep(generation, wait) {
                return; // superseded by a fresh registration
            }
            wait = (wait * 2).min(Duration::from_secs(30 * 60));
        }
    });
}

fn run(
    app: AppHandle,
    token_cell: Arc<Mutex<Option<String>>>,
    cfg: FcmConfig,
    creds_path: PathBuf,
    generation: u64,
) -> Result<(), String> {
    let android_app = cfg.android_app();
    let mut client =
        FcmClient::new(cfg.api_key, cfg.app_id, cfg.project_id).map_err(|e| e.to_string())?;
    client.android_app = Some(android_app);

    // Inbound data message -> Engine::on_signal. The data fields set by the
    // server (server/src/signal.rs) are `type`, `from`, `to`, `payload`.
    let handle = app.clone();
    let deliver = Arc::new(move |payload: Vec<u8>| {
        match extract_signal(&payload) {
            Some((from, sig)) => {
                // Self-test pong: the server echoes our ping as `selfping:<sid>`.
                // Receiving it proves the full push path works; don't forward it
                // to the Engine (it isn't a real Signal).
                if let Some(sid) = sig.strip_prefix("selfping:") {
                    emit_fcm(
                        &handle,
                        "info",
                        &from,
                        &format!("self-test pong received — FCM round-trip OK ✓ ({sid})"),
                    );
                } else {
                    emit_fcm(&handle, "message", &from, &preview(&sig));
                    if let Some(engine) = handle.try_state::<Eng>() {
                        engine.on_signal(&from, &sig);
                    }
                }
            }
            None => {
                let raw = String::from_utf8_lossy(&payload);
                emit_fcm(
                    &handle,
                    "message",
                    "",
                    &format!("unparsed payload: {}", preview(&raw)),
                );
            }
        }
    });
    // Android-mode messages arrive in the clear as key/value `app_data`.
    let on_message = move |msg: ManualDataMessageStanza| {
        let fields: serde_json::Map<String, serde_json::Value> = msg
            .app_data
            .iter()
            .map(|d| (d.key.clone(), serde_json::Value::String(d.value.clone())))
            .collect();
        deliver(serde_json::Value::Object(fields).to_string().into_bytes());
    };

    // Reuse a persisted registration if we have one; otherwise register fresh.
    let (token, android_id, security_token) = match load_creds(&creds_path) {
        Some(creds) => {
            client.android_id = creds.android_id;
            client.security_token = creds.security_token;
            client.gcm_token = Some(creds.gcm_token);
            client.fcm_token = Some(creds.fcm_token.clone());
            client
                .load_keys(&creds.private_key_base64, &creds.auth_secret_base64)
                .map_err(|e| e.to_string())?;
            (creds.fcm_token, creds.android_id, creds.security_token)
        }
        None => {
            let (private_key_base64, auth_secret_base64) =
                client.create_new_keys().map_err(|e| e.to_string())?;
            client
                .load_keys(&private_key_base64, &auth_secret_base64)
                .map_err(|e| e.to_string())?;
            // Google refuses a registration made right after check-in
            // (PHONE_REGISTRATION_ERROR); a retry a few seconds later on the
            // same checked-in device succeeds. `register` keeps the device
            // id, so retrying on this client doesn't check in again.
            let mut attempt = 1;
            let (fcm_token, gcm_token, android_id, security_token) = loop {
                match client.register() {
                    Ok(r) => break r,
                    Err(e) if attempt < 8 => {
                        eprintln!("[fcm] register try {attempt}: {e}; retrying");
                        attempt += 1;
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    Err(e) => return Err(e.to_string()),
                }
            };
            save_creds(
                &creds_path,
                &Creds {
                    fcm_token: fcm_token.clone(),
                    gcm_token,
                    android_id,
                    security_token,
                    private_key_base64,
                    auth_secret_base64,
                },
            );
            (fcm_token, android_id, security_token)
        }
    };
    if !GENERATION.is_current(generation) {
        return Ok(()); // superseded while registering: don't publish a stale token
    }

    // Publish the token: the Platform now reports it, and a refresh pushes it to
    // the registry. If we aren't logged in yet, the refresh is a no-op; the
    // frontend refreshes again after login, by which point `fcm_token()` is set.
    let token_preview: String = token.chars().take(12).collect();
    emit_fcm(
        &app,
        "token",
        "",
        &format!("FCM token acquired ({token_preview}…)"),
    );
    *token_cell.lock().unwrap() = Some(token);
    if let Some(engine) = app.try_state::<Eng>() {
        let _ = engine.refresh_from_server();
    }

    // Listen until superseded; reconnect with a fixed backoff on any drop.
    let mut received = Vec::new();
    loop {
        emit_fcm(
            &app,
            "info",
            "",
            "MTalk connected — listening for push signals",
        );
        match listen(
            android_id,
            security_token,
            generation,
            &mut received,
            &on_message,
        ) {
            Ok(()) => return Ok(()), // superseded by a fresh registration
            Err(e) => {
                emit_fcm(
                    &app,
                    "error",
                    "",
                    &format!("listen ended: {e}; reconnecting in 10s"),
                );
                eprintln!("[fcm] listen ended: {e}; reconnecting in 10s");
            }
        }
        if !GENERATION.sleep(generation, Duration::from_secs(10)) {
            return Ok(()); // superseded by a fresh registration
        }
    }
}

/// Client heartbeat cadence, so an idle connection isn't silently dropped.
const HEARTBEAT: Duration = Duration::from_secs(5 * 60);

/// One MTalk session. The crate's `start_listening` blocks in a socket read
/// with no timeout, so a superseded receiver could only notice once the
/// server sent something; this loop reads with a `POLL` timeout instead and
/// returns `Ok(())` as soon as `generation` is stale. `Err` means the
/// connection dropped. `received` holds persistent ids already delivered, so
/// a message the server redelivers after a reconnect isn't handled twice.
fn listen(
    android_id: u64,
    security_token: u64,
    generation: u64,
    received: &mut Vec<String>,
    on_message: &dyn Fn(ManualDataMessageStanza),
) -> Result<(), String> {
    let addr = FCM_SOCKET_ADDRESS
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("mtalk: no address")?;
    let tcp =
        TcpStream::connect_timeout(&addr, Duration::from_secs(15)).map_err(|e| e.to_string())?;
    let _ = tcp.set_nodelay(true);
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(15)));
    let connector = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let mut tls = connector
        .connect("mtalk.google.com", tcp)
        .map_err(|e| e.to_string())?;
    tls.get_ref()
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    let login = create_login_request_raw(android_id, security_token, received)
        .map_err(|e| e.to_string())?;
    send(&mut tls, &login)?;

    let mut frames = Frames::default();
    let mut buf = [0u8; 16 * 1024];
    let mut last_beat = Instant::now();
    loop {
        if !GENERATION.is_current(generation) {
            let _ = tls.shutdown();
            return Ok(());
        }
        if last_beat.elapsed() >= HEARTBEAT {
            send(&mut tls, &[K_HEARTBEAT_PING_TAG, 0])?;
            last_beat = Instant::now();
        }
        match tls.read(&mut buf) {
            Ok(0) => return Err("connection closed by peer".into()),
            Ok(n) => frames.push(&buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(e) => return Err(e.to_string()),
        }
        while let Some((tag, payload)) = frames.next_frame()? {
            match tag {
                K_HEARTBEAT_PING_TAG => send(&mut tls, &[K_HEARTBEAT_ACK_TAG, 0])?,
                K_CLOSE_TAG => return Err("server closed the stream".into()),
                K_DATA_MESSAGE_STANZA_TAG => {
                    let msg = decode_data_message_stanza(&payload).map_err(|e| e.to_string())?;
                    if let Some(id) = &msg.persistent_id {
                        if received.contains(id) {
                            continue;
                        }
                        if received.len() >= 100 {
                            received.remove(0);
                        }
                        received.push(id.clone());
                    }
                    on_message(msg);
                }
                _ => {} // login response, heartbeat ack, iq, ...
            }
        }
    }
}

fn send(tls: &mut native_tls::TlsStream<TcpStream>, bytes: &[u8]) -> Result<(), String> {
    tls.write_all(bytes)
        .and_then(|()| tls.flush())
        .map_err(|e| e.to_string())
}

/// MTalk framing: the server opens with a one-byte protocol version, then
/// sends `tag, varint size, payload` frames.
#[derive(Default)]
struct Frames {
    buf: Vec<u8>,
    version_seen: bool,
}

impl Frames {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete frame, or `None` until more bytes arrive.
    fn next_frame(&mut self) -> Result<Option<(u8, Vec<u8>)>, String> {
        if !self.version_seen {
            let Some(&version) = self.buf.first() else {
                return Ok(None);
            };
            if version < KMCS_VERSION && version != 38 {
                return Err(format!("mtalk: bad version {version}"));
            }
            self.buf.remove(0);
            self.version_seen = true;
        }
        let Some(&tag) = self.buf.first() else {
            return Ok(None);
        };
        let (mut size, mut shift, mut at) = (0usize, 0, 1);
        loop {
            let Some(&b) = self.buf.get(at) else {
                return Ok(None);
            };
            size |= usize::from(b & 0x7f) << shift;
            at += 1;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if at > 5 {
                return Err("mtalk: bad frame size".into());
            }
        }
        if self.buf.len() < at + size {
            return Ok(None);
        }
        let payload = self.buf[at..at + size].to_vec();
        self.buf.drain(..at + size);
        Ok(Some((tag, payload)))
    }
}

/// Pull `(from, payload)` out of a decrypted data message. The data fields may
/// arrive at the top level or nested under a `data` object depending on how FCM
/// packages the web-push body, so check both.
fn extract_signal(bytes: &[u8]) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let obj = v
        .get("data")
        .and_then(|d| d.as_object())
        .or_else(|| v.as_object())?;
    // `from` is an FCM-reserved data key, so the server sends the sender id as
    // `sender`; accept the legacy `from` too for forward/backward compatibility.
    let from = obj
        .get("sender")
        .or_else(|| obj.get("from"))
        .and_then(|v| v.as_str())?
        .to_string();
    let payload = obj.get("payload")?.as_str()?.to_string();
    Some((from, payload))
}

fn load_creds(path: &Path) -> Option<Creds> {
    let creds: Creds = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())?;
    if creds.android_id == 0
        || creds.security_token == 0
        || creds.private_key_base64.is_empty()
        || creds.auth_secret_base64.is_empty()
        || creds.fcm_token.is_empty()
    {
        return None;
    }
    Some(creds)
}

fn save_creds(path: &Path, creds: &Creds) {
    if let Ok(bytes) = serde_json::to_vec_pretty(creds) {
        let _ = std::fs::write(path, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_supersedes_older_generations() {
        let g = Generation::new();
        let first = g.current();
        assert!(g.is_current(first));
        g.bump();
        assert!(!g.is_current(first));
        assert!(g.is_current(g.current()));
    }

    #[test]
    fn sleep_runs_to_the_end_while_current() {
        let g = Generation::new();
        let start = Instant::now();
        assert!(g.sleep(g.current(), Duration::from_millis(50)));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn sleep_stops_promptly_when_superseded() {
        let g = Arc::new(Generation::new());
        let generation = g.current();
        let bumper = g.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            bumper.bump();
        });
        let start = Instant::now();
        assert!(!g.sleep(generation, Duration::from_secs(60)));
        assert!(start.elapsed() < POLL + Duration::from_secs(1));
    }

    #[test]
    fn frames_split_across_reads() {
        let mut f = Frames::default();
        // version, then a heartbeat ping (tag 0, size 0), then a data frame
        // whose 200-byte size needs a two-byte varint, delivered in pieces.
        f.push(&[
            KMCS_VERSION,
            K_HEARTBEAT_PING_TAG,
            0,
            K_DATA_MESSAGE_STANZA_TAG,
            0xc8,
        ]);
        assert_eq!(
            f.next_frame().unwrap(),
            Some((K_HEARTBEAT_PING_TAG, vec![]))
        );
        assert_eq!(f.next_frame().unwrap(), None);
        f.push(&[0x01]);
        f.push(&[7; 150]);
        assert_eq!(f.next_frame().unwrap(), None);
        f.push(&[7; 50]);
        assert_eq!(
            f.next_frame().unwrap(),
            Some((K_DATA_MESSAGE_STANZA_TAG, vec![7; 200]))
        );
        assert_eq!(f.next_frame().unwrap(), None);
    }

    #[test]
    fn frames_reject_old_version() {
        let mut f = Frames::default();
        f.push(&[1, 0, 0]);
        assert!(f.next_frame().is_err());
    }
}
