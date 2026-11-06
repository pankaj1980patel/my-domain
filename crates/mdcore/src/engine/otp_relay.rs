//! Verification codes on the computer (see `features::otp_relay`).

use super::*;
use crate::features::otp_relay::{self, VerificationCode};

impl<P: Platform> Engine<P> {
    /// Phone: send verification code `code` (found in a notification of
    /// `app`, from `issuer`, which may be empty) to those of `nodes` that
    /// are your computers and take codes. Only `{code, app, issuer,
    /// expires_at}` goes, over a live link (else it waits up to two
    /// minutes, connecting meanwhile). Returns the computers it went or
    /// will go to; an error if none of them takes codes.
    pub fn send_verification_code(
        &self,
        code: &str,
        app: &str,
        issuer: &str,
        nodes: &[String],
    ) -> crate::Result<Vec<String>> {
        otp_relay::send(&self.ctx, code, app, issuer, nodes)
    }

    /// Phone: your computers, each with whether it takes verification
    /// codes (an older app doesn't), for "choose computers".
    pub fn verification_code_computers(&self) -> Vec<(String, bool)> {
        otp_relay::computers(&self.ctx)
    }

    /// Whether `node_id` is one of your computers and takes verification
    /// codes.
    pub fn verification_codes_supported(&self, node_id: &str) -> bool {
        otp_relay::eligible(&self.ctx, node_id)
    }

    /// How many codes wait for a link to come up.
    pub fn verification_codes_waiting(&self) -> usize {
        otp_relay::waiting(&self.ctx)
    }

    /// Computer: the verification codes shown here that haven't expired,
    /// newest first.
    pub fn verification_codes(&self) -> Vec<VerificationCode> {
        otp_relay::current(&self.ctx)
    }

    /// Computer: forget code `id` now (`VerificationCodeGone`). False if
    /// it was gone already.
    pub fn dismiss_verification_code(&self, id: &str) -> bool {
        otp_relay::dismiss(&self.ctx, id)
    }

    /// The clipboard now holds `text`, put there by the app itself (a
    /// verification code copied, say): the clipboard sync takes it as
    /// seen, so it isn't sent to your other devices or kept in the
    /// clipboard history. Call it before writing the clipboard.
    pub fn clipboard_written_here(&self, text: &str) {
        *self.ctx.clip_last.lock() = text.to_string();
    }
}
