// "Download on <device>" (the core's `features::remote_download`): a link
// shared here is downloaded by another of your devices, straight from the
// web, or another device asks this computer to download one.
//
//  - commands for the "Download on…" picker (command palette, tray, Send
//    to…, the device page), the question asked here the first time a device
//    asks, and Settings → "Downloads from my devices";
//  - the core's `remote-download` event (`on_event`, from the event sink):
//    the Transfers list learns the link and its stage (`transfers`), and a
//    notification asks, or says it's done or why it failed. The file is
//    never opened by itself.
//
// Numbers go to the UI as f64 (TypeScript has no u64).

use mdcore::model::{RemoteDownloadItem, RemoteDownloadSettings};
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

/// A download either way, as the UI shows it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct RemoteDownloadView {
    /// Also its transfer id.
    pub id: String,
    /// The device that asked (`here`), or the one downloading.
    pub node_id: String,
    pub here: bool,
    pub url: String,
    pub host: String,
    pub name: String,
    pub size: f64,
    pub done: f64,
    /// "requested", "checking", "asking", "queued", "downloading", "done",
    /// "failed", "cancelled" or "refused".
    pub state: String,
    pub error: Option<String>,
    pub path: Option<String>,
    pub started_at: f64,
}

impl From<RemoteDownloadItem> for RemoteDownloadView {
    fn from(i: RemoteDownloadItem) -> Self {
        RemoteDownloadView {
            id: i.id,
            node_id: i.node_id,
            here: i.here,
            url: i.url,
            host: i.host,
            name: i.name,
            size: i.size as f64,
            done: i.done as f64,
            state: i.state,
            error: i.error,
            path: i.path,
            started_at: i.started_at as f64,
        }
    }
}

/// Settings → "Downloads from my devices".
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct DownloadSettingsView {
    /// "ask" (the first time for each device), "always" or "never".
    pub policy: String,
    /// Links to the local network are allowed too.
    pub allow_lan: bool,
    /// The biggest download in MB; 0: no limit.
    pub max_mb: u32,
    /// Devices allowed without asking.
    pub allowed: Vec<String>,
}

impl From<RemoteDownloadSettings> for DownloadSettingsView {
    fn from(s: RemoteDownloadSettings) -> Self {
        DownloadSettingsView {
            policy: s.policy,
            allow_lan: s.allow_lan,
            max_mb: u32::try_from(s.max_mb).unwrap_or(u32::MAX),
            allowed: s.allowed,
        }
    }
}

impl From<DownloadSettingsView> for RemoteDownloadSettings {
    fn from(s: DownloadSettingsView) -> Self {
        RemoteDownloadSettings {
            policy: s.policy,
            allow_lan: s.allow_lan,
            max_mb: u64::from(s.max_mb),
            allowed: s.allowed,
        }
    }
}

/// Ask `node_id` to download the link in `text`.
#[tauri::command]
#[specta::specta]
pub fn remote_download_request(
    node_id: String,
    text: String,
    engine: State<Eng>,
) -> Result<RemoteDownloadView, CoreError> {
    Ok(engine.request_remote_download(&node_id, &text)?.into())
}

/// The web link copied on this computer, for the "Download on…" picker
/// (none from a password manager's copy).
#[tauri::command]
#[specta::specta]
pub fn remote_download_copied_link() -> Result<String, CoreError> {
    let text = crate::clip_image::copied_text();
    let (concealed, _) = crate::clip_privacy::manual_check();
    crate::read_later::link_from_clipboard(text.as_deref(), concealed)
        .map_err(|e| CoreError::InvalidInput(e.message().into()))
}

/// The devices that download links (node ids).
#[tauri::command]
#[specta::specta]
pub fn remote_download_targets(engine: State<Eng>) -> Vec<String> {
    engine
        .get_peers()
        .into_iter()
        .map(|p| p.node_id)
        .filter(|n| engine.remote_download_supported(n))
        .collect()
}

/// Answer a device asking this computer to download a link.
#[tauri::command]
#[specta::specta]
pub fn remote_download_answer(
    id: String,
    allow: bool,
    always: bool,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.answer_remote_download(&id, allow, always)
}

/// Downloads either way since the app started, the newest first.
#[tauri::command]
#[specta::specta]
pub fn remote_downloads(engine: State<Eng>) -> Vec<RemoteDownloadView> {
    engine
        .remote_downloads()
        .into_iter()
        .map(Into::into)
        .collect()
}

#[tauri::command]
#[specta::specta]
pub fn remote_download_settings(engine: State<Eng>) -> DownloadSettingsView {
    engine.remote_download_settings().into()
}

