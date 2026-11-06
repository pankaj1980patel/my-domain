//! A device whose key changed (reinstalled, data cleared, or someone else
//! using its id): its direct connections are refused until the user trusts
//! the new key (see `device_key`).

use super::*;
use crate::device_key::{self, KeyChange};
use crate::features::diag;

/// `node` connected with another key than the one remembered, and was
/// refused. The first time with this key: tell the apps
/// (`CoreEvent::KeyChanged`). Every time: the connection check notes it.
pub(crate) fn key_refused<P: Platform>(ctx: &NetCtx<P>, node: &str, first: bool) {
    let last = diag::attempts(ctx, node).last().map(|a| a.stage.clone());
    if last.as_deref() != Some("key_changed") {
        diag::note(ctx, node, "key_changed", None);
    }
    if !first {
        return;
    }
    let Some(change) = changes(ctx).into_iter().find(|c| c.node_id == node) else {
        return;
    };
    ctx.sink.emit(CoreEvent::KeyChanged {
        node_id: change.node_id,
        name: change.name,
        old_fingerprint: change.old_fingerprint,
        new_fingerprint: change.new_fingerprint,
    });
}

fn changes<P: Platform>(ctx: &NetCtx<P>) -> Vec<KeyChange> {
    let peers = ctx.peers.lock();
    device_key::changes(&ctx.peer_keys, &*ctx.platform, |node| {
        peers
            .get(node)
            .map(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| node.to_string())
    })
}

/// `node` is refused for a key change (waiting or dismissed).
pub(crate) fn key_changed<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    device_key::has_change(&ctx.peer_keys, &*ctx.platform, node)
}

impl<P: Platform> Engine<P> {
    /// Devices whose key changed and are refused for it, newest first
    /// (dismissed ones too, marked).
    pub fn key_changes(&self) -> Vec<KeyChange> {
        changes(&self.ctx)
    }

    /// The user trusts `node_id`'s new key, the one whose fingerprint they
    /// were shown (`new_fingerprint`): it's remembered in place of the old
    /// one (only that key: any other is refused again), and a direct link
    /// is tried. Fails when no change is recorded, or another key turned up
    /// since.
    pub fn trust_new_key(&self, node_id: &str, new_fingerprint: &str) -> crate::Result<()> {
        device_key::trust(
            &self.ctx.peer_keys,
            &*self.ctx.platform,
            node_id,
            new_fingerprint,
        )
        .map_err(CoreError::InvalidInput)?;
        diag::note(&self.ctx, node_id, "key_trusted", None);
        self.ctx.quic_failed.lock().remove(node_id);
        // Reconnect off the caller's thread (it may be the QUIC runtime's).
        let ctx = self.ctx.clone();
        let node = node_id.to_string();
        std::thread::spawn(move || {
            transport::quic::ensure_link(&ctx, &node);
        });
        Ok(())
    }

    /// "Not now": `node_id`'s new key stays refused and isn't asked about
    /// again (another key would be). False: no change recorded for it.
    pub fn dismiss_key_change(&self, node_id: &str) -> bool {
        device_key::dismiss(&self.ctx.peer_keys, &*self.ctx.platform, node_id)
    }
}

/// Comparing keys with one of your devices (`Engine::key_verification`).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KeyVerification {
    pub node_id: String,
    pub name: String,
    /// This device's short fingerprint (its own Settings shows it too).
    pub own_fingerprint: String,
    /// The other device's, as this device remembers its key.
    pub their_fingerprint: String,
    /// 12 groups of 5 digits from both keys: the same on both devices.
    pub safety_number: String,
    /// The user verified this key (cleared when its key changes).
    pub verified: bool,
    /// Its key changed and that isn't settled: verify after.
    pub key_changed: bool,
    /// What the QR code holds (`device_key::VerifyCode`): the other device
    /// scans it.
    pub code: String,
    /// That QR code: modules per side, and its dark modules as an SVG path
    /// with a 4-module margin (`viewBox="0 0 {qr_size+8} {qr_size+8}"`).
    pub qr_size: u32,
    pub qr_path: String,
}

impl<P: Platform> Engine<P> {
    /// This device's own key, as a short fingerprint (four groups of four
    /// hex digits): what other devices show for it.
    pub fn own_fingerprint(&self) -> String {
        device_key::fingerprint(&self.ctx.device_key.public())
    }

    /// Fingerprints, safety number and QR code to compare with `node_id`;
    /// `None` until it has connected with a security key.
    pub fn key_verification(&self, node_id: &str) -> Option<KeyVerification> {
        let ctx = &self.ctx;
        let theirs = device_key::pinned(&ctx.peer_keys, &*ctx.platform, node_id)?;
        let mine = ctx.device_key.public();
        let me = ctx.identity.lock().node_id.clone();
        let code = device_key::VerifyCode {
            from: me,
            from_key: mine.clone(),
            to: node_id.to_string(),
            to_key: theirs.clone(),
        }
        .to_uri();
        // About 170 bytes: always fits.
        let qr = crate::qr::QrCode::encode(code.as_bytes());
        let name = ctx
            .peers
            .lock()
            .get(node_id)
            .map(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| node_id.to_string());
        Some(KeyVerification {
            node_id: node_id.to_string(),
            name,
            own_fingerprint: device_key::fingerprint(&mine),
            their_fingerprint: device_key::fingerprint(&theirs),
            safety_number: device_key::safety_number(&mine, &theirs),
            verified: device_key::is_verified(&ctx.peer_keys, &*ctx.platform, node_id),
            key_changed: key_changed(ctx, node_id),
            qr_size: qr.as_ref().map_or(0, |q| q.size as u32),
            qr_path: qr.map(|q| q.svg_path()).unwrap_or_default(),
            code,
        })
    }

    /// The user compared and the keys match (`true`), or takes that back.
    /// Fails when `node_id` hasn't connected with a key, or its key changed
    /// and that isn't settled.
    pub fn set_device_verified(&self, node_id: &str, verified: bool) -> crate::Result<()> {
        device_key::set_verified(&self.ctx.peer_keys, &*self.ctx.platform, node_id, verified)
            .map_err(CoreError::InvalidInput)
    }

    /// `node_id`'s key is verified (and still the one remembered).
    pub fn is_device_verified(&self, node_id: &str) -> bool {
        device_key::is_verified(&self.ctx.peer_keys, &*self.ctx.platform, node_id)
    }

    /// A verify QR code scanned on this device: when it was made for this
    /// device by one of yours and both keys match, that device is marked
    /// verified.
    pub fn verify_scanned(&self, text: &str) -> device_key::VerifyScan {
        let me = self.ctx.identity.lock().node_id.clone();
        device_key::check_scan(
            &self.ctx.peer_keys,
            &*self.ctx.platform,
            &me,
            &self.ctx.device_key.public(),
            text,
        )
    }
}
