// Insert from phone (mdcore `features::insert_capture`), like Continuity
// Camera: "Take a photo with <phone>" and "Scan documents with <phone>",
// from the device page, the tray, the command palette or a keyboard
// shortcut.
//
//  - the phone opens its camera only once its owner acts on it (the app in
//    front shows the capture screen; else a notice that opens it);
//  - what comes back is saved in Downloads like any file (the Transfers
//    list keeps it) AND put on the clipboard: a photo as a picture, a scan
//    (a PDF) as a file where the clipboard takes files, else its path;
//  - "Insert where I am" (Settings, off by default) then pastes it into the
//    app in front, with the keyboard permission the phone's touchpad
//    already uses (Accessibility on a Mac; never asked for here). Only when
//    my-domain's own window isn't the one in front;
//  - either end can cancel; the core drops a request after 3 minutes.
//
// The pref is kept in `<app config dir>/prefs.json`, key `insert_paste`
// ("on" / "off").

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::{notify, Eng};

const PREF_PASTE: &str = "insert_paste";
/// The clipboard settles before the paste keys go.
const PASTE_DELAY: Duration = Duration::from_millis(250);

pub struct InsertPrefs {
    prefs: Option<FileKv>,
    paste: AtomicBool,
}

impl InsertPrefs {
    pub fn load(app: &AppHandle) -> Self {
        let prefs = app
            .path()
            .app_config_dir()
            .ok()
            .map(|d| FileKv::new(d.join("prefs.json")));
        let paste = prefs.as_ref().and_then(|kv| kv.get(PREF_PASTE)).as_deref() == Some("on");
        InsertPrefs {
            prefs,
            paste: AtomicBool::new(paste),
        }
    }
}

/// How what arrived goes on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipAs {
    /// A picture (a photo, JPEG or PNG).
    Picture,
    /// The file itself (a scan's PDF), pasted as an attachment.
    File,
}

/// What `name` (from a request for `mode`) goes on the clipboard as.
pub fn clip_as(mode: &str, name: &str) -> ClipAs {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match (mode, ext.as_str()) {
        ("photo", "jpg" | "jpeg" | "png") => ClipAs::Picture,
        _ => ClipAs::File,
    }
}

/// The notice's title: "Scan from Pixel saved".
pub fn saved_title(mode: &str, device: &str) -> String {
    if mode == "scan" {
        format!("Scan from {device} saved")
    } else {
        format!("Photo from {device} saved")
    }
}

/// Why a request ended, for people.
pub fn ended_text(reason: &str, device: &str) -> String {
    match reason {
        "timeout" => format!("{device} didn\u{2019}t answer in 3 minutes."),
        "no_permission" => format!("my-domain can\u{2019}t use the camera on {device}."),
        "failed" => format!("The file from {device} didn\u{2019}t arrive."),
        "unsupported" => format!("{device} can\u{2019}t take photos."),
        "error" => format!("Something went wrong on {device}."),
        _ => format!("Cancelled on {device}."),
    }
}

/// What a request is, for people: "Take a photo with Pixel".
pub fn describe(mode: &str, device: &str) -> String {
    if mode == "scan" {
        format!("Scan documents with {device}")
    } else {
        format!("Take a photo with {device}")
    }
}

/// Ask `node` (empty: "your phone") for a photo or a scan, and say so in a
/// notice (from the tray and shortcuts, where nothing else shows it). Off
/// the caller's thread.
pub fn ask(app: &AppHandle, node: &str, mode: &str) {
    let (app, node, mode) = (app.clone(), node.to_string(), mode.to_string());
    std::thread::spawn(move || ask_now(&app, &node, &mode));
}

fn ask_now(app: &AppHandle, node: &str, mode: &str) {
    let Some(engine) = app.try_state::<Eng>() else {
        return;
    };
    let node = if node.is_empty() {
        match crate::shortcut::find_phone(app, &engine) {
            Some(phone) => phone,
            None => {
                notify::post_always(
                    app,
                    "No phone to ask",
                    "Sign in to my-domain on your Android phone with the same account.",
                );
                return;
            }
        }
    } else {
        node.to_string()
    };
    let node = node.as_str();
    let device = notify::device_name(app, node);
    match engine.insert_request(node, mode) {
        Ok(_) => notify::post_always(
            app,
            &describe(mode, &device),
            &format!("Take it on {device}. Tap the notice there if the app isn\u{2019}t open."),
        ),
        Err(e) => notify::post_always(
            app,
            &format!("Couldn\u{2019}t ask {device}"),
            &e.to_string(),
        ),
    }
}

