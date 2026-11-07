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

/// Remove `node_id` for good, for that install (`mdcore::features::revoke`):
/// your other devices hear of it and refuse it too, even with the
/// passphrase. Undone with `allow_device_again`.
#[tauri::command]
#[specta::specta]
pub fn remove_device(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.remove_device(&node_id)
}

/// "Allow again": take a removal back (on all your devices).
#[tauri::command]
#[specta::specta]
pub fn allow_device_again(node_id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.allow_device_again(&node_id)
}

/// The removed devices (Settings), the most recently removed first.
#[tauri::command]
#[specta::specta]
pub fn removed_devices(engine: State<Eng>) -> Vec<mdcore::features::revoke::RemovedDevice> {
    engine.removed_devices()
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

// --- Change the passphrase on all devices (`mdcore::features::rekey`) ---

/// Start changing the passphrase on all your devices (Settings → Security).
#[tauri::command]
#[specta::specta]
pub fn start_key_rotation(
    new_passphrase: String,
    engine: State<Eng>,
) -> Result<mdcore::features::rekey::KeyRotation, CoreError> {
    engine.start_key_rotation(&new_passphrase)
}

/// The passphrase change under way (or the last one).
#[tauri::command]
#[specta::specta]
pub fn key_rotation(engine: State<Eng>) -> mdcore::features::rekey::KeyRotation {
    engine.key_rotation()
}

/// Finish: every device switches (pressed again: stop waiting for the
/// devices that are offline).
#[tauri::command]
#[specta::specta]
pub fn finish_key_rotation(
    engine: State<Eng>,
) -> Result<mdcore::features::rekey::KeyRotation, CoreError> {
    engine.finish_key_rotation()
}

/// Cancel before finishing.
#[tauri::command]
#[specta::specta]
pub fn cancel_key_rotation(
    engine: State<Eng>,
) -> Result<mdcore::features::rekey::KeyRotation, CoreError> {
    engine.cancel_key_rotation()
}

/// Clear a finished change, or a "type the new passphrase" notice.
#[tauri::command]
#[specta::specta]
pub fn dismiss_key_rotation(engine: State<Eng>) {
    engine.dismiss_key_rotation();
}

/// How strong a passphrase being typed is: 0 (too short) to 4.
#[tauri::command]
#[specta::specta]
pub fn passphrase_strength(passphrase: String, engine: State<Eng>) -> u8 {
    engine.passphrase_strength(&passphrase)
}
