//! A device whose security key changed (reinstalled, its data cleared, or
//! someone else using its id): the core refuses its direct connections
//! until you trust the new key (`mdcore::device_key`). This shows a pop-up
//! once per new key; the device's page and the connection map show a
//! banner with "Trust the new key" (after a confirm showing both
//! fingerprints) and "Not now".

use mdcore::device_key::KeyChange;
use mdcore::engine::KeyVerification;
use mdcore::CoreError;
use tauri::{AppHandle, State};

use crate::Eng;

/// The devices whose key changed, newest first (dismissed ones marked).
#[tauri::command]
#[specta::specta]
pub fn key_changes(engine: State<Eng>) -> Vec<KeyChange> {
    engine.key_changes()
}

/// Trust `node_id`'s new key: the one whose fingerprint you were shown
/// (`new_fingerprint`). Fails when its key changed again since.
#[tauri::command]
#[specta::specta]
pub fn trust_new_key(
    node_id: String,
    new_fingerprint: String,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.trust_new_key(&node_id, &new_fingerprint)
}

/// "Not now": keep refusing it, stop asking (until another key turns up).
#[tauri::command]
#[specta::specta]
pub fn dismiss_key_change(node_id: String, engine: State<Eng>) -> bool {
    engine.dismiss_key_change(&node_id)
}

/// This computer's own security key, as a short fingerprint (Settings, Add
/// a device): what your other devices show for it.
#[tauri::command]
#[specta::specta]
pub fn own_fingerprint(engine: State<Eng>) -> String {
    engine.own_fingerprint()
}

/// Both fingerprints, the safety number and the QR code to compare with
/// `node_id` (none until it has connected with a security key).
#[tauri::command]
#[specta::specta]
pub fn key_verification(node_id: String, engine: State<Eng>) -> Option<KeyVerification> {
    engine.key_verification(&node_id)
}

/// You compared and they match (`verified`), or you take that back.
#[tauri::command]
#[specta::specta]
pub fn set_device_verified(
    node_id: String,
    verified: bool,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.set_device_verified(&node_id, verified)
}

/// The pop-up for a device whose key changed: (title, body).
pub fn notice(device: &str) -> (String, String) {
    (
        format!("{device}’s security key changed"),
        "This happens when the app is reinstalled or its data is cleared. If you didn’t do that, don’t trust it. Open its page to decide."
            .to_string(),
    )
}

/// Tell the user (once per new key: the core sends the event once).
pub fn on_changed(app: &AppHandle, node_id: &str) {
    let device = crate::notify::device_name(app, node_id);
    let (title, body) = notice(&device);
    crate::notify::post_always(app, &title, &body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_names_the_device_and_warns() {
        let (title, body) = notice("Pixel");
        assert_eq!(title, "Pixel’s security key changed");
        assert!(body.contains("reinstalled"));
        assert!(body.contains("don’t trust it"));
        assert!(!title.contains('\''), "typographic apostrophes only");
    }
}