/// The photo or scan arrived (`path`, in Downloads): keep it in the
/// Transfers list, put it on the clipboard, say so, and paste it where the
/// user is if they asked for that.
pub fn on_received(
    app: &AppHandle,
    id: &str,
    from: &str,
    mode: &str,
    name: &str,
    path: &str,
    size: u64,
) {
    crate::transfers::on_received(app, id, from, name, path, size);
    let (app, from, mode, name, path) = (
        app.clone(),
        from.to_string(),
        mode.to_string(),
        name.to_string(),
        path.to_string(),
    );
    std::thread::spawn(move || {
        let device = notify::device_name(&app, &from);
        let file = Path::new(&path);
        let copied = match clip_as(&mode, &name) {
            ClipAs::Picture => crate::clip_image::put_picture_on_clipboard(file),
            ClipAs::File => crate::clip_image::put_file_on_clipboard(file),
        };
        let body = match &copied {
            Ok(()) => format!("{name} is in Downloads and on the clipboard."),
            Err(_) => {
                // At least its path, to paste where a file can't go.
                let _ = crate::clip_image::put_text_on_clipboard(&path);
                format!("{name} is in Downloads; its path is on the clipboard.")
            }
        };
        notify::post_if_away(&app, &saved_title(&mode, &device), &body);
        if copied.is_ok() && wants_paste(&app) {
            std::thread::sleep(PASTE_DELAY);
            if let Err(why) = crate::remote_input::paste() {
                notify::post(&app, "Couldn\u{2019}t paste it here", why);
            }
        }
    });
}

/// "Insert where I am" is on, and my-domain's window isn't the one in
/// front (pasting into it would go nowhere useful).
fn wants_paste(app: &AppHandle) -> bool {
    let on = app
        .try_state::<InsertPrefs>()
        .is_some_and(|p| p.paste.load(Ordering::Relaxed));
    let ours_in_front = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    on && !ours_in_front
}

/// A request ended without an answer: say why (the window shows it too).
pub fn on_ended(app: &AppHandle, node: &str, reason: &str) {
    let (app, node, reason) = (app.clone(), node.to_string(), reason.to_string());
    std::thread::spawn(move || {
        let device = notify::device_name(&app, &node);
        notify::post_if_away(&app, "Nothing inserted", &ended_text(&reason, &device));
    });
}

// --- commands ---

/// Ask a phone for a photo (`mode` "photo") or a scan ("scan"). Returns the
/// request's id; `insert-state`, `insert-received` and `insert-ended`
/// events follow.
#[tauri::command]
#[specta::specta]
pub fn insert_request(
    node_id: String,
    mode: String,
    engine: State<Eng>,
) -> Result<String, CoreError> {
    engine.insert_request(&node_id, &mode)
}

/// Cancel a request this computer made (the phone closes its camera).
#[tauri::command]
#[specta::specta]
pub fn insert_cancel(id: String, engine: State<Eng>) {
    engine.insert_cancel(&id, "cancelled");
}

/// Whether a device can be asked for a photo or a scan.
#[tauri::command]
#[specta::specta]
pub fn insert_supported(node_id: String, engine: State<Eng>) -> bool {
    engine.insert_supported(&node_id)
}

#[derive(serde::Serialize, specta::Type)]
pub struct InsertSettings {
    /// "Insert where I am": paste what arrives into the app in front.
    pub paste: bool,
    /// Why pasting can't work yet ("accessibility" on a Mac until allowed,
    /// "display" on Linux without X), if so.
    pub blocked: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn insert_settings(state: State<InsertPrefs>) -> InsertSettings {
    InsertSettings {
        paste: state.paste.load(Ordering::Relaxed),
        blocked: crate::remote_input::ready().err().map(str::to_string),
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_insert_paste(on: bool, state: State<InsertPrefs>) {
    state.paste.store(on, Ordering::Relaxed);
    if let Some(kv) = &state.prefs {
        let _ = kv.set(PREF_PASTE, if on { "on" } else { "off" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_a_photo_is_a_picture_and_a_scan_is_a_file() {
        assert_eq!(clip_as("photo", "Photo 12.jpg"), ClipAs::Picture);
        assert_eq!(clip_as("photo", "x.JPEG"), ClipAs::Picture);
        assert_eq!(clip_as("photo", "x.png"), ClipAs::Picture);
        assert_eq!(clip_as("scan", "Scan.pdf"), ClipAs::File);
        // A photo in a format the clipboard can't hold as a picture.
        assert_eq!(clip_as("photo", "x.heic"), ClipAs::File);
        assert_eq!(clip_as("photo", "noext"), ClipAs::File);
    }

    #[test]
    fn insert_says_what_happened() {
        assert_eq!(saved_title("scan", "Pixel"), "Scan from Pixel saved");
        assert_eq!(saved_title("photo", "Pixel"), "Photo from Pixel saved");
        assert_eq!(describe("photo", "Pixel"), "Take a photo with Pixel");
        assert_eq!(describe("scan", "Pixel"), "Scan documents with Pixel");
        assert!(ended_text("timeout", "Pixel").contains("3 minutes"));
        assert!(ended_text("no_permission", "Pixel").contains("camera"));
        assert!(ended_text("cancelled", "Pixel").starts_with("Cancelled"));
        assert!(ended_text("whatever", "Pixel").starts_with("Cancelled"));
    }
}
