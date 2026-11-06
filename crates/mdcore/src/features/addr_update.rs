//! Peers keep each other's addresses fresh, without the server. A device
//! sends its current QUIC addresses (LAN IPv4, global IPv6, router mapping,
//! QUIC port) to every connected peer when a link comes up, after a network
//! change, and when its router mapping changes. The receiver updates its
//! entry (and so its peer cache), so the next reconnect can go direct.
//!
//! It also says `labels: 1` when this device shares device labels
//! (`features::labels`) and `notes: 1` when it shares notes
//! (`features::notes`), so each end of a new link learns that at once;
//! older apps ignore the fields. Likewise `ping: 1` when it answers pings
//! and speed tests (`features::diag`), `wol: 1` when it sends
//! Wake-on-LAN packets for the others (`features::wake`), and `links_map: 1`
//! when it shares its link list for the map (`features::links_map`),
//! `nearby: 1` for nearby media mode (`features::nearby`) and
//! `audio_codec: 1` when it reads the smaller audio
//! (`features::audio_codec`), and `print: 1` while a computer prints for
//! your phones (`features::print`), and `device`, what kind of device it
//! is (`features::device_kind`).
//!
//! Only peers we already know are updated: this message can't create one.
//! It arrives end-to-end encrypted with the shared key, like every message.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

pub struct AddrUpdateHandler;

/// Tell `peers` (or every live peer when `None`) where we can be reached now.
pub fn announce<P: Platform>(ctx: &NetCtx<P>, peers: Option<&[String]>) {
    let msg = {
        let id = ctx.identity.lock();
        let mut msg = serde_json::json!({
            "type": "addr_update",
            "from": id.node_id,
            "ip": id.ip,
            "ipv6": id.ipv6,
            "mapped_quic": id.mapped_quic,
            "quic_port": id.quic_port,
        });
        if ctx.config.shared_labels {
            msg[super::labels::FLAG] = 1.into();
        }
        super::notes::extend_status(ctx, &mut msg);
        super::read_later::extend_status(ctx, &mut msg);
        super::diag::extend_status(ctx, &mut msg);
        super::links_map::extend_status(ctx, &mut msg);
        super::nearby::extend_status(ctx, &mut msg);
        super::audio_share::extend_codec(ctx, &mut msg);
        super::camera::extend_status(ctx, &mut msg);
        super::insert_capture::extend_status(ctx, &mut msg);
        super::timers::extend_status(ctx, &mut msg);
        super::wifi_direct::extend_status(ctx, &mut msg);
        super::kvm::extend_status(ctx, &mut msg);
        super::print::extend_status(ctx, &mut msg);
        super::remote_download::extend_status(ctx, &mut msg);
        super::notif_rich::extend_status(ctx, &mut msg);
        super::batches::extend_status(ctx, &mut msg);
        super::media::extend_status(ctx, &mut msg);
        super::device_kind::extend_status(ctx, &mut msg);
        if ctx.config.wake_relay {
            msg[super::wake::FLAG] = 1.into();
        }
        msg.to_string()
    };
    let targets: Vec<String> = match peers {
        Some(p) => p.to_vec(),
        None => ctx.p2p_conns.lock().keys().cloned().collect(),
    };
    for peer in targets {
        send_control_to(ctx, &peer, &msg);
    }
}

impl<P: Platform> ControlHandler<P> for AddrUpdateHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["addr_update"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.str("from");
        let changed = {
            let mut peers = ctx.peers.lock();
            let Some(p) = peers.get_mut(&from) else {
                return; // unknown device: never create peers from this
            };
            let before = p.clone();
            if let Ok(ip) = msg.str("ip").parse::<Ipv4Addr>() {
                if !ip.is_unspecified() {
                    p.ip = ip.to_string();
                }
            }
            p.ipv6 = msg.opt("ipv6").filter(|s| s.parse::<IpAddr>().is_ok());
            p.mapped_quic = msg
                .opt("mapped_quic")
                .filter(|s| s.parse::<SocketAddr>().is_ok());
            if let Some(port) = msg
                .json("quic_port")
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
            {
                if port != 0 {
                    p.quic_port = port;
                }
            }
            *p != before
        };
        super::labels::remember(ctx, &from, &msg.json(super::labels::FLAG));
        super::notes::remember(ctx, &from, &msg.json(super::notes::FLAG));
        super::read_later::remember(ctx, &from, &msg.json(super::read_later::FLAG));
        super::diag::remember(ctx, &from, &msg.json(super::diag::FLAG));
        super::links_map::remember(ctx, &from, &msg.json(super::links_map::FLAG));
        super::nearby::remember(ctx, &from, &msg.json(super::nearby::FLAG));
        super::audio_share::remember_codec(ctx, &from, &msg.json(super::audio_share::CODEC_FLAG));
        super::camera::remember(ctx, &from, &msg.json(super::camera::FLAG));
        super::insert_capture::remember(ctx, &from, &msg.json(super::insert_capture::FLAG));
        super::notif_rich::remember(ctx, &from, &msg.json(super::notif_rich::FLAG));
        super::device_kind::remember(
            ctx,
            &from,
            &msg.json(super::device_kind::FLAG),
            &serde_json::Value::Null,
            &msg.json(super::kvm::OS_FLAG),
        );
        super::timers::remember(ctx, &from, &msg.json(super::timers::FLAG));
        super::wifi_direct::remember(ctx, &from, &msg.json(super::wifi_direct::FLAG));
        super::kvm::remember(
            ctx,
            &from,
            &msg.json(super::kvm::FLAG),
            &msg.json(super::kvm::OS_FLAG),
        );
        super::print::remember(ctx, &from, &msg.json(super::print::FLAG));
        super::remote_download::remember(ctx, &from, &msg.json(super::remote_download::FLAG));
        super::batches::remember(ctx, &from, &msg.json(super::batches::FLAG));
        super::media::remember(ctx, &from, &msg.json(super::media::FLAG));
        super::wake::remember_caps(ctx, &from, &msg.json(super::wake::FLAG));
        if changed {
            let list = ctx.peers.lock().values().cloned().collect();
            ctx.sink.emit(CoreEvent::PeersUpdated { peers: list });
        }
    }
}
