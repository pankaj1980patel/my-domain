//! Live links (QUIC, hole punch, WebRTC, relay), the live-connection
//! registry, and the shared incoming-message pipeline.
//!
//! Every message rides a live link. The old one-off paths (a TCP connection
//! or a single UDP datagram per message, and direct WebSocket links) are
//! gone: QUIC covers each of those cases, encrypted and reliable.

pub mod punch;
pub mod quality;
pub mod quic;
pub mod relay;
#[cfg(feature = "webrtc")]
pub mod rtc;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use serde::Serialize;

use crate::crypto::{decrypt, encrypt};
use crate::engine::{KeyHolder, NetCtx, WsConns};
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{Envelope, IncomingMessage, Plaintext, WsFrame};
use crate::platform::Platform;

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Unified live-connection model — the single source of truth for "who is
// connected, over what transport". The UI green dot reads ONLY this.
// ---------------------------------------------------------------------------

/// Which transport a live P2P link is using. Serializes lowercase for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    Punch,
    WebRtc,
    Quic,
    Relay,
}

/// A live peer-to-peer link. `last_seen` drives heartbeat staleness for the
/// links without their own keep-alive (WebRTC); QUIC and the relay notice a
/// dead connection themselves.
#[derive(Clone)]
pub struct P2pConn {
    pub kind: TransportKind,
    pub last_seen: u64,
    pub addr: Option<SocketAddr>,
}

/// node_id -> live link.
pub type P2pConns = Arc<Mutex<HashMap<String, P2pConn>>>;

/// The peer has a live link that isn't through the relay.
pub fn is_direct<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    ctx.p2p_conns
        .lock()
        .get(node_id)
        .is_some_and(|c| c.kind != TransportKind::Relay)
}

/// Mark a peer live (first time) or refresh an existing link. The transport is
/// fixed at establishment by whoever created the link (ws/punch/webrtc); a
/// refresh (e.g. a heartbeat ping) only bumps `last_seen` and never downgrades
/// the kind. Emits `PeerConnected` only on first appearance. Never emits under
/// the lock.
pub fn mark_live<P: Platform>(
    ctx: &NetCtx<P>,
    node_id: &str,
    kind: TransportKind,
    addr: Option<SocketAddr>,
) {
    let newly = {
        let mut m = ctx.p2p_conns.lock();
        match m.get_mut(node_id) {
            // A relayed peer that now has a direct link is UPGRADED (and the
            // UI told, with the new transport). Never the other way round.
            Some(c) if c.kind == TransportKind::Relay && kind != TransportKind::Relay => {
                *c = P2pConn {
                    kind,
                    last_seen: now_secs(),
                    addr,
                };
                true
            }
            Some(c) => {
                c.last_seen = now_secs();
                if addr.is_some() {
                    c.addr = addr;
                }
                false
            }
            None => {
                m.insert(
                    node_id.to_string(),
                    P2pConn {
                        kind,
                        last_seen: now_secs(),
                        addr,
                    },
                );
                true
            }
        }
    };
    if newly {
        ctx.sink.emit(CoreEvent::PeerConnected {
            node_id: node_id.to_string(),
            transport: kind,
        });
        // Give the peer our current addresses (it may only know stale ones
        // from the server), off this thread: sending may block.
        let (ctx, peer) = (ctx.clone(), node_id.to_string());
        std::thread::spawn(move || crate::features::addr_update::announce(&ctx, Some(&[peer])));
    }
}

/// Refresh `last_seen` for an existing live peer (e.g. on pong/keepalive).
pub fn touch_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    if let Some(c) = ctx.p2p_conns.lock().get_mut(node_id) {
        c.last_seen = now_secs();
    }
}

/// Drop a peer's live link. Emits `PeerDisconnected` if it was present.
pub fn drop_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    let removed = ctx.p2p_conns.lock().remove(node_id).is_some();
    if removed {
        ctx.sink.emit(CoreEvent::PeerDisconnected {
            node_id: node_id.to_string(),
        });
    }
}

/// Snapshot of live peers and their transports.
pub fn live_peers<P: Platform>(
    ctx: &NetCtx<P>,
) -> Vec<(String, TransportKind, Option<SocketAddr>)> {
    ctx.p2p_conns
        .lock()
        .iter()
        .map(|(k, v)| (k.clone(), v.kind, v.addr))
        .collect()
}

/// Snapshot of live peers with their `last_seen` — for heartbeat staleness checks.
pub fn live_snapshot<P: Platform>(ctx: &NetCtx<P>) -> Vec<(String, TransportKind, u64)> {
    ctx.p2p_conns
        .lock()
        .iter()
        .map(|(k, v)| (k.clone(), v.kind, v.last_seen))
        .collect()
}

