// Notes (the core's `features::notes`): a small notepad that's the same on
// all your devices. Thin commands for the Notes page; notes changed on your
// other devices arrive by themselves as the `notes-changed` event (the
// changed notes and the deleted ids), straight from the core.
//
// Times go to the UI as f64 (TypeScript has no u64).

use mdcore::model::Note;
use mdcore::CoreError;
use tauri::{AppHandle, State};

use crate::Eng;

/// One note, as the UI shows it (the core's `Note`).
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct NoteView {
    pub id: String,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    /// Lines starting with "[ ] " / "[x] " are drawn as checkboxes.
    pub checklist: bool,
    pub color: Option<String>,
    /// Unix ms.
    pub created_at: f64,
    /// Unix ms.
    pub updated_at: f64,
    /// The device that last changed it (node id) and its name then.
    pub by: String,
    pub by_name: String,
    /// A conflict copy: the note it split from.
    pub conflict_of: Option<String>,
    /// A reminder: when (unix ms) ...
    pub remind_at: Option<f64>,
    /// ... and marked Done (on any device).
    pub remind_done: bool,
}

impl From<Note> for NoteView {
    fn from(n: Note) -> Self {
        NoteView {
            id: n.id,
            title: n.title,
            body: n.body,
            pinned: n.pinned,
            checklist: n.checklist,
            color: n.color,
            created_at: n.created_at as f64,
            updated_at: n.updated_at as f64,
            by: n.by,
            by_name: n.by_name,
            conflict_of: n.conflict_of,
            remind_at: n.remind_at.map(|t| t as f64),
            remind_done: n.remind_done,
        }
    }
}

/// Every note, pinned first, then the most recently changed.
#[tauri::command]
#[specta::specta]
pub fn notes_list(engine: State<Eng>) -> Vec<NoteView> {
    engine.notes().into_iter().map(Into::into).collect()
}

/// Save a note (`id` null: a new one; a deleted one comes back, which is
/// how Undo works). Goes to your other devices. Answers with it as saved.
/// `remind_at` (unix ms, null: none) and `remind_done`: its reminder.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub fn note_save(
    id: Option<String>,
    title: String,
    body: String,
    pinned: bool,
    checklist: bool,
    color: Option<String>,
    remind_at: Option<f64>,
    remind_done: bool,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<NoteView, CoreError> {
    let remind_at = remind_at
        .filter(|t| t.is_finite() && *t > 0.0)
        .map(|t| t as u64);
    let saved = engine.save_note(
        id.as_deref(),
        &title,
        &body,
        pinned,
        checklist,
        color.as_deref(),
        remind_at,
        remind_done,
    )?;
    crate::note_reminders::changed(&app);
    Ok(saved.into())
}

/// Delete a note on all your devices.
#[tauri::command]
#[specta::specta]
pub fn note_delete(id: String, app: AppHandle, engine: State<Eng>) -> Result<(), CoreError> {
    engine.delete_note(&id)?;
    crate::note_reminders::changed(&app);
    Ok(())
}
