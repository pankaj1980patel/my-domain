//! Changing the passphrase on all your devices (see `features::rekey`).

use super::*;
use crate::features::rekey::{self, KeyRotation};

impl<P: Platform> Engine<P> {
    /// Start changing the passphrase to `new_passphrase` on all your
    /// devices: the new key goes to each trusted device (not removed, its
    /// device key pinned), sealed to that device and signed by this one.
    /// Nothing switches until `finish_key_rotation`; `cancel_key_rotation`
    /// drops it. The server isn't involved (the account password stays).
    pub fn start_key_rotation(&self, new_passphrase: &str) -> crate::Result<KeyRotation> {
        let username = self.username.lock().clone().ok_or(CoreError::NotLoggedIn)?;
        rekey::start(&self.ctx, &username, new_passphrase)?;
        Ok(self.key_rotation())
    }

    /// The change under way (or the last one), and whether this device
    /// needs the new passphrase typed in.
    pub fn key_rotation(&self) -> KeyRotation {
        rekey::info(&self.ctx)
    }

    /// Finish (on the device that started it): everyone switches to the
    /// new key; devices still offline get it when they come back, until it
    /// expires. Pressed again while some haven't switched: stop waiting
    /// for them (they'll need the new passphrase typed in).
    pub fn finish_key_rotation(&self) -> crate::Result<KeyRotation> {
        rekey::finish(&self.ctx)?;
        // Our LAN tag follows the key: tell the network.
        if self.ctx.config.mdns {
            netchange::announce(&self.ctx, &self.disco_send);
        }
        Ok(self.key_rotation())
    }

    /// Cancel before finishing: every device drops the new key.
    pub fn cancel_key_rotation(&self) -> crate::Result<KeyRotation> {
        rekey::cancel(&self.ctx)?;
        Ok(self.key_rotation())
    }

    /// Clear a finished change from Settings, and a "needs the new
    /// passphrase" notice.
    pub fn dismiss_key_rotation(&self) {
        rekey::dismiss(&self.ctx);
    }

    /// A rough strength for a passphrase being typed: 0 (too short) to 4.
    pub fn passphrase_strength(&self, passphrase: &str) -> u8 {
        rekey::strength(passphrase)
    }

    /// Tests only: the offer waiting for `node_id`, as it goes on the wire.
    #[doc(hidden)]
    pub fn test_key_rotation_offer(&self, node_id: &str) -> Option<String> {
        rekey::offer_message(&self.ctx, node_id)
    }

    /// Tests only: send a raw control message to `node_id` (plays a device
    /// that forges or tampers with one).
    #[doc(hidden)]
    pub fn test_send_control(&self, node_id: &str, json: &str) {
        crate::transport::send_control_to(&self.ctx, node_id, json);
    }
}
