// "Use nearby mode for media" (mdcore `features::nearby`): while audio, a
// screen or a phone's camera streams with a device that's near (on this
// network, or in Bluetooth range), the stream moves to a direct link on
// the local network and buffers less. On by default; the switch lives in
// `<app config dir>/prefs.json` (key `nearby_media`, in the settings
// backup) and is handed to the core at start and when changed.

use mdcore::features::nearby::NearbyStatus;
use mdcore::kv::FileKv;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

const KEY: &str = "nearby_media";

fn prefs(app: &AppHandle) -> Option<FileKv> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")))
}

/// The switch as saved: on unless turned off.
pub fn saved(value: Option<&str>) -> bool {
    value != Some("off")
}

/// Hand the saved switch to the core (at start).
pub fn init(app: &AppHandle, engine: &Eng) {
    let v = prefs(app).and_then(|kv| kv.get(KEY));
    engine.set_nearby_media(saved(v.as_deref()));
}

/// Whether nearby mode is on.
#[tauri::command]
#[specta::specta]
pub fn nearby_media(engine: State<Eng>) -> bool {
    engine.nearby_media()
}

/// Turn nearby mode on or off (kept).
#[tauri::command]
#[specta::specta]
pub fn set_nearby_media(on: bool, app: AppHandle, engine: State<Eng>) {
    engine.set_nearby_media(on);
    if let Some(kv) = prefs(&app) {
        if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
            eprintln!("[nearby] couldn't save the setting: {e}");
        }
    }
}

/// How a media stream with `node_id` stands: near, on a local low-latency
/// link (the badge), its latency now and before the last move.
#[tauri::command]
#[specta::specta]
pub fn nearby_status(node_id: String, engine: State<Eng>) -> NearbyStatus {
    engine.nearby_status(&node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_unless_turned_off() {
        assert!(saved(None));
        assert!(saved(Some("on")));
        assert!(!saved(Some("off")));
        assert!(saved(Some("")));
    }
}
