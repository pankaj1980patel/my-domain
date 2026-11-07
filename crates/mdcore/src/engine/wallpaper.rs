//! Each device's wallpaper look (see `features::wallpaper`).

use std::collections::BTreeMap;

use super::*;
use crate::features::wallpaper;
use crate::model::Wallpaper;

impl<P: Platform> Engine<P> {
    /// This device's wallpaper look: a few dominant colours (`#rrggbb`)
    /// and, if the app could read it, a small picture (JPEG, WebP or PNG,
    /// at most `features::wallpaper::MAX_IMAGE` bytes; a bigger or other
    /// one is left out). Only a change goes to your other devices. Nothing
    /// usable (no colours, no picture) is the same as `clear_wallpaper`.
    pub fn set_wallpaper(&self, colors: Vec<String>, image: Option<Vec<u8>>) {
        let w = wallpaper::build(&colors, image.as_deref());
        wallpaper::set_own(&self.ctx, w);
    }

    /// Share no wallpaper look (the setting is off): your other devices
    /// are told to forget the one they had.
    pub fn clear_wallpaper(&self) {
        wallpaper::set_own(&self.ctx, None);
    }

    /// This device's own look as shared (`None`: nothing shared).
    pub fn own_wallpaper(&self) -> Option<Wallpaper> {
        wallpaper::own(&self.ctx)
    }

    /// What `node_id` shared, if anything.
    pub fn wallpaper(&self, node_id: &str) -> Option<Wallpaper> {
        wallpaper::of(&self.ctx, node_id)
    }

    /// What each of your devices shared, by node id.
    pub fn wallpapers(&self) -> BTreeMap<String, Wallpaper> {
        wallpaper::all(&self.ctx)
    }
}
