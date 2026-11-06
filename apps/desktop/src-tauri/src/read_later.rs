// Read later (the core's `features::read_later`): links saved to read
// later, the same on all your devices. Thin commands for the Read later
// page and the command palette, and the tray's "Save copied link to Read
// later"; links changed on your other devices arrive by themselves as the
// `read-later-changed` event (the changed links and the deleted ids),
// straight from the core.
//
// Nothing is fetched from the web: a link's title is the one given, else
// its host. A link is opened in the browser only from the list (by id), so
// the page can't open anything else.
//
// Times go to the UI as f64 (TypeScript has no u64).

use mdcore::features::read_later::find_url;
use mdcore::model::ReadLaterItem;
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

/// One saved link, as the UI shows it (the core's `ReadLaterItem`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct ReadLaterView {
    pub id: String,
    pub url: String,
    /// The title the share gave, else the host.
    pub title: String,
    pub host: String,
    pub read: bool,
    /// Unix ms, when it was (last) saved.
    pub saved_at: f64,
    /// The name of the device it was saved on.
    pub saved_on: String,
    /// Unix ms.
    pub updated_at: f64,
}

impl From<ReadLaterItem> for ReadLaterView {
    fn from(i: ReadLaterItem) -> Self {
        ReadLaterView {
            id: i.id,
            url: i.url,
            title: i.title,
            host: i.host,
            read: i.read,
            saved_at: i.saved_at as f64,
            saved_on: i.saved_on,
            updated_at: i.updated_at as f64,
        }
    }
}

/// Why a clipboard can't be saved as a link.
#[derive(Debug, PartialEq, Eq)]
pub enum NoLink {
    /// Nothing, or no web link, is copied.
    Nothing,
    /// What's copied is marked concealed (a password manager's): it stays
    /// on this computer.
    Private,
}

impl NoLink {
    fn message(&self) -> &'static str {
        match self {
            NoLink::Nothing => "No link is copied. Copy a web link first.",
            NoLink::Private => {
                "What's copied is marked private (a password manager's), so it stays here."
            }
        }
    }
}

/// The link to save from a clipboard's text (`concealed`: the OS says it's
/// a password manager's, which never leaves the computer).
pub fn link_from_clipboard(text: Option<&str>, concealed: bool) -> Result<String, NoLink> {
    if concealed {
        return Err(NoLink::Private);
    }
    text.and_then(find_url).ok_or(NoLink::Nothing)
}

/// The link in `text` (a typed or pasted link, perhaps with words around).
pub fn link_in(text: &str) -> Result<String, CoreError> {
    find_url(text).ok_or_else(|| CoreError::InvalidInput("That isn't a web link.".into()))
}

/// Every saved link, the most recently saved first.
#[tauri::command]
#[specta::specta]
pub fn read_later_list(engine: State<Eng>) -> Vec<ReadLaterView> {
    engine.read_later().into_iter().map(Into::into).collect()
}

/// Save a link (`text`: the link, or text with one in it; `title` null or
/// blank: the link's host). Goes to your other devices.
#[tauri::command]
#[specta::specta]
pub fn read_later_save(
    text: String,
    title: Option<String>,
    engine: State<Eng>,
) -> Result<ReadLaterView, CoreError> {
    let url = link_in(&text)?;
    Ok(engine.save_read_later(&url, title.as_deref())?.into())
}

/// Save the link that's copied on this computer.
#[tauri::command]
#[specta::specta]
pub fn read_later_save_clipboard(engine: State<Eng>) -> Result<ReadLaterView, CoreError> {
    save_clipboard(&engine)
}

fn save_clipboard(engine: &Eng) -> Result<ReadLaterView, CoreError> {
    let text = crate::clip_image::copied_text();
    let (concealed, _) = crate::clip_privacy::manual_check();
    let url = link_from_clipboard(text.as_deref(), concealed)
        .map_err(|e| CoreError::InvalidInput(e.message().into()))?;
    Ok(engine.save_read_later(&url, None)?.into())
}

/// Mark a saved link read, or unread again.
#[tauri::command]
#[specta::specta]
pub fn read_later_mark(
    id: String,
    read: bool,
    engine: State<Eng>,
) -> Result<ReadLaterView, CoreError> {
    Ok(engine.mark_read_later(&id, read)?.into())
}

/// Delete a saved link on all your devices.
#[tauri::command]
#[specta::specta]
pub fn read_later_delete(id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.delete_read_later(&id)
}

/// Open a saved link in the browser, and mark it read.
#[tauri::command]
#[specta::specta]
pub fn read_later_open(
    id: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<ReadLaterView, CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let item = engine
        .read_later()
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| CoreError::InvalidInput("no such link".into()))?;
    app.opener()
        .open_url(item.url.clone(), None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))?;
    if item.read {
        return Ok(item.into());
    }
    Ok(engine.mark_read_later(&id, true)?.into())
}

/// The tray's "Save copied link to Read later": off the main thread, says
/// how it went.
pub fn tray_save_clipboard(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        match save_clipboard(&engine) {
            Ok(item) => {
                crate::notify::post_always(&app, "Saved to Read later", &item.title);
                // The page shows what's saved here at once.
                let _ = app.emit("read-later-saved", &item);
            }
            Err(e) => crate::notify::post_always(&app, "Couldn't save the link", &e.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copied_link_is_found_and_private_clips_stay() {
        assert_eq!(
            link_from_clipboard(Some("read this: https://blog.dev/post."), false),
            Ok("https://blog.dev/post".into())
        );
        assert_eq!(
            link_from_clipboard(Some("https://blog.dev/post"), true),
            Err(NoLink::Private)
        );
        assert_eq!(
            link_from_clipboard(Some("just words"), false),
            Err(NoLink::Nothing)
        );
        assert_eq!(link_from_clipboard(None, false), Err(NoLink::Nothing));
        assert!(link_in("file:///etc/hosts").is_err());
        assert_eq!(link_in(" HTTPS://x.io ").unwrap(), "https://x.io");
    }

    #[test]
    fn times_go_to_the_ui_as_numbers() {
        let v = ReadLaterView::from(ReadLaterItem {
            id: "u1".into(),
            url: "https://x.io".into(),
            title: "x.io".into(),
            host: "x.io".into(),
            read: true,
            saved_at: 1_700_000_000_000,
            saved_on: "Pixel".into(),
            updated_at: 1_700_000_000_001,
            by: "node".into(),
        });
        assert_eq!(v.saved_at, 1_700_000_000_000.0);
        assert!(v.read);
        assert_eq!(v.saved_on, "Pixel");
    }
}
