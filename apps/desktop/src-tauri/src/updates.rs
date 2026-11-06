// Automatic updates (Settings → Updates), through Tauri's updater plugin.
//
//  - where: `latest.json` on the newest published GitHub release
//    (`ENDPOINT`, also in tauri.conf.json), written by the release workflow;
//    every download is checked against the public key in tauri.conf.json
//    (`plugins > updater > pubkey`) before it's installed;
//  - when: `FIRST_DELAY` after start, then every `EVERY`, and on "Check now";
//    never in a debug build, nor in a build whose public key is still the
//    placeholder (`PUBKEY_PLACEHOLDER`: the release was built without the
//    signing secrets, see docs/RELEASING.md);
//  - stable releases only: a version with a pre-release part (1.2.0-beta.1)
//    is never offered (`is_newer`);
//  - found: the UI shows a banner (version, its notes, "Install and
//    restart", "Later"); nothing installs by itself unless "Install updates
//    automatically" is on (off by default): then it downloads in the
//    background and installs when the app quits (`on_exit_requested`), or
//    at once on "Restart now";
//  - no network, a server error, a bad signature: only a line in Settings
//    (`UpdateStatus::error`), no pop-up;
//  - kept in `<app config dir>/prefs.json`: `updates_auto` ("on"/"off").

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Where the release workflow publishes `latest.json`.
pub const ENDPOINT: &str =
    "https://github.com/pankaj1980patel/my-domain/releases/latest/download/latest.json";
/// tauri.conf.json's `pubkey` until a real key is put there.
pub const PUBKEY_PLACEHOLDER: &str = "REPLACE_WITH_UPDATER_PUBLIC_KEY";
/// The first check, after start.
pub const FIRST_DELAY: Duration = Duration::from_secs(30);
/// Then this often.
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How often the schedule looks at the clock (wall clock, so a computer
/// waking from sleep catches up).
const TICK: Duration = Duration::from_secs(30);
/// The UI hears the download's progress at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

const KEY_AUTO: &str = "updates_auto";
/// Sent whenever `UpdateStatus` changes.
pub const EVENT: &str = "update-status";

/// A release found newer than this one.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct AvailableUpdate {
    pub version: String,
    /// The release notes from latest.json (plain text / Markdown).
    pub notes: Option<String>,
    /// When it was published (RFC 3339), if latest.json says.
    pub date: Option<String>,
}

/// Everything Settings and the banner show.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct UpdateStatus {
    /// This app's version.
    pub current: String,
    /// Why updates are off here: "dev" (a debug build) or "unsigned" (built
    /// without the updater key). `None`: they're on.
    pub disabled: Option<String>,
    /// "Install updates automatically".
    pub auto_install: bool,
    /// Always "stable" for now.
    pub channel: String,
    /// "idle", "checking", "downloading", "ready" (downloaded, installs on
    /// quit or on restart) or "installing".
    pub phase: String,
    /// Unix seconds of the last finished check.
    pub last_check: Option<f64>,
    /// The last check or install failed: "network", "server", "signature"
    /// or "install". Cleared by the next success.
    pub error: Option<String>,
    pub available: Option<AvailableUpdate>,
    /// "Later" was chosen for `available` (the banner stays hidden until a
    /// newer one or the next start).
    pub later: bool,
    /// Bytes downloaded so far and the total, if known.
    pub downloaded: f64,
    pub total: Option<f64>,
}

/// The updater's state for the app's lifetime.
pub struct Updates {
    kv: Option<FileKv>,
    auto: AtomicBool,
    status: Mutex<UpdateStatus>,
    /// What the last check found, to download and install.
    update: Mutex<Option<Update>>,
    /// The verified download, waiting to be installed.
    bytes: Mutex<Option<Vec<u8>>>,
    /// A check or download running.
    busy: AtomicBool,
    /// Installing on quit: the next exit request goes through.
    exiting: AtomicBool,
    /// "Later" was chosen for this version.
    later: Mutex<Option<String>>,
}

impl Updates {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let auto = kv.as_ref().and_then(|kv| kv.get(KEY_AUTO)).as_deref() == Some("on");
        let pubkey = app
            .config()
            .plugins
            .0
            .get("updater")
            .and_then(|u| u.get("pubkey"))
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string();
        let status = UpdateStatus {
            current: app.package_info().version.to_string(),
            disabled: disabled_reason(cfg!(debug_assertions), &pubkey).map(str::to_string),
            auto_install: auto,
            channel: "stable".into(),
            phase: "idle".into(),
            last_check: None,
            error: None,
            available: None,
            later: false,
            downloaded: 0.0,
            total: None,
        };
        Updates {
            kv,
            auto: AtomicBool::new(auto),
            status: Mutex::new(status),
            update: Mutex::new(None),
            bytes: Mutex::new(None),
            busy: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            later: Mutex::new(None),
        }
    }

    fn snapshot(&self) -> UpdateStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|e| e.into_inner().clone())
    }

    fn enabled(&self) -> bool {
        self.snapshot().disabled.is_none()
    }
}

