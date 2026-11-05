// Notes (the core's `features::notes`): a small notepad that's the same on
// all your devices. Thin commands for the Notes page; notes changed on your
// other devices arrive by themselves as the `notes-changed` event (the
// changed notes and the deleted ids), straight from the core.
//
// Times go to the UI as f64 (TypeScript has no u64).

use mdcore::model::Note;
use mdcore::CoreError;
use tauri::State;

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
#[tauri::command]
#[specta::specta]
pub fn note_save(
    id: Option<String>,
    title: String,
    body: String,
    pinned: bool,
    checklist: bool,
    color: Option<String>,
    engine: State<Eng>,
) -> Result<NoteView, CoreError> {
    engine
        .save_note(
            id.as_deref(),
            &title,
            &body,
            pinned,
            checklist,
            color.as_deref(),
        )
        .map(Into::into)
}

/// Delete a note on all your devices.
#[tauri::command]
#[specta::specta]
pub fn note_delete(id: String, engine: State<Eng>) -> Result<(), CoreError> {
    engine.delete_note(&id)
}
