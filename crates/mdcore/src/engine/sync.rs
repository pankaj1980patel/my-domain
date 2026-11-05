//! Synced folders (see `features::folder_sync`).

use super::*;
use crate::features::folder_sync;
use crate::model::SyncPair;

impl<P: Platform> Engine<P> {
    /// Every synced folder here: active ones, invitations waiting for an
    /// answer here, and ones waiting for the other device's answer. The
    /// `SyncPairs` event brings the same list whenever it changes.
    pub fn sync_pairs(&self) -> Vec<SyncPair> {
        folder_sync::pairs(&self.ctx)
    }

    /// Start syncing the folder at `path` with device `node_id`; `mode` is
    /// this side's part: "both", "send" or "receive". The other device is
    /// asked (it picks where the folder goes there). Returns the pair id.
    pub fn sync_create(&self, node_id: &str, path: &str, mode: &str) -> crate::Result<String> {
        folder_sync::create(&self.ctx, node_id, path, mode)
    }

    /// Accept invitation `id`, keeping the folder at `path` here (made if
    /// it isn't there; files already in it are merged, never overwritten).
    pub fn sync_accept(&self, id: &str, path: &str) -> crate::Result<()> {
        folder_sync::accept(&self.ctx, id, path)
    }

    /// Stop syncing `id` on both devices, or decline the invitation. The
    /// files stay where they are.
    pub fn sync_remove(&self, id: &str) -> crate::Result<()> {
        folder_sync::remove(&self.ctx, id)
    }

    /// Pause or resume `id` (on this device).
    pub fn sync_pause(&self, id: &str, paused: bool) -> crate::Result<()> {
        folder_sync::set_paused(&self.ctx, id, paused)
    }

    /// Compare `id` with the other device now, instead of at the next
    /// round (connecting to it if there's no link).
    pub fn sync_now(&self, id: &str) -> crate::Result<()> {
        folder_sync::sync_now(&self.ctx, id)
    }

    /// Files bigger than this (bytes) are left out of synced folders.
    pub fn sync_max_size(&self) -> u64 {
        folder_sync::max_size(&self.ctx)
    }

    pub fn sync_set_max_size(&self, bytes: u64) -> crate::Result<()> {
        folder_sync::set_max_size(&self.ctx, bytes)
    }

    /// Device `node_id` said it can sync folders (its app is new enough).
    pub fn sync_supported(&self, node_id: &str) -> bool {
        folder_sync::supports(&self.ctx, node_id)
    }
}