/// Why this build never checks: a debug build, or a key not set up.
pub fn disabled_reason(debug: bool, pubkey: &str) -> Option<&'static str> {
    if debug {
        Some("dev")
    } else if pubkey.trim().is_empty() || pubkey.trim() == PUBKEY_PLACEHOLDER {
        Some("unsigned")
    } else {
        None
    }
}

/// A version's numbers and pre-release part ("1.2.3-beta.1+build" →
/// ([1, 2, 3], "beta.1")). Missing numbers count as 0; build metadata is
/// ignored. `None` when it isn't a version.
fn parse(v: &str) -> Option<([u64; 3], String)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next().unwrap_or_default();
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, p.to_string()),
        None => (v, String::new()),
    };
    let mut nums = [0u64; 3];
    let mut parts = core.split('.');
    for n in nums.iter_mut() {
        if let Some(p) = parts.next() {
            *n = p.parse().ok()?;
        }
    }
    if parts.next().is_some() || core.is_empty() {
        return None;
    }
    Some((nums, pre))
}

/// Whether `remote` should be offered to an app at `current`: a stable
/// release (no pre-release part) with a higher version. Anything that
/// doesn't parse is never offered.
pub fn is_newer(current: &str, remote: &str) -> bool {
    let (Some((cur, _)), Some((rem, pre))) = (parse(current), parse(remote)) else {
        return false;
    };
    // Stable channel only.
    pre.is_empty() && rem > cur
}

/// Whether a scheduled check is due: the first `FIRST_DELAY` after
/// `started`, then `EVERY` after the last check (all unix seconds).
pub fn check_due(now: u64, started: u64, last_check: Option<u64>) -> bool {
    match last_check {
        None => now >= started + FIRST_DELAY.as_secs(),
        // A clock set back past the last check: check again.
        Some(last) => now >= last + EVERY.as_secs() || now < last,
    }
}

/// What a failure is called in the status line.
fn error_kind(e: &tauri_plugin_updater::Error) -> &'static str {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Reqwest(_) | E::Network(_) => "network",
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) => "signature",
        _ => "server",
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn set(app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
    let Some(u) = app.try_state::<Updates>() else {
        return;
    };
    let snap = {
        let mut s = u.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        s.clone()
    };
    let _ = app.emit(EVENT, snap);
}

/// The schedule: checks while the app runs. Does nothing in a debug build
/// or one without the key.
pub fn spawn(app: AppHandle) {
    if !app.state::<Updates>().enabled() {
        return;
    }
    let started = now();
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        let last = app
            .state::<Updates>()
            .snapshot()
            .last_check
            .map(|t| t as u64);
        if check_due(now(), started, last) {
            tauri::async_runtime::block_on(check(&app));
        }
    });
}

