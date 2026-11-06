//! Nearby media mode and the audio receive stats (`features::nearby`,
//! `features::audio_share`).

use super::*;
use crate::features::audio_share::{self, AudioRxStats};
use crate::features::nearby::{self, NearbyStatus};

impl<P: Platform> Engine<P> {
    /// The switch "Use nearby mode for media" (on by default).
    pub fn nearby_media(&self) -> bool {
        nearby::enabled(&self.ctx)
    }

    pub fn set_nearby_media(&self, on: bool) {
        nearby::set_enabled(&self.ctx, on);
    }

    /// How a media stream with `node_id` stands: near or not, on a local
    /// low-latency link or not, and its end-to-end latency (now and before
    /// the last move).
    pub fn nearby_status(&self, node_id: &str) -> NearbyStatus {
        nearby::status(&self.ctx, node_id)
    }

    /// `node_id` knows nearby mode (its app is new enough).
    pub fn nearby_supported(&self, node_id: &str) -> bool {
        nearby::supports(&self.ctx, node_id)
    }

    /// What arrived from audio source `node_id`: records played, lost ones
    /// recovered from the redundant copy, repeats dropped.
    pub fn audio_rx_stats(&self, node_id: &str) -> AudioRxStats {
        audio_share::rx_stats(&self.ctx, node_id)
    }

    /// Close the QUIC link to `node_id` as if the local network went
    /// (tests of the fall back to the relay). False without a QUIC link.
    #[doc(hidden)]
    pub fn drop_quic_link(&self, node_id: &str) -> bool {
        self.ctx.quic.close_peer(node_id, b"network gone")
    }
}
