//! The "Read later" list shared between your devices (see
//! `features::read_later`).

use super::*;
use crate::features::read_later;
use crate::model::ReadLaterItem;

impl<P: Platform> Engine<P> {
    /// Every saved link, the most recently saved first.
    pub fn read_later(&self) -> Vec<ReadLaterItem> {
        read_later::all(&self.ctx)
    }

    /// Save a link to read later (`title` none or blank: the link's host;
    /// nothing is fetched). Saving a link already in the list puts it back
    /// on top, unread; a deleted one comes back. Goes to your linked devices
    /// that share the list at once, the others at their next link. Makes no
    /// event here. Answers with the link saved.
    pub fn save_read_later(&self, url: &str, title: Option<&str>) -> crate::Result<ReadLaterItem> {
        read_later::save_item(&self.ctx, url, title)
    }

    /// Mark a saved link read (or unread again) on all your devices.
    pub fn mark_read_later(&self, id: &str, read: bool) -> crate::Result<ReadLaterItem> {
        read_later::mark_read(&self.ctx, id, read)
    }

    /// Delete a saved link on all your devices (saving it again brings it
    /// back).
    pub fn delete_read_later(&self, id: &str) -> crate::Result<()> {
        read_later::delete_item(&self.ctx, id)
    }

    /// Device `node_id` said it shares the read-later list (its app is new
    /// enough).
    pub fn read_later_supported(&self, node_id: &str) -> bool {
        read_later::supports(&self.ctx, node_id)
    }
}