#[tauri::command]
#[specta::specta]
pub fn set_remote_download_settings(
    settings: DownloadSettingsView,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.set_remote_download_settings(settings.into())
}

/// "Downloading 2 MB" / "" when the size isn't known.
fn size_text(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 * 1024 => format!("{} KB", b.div_ceil(1024)),
        b if b < 1024 * 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b => format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}

/// The notification for a download's new state: (title, body), or none.
pub fn notice(item: &RemoteDownloadItem, device: &str) -> Option<(String, String)> {
    let from = if item.host.is_empty() {
        String::new()
    } else {
        format!(" from {}", item.host)
    };
    match (item.here, item.state.as_str()) {
        (true, "asking") => {
            let size = size_text(item.size);
            let size = if size.is_empty() {
                String::new()
            } else {
                format!(" ({size})")
            };
            Some((
                format!("{device} wants this computer to download a file"),
                format!("{}{size}{from}. Open my-domain to allow it.", item.name),
            ))
        }
        (true, "done") => Some((
            format!("Downloaded {}", item.name),
            format!(
                "{}{from}, asked by {device}.",
                "Saved in your downloads folder"
            ),
        )),
        (true, "failed") => Some((
            format!("Couldn't download {}", item.name),
            item.error.clone().unwrap_or_default(),
        )),
        (false, "done") => Some((
            format!("{device} downloaded {}", item.name),
            format!("The file{from} is in its downloads folder."),
        )),
        (false, "failed" | "refused") => Some((
            format!("{device} didn't download {}", item.name),
            item.error.clone().unwrap_or_default(),
        )),
        _ => None,
    }
}

/// The core's `remote-download` event (from the event sink).
pub fn on_event(app: &AppHandle, item: &RemoteDownloadItem) {
    crate::transfers::on_remote_download(app, item);
    let (app, item) = (app.clone(), item.clone());
    std::thread::spawn(move || {
        let device = crate::notify::device_name(&app, &item.node_id);
        let Some((title, body)) = notice(&item, &device) else {
            return;
        };
        if item.state == "asking" {
            // A question: shown even while quiet.
            crate::notify::post_always(&app, &title, &body);
        } else if item.here {
            crate::notify::post(&app, &title, &body);
        } else {
            crate::notify::post_if_away(&app, &title, &body);
        }
    });
}

/// The tray's "Download copied link on…": the window comes forward with
/// the picker (event `download-link-pick`, the link copied now).
pub fn tray_pick(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if app.try_state::<Eng>().is_none() {
            return;
        }
        match remote_download_copied_link() {
            Ok(link) => {
                crate::tray::show_window(&app);
                let _ = app.emit("download-link-pick", serde_json::json!({ "url": link }));
            }
            Err(e) => {
                crate::notify::post_always(&app, "Couldn't download the link", &e.to_string())
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(here: bool, state: &str) -> RemoteDownloadItem {
        RemoteDownloadItem {
            id: "dl1".into(),
            node_id: "n".into(),
            here,
            url: "https://example.com/a.zip".into(),
            host: "example.com".into(),
            name: "a.zip".into(),
            size: 3 * 1024 * 1024,
            state: state.into(),
            error: Some("the site says the file isn't there".into()),
            ..RemoteDownloadItem::default()
        }
    }

    #[test]
    fn notices_say_what_happened() {
        let (t, b) = notice(&item(true, "asking"), "Pixel").unwrap();
        assert_eq!(t, "Pixel wants this computer to download a file");
        assert_eq!(
            b,
            "a.zip (3.0 MB) from example.com. Open my-domain to allow it."
        );
        let (t, b) = notice(&item(true, "done"), "Pixel").unwrap();
        assert_eq!(t, "Downloaded a.zip");
        assert!(b.contains("from example.com, asked by Pixel"));
        let (t, _) = notice(&item(false, "done"), "Laptop").unwrap();
        assert_eq!(t, "Laptop downloaded a.zip");
        let (_, b) = notice(&item(false, "refused"), "Laptop").unwrap();
        assert!(b.contains("isn't there"));
        // Cancelled or under way: nothing.
        for s in ["cancelled", "queued", "downloading", "checking"] {
            assert!(notice(&item(true, s), "Pixel").is_none(), "{s}");
        }
    }

    #[test]
    fn settings_round_trip() {
        let s = RemoteDownloadSettings {
            policy: "always".into(),
            allow_lan: true,
            max_mb: 0,
            allowed: vec!["a".into()],
        };
        let v: DownloadSettingsView = s.clone().into();
        assert_eq!(RemoteDownloadSettings::from(v), s);
        assert_eq!(size_text(0), "");
        assert_eq!(size_text(1500), "2 KB");
        assert_eq!(size_text(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