/// Decrypt an envelope into a UI message; undecryptable messages are surfaced
/// (not silently dropped).
/// How often one sender IP may produce a visible "could not decrypt" notice.
const UNDECRYPTABLE_NOTICE_EVERY_SECS: u64 = 60;
/// Cap on tracked sender IPs, so a flood from many addresses can't grow memory.
const UNDECRYPTABLE_MAX_TRACKED: usize = 1024;

/// An envelope from `ip` failed to decrypt. Count it, and surface at most one
/// notice per sender IP per minute: enough to tell a user their passphrase
/// doesn't match, without letting any LAN host flood the chat with junk.
fn note_undecryptable<P: Platform>(ctx: &NetCtx<P>, ip: String, protocol: &str) {
    ctx.decrypt_fail_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now = now_secs();
    {
        let mut last = ctx.decrypt_fail_last.lock();
        if let Some(t) = last.get(&ip) {
            if now.saturating_sub(*t) < UNDECRYPTABLE_NOTICE_EVERY_SECS {
                return;
            }
        }
        if last.len() >= UNDECRYPTABLE_MAX_TRACKED {
            last.clear();
        }
        last.insert(ip.clone(), now);
    }
    ctx.sink.emit(CoreEvent::MessageReceived(IncomingMessage {
        from: "(unknown)".into(),
        ip,
        protocol: protocol.into(),
        text: "🔒 message could not be decrypted (wrong encryption key)".into(),
        ts: now,
        ok: false,
    }));
}

/// Send an already-built control-message JSON to one peer, fire and forget.
/// Over its live link if there is one; otherwise a QUIC link is opened first
/// in the background (features never block on connecting). Shared by every
/// discrete feature.
pub(crate) fn send_control_to<P: Platform>(ctx: &NetCtx<P>, node_id: &str, text: &str) {
    let from = ctx.identity.lock().name.clone();
    if ctx.ws_conns.lock().contains_key(node_id) {
        let _ = send_payload(&ctx.ws_conns, &ctx.key, &from, node_id, text);
        return;
    }
    let (ctx, node_id, text) = (ctx.clone(), node_id.to_string(), text.to_string());
    std::thread::spawn(move || {
        if quic::ensure_link(&ctx, &node_id) {
            let _ = send_payload(&ctx.ws_conns, &ctx.key, &from, &node_id, &text);
        }
    });
}

/// Broadcast a control-message JSON to all known peers.
pub fn broadcast_control<P: Platform>(ctx: &NetCtx<P>, text: &str) {
    let ids: Vec<String> = ctx.peers.lock().keys().cloned().collect();
    for id in ids {
        send_control_to(ctx, &id, text);
    }
}

/// Decrypt and dispatch an incoming envelope. Clipboard control messages are
/// handled in-band (never shown as chat); everything else is emitted as a
/// normal `MessageReceived`.
pub fn handle_incoming<P: Platform>(ctx: &NetCtx<P>, env: &Envelope, ip: String, protocol: &str) {
    let pt = {
        let guard = ctx.key.lock();
        guard.as_ref().and_then(|k| decrypt(k, env))
    };
    let Some(pt) = pt else {
        note_undecryptable(ctx, ip, protocol);
        return;
    };
    let Ok(msg) = serde_json::from_slice::<Plaintext>(&pt) else {
        return;
    };

    // Discrete-feature control message? (clipboard / notification / call / …)
    let control = ctx.control.clone();
    if control.dispatch(ctx, &msg.from, &ip, protocol, &msg.text) {
        return;
    }

    ctx.sink.emit(CoreEvent::MessageReceived(IncomingMessage {
        from: msg.from,
        ip,
        protocol: protocol.into(),
        text: msg.text,
        ts: now_secs(),
        ok: true,
    }));
}

/// Encrypt `text` and send it to `node_id` over its live link. Shared by the
/// chat send command and every feature.
pub fn send_payload(
    ws_conns: &WsConns,
    key: &KeyHolder,
    from: &str,
    node_id: &str,
    text: &str,
) -> crate::Result<()> {
    let k = key.lock().ok_or(CoreError::NoKey)?;
    let plaintext = serde_json::to_vec(&Plaintext {
        from: from.to_string(),
        text: text.to_string(),
    })
    .map_err(|e| e.to_string())?;
    let env = encrypt(&k, &plaintext).ok_or("encryption failed")?;
    let frame = serde_json::to_string(&WsFrame::Msg {
        nonce: env.nonce.clone(),
        ciphertext: env.ciphertext.clone(),
    })
    .map_err(|e| e.to_string())?;
    let sender = ws_conns
        .lock()
        .get(node_id)
        .map(|(_, s)| s.clone())
        .ok_or_else(|| CoreError::NoLink("not connected".into()))?;
    sender
        .send(frame)
        .map_err(|_| CoreError::NoLink("connection closed".into()))?;
    Ok(())
}
