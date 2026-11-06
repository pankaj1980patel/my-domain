//! Removing a device for good, and allowing it again (see
//! `features::revoke`).

use super::*;
use crate::features::revoke::{self, RemovedDevice};

impl<P: Platform> Engine<P> {
    /// Remove `node_id` from your devices, for that install: a revocation
    /// signed by this device's key goes to all your devices, which drop it
    /// from their lists, close its links and refuse it from then on (its
    /// node id and its key), even though it may still have the passphrase.
    /// Undone with `allow_device_again`.
    pub fn remove_device(&self, node_id: &str) -> crate::Result<()> {
        revoke::remove(&self.ctx, node_id)
    }

    /// "Allow again": take `node_id`'s removal back (signed the same way,
    /// sent to all your devices). It's found on the network again as
    /// before.
    pub fn allow_device_again(&self, node_id: &str) -> crate::Result<()> {
        revoke::allow(&self.ctx, node_id)?;
        netchange::announce(&self.ctx, &self.disco_send);
        Ok(())
    }

    /// The devices removed (here or on another of your devices), the most
    /// recently removed first.
    pub fn removed_devices(&self) -> Vec<RemovedDevice> {
        revoke::removed(&self.ctx)
    }

    /// `node_id` was removed (and not allowed again).
    pub fn is_device_removed(&self, node_id: &str) -> bool {
        self.ctx.revoked.is_revoked(node_id)
    }
}
