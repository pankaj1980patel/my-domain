//! Live links (QUIC, hole punch, WebRTC, relay), the live-connection
//! registry, and the shared incoming-message pipeline.
//!
//! Every message rides a live link. The old one-off paths (a TCP connection
//! or a single UDP datagram per message, and direct WebSocket links) are
//! gone: QUIC covers each of those cases, encrypted and reliable.

pub mod envelope;
pub mod media;
pub mod outbox;
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
use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::{Envelope, IncomingMessage, Plaintext, WsFrame};
use crate::platform::Platform;

pub(crate) fn now_secs() -> u64 {
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum TransportKind {
    Punch,
    WebRtc,
    Quic,
    Relay,
    /// Bluetooth LE, carried by the platform (`engine::ble`).
    Ble,
}

impl TransportKind {
    /// A link to use only until a direct one comes up (relay, Bluetooth).
    pub fn is_fallback(self) -> bool {
        matches!(self, TransportKind::Relay | TransportKind::Ble)
    }
}

/// A live peer-to-peer link. `last_seen` drives heartbeat staleness for the
/// links without their own keep-alive (WebRTC); QUIC and the relay notice a
/// dead connection themselves.
#[derive(Clone)]
pub struct P2pConn {
    pub kind: TransportKind,
    pub last_seen: u64,
    pub addr: Option<SocketAddr>,
    /// Last time a message went over it either way (keep-alives don't count).
    pub last_used: u64,
}

/// node_id -> live link.
pub type P2pConns = Arc<Mutex<HashMap<String, P2pConn>>>;

/// The peer has a live link (any kind). A route in `ws_conns` alone isn't
/// one: the relay rung registers its route before the peer has answered.
pub fn is_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    ctx.p2p_conns.lock().contains_key(node_id)
}

/// The peer has a live link that isn't through the relay.
pub fn is_direct<P: Platform>(ctx: &NetCtx<P>, node_id: &str) -> bool {
    ctx.p2p_conns
        .lock()
        .get(node_id)
        .is_some_and(|c| !c.kind.is_fallback())
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
    // A device you removed gets no link, whatever it came over.
    if ctx.revoked.is_revoked(node_id) {
        return;
    }
    let newly = {
        let mut m = ctx.p2p_conns.lock();
        match m.get_mut(node_id) {
            // A relayed peer that now has a direct link is UPGRADED (and the
            // UI told, with the new transport). Never the other way round.
            Some(c) if c.kind.is_fallback() && !kind.is_fallback() => {
                *c = P2pConn {
                    kind,
                    last_seen: now_secs(),
                    addr,
                    last_used: c.last_used,
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
                        last_used: now_secs(),
                    },
                );
                true
            }
        }
    };
    if newly {
        let how = serde_json::to_value(kind).ok();
        let how = how.as_ref().and_then(|v| v.as_str());
        crate::features::diag::note(ctx, node_id, "link_up", how);
        crate::features::reconnect::link_up(ctx, node_id);
        ctx.sink.emit(CoreEvent::PeerConnected {
            node_id: node_id.to_string(),
            transport: kind,
        });
        crate::engine::learn_unknown(ctx, node_id);
        peer_came_up(ctx, node_id);
    }
}

/// A link to `node_id` is new, or its peer restarted (a QUIC `Hello` with
/// a new `boot`, which replaces the old link without a new `PeerConnected`):
/// send what waited for it, and tell it what a fresh start doesn't know.
pub fn peer_came_up<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    outbox::flush(ctx, node_id);
    // Our current addresses (it may only know stale ones from the server),
    // our Do Not Disturb, if on, the device labels, the notes' digest and the timers;
    // off this thread: sending may block.
    let (ctx, peer) = (ctx.clone(), node_id.to_string());
    std::thread::spawn(move || {
        crate::features::addr_update::announce(&ctx, Some(std::slice::from_ref(&peer)));
        // A verification code that waited for this link (two minutes at most).
        crate::features::otp_relay::link_up(&ctx, &peer);
        crate::features::dnd::link_up(&ctx, &peer);
        crate::features::labels::link_up(&ctx, &peer);
        crate::features::revoke::link_up(&ctx, &peer);
        crate::features::notes::link_up(&ctx, &peer);
        crate::features::timers::link_up(&ctx, &peer);
        crate::features::read_later::link_up(&ctx, &peer);
        crate::features::links_map::link_up(&ctx, &peer);
        crate::features::kvm::link_up(&ctx, &peer);
    });
}

