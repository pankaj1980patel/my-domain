//! The gallery, photo backup, battery alerts and keep-awake (see
//! `features::gallery`, `features::backup`, `features::battery`).

use super::*;
use crate::model::{BackupItem, BackupStatus, BatteryAlertSettings, GalleryItem};

/// Longest a device can be kept awake in one go, in minutes.
pub const MAX_AWAKE_MINUTES: u32 = 24 * 60;

impl<P: Platform> Engine<P> {
    // --- gallery: the phone's side ---

    /// This device has a photo gallery the host lists (`GalleryRequest`).
    /// Until it's set, gallery requests are answered "no photo gallery".
    pub fn set_gallery_enabled(&self, on: bool) {
        self.ctx
            .extras
            .gallery
            .enabled
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The host's answer to `GalleryRequest` `req` from `node_id`: the page
    /// (newest first, each with its path here and an optional thumbnail),
    /// the offset of the next page, or why there's no listing (e.g. "photo
    /// access isn't allowed on the phone").
    pub fn gallery_reply(
        &self,
        node_id: &str,
        req: &str,
        items: Vec<GalleryItem>,
        next: Option<u64>,
        error: Option<&str>,
    ) {
        crate::features::gallery::reply(&self.ctx, node_id, req, items, next, error);
    }

    // --- gallery: the asking side ---

    /// Ask a device for `limit` of its photos and videos (newest first)
    /// from `offset`, with thumbnails if `thumbs`. Returns the request id;
    /// the answer comes as `GalleryListing`, then a `GalleryThumb` per item.
    pub fn list_gallery(
        &self,
        node_id: &str,
        offset: u64,
        limit: u32,
        thumbs: bool,
    ) -> crate::Result<String> {
        let req = transport::outbox::new_id();
        let me = self.ctx.identity.lock().node_id.clone();
        let limit = limit.clamp(1, crate::features::gallery::MAX_PAGE);
        let m = serde_json::json!({
            "type": "gallery_list", "from": me, "req": req,
            "offset": offset, "limit": limit, "thumbs": thumbs,
        })
        .to_string();
        self.send(node_id, &m)?;
        Ok(req)
    }

    /// Fetch gallery item `item` (an id from `GalleryListing`) into the
    /// downloads folder. Returns the transfer id; the file arrives like any
    /// download (`FileProgress`, then `FileReceived` or `FileFailed`).
    pub fn download_gallery_item(
        &self,
        node_id: &str,
        item: &str,
        name: &str,
    ) -> crate::Result<String> {
        if !crate::features::gallery::is_item_id(item) {
            return Err("not a gallery item".to_string().into());
        }
        if self.ctx.files.download_dir.lock().is_none() {
            return Err("this device has no downloads folder".to_string().into());
        }
        let id = transport::outbox::new_id();
        let me = self.ctx.identity.lock().node_id.clone();
        let m = serde_json::json!({ "type": "gallery_get", "from": me, "id": id, "item": item })
            .to_string();
        self.send(node_id, &m)?;
        self.ctx.sink.emit(CoreEvent::FileProgress {
            id: id.clone(),
            node_id: node_id.to_string(),
            name: crate::features::files::safe_name(name),
            done: 0,
            total: 0,
            incoming: true,
        });
        Ok(id)
    }

    // --- photo backup (the phone's side; computers just receive) ---

    /// Turn the photo backup on or off, choose the computer (`target`, a
    /// node id) and whether to wait for charging. Turning it on the first
    /// time starts from now (photos added from then on).
    pub fn backup_configure(
        &self,
        enabled: bool,
        target: &str,
        charging_only: bool,
    ) -> crate::Result<()> {
        crate::features::backup::configure(&self.ctx, enabled, target, charging_only)
    }

    /// The host's view of the conditions: on unmetered Wi-Fi, charging.
    pub fn backup_conditions(&self, unmetered: bool, charging: bool) {
        crate::features::backup::conditions(&self.ctx, unmetered, charging);
    }

    /// New photos/videos to back up (already sent or waiting ones are
    /// skipped). Returns how many were added.
    pub fn backup_enqueue(&self, items: Vec<BackupItem>) -> u32 {
        crate::features::backup::enqueue(&self.ctx, items)
    }

    /// Where the backup stands.
    pub fn backup_state(&self) -> BackupStatus {
        crate::features::backup::status(&self.ctx)
    }

    // --- battery alerts ---

    /// Whether this device tells the others when its battery is low (at or
    /// below `low` percent) or charged (to `charged` percent, or full).
    pub fn battery_alerts(&self) -> BatteryAlertSettings {
        crate::features::battery::settings(&self.ctx)
    }

    pub fn set_battery_alerts(&self, settings: BatteryAlertSettings) -> crate::Result<()> {
        crate::features::battery::set_settings(&self.ctx, settings)
    }

    // --- keep awake ---

    /// Keep a device from sleeping for `minutes` (0 = stop), if it lists
    /// "keep_awake" in its status actions. Arrives there as the action
    /// `keep_awake {"minutes": n}`. Returns the transport used, or "queued".
    pub fn keep_awake(&self, node_id: &str, minutes: u32) -> crate::Result<String> {
        if minutes > MAX_AWAKE_MINUTES {
            return Err("at most 24 hours at a time".to_string().into());
        }
        let args = serde_json::json!({ "minutes": minutes }).to_string();
        self.run_action(node_id, "keep_awake", &args)
    }
}
