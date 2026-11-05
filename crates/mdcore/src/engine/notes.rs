//! The notepad shared between your devices (see `features::notes`).

use super::*;
use crate::features::notes::{self, Edit};
use crate::model::Note;

impl<P: Platform> Engine<P> {
    /// Every note, pinned first, then the most recently changed.
    pub fn notes(&self) -> Vec<Note> {
        notes::all(&self.ctx)
    }

    /// One note (none: no such note, or deleted).
    pub fn note(&self, id: &str) -> Option<Note> {
        notes::get(&self.ctx, id)
    }

    /// Save a note (`id` none: a new one; a deleted one comes back). Goes
    /// to your linked devices that share notes at once, the others at
    /// their next link. Makes no event here. Answers with the note saved.
    pub fn save_note(
        &self,
        id: Option<&str>,
        title: &str,
        body: &str,
        pinned: bool,
        checklist: bool,
        color: Option<&str>,
    ) -> crate::Result<Note> {
        let edit = Edit {
            title: title.to_string(),
            body: body.to_string(),
            pinned,
            checklist,
            color: color.map(str::to_string),
        };
        notes::save_note(&self.ctx, id, edit)
    }

    /// Delete a note on all your devices (saving it again brings it back).
    pub fn delete_note(&self, id: &str) -> crate::Result<()> {
        notes::delete_note(&self.ctx, id)
    }

    /// Device `node_id` said it shares notes (its app is new enough).
    pub fn notes_supported(&self, node_id: &str) -> bool {
        notes::supports(&self.ctx, node_id)
    }
}
