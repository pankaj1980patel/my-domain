//! Wi-Fi Direct for big file transfers (see `features::wifi_direct`): what
//! the platform can do, the user's switch, and the platform's answers to
//! `CoreEvent::WifiDirectRequest`.

use super::*;
use crate::features::wifi_direct;

impl<P: Platform> Engine<P> {
    /// What this device can do for big transfers: host (`CAN_HOST`, 1) or
    /// join (`CAN_JOIN`, 2) a Wi-Fi Direct group, start a hotspot
    /// (`CAN_HOTSPOT`, 4: a phone) or join a phone's (`CAN_JOIN_HOTSPOT`,
    /// 8: a computer); 0 (the default) for nothing. The linked devices
    /// hear it.
    pub fn set_wifi_direct_caps(&self, caps: u64) {
        wifi_direct::set_caps(&self.ctx, caps);
    }

    /// "Use Wi-Fi Direct for big files when nearby" (on unless turned
    /// off). Off ends any group now and tells the other devices.
    pub fn set_wifi_direct_enabled(&self, on: bool) {
        wifi_direct::set_enabled(&self.ctx, on);
    }

    pub fn wifi_direct_enabled(&self) -> bool {
        wifi_direct::enabled(&self.ctx)
    }

    /// The platform made the group (or started the hotspot) for request
    /// `sid`, named `ssid` with `passphrase` (what it really used);
    /// `host_ip`: its own address there, "" if not known.
    pub fn wifi_direct_hosted(&self, sid: &str, ssid: &str, passphrase: &str, host_ip: &str) {
        wifi_direct::hosted(&self.ctx, sid, ssid, passphrase, host_ip);
    }

    /// The user said whether this computer may leave its Wi-Fi for the
    /// phone's hotspot (request `sid`, op "consent").
    pub fn wifi_direct_consent(&self, sid: &str, yes: bool) {
        wifi_direct::consent(&self.ctx, sid, yes);
    }

    /// How transfers with `node_id` go right now: "wifi_direct",
    /// "hotspot", or none (over the link).
    pub fn wifi_direct_via(&self, node_id: &str) -> Option<String> {
        wifi_direct::active_via(&self.ctx, node_id).map(str::to_string)
    }

    /// The platform joined the group for request `sid`; its host is at
    /// `host_ip` on it.
    pub fn wifi_direct_joined(&self, sid: &str, host_ip: &str) {
        wifi_direct::joined(&self.ctx, sid, host_ip);
    }

    /// The platform couldn't host or join for request `sid` (`reason`,
    /// for the log): the transfer stays on its link.
    pub fn wifi_direct_failed(&self, sid: &str, reason: &str) {
        if !reason.is_empty() {
            eprintln!("[wifi-direct] {sid}: {reason}");
        }
        wifi_direct::failed(&self.ctx, sid);
    }

    /// Transfers with `node_id` go over Wi-Fi Direct right now.
    pub fn wifi_direct_active(&self, node_id: &str) -> bool {
        wifi_direct::active(&self.ctx, node_id)
    }

    /// What `node_id` said it can do with Wi-Fi Direct (0: nothing, or an
    /// app from before).
    pub fn wifi_direct_peer_caps(&self, node_id: &str) -> u64 {
        wifi_direct::peer_caps(&self.ctx, node_id)
    }
}