/// Ask the server; on a find, tell the UI (and download it, with
/// "Install updates automatically" on).
async fn check(app: &AppHandle) {
    let u = app.state::<Updates>();
    if !u.enabled() || u.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    set(app, |s| {
        if s.phase == "idle" {
            s.phase = "checking".into();
        }
    });
    let endpoint: tauri::Url = ENDPOINT.parse().expect("ENDPOINT is a URL");
    let result = match app
        .updater_builder()
        .version_comparator(|current, release| {
            is_newer(&current.to_string(), &release.version.to_string())
        })
        .timeout(Duration::from_secs(30))
        .endpoints(vec![endpoint])
        .and_then(|b| b.build())
    {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let finished = Some(now() as f64);
    let mut download = false;
    match result {
        Ok(Some(update)) => {
            let found = AvailableUpdate {
                version: update.version.clone(),
                notes: update.body.clone().filter(|n| !n.trim().is_empty()),
                date: update
                    .raw_json
                    .get("pub_date")
                    .and_then(|d| d.as_str())
                    .map(str::to_string),
            };
            let same = u.snapshot().available.as_ref().map(|a| a.version.clone())
                == Some(found.version.clone());
            if !same {
                // A newer one than what was downloaded: start over.
                *u.bytes.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            *u.update.lock().unwrap_or_else(|e| e.into_inner()) = Some(update);
            let later = u
                .later
                .lock()
                .map(|l| l.as_deref() == Some(found.version.as_str()))
                .unwrap_or(false);
            download = u.auto.load(Ordering::SeqCst)
                && u.bytes.lock().map(|b| b.is_none()).unwrap_or(true);
            set(app, |s| {
                if !same {
                    s.phase = "idle".into();
                    s.downloaded = 0.0;
                    s.total = None;
                }
                if s.phase == "checking" {
                    s.phase = "idle".into();
                }
                s.available = Some(found);
                s.later = later;
                s.error = None;
                s.last_check = finished;
            });
        }
        Ok(None) => set(app, |s| {
            if s.phase == "checking" {
                s.phase = "idle".into();
            }
            s.error = None;
            s.last_check = finished;
        }),
        Err(e) => {
            eprintln!("[updates] check failed: {e}");
            set(app, |s| {
                if s.phase == "checking" {
                    s.phase = "idle".into();
                }
                s.error = Some(error_kind(&e).into());
                s.last_check = finished;
            });
        }
    }
    u.busy.store(false, Ordering::SeqCst);
    if download {
        let _ = fetch(app).await;
    }
}

/// Download (and verify) what the last check found, unless it already is.
async fn fetch(app: &AppHandle) -> Result<(), String> {
    let u = app.state::<Updates>();
    if u.bytes.lock().map(|b| b.is_some()).unwrap_or(false) {
        return Ok(());
    }
    let Some(update) = u.update.lock().ok().and_then(|g| g.clone()) else {
        return Err("No update to install.".into());
    };
    if u.busy.swap(true, Ordering::SeqCst) {
        return Err("Already checking or downloading.".into());
    }
    set(app, |s| {
        s.phase = "downloading".into();
        s.downloaded = 0.0;
        s.total = None;
        s.error = None;
    });
    let mut got = 0usize;
    let mut told = std::time::Instant::now() - PROGRESS_EVERY;
    let result = update
        .download(
            |chunk, total| {
                got += chunk;
                if told.elapsed() >= PROGRESS_EVERY {
                    told = std::time::Instant::now();
                    set(app, |s| {
                        s.downloaded = got as f64;
                        s.total = total.map(|t| t as f64);
                    });
                }
            },
            || {},
        )
        .await;
    u.busy.store(false, Ordering::SeqCst);
    match result {
        Ok(bytes) => {
            let len = bytes.len() as f64;
            *u.bytes.lock().unwrap_or_else(|e| e.into_inner()) = Some(bytes);
            set(app, |s| {
                s.phase = "ready".into();
                s.downloaded = len;
                s.total = Some(len);
            });
            Ok(())
        }
        Err(e) => {
            eprintln!("[updates] download failed: {e}");
            let kind = error_kind(&e);
            set(app, |s| {
                s.phase = "idle".into();
                s.error = Some(kind.into());
            });
            Err(e.to_string())
        }
    }
}

/// Install the downloaded update. `restart`: Windows' installer starts the
/// app again (on quit it doesn't).
fn install(app: &AppHandle, restart: bool) -> Result<(), String> {
    let u = app.state::<Updates>();
    let Some(bytes) = u.bytes.lock().ok().and_then(|mut b| b.take()) else {
        return Err("Nothing downloaded to install.".into());
    };
    let Some(update) = u.update.lock().ok().and_then(|g| g.clone()) else {
        return Err("No update to install.".into());
    };
    set(app, |s| s.phase = "installing".into());
    // Windows: this starts the installer and exits the app.
    match update.restart_after_install(restart).install(&bytes) {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("[updates] install failed: {e}");
            set(app, |s| {
                s.phase = "idle".into();
                s.error = Some("install".into());
            });
            Err(e.to_string())
        }
    }
}

/// The app is quitting: with an update downloaded ("Install updates
/// automatically"), hold the exit, install it off the main thread, then
/// quit. Returns whether the exit was held.
pub fn on_exit_requested(app: &AppHandle) -> bool {
    let Some(u) = app.try_state::<Updates>() else {
        return false;
    };
    if u.exiting.swap(true, Ordering::SeqCst) {
        return false;
    }
    if !u.auto.load(Ordering::SeqCst) || u.bytes.lock().map(|b| b.is_none()).unwrap_or(true) {
        return false;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = install(&app, false);
        app.exit(0);
    });
    true
}

/// Settings → Updates and the banner.
#[tauri::command]
#[specta::specta]
pub fn update_status(u: State<Updates>) -> UpdateStatus {
    u.snapshot()
}

/// "Check now".
#[tauri::command]
#[specta::specta]
pub async fn update_check(app: AppHandle) -> UpdateStatus {
    check(&app).await;
    app.state::<Updates>().snapshot()
}

/// "Install and restart" (and "Restart now" once it's downloaded):
/// download if needed, install, start the new version.
#[tauri::command]
#[specta::specta]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if !app.state::<Updates>().enabled() {
        return Err("Updates are off in this build.".into());
    }
    fetch(&app).await?;
    install(&app, true)?;
    // macOS and Linux: the new version is in place; start it.
    app.restart();
}

