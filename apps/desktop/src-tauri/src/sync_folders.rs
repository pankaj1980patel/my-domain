// Synced folders (the core's `features::folder_sync`): thin commands for the
// device page's "Synced folders" tile and the Settings overview. The list
// also arrives by itself as the `sync-pairs` event whenever it changes.
//
// Numbers go to the UI as f64 (TypeScript has no u64).

use std::path::PathBuf;

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
    /// The other device's files kept here under another name: theirs
    /// differs from another file's only by letter case.
    pub renamed: u32,
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
            renamed: p.renamed,
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

/// The folders of the synced folders that sync (accepted on both sides,
/// paused or not): files in them may be opened and revealed.
pub fn synced_roots(pairs: &[SyncPair]) -> Vec<PathBuf> {
    pairs
        .iter()
        .filter(|p| !p.path.is_empty() && p.state != "invited" && p.state != "pending")
        .map(|p| PathBuf::from(&p.path))
        .collect()
}

/// `path` as a file inside one of `roots`, after resolving links and `..`
/// on both (so a link inside a root that leads out of it is refused): the
/// UI may only open or reveal files this app saved or syncs.
pub fn openable_file(path: &str, roots: &[PathBuf]) -> Result<PathBuf, CoreError> {
    let bad = || CoreError::InvalidInput("not a received file".into());
    let file = std::fs::canonicalize(path).map_err(|_| bad())?;
    let inside = |root: &PathBuf| {
        !root.as_os_str().is_empty()
            && std::fs::canonicalize(root).is_ok_and(|r| file.starts_with(&r))
    };
    if file.is_file() && roots.iter().any(inside) {
        Ok(file)
    } else {
        Err(bad())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("md-open-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pair(path: &Path, state: &str) -> SyncPair {
        SyncPair {
            id: "p".into(),
            path: path.to_string_lossy().into_owned(),
            state: state.into(),
            ..SyncPair::default()
        }
    }

    #[test]
    fn only_synced_folders_that_sync_are_roots() {
        let (a, b, c) = (Path::new("/a"), Path::new("/b"), Path::new("/c"));
        let pairs = [
            pair(a, "in_sync"),
            pair(b, "paused"),
            pair(c, "pending"),
            pair(Path::new(""), "invited"),
            pair(Path::new("/d"), "invited"),
        ];
        assert_eq!(synced_roots(&pairs), [a.to_path_buf(), b.to_path_buf()]);
    }

    #[test]
    fn files_open_only_inside_the_downloads_or_a_synced_folder() {
        let base = temp("roots");
        let (downloads, synced, outside) = (
            base.join("Downloads"),
            base.join("Notes"),
            base.join("Private"),
        );
        for d in [&downloads, &synced, &outside] {
            std::fs::create_dir_all(d.join("sub")).unwrap();
            std::fs::write(d.join("sub").join("f.txt"), b"x").unwrap();
        }
        let roots = vec![downloads.clone(), synced.clone()];
        let s = |p: &Path| p.to_string_lossy().into_owned();
        for ok in [downloads.join("sub/f.txt"), synced.join("sub/f.txt")] {
            let got = openable_file(&s(&ok), &roots).unwrap();
            assert_eq!(got, std::fs::canonicalize(&ok).unwrap());
        }
        for bad in [
            outside.join("sub/f.txt"),
            // Out through "..".
            synced.join("sub/../../Private/sub/f.txt"),
            // A folder, not a file.
            synced.join("sub"),
            // Not there.
            synced.join("nope.txt"),
            PathBuf::from(""),
        ] {
            assert!(openable_file(&s(&bad), &roots).is_err(), "{bad:?}");
        }
        // A synced folder that went: nothing under it opens.
        assert!(openable_file(&s(&synced.join("sub/f.txt")), &roots[..1]).is_err());
        // An empty root (a pair with no folder) allows nothing.
        assert!(openable_file(&s(&outside.join("sub/f.txt")), &[PathBuf::new()]).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_a_synced_folder_is_refused() {
        let base = temp("link");
        let (synced, outside) = (base.join("Notes"), base.join("Private"));
        std::fs::create_dir_all(&synced).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), synced.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, synced.join("dir")).unwrap();
        let roots = vec![synced.clone()];
        for p in [
            synced.join("link.txt"),
            synced.join("dir").join("secret.txt"),
        ] {
            assert!(
                openable_file(&p.to_string_lossy(), &roots).is_err(),
                "{p:?}"
            );
        }
        // A synced folder reached through a link still works.
        let alias = base.join("alias");
        std::os::unix::fs::symlink(&synced, &alias).unwrap();
        std::fs::write(synced.join("ok.txt"), b"x").unwrap();
        assert!(openable_file(&alias.join("ok.txt").to_string_lossy(), &roots).is_ok());
        assert!(openable_file(&synced.join("ok.txt").to_string_lossy(), &[alias]).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }
}
