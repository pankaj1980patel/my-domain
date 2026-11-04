//! Bluetooth LE links, carried by the platform: the app's own Bluetooth code
//! (Android's GATT server, the desktop's BLE client) moves the bytes, and
//! the core treats the connection as one more link.
//!
//! The platform calls [`Engine::ble_opened`] when a connection comes up,
//! [`Engine::ble_frame`] with each whole frame that arrives (it splits and
//! joins frames to fit Bluetooth packets itself), and
//! [`Engine::ble_closed`] when it ends. Frames are the same `Hello` / `Msg`
//! JSON the other links use, so messages stay end-to-end encrypted and
//! every feature works over Bluetooth, slowly.
//!
//! A Bluetooth link is a fallback, like the relay: it doesn't count as
//! direct, the ladder keeps looking for a better path, and any direct link
//! replaces it. It's what's left when there's no shared network or no
//! internet. Nothing proves who is on the other end (no TLS): only the
//! message encryption protects it, as on the relay.

use std::sync::atomic::AtomicU64;

use super::*;
use crate::model::{Envelope, WsFrame};
use crate::transport::{drop_live, handle_incoming, mark_live, TransportKind};

/// Link ids in their own range (QUIC 3M, relay 4M, ...).
static BLE_CONN_SEQ: AtomicU64 = AtomicU64::new(5_000_000);

/// What the platform's Bluetooth code does for the core.
pub trait BleSender: Send + Sync {
    /// Send one whole frame to the other end.
    fn send(&self, frame: Vec<u8>);
}

/// One open Bluetooth connection.
pub(crate) struct BleLink {
    /// The device on the other end, once its `Hello` arrived.
    peer: Option<String>,
    /// Its outgoing queue (drained into the platform's sender).
    tx: tokio::sync::mpsc::UnboundedSender<String>,
}

/// Open Bluetooth connections, by link id.
pub(crate) type BleLinks = Arc<Mutex<HashMap<u64, BleLink>>>;

impl<P: Platform> Engine<P> {
    /// A Bluetooth connection came up. Returns its id for `ble_frame` and
    /// `ble_closed`. Our `Hello` goes out at once.
    pub fn ble_opened(&self, sender: Arc<dyn BleSender>) -> u64 {
        let id = BLE_CONN_SEQ.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let hello = {
            let me = self.ctx.identity.lock();
            serde_json::to_string(&WsFrame::Hello {
                node_id: me.node_id.clone(),
                name: me.name.clone(),
                boot: String::new(),
                bin_media: 0,
            })
            .unwrap_or_default()
        };
        let _ = tx.send(hello);
        self.ctx
            .ble_links
            .lock()
            .insert(id, BleLink { peer: None, tx });
        // Ends when the link is closed (its queue's last sender dropped).
        std::thread::spawn(move || {
            while let Some(frame) = rx.blocking_recv() {
                sender.send(frame.into_bytes());
            }
        });
        id
    }

    /// A whole frame arrived on Bluetooth connection `id`.
    pub fn ble_frame(&self, id: u64, frame: &[u8]) {
        let Ok(frame) = serde_json::from_slice::<WsFrame>(frame) else {
            return;
        };
        let ctx = &self.ctx;
        let peer = match frame {
            WsFrame::Hello { node_id, .. } => {
                if let Some(link) = ctx.ble_links.lock().get_mut(&id) {
                    link.peer = Some(node_id.clone());
                }
                node_id
            }
            WsFrame::Msg { nonce, ciphertext } => {
                let Some(peer) = ctx.ble_links.lock().get(&id).and_then(|l| l.peer.clone()) else {
                    return; // no Hello yet
                };
                self.route_ble(id, &peer);
                let env = Envelope { nonce, ciphertext };
                handle_incoming(ctx, &env, Some(&peer), String::new(), "BLE");
                return;
            }
            WsFrame::Bye => return, // relay only
        };
        self.route_ble(id, &peer);
    }

    /// Bluetooth connection `id` ended.
    pub fn ble_closed(&self, id: u64) {
        let Some(link) = self.ctx.ble_links.lock().remove(&id) else {
            return;
        };
        let Some(peer) = link.peer else {
            return;
        };
        let ours = {
            let mut conns = self.ctx.ws_conns.lock();
            let ours = conns.get(&peer).is_some_and(|(c, _)| *c == id);
            if ours {
                conns.remove(&peer);
            }
            ours
        };
        if ours {
            drop_live(&self.ctx, &peer);
        }
    }

    /// Route `peer` over Bluetooth link `id` when it has no route at all
    /// (first `Hello`, or a better link that has since gone).
    fn route_ble(&self, id: u64, peer: &str) {
        let Some(tx) = self.ctx.ble_links.lock().get(&id).map(|l| l.tx.clone()) else {
            return;
        };
        {
            let mut conns = self.ctx.ws_conns.lock();
            if conns.contains_key(peer) {
                return;
            }
            conns.insert(peer.to_string(), (id, Route::direct(tx)));
        }
        mark_live(&self.ctx, peer, TransportKind::Ble, None);
    }
}
