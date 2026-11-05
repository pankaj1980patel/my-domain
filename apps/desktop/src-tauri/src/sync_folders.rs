// Synced folders (the core's `features::folder_sync`): thin commands for the
// device page's "Synced folders" tile and the Settings overview. The list
// also arrives by itself as the `sync-pairs` event whenever it changes.
//
// Numbers go to the UI as f64 (TypeScript has no u64).

use mdcore::model::SyncPair;
use mdcore::CoreError;
use tauri::{AppHandle, State};

use crate::Eng;

/// One synced folder, as the UI shows it (the core's `SyncPair`).
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct SyncPairView {
    pub id: String,
    pub peer: String,
    pub name: String,
    pub path: String,
    /// "both", "send" or "receive" (this computer's part).
    pub mode: String,
    pub paused: bool,
    /// "invited", "pending", "paused", "error", "syncing", "in_sync" or
    /// "waiting".
    pub state: String,
    pub syncing: u32,
    pub files: u32,
    pub conflicts: u32,
    pub skipped: u32,
    /// Unix seconds, 0 = never.
    pub last_synced: f64,
    pub error: Option<String>,
}

impl From<SyncPair> for SyncPairView {
    fn from(p: SyncPair) -> Self {
        SyncPairView {
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
            last_synced: p.last_synced as f64,
            error: p.error,
        }
    }
}

/// Every synced folder on this computer.
#[tauri::command]
#[specta::specta]
pub fn sync_pairs(engine: State<Eng>) -> Vec<SyncPairView> {
    engine.sync_pairs().into_iter().map(Into::into).collect()
}

/// Sync the folder at `path` with `node_id` (`mode`: "both", "send" or
/// "receive"). The other device is asked. Returns the pair id.
#[tauri::command]
#[specta::specta]
pub fn sync_create(
    node_id: String,
    path: String,
    mode: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.sync_create(&node_id, &path, &mode)
}

/// Accept an invitation, keeping the folder at `path` here.
#[tauri::command]
#[specta::specta]
pub fn sync_accept(id: String, path: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.sync_accept(&id, &path)
}

/// Stop syncing (both devices), or decline an invitation. Files stay.
#[tauri::command]
#[specta::specta]
pub fn sync_remove(id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.sync_remove(&id)
}

/// Pause or resume a synced folder on this computer.
#[tauri::command]
#[specta::specta]
pub fn sync_pause(id: String, paused: bool, engine: State<Eng>) -> Result<(), CoreError> {
    engine.sync_pause(&id, paused)
}

/// Compare with the other device now.
#[tauri::command]
#[specta::specta]
pub fn sync_now(id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.sync_now(&id)
}

/// The size limit for synced files, in MB.
#[tauri::command]
#[specta::specta]
pub fn sync_max_size_mb(engine: State<Eng>) -> f64 {
    (engine.sync_max_size() >> 20) as f64
}

#[tauri::command]
#[specta::specta]
pub fn sync_set_max_size_mb(mb: u32, engine: State<Eng>) -> Result<(), CoreError> {
    engine.sync_set_max_size(u64::from(mb) << 20)
}

/// Open a synced folder in the file manager (only a synced folder's own
/// path: the UI can't open anything else through this).
#[tauri::command]
#[specta::specta]
pub fn sync_open_folder(id: String, app: AppHandle, engine: State<Eng>) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let pair = engine
        .sync_pairs()
        .into_iter()
        .find(|p| p.id == id && !p.path.is_empty())
        .ok_or_else(|| CoreError::InvalidInput("no such synced folder".into()))?;
    app.opener()
        .open_path(pair.path, None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Invitations already announced (once each per run).
static ANNOUNCED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The list changed (`sync-pairs`): a notice for each new invitation, so
/// it's answered even with the window closed. Off the core's thread.
pub fn on_pairs(app: &AppHandle, pairs: &[SyncPair]) {
    let new: Vec<(String, String)> = {
        let Ok(mut seen) = ANNOUNCED.lock() else {
            return;
        };
        let mut new = Vec::new();
        for p in pairs.iter().filter(|p| p.state == "invited") {
            if !seen.contains(&p.id) {
                seen.push(p.id.clone());
                new.push((p.peer.clone(), p.name.clone()));
            }
        }
        new
    };
    if new.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        for (peer, name) in new {
            let device = crate::notify::device_name(&app, &peer);
            crate::notify::post(
                &app,
                &format!("{device} wants to sync a folder"),
                &format!("“{name}”: open {device}’s page in my-domain to accept or decline."),
            );
        }
    });
}
