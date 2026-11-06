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

    /// Which way audio with `node_id` goes: "auto" (a network link first,
    /// else Bluetooth), "wifi" (never Bluetooth) or "bluetooth" (even with a
    /// network link). Chosen per device, kept across restarts.
    pub fn audio_route(&self, node_id: &str) -> String {
        audio_share::route_of(&self.ctx, node_id)
            .as_str()
            .to_string()
    }

    pub fn set_audio_route(&self, node_id: &str, route: &str) {
        audio_share::set_route(&self.ctx, node_id, audio_share::AudioRoute::parse(route));
    }

    /// Which way audio with `node_id` went in the last moments:
    /// "bluetooth", "network", or "" (none).
    pub fn audio_via(&self, node_id: &str) -> String {
        audio_share::via(&self.ctx, node_id).to_string()
    }

    /// A stream with `node_id` wants its Bluetooth link kept even with a
    /// network link (Bluetooth only was chosen): the desktop's Bluetooth
    /// code doesn't drop it then.
    pub fn audio_wants_ble(&self, node_id: &str) -> bool {
        audio_share::wants_ble(&self.ctx, node_id)
    }

    /// `node_id` reads Opus audio (`audio_codec: 2`), so audio can go to
    /// it over Bluetooth.
    pub fn audio_opus(&self, node_id: &str) -> bool {
        audio_share::opus_of(&self.ctx, node_id)
    }

    /// Close the QUIC link to `node_id` as if the local network went
    /// (tests of the fall back to the relay). False without a QUIC link.
    #[doc(hidden)]
    pub fn drop_quic_link(&self, node_id: &str) -> bool {
        self.ctx.quic.close_peer(node_id, b"network gone")
    }
}
