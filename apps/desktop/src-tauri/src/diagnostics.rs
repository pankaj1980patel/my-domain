//! "Check connection" on a device's page: the core's report on one
//! device's link (`Engine::diagnose`) and its speed test, whose progress
//! comes as `speed-test` events.

use mdcore::features::diag::Diagnosis;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

/// The connection report for a device. Takes a few seconds (it pings the
/// device and checks the relay server), so it runs off the UI's thread.
#[tauri::command]
#[specta::specta]
pub async fn diagnose(node_id: String, app: AppHandle) -> Result<Diagnosis, CoreError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Eng>().diagnose(&node_id))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

/// Start a speed test with a device (`bytes` each way, at most 32 MB).
/// Returns its id; `speed-test` events follow.
#[tauri::command]
#[specta::specta]
pub fn speed_test(node_id: String, bytes: u32, engine: State<Eng>) -> Result<String, CoreError> {
    engine.speed_test(&node_id, bytes)
}

/// Stop a running speed test.
#[tauri::command]
#[specta::specta]
pub fn cancel_speed_test(id: String, engine: State<Eng>) -> bool {
    engine.cancel_speed_test(&id)
}