/// "Later": hide the banner for this version until the next start.
#[tauri::command]
#[specta::specta]
pub fn update_later(app: AppHandle, u: State<Updates>) {
    let version = u.snapshot().available.map(|a| a.version);
    if let Ok(mut l) = u.later.lock() {
        *l = version;
    }
    set(&app, |s| s.later = true);
}

/// "Install updates automatically".
#[tauri::command]
#[specta::specta]
pub async fn set_update_auto(app: AppHandle, on: bool) -> UpdateStatus {
    let u = app.state::<Updates>();
    u.auto.store(on, Ordering::SeqCst);
    if let Some(kv) = &u.kv {
        if let Err(e) = kv.set(KEY_AUTO, if on { "on" } else { "off" }) {
            eprintln!("[updates] couldn't save the setting: {e}");
        }
    }
    set(&app, |s| s.auto_install = on);
    // Turned on with an update already found: fetch it now.
    let found = u.update.lock().map(|g| g.is_some()).unwrap_or(false);
    if on && found && u.enabled() {
        let _ = fetch(&app).await;
    }
    u.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn newer_versions() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.9.9", "1.0.0"));
        assert!(is_newer("1.2.3", "v1.10.0"));
        assert!(is_newer("1.2", "1.2.1"));
        assert!(is_newer("1.0.0", "1.0.1+build.5"));
    }

    #[test]
    fn same_or_older_isnt_offered() {
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.0.0", "1.0.0+other"));
    }

    #[test]
    fn stable_channel_only() {
        assert!(!is_newer("0.1.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.1.0", "1.0.0-rc.1"));
        // A pre-release running here still moves on to the stable one.
        assert!(is_newer("1.0.0-rc.1", "1.0.1"));
    }

    #[test]
    fn junk_isnt_offered() {
        assert!(!is_newer("0.1.0", ""));
        assert!(!is_newer("0.1.0", "latest"));
        assert!(!is_newer("0.1.0", "1.2.3.4"));
        assert!(!is_newer("garbage", "9.9.9"));
    }

    #[test]
    fn schedule() {
        let start = 1_000_000;
        // Not in the first 30 s, then once.
        assert!(!check_due(start, start, None));
        assert!(!check_due(start + 29, start, None));
        assert!(check_due(start + 30, start, None));
        // Then every 6 h after the last check.
        let last = start + 31;
        assert!(!check_due(last + 60, start, Some(last)));
        assert!(!check_due(last + 6 * 3600 - 1, start, Some(last)));
        assert!(check_due(last + 6 * 3600, start, Some(last)));
        // Back from a long sleep: due at once.
        assert!(check_due(last + 3 * 86_400, start, Some(last)));
        // The clock went back past the last check.
        assert!(check_due(last - 100, start, Some(last)));
        // A failed check counts as a check (no hammering while offline).
        assert_eq!(FIRST_DELAY, Duration::from_secs(30));
        assert_eq!(EVERY, Duration::from_secs(21_600));
    }

    #[test]
    fn off_in_dev_and_unsigned_builds() {
        assert_eq!(disabled_reason(true, "dW50cnVzdGVk"), Some("dev"));
        assert_eq!(disabled_reason(false, PUBKEY_PLACEHOLDER), Some("unsigned"));
        assert_eq!(disabled_reason(false, "  "), Some("unsigned"));
        assert_eq!(disabled_reason(false, "dW50cnVzdGVk"), None);
    }

    /// tauri.conf.json sets the updater up the way this module expects.
    #[test]
    fn config_sanity() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
        let updater = &conf["plugins"]["updater"];
        assert_eq!(updater["endpoints"], serde_json::json!([ENDPOINT]));
        // Only https, and on the latest *published* release.
        assert!(ENDPOINT.starts_with("https://github.com/"));
        assert!(ENDPOINT.ends_with("/releases/latest/download/latest.json"));
        // The placeholder, or a real minisign public key (never a private
        // one): base64 of "untrusted comment: minisign public key ...".
        let pubkey = updater["pubkey"].as_str().expect("pubkey is a string");
        if pubkey != PUBKEY_PLACEHOLDER {
            let text = base64::engine::general_purpose::STANDARD
                .decode(pubkey)
                .expect("pubkey is base64");
            let text = String::from_utf8(text).expect("pubkey is text");
            assert!(
                text.starts_with("untrusted comment: minisign public key"),
                "{text}"
            );
            assert!(!text.contains("secret"), "a private key in tauri.conf.json");
        }
        // The version the updater compares is the app's.
        assert_eq!(conf["version"], env!("CARGO_PKG_VERSION"));
        assert!(parse(env!("CARGO_PKG_VERSION")).is_some());
    }
}