/// The live link to `node_id` is now carried another way: a QUIC link on
/// the local network took over from a punched or WebRTC one, or the relay
/// took over from a local link that went (`features::nearby`). Updates the
/// kind and address; when how it's carried changed, tells the UI with
/// `PeerConnected` (no `PeerDisconnected` in between: the stream never
/// stopped) and the map.
pub fn relabel<P: Platform>(
    ctx: &NetCtx<P>,
    node_id: &str,
    kind: TransportKind,
    addr: Option<SocketAddr>,
) {
    let lan = crate::features::diag::is_lan;
    let changed = {
        let mut m = ctx.p2p_conns.lock();
        let Some(c) = m.get_mut(node_id) else {
            return;
        };
        let changed = c.kind != kind || lan(c.addr) != lan(addr);
        c.kind = kind;
        c.addr = addr;
        c.last_seen = now_secs();
        changed
    };
    if changed {
        let how = crate::features::links_map::how(kind, lan(addr));
        crate::features::diag::note(ctx, node_id, "link_up", Some(how));
        ctx.sink.emit(CoreEvent::PeerConnected {
            node_id: node_id.to_string(),
            transport: kind,
        });
        crate::features::links_map::link_up(ctx, node_id);
    }
}

/// Refresh `last_seen` for an existing live peer (e.g. on pong/keepalive).
pub fn touch_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    if let Some(c) = ctx.p2p_conns.lock().get_mut(node_id) {
        c.last_seen = now_secs();
    }
}

/// A message just went over `node_id`'s link (see `P2pConn::last_used`).
pub fn note_use(p2p: &P2pConns, node_id: &str) {
    if let Some(c) = p2p.lock().get_mut(node_id) {
        c.last_used = now_secs();
    }
}

/// Links that carried no message for `idle_secs` (for `close_link`).
pub fn idle_links<P: Platform>(ctx: &NetCtx<P>, idle_secs: u64) -> Vec<String> {
    let now = now_secs();
    ctx.p2p_conns
        .lock()
        .iter()
        .filter(|(_, c)| now.saturating_sub(c.last_used) >= idle_secs)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Whether the link to `node_id` carried a message in the last `secs`.
pub fn used_within<P: Platform>(ctx: &NetCtx<P>, node_id: &str, secs: u64) -> bool {
    let now = now_secs();
    ctx.p2p_conns
        .lock()
        .get(node_id)
        .is_some_and(|c| now.saturating_sub(c.last_used) < secs)
}

/// Close a peer's link on purpose, for `why` (`idle`: unused in the
/// background; `manual`: the Disconnect button; `dead`: it carried
/// nothing). A QUIC link is closed (the other end is told why) and its task
/// cleans up; any other link is just forgotten.
pub fn close_link<P: Platform>(ctx: &NetCtx<P>, node_id: &str, why: &str) {
    crate::features::reconnect::closing(ctx, node_id, why);
    if ctx.quic.close_peer(node_id, why.as_bytes()) {
        return;
    }
    relay::forget_peer(ctx, node_id);
    ctx.ws_conns.lock().remove(node_id);
    drop_live(ctx, node_id);
}

/// Drop a peer's live link. Emits `PeerDisconnected` if it was present.
pub fn drop_live<P: Platform>(ctx: &NetCtx<P>, node_id: &str) {
    let removed = ctx.p2p_conns.lock().remove(node_id).is_some();
    if removed {
        let why = crate::features::reconnect::link_down(ctx, node_id);
        crate::features::diag::note(ctx, node_id, "link_down", Some(&why));
        crate::features::links_map::link_down(ctx, node_id);
        crate::features::kvm::link_down(ctx, node_id);
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
fn note_undecryptable<P: Platform>(
    ctx: &NetCtx<P>,
    from_node: Option<&str>,
    ip: String,
    protocol: &str,
) {
    ctx.decrypt_fail_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Some(node) = from_node {
        crate::features::links_map::undecryptable(ctx, node);
    }
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
        node_id: from_node.map(str::to_string),
        ip,
        protocol: protocol.into(),
        text: "🔒 message could not be decrypted (wrong encryption key)".into(),
        ts: now,
        ok: false,
        id: None,
    }));
}

