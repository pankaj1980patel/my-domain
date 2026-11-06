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
//!
//! Audio rides it too, to a device that reads Opus (`audio_codec: 2`): a
//! frame that starts with the media record version (`transport::media`,
//! 1; a JSON frame starts with `{`) is a sealed binary record, sent by
//! `ble_send_media` straight onto the link whatever the device's route is
//! (a stream may choose Bluetooth with a network link up). At most
//! [`MEDIA_QUEUE`] records wait for the radio; more are dropped (late audio
//! is noise).

use std::sync::atomic::AtomicUsize;

use std::sync::atomic::AtomicU64;

use super::*;
use crate::model::WsFrame;
use crate::transport::{drop_live, handle_incoming, mark_live, TransportKind};

/// Media records waiting for the radio at most, per link (a quarter
/// second of Bluetooth audio).
pub const MEDIA_QUEUE: usize = 4;

/// Link ids in their own range (QUIC 3M, relay 4M, ...).
static BLE_CONN_SEQ: AtomicU64 = AtomicU64::new(5_000_000);

/// What the platform's Bluetooth code does for the core.
pub trait BleSender: Send + Sync {
    /// Send one whole frame to the other end.
    fn send(&self, frame: Vec<u8>);

    /// Frames handed over and not yet on the air (0 if it doesn't count):
    /// media records wait no more than `MEDIA_QUEUE` in all.
    fn pending(&self) -> usize {
        0
    }
}

/// One open Bluetooth connection.
pub(crate) struct BleLink {
    /// The device on the other end, once its `Hello` arrived.
    peer: Option<String>,
    /// Its outgoing queue (drained into the platform's sender).
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    /// The same queue, for binary media records, and how many wait.
    bin: std::sync::mpsc::Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    sender: Arc<dyn BleSender>,
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
                lane: String::new(),
            })
            .unwrap_or_default()
        };
        let _ = tx.send(hello);
        // One queue to the radio, in order: the messages are moved onto it
        // by a thread of their own.
        let (bin, out) = std::sync::mpsc::channel::<Vec<u8>>();
        let queued = Arc::new(AtomicUsize::new(0));
        let to_out = bin.clone();
        // Ends when the link is closed (its queue's last sender dropped).
        std::thread::spawn(move || {
            while let Some(frame) = rx.blocking_recv() {
                if to_out.send(frame.into_bytes()).is_err() {
                    break;
                }
            }
        });
        let q = queued.clone();
        let platform = sender.clone();
        std::thread::spawn(move || {
            while let Ok(frame) = out.recv() {
                let media = frame.first() == Some(&crate::transport::media::VERSION);
                sender.send(frame);
                if media {
                    q.fetch_sub(1, Ordering::Relaxed);
                }
            }
        });
        self.ctx.ble_links.lock().insert(
            id,
            BleLink {
                peer: None,
                tx,
                bin,
                queued,
                sender: platform,
            },
        );
        id
    }

    /// A whole frame arrived on Bluetooth connection `id`.
    pub fn ble_frame(&self, id: u64, frame: &[u8]) {
        if frame.first() == Some(&crate::transport::media::VERSION) {
            let Some(peer) = self
                .ctx
                .ble_links
                .lock()
                .get(&id)
                .and_then(|l| l.peer.clone())
            else {
                return; // no Hello yet
            };
            self.route_ble(id, &peer);
            crate::transport::media::on_record_via(&self.ctx, &peer, frame, true);
            return;
        }
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
            WsFrame::Msg(env) => {
                let Some(peer) = ctx.ble_links.lock().get(&id).and_then(|l| l.peer.clone()) else {
                    return; // no Hello yet
                };
                self.route_ble(id, &peer);
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
        // A device you removed gets no link (`features::revoke`).
        if self.ctx.revoked.is_revoked(peer) {
            return;
        }
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

/// Link `id` is a Bluetooth one.
pub(crate) fn is_ble_link<P: Platform>(ctx: &NetCtx<P>, id: u64) -> bool {
    ctx.ble_links.lock().contains_key(&id)
}

/// Send a media frame to `node` as a sealed binary record over its
/// Bluetooth link (whatever its route is). Dropped when the radio is
/// behind.
pub(crate) fn ble_send_media<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    h: &crate::transport::media::Header,
    payload: &[u8],
) -> crate::transport::media::Sent {
    use crate::transport::media::{encode, Sent};
    let Some(key) = *ctx.key.lock() else {
        return Sent::NotBinary;
    };
    let links = ctx.ble_links.lock();
    let Some(link) = links.values().find(|l| l.peer.as_deref() == Some(node)) else {
        return Sent::NotBinary;
    };
    if link.queued.load(Ordering::Relaxed) + link.sender.pending() >= MEDIA_QUEUE {
        return Sent::Dropped;
    }
    let Some(record) = encode(&key, h, payload) else {
        return Sent::NotBinary;
    };
    link.queued.fetch_add(1, Ordering::Relaxed);
    if link.bin.send(record).is_err() {
        link.queued.fetch_sub(1, Ordering::Relaxed);
        return Sent::Dropped;
    }
    Sent::Yes
}

/// A Bluetooth connection to `node` is open (it's in range).
pub(crate) fn ble_linked<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.ble_links
        .lock()
        .values()
        .any(|l| l.peer.as_deref() == Some(node))
}
