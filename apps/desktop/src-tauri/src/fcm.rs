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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use fcm_receiver_rs::client::{AndroidApp, FcmClient};

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

/// Bumped by `reregister`; a receiver thread from an older generation stops at
/// its next reconnect instead of listening on a dead registration forever.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Throw away the saved registration and register a fresh token. Used when
/// FCM reports our token as unregistered (it expired or was revoked): the old
/// saved registration is kept as `fcm_creds.stale.json` for debugging.
pub fn reregister(app: AppHandle, token_cell: Arc<Mutex<Option<String>>>) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::rename(dir.join(CREDS_FILE), dir.join("fcm_creds.stale.json"));
    }
    *token_cell.lock().unwrap() = None;
    GENERATION.fetch_add(1, Ordering::SeqCst);
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

    let generation = GENERATION.load(Ordering::SeqCst);
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
            std::thread::sleep(wait);
            if GENERATION.load(Ordering::SeqCst) != generation {
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
    // Web-push messages arrive encrypted and come out of `on_data_message`
    // as JSON; Android-mode messages arrive in the clear as key/value
    // `app_data` on `on_raw_message`. Both feed the same handler.
    let on_json = deliver.clone();
    client.on_data_message = Some(Arc::new(move |payload: Vec<u8>| on_json(payload)));
    client.on_raw_message = Some(Arc::new(move |msg| {
        let fields: serde_json::Map<String, serde_json::Value> = msg
            .app_data
            .iter()
            .map(|d| (d.key.clone(), serde_json::Value::String(d.value.clone())))
            .collect();
        deliver(serde_json::Value::Object(fields).to_string().into_bytes());
    }));

    // Reuse a persisted registration if we have one; otherwise register fresh.
    let token = match load_creds(&creds_path) {
        Some(creds) => {
            client.android_id = creds.android_id;
            client.security_token = creds.security_token;
            client.gcm_token = Some(creds.gcm_token);
            client.fcm_token = Some(creds.fcm_token.clone());
            client
                .load_keys(&creds.private_key_base64, &creds.auth_secret_base64)
                .map_err(|e| e.to_string())?;
            creds.fcm_token
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
            fcm_token
        }
    };

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

    // Listen forever; reconnect with a fixed backoff on any drop.
    loop {
        emit_fcm(
            &app,
            "info",
            "",
            "MTalk connected — listening for push signals",
        );
        if let Err(e) = client.start_listening() {
            emit_fcm(
                &app,
                "error",
                "",
                &format!("listen ended: {e}; reconnecting in 10s"),
            );
            eprintln!("[fcm] listen ended: {e}; reconnecting in 10s");
        }
        std::thread::sleep(Duration::from_secs(10));
        if GENERATION.load(Ordering::SeqCst) != generation {
            return Ok(()); // superseded by a fresh registration
        }
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