/// Send an already-built control-message JSON to one peer, fire and forget.
/// Over its live link if there is one; otherwise a QUIC link is opened first
/// in the background (features never block on connecting). Shared by every
/// discrete feature.
pub(crate) fn send_control_to<P: Platform>(ctx: &NetCtx<P>, node_id: &str, text: &str) {
    let from = ctx.identity.lock().name.clone();
    if ctx.ws_conns.lock().contains_key(node_id) {
        let _ = send_payload(ctx, &from, node_id, text);
        return;
    }
    let (ctx, node_id, text) = (ctx.clone(), node_id.to_string(), text.to_string());
    std::thread::spawn(move || {
        if quic::ensure_link(&ctx, &node_id) {
            let _ = send_payload(&ctx, &from, &node_id, &text);
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

/// Open an envelope that came over `protocol` from `from_node` (the link's
/// sender): its plaintext, or `None` when it doesn't decrypt (surfaced as
/// before) or is refused (`envelope::Guard`: counted and logged, never
/// shown). See `transport::envelope`.
fn open_envelope<P: Platform>(
    ctx: &NetCtx<P>,
    env: &Envelope,
    from_node: Option<&str>,
    ip: &str,
    protocol: &str,
) -> Option<Vec<u8>> {
    let Some(key) = *ctx.key.lock() else {
        note_undecryptable(ctx, from_node, ip.to_string(), protocol);
        return None;
    };
    // A device you removed: nothing it sends is used (`features::revoke`).
    if let Some(node) = from_node.filter(|n| ctx.revoked.is_revoked(n)) {
        refused(ctx, node, protocol, envelope::Reject::Revoked);
        return None;
    }
    // An older app: version 1 only, nothing checked.
    if !ctx.config.envelope_v2 {
        let pt = decrypt(&key, env);
        if pt.is_none() {
            note_undecryptable(ctx, from_node, ip.to_string(), protocol);
        }
        return pt;
    }
    let from = from_node.unwrap_or_default();
    let (pt, verdict) = if env.v == 0 {
        let Some(pt) = decrypt(&key, env) else {
            note_undecryptable(ctx, from_node, ip.to_string(), protocol);
            return None;
        };
        (pt, ctx.env.check_v1(from))
    } else {
        let Some((hdr, pt)) = envelope::open(&key, env) else {
            note_undecryptable(ctx, from_node, ip.to_string(), protocol);
            return None;
        };
        let me = ctx.identity.lock().node_id.clone();
        // The sender's signature, when it's the link's and for this device
        // (anything else is refused below, as spoofed or misaddressed).
        let signed = if hdr.src == from && hdr.dst == me {
            match check_signature(ctx, from, env) {
                Ok(signed) => signed,
                Err(why) => {
                    refused(ctx, from, protocol, why);
                    return None;
                }
            }
        } else {
            false
        };
        // Copies can only be recorded where the link isn't encrypted end
        // to end: those ids go to disk too.
        let recordable = matches!(protocol, "RELAY" | "BLE");
        let verdict = ctx.env.check(&*ctx.platform, &me, from, &hdr, recordable);
        if signed && verdict.is_ok() {
            ctx.env.signed_by(&*ctx.platform, from);
        }
        (pt, verdict)
    };
    match verdict {
        Ok(()) => Some(pt),
        Err(why) => {
            refused(ctx, from, protocol, why);
            None
        }
    }
}

/// Count and log a refused envelope from `from`.
fn refused<P: Platform>(ctx: &NetCtx<P>, from: &str, protocol: &str, why: envelope::Reject) {
    let n = ctx.env.count(from, why);
    // The first of each kind, then every hundredth: a flood of copies
    // mustn't flood the log. Never the content.
    if n == 1 || n.is_multiple_of(100) {
        eprintln!(
            "[envelope] refused ({}) from {from} over {protocol}: {n} so far",
            why.as_str()
        );
    }
}

/// Check a version 2 envelope's signature (`transport::envelope`, `env: 3`)
/// against the device key pinned for its sender `from`. `Ok(true)`: signed
/// and it checks out; `Ok(false)`: unsigned, or signed with no key to check
/// it against, from a device not seen signing (taken on the account key
/// alone, as before signatures).
fn check_signature<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    env: &Envelope,
) -> Result<bool, envelope::Reject> {
    use envelope::Reject;
    if !ctx.config.envelope_sig {
        return Ok(false);
    }
    let required = ctx.env.requires_sig(from);
    if env.sig.is_empty() {
        return if required {
            Err(Reject::Unsigned)
        } else {
            Ok(false)
        };
    }
    if let Some(pin) = crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, from) {
        return if envelope::signature_ok(env, &pin) {
            Ok(true)
        } else {
            Err(Reject::BadSignature)
        };
    }
    // Never met directly: the key it sent along, on first use (as a QUIC
    // handshake would pin it), once its signature checks out.
    if !env.pk.is_empty() {
        if !envelope::signature_ok(env, &env.pk) {
            return Err(Reject::BadSignature);
        }
        if ctx.revoked.key_revoked(&env.pk) {
            return Err(Reject::Revoked);
        }
        return match crate::device_key::check(&ctx.peer_keys, &*ctx.platform, from, Some(&env.pk)) {
            crate::device_key::Verdict::Accept => Ok(true),
            _ => Err(Reject::BadSignature),
        };
    }
    if required {
        Err(Reject::Unsigned)
    } else {
        Ok(false)
    }
}

/// Decrypt and dispatch an incoming envelope. Clipboard control messages are
/// handled in-band (never shown as chat); everything else is emitted as a
/// normal `MessageReceived`.
pub fn handle_incoming<P: Platform>(
    ctx: &NetCtx<P>,
    env: &Envelope,
    from_node: Option<&str>,
    ip: String,
    protocol: &str,
) {
    let Some(pt) = open_envelope(ctx, env, from_node, &ip, protocol) else {
        return;
    };
    let Ok(msg) = serde_json::from_slice::<Plaintext>(&pt) else {
        return;
    };
    if let Some(node) = from_node {
        note_use(&ctx.p2p_conns, node);
    }

    // An acknowledgement of a chat message we sent.
    if let Some(id) = &msg.ack {
        if let Some(node) = from_node {
            if outbox::acked(ctx, node, id) {
                ctx.sink.emit(CoreEvent::MessageDelivered {
                    node_id: node.to_string(),
                    id: id.clone(),
                });
            }
        }
        return;
    }
    // A chat message with an id: acknowledge it (every time: the sender
    // resends until it hears back), and show it only the first time.
    if let (Some(id), Some(node)) = (&msg.id, from_node) {
        let ack = Plaintext {
            from: ctx.identity.lock().name.clone(),
            ack: Some(id.clone()),
            ..Default::default()
        };
        let _ = send_plaintext(ctx, node, &ack);
        if !outbox::first_arrival(&ctx.seen, node, id) {
            return;
        }
    }

    // Discrete-feature control message? (clipboard / notification / call / …)
    let control = ctx.control.clone();
    if control.dispatch(ctx, &msg.from, from_node, &ip, protocol, &msg.text) {
        return;
    }

    ctx.sink.emit(CoreEvent::MessageReceived(IncomingMessage {
        from: msg.from,
        node_id: from_node.map(str::to_string),
        ip,
        protocol: protocol.into(),
        text: msg.text,
        ts: now_secs(),
        ok: true,
        id: msg.id,
    }));
}

/// Encrypt `text` and send it to `node_id` over its live link. Shared by the
/// chat send command and every feature.
pub fn send_payload<P: Platform>(
    ctx: &NetCtx<P>,
    from: &str,
    node_id: &str,
    text: &str,
) -> crate::Result<()> {
    let pt = Plaintext {
        from: from.to_string(),
        text: text.to_string(),
        ..Default::default()
    };
    send_plaintext(ctx, node_id, &pt)
}

/// Encrypt a message body for `node_id` into the frame a link carries:
/// envelope version 2 when it reads it, else version 1 (`envelope`).
pub(crate) fn seal<P: Platform>(
    ctx: &NetCtx<P>,
    node_id: &str,
    pt: &Plaintext,
) -> crate::Result<String> {
    let k = ctx.key.lock().ok_or(CoreError::NoKey)?;
    let plaintext = serde_json::to_vec(pt).map_err(|e| e.to_string())?;
    let env = if ctx.config.envelope_v2 && ctx.env.sends_v2(node_id) {
        let me = ctx.identity.lock().node_id.clone();
        let mut env = envelope::seal(&k, &me, node_id, &plaintext);
        if ctx.config.envelope_sig && ctx.env.sends_sig(node_id) {
            if let Some(env) = env.as_mut() {
                // Never met directly: it may not have our key pinned.
                let with_key =
                    crate::device_key::pinned(&ctx.peer_keys, &*ctx.platform, node_id).is_none();
                envelope::sign(env, &ctx.device_key, with_key);
            }
        }
        env
    } else {
        encrypt(&k, &plaintext)
    };
    let env = env.ok_or("encryption failed")?;
    Ok(serde_json::to_string(&WsFrame::Msg(env)).map_err(|e| e.to_string())?)
}

/// Encrypt a message body and send it to `node_id` over its live link.
pub fn send_plaintext<P: Platform>(
    ctx: &NetCtx<P>,
    node_id: &str,
    pt: &Plaintext,
) -> crate::Result<()> {
    let frame = seal(ctx, node_id, pt)?;
    let sender = ctx
        .ws_conns
        .lock()
        .get(node_id)
        .map(|(_, s)| s.clone())
        .ok_or_else(|| CoreError::NoLink("not connected".into()))?;
    if !sender.send(frame) {
        return Err(CoreError::NoLink("connection closed".into()));
    }
    note_use(&ctx.p2p_conns, node_id);
    Ok(())
}
