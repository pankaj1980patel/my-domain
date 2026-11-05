//! Device labels shared between your devices (see `features::labels`).

use super::*;
use crate::features::labels;
use crate::model::SharedLabel;

impl<P: Platform> Engine<P> {
    /// Whether this device shares the nicknames, icons and colours you give
    /// your devices with your other devices (and takes theirs). On until
    /// turned off; kept across restarts.
    pub fn labels_share(&self) -> bool {
        labels::sharing(&self.ctx)
    }

    /// Turn label sharing on or off. Turned on, this device's labels go to
    /// your linked devices and newer ones come back (`LabelsChanged`).
    pub fn set_labels_share(&self, on: bool) {
        labels::set_sharing(&self.ctx, on);
    }

    /// The app set `node_id`'s label (all none: cleared it): kept, and sent
    /// to your other devices while sharing is on. Makes no event here.
    pub fn set_shared_label(
        &self,
        node_id: &str,
        nickname: Option<&str>,
        icon: Option<&str>,
        color: Option<&str>,
    ) -> crate::Result<()> {
        labels::set(&self.ctx, node_id, nickname, icon, color)
    }

    /// A label the app had before labels were shared: added unless the
    /// core already has one for `node_id` (anything set since wins over
    /// it). Call for each of the app's labels at start; true if added.
    pub fn seed_label(
        &self,
        node_id: &str,
        nickname: Option<&str>,
        icon: Option<&str>,
        color: Option<&str>,
    ) -> bool {
        labels::seed(&self.ctx, node_id, nickname, icon, color)
    }

    /// Every shared label, cleared ones included.
    pub fn shared_labels(&self) -> Vec<SharedLabel> {
        labels::all(&self.ctx)
    }

    /// Device `node_id` said it shares labels (its app is new enough).
    pub fn labels_supported(&self, node_id: &str) -> bool {
        labels::supports(&self.ctx, node_id)
    }
}
