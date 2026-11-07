//! Per-device permissions (mdcore `features::perms`): which of your devices
//! may control this computer, see its screen, or take over its mouse and
//! keyboard. All off until allowed for a device; the device page lists the
//! switches, Settings sums them up, the health check has a grey line for
//! each device allowed anything.
//!
//! A device asking for one it isn't allowed is refused (it says why), and
//! this computer asks its owner (`permission-ask`: Allow once / Always /
//! Don't allow) while the app runs: the window comes up for it. Never
//! allowed without that answer.

use mdcore::features::perms::{PeerPermissions, PermissionNotice};
use mdcore::CoreError;
use tauri::{AppHandle, State};

use crate::Eng;

/// What `node_id` is allowed here.
#[tauri::command]
#[specta::specta]
pub fn peer_permissions(node_id: String, engine: State<Eng>) -> PeerPermissions {
    engine.peer_permissions(&node_id)
}

/// Every device allowed anything here.
#[tauri::command]
#[specta::specta]
pub fn all_permissions(engine: State<Eng>) -> Vec<PeerPermissions> {
    engine.all_permissions()
}

/// Allow (`on`) `node_id` to do `perm` ("control", "screen", "kvm") here,
/// or not. Off ends what it allowed at once.
#[tauri::command]
#[specta::specta]
pub fn set_peer_permission(
    node_id: String,
    perm: String,
    on: bool,
    engine: State<Eng>,
) -> Result<PeerPermissions, CoreError> {
    engine.set_peer_permission(&node_id, &perm, on)?;
    Ok(engine.peer_permissions(&node_id))
}

/// The owner's answer to `permission-ask`: "once", "always" or "deny".
#[tauri::command]
#[specta::specta]
pub fn permission_answer(
    node_id: String,
    perm: String,
    answer: String,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.permission_answer(&node_id, &perm, &answer)
}

/// After the upgrade: what was on for every device and is now off.
#[tauri::command]
#[specta::specta]
pub fn permission_notice(engine: State<Eng>) -> Option<PermissionNotice> {
    engine.permission_notice()
}

#[tauri::command]
#[specta::specta]
pub fn dismiss_permission_notice(engine: State<Eng>) {
    engine.dismiss_permission_notice()
}

/// A device asked for something it isn't allowed: bring the window up,
/// where the prompt shows (the event reaches it as `permission-ask`).
pub fn on_ask(app: &AppHandle) {
    crate::tray::show_window(app);
}
