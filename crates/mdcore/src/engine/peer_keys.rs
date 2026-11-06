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
