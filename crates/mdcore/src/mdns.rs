//! Standard LAN discovery: mDNS / DNS-SD (what Bonjour, Android NSD and
//! Avahi speak). Each engine advertises `_mydomain._udp.local.` with its
//! node id, name and ports in TXT records, and browses for the others.
//!
//! Compared with our own beacons (discovery.rs), it's continuous and
//! automatic (no "Scan" button), it is the protocol routers and OSes expect
//! on a LAN, and it follows interface changes by itself. The beacons and the
//! /24 sweep stay as a fallback for networks that drop mDNS.
//!
//! Peers found here go through the same `merge_beacon` trust rules as
//! beacons (a LAN entry can't repoint a registry peer to another IP).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::discovery::merge_beacon;
use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::{Beacon, Identity};
use crate::platform::Platform;

pub const SERVICE_TYPE: &str = "_mydomain._udp.local.";

/// Advertise this engine and browse for others until the engine shuts down.
/// Best effort: if mDNS can't start (no multicast, port taken), discovery
/// falls back to beacons. `loopback` also uses 127.0.0.1 (tests).
pub fn start<P: Platform>(ctx: NetCtx<P>, loopback: bool) {
    let Ok(daemon) = ServiceDaemon::new() else {
        return;
    };
    if loopback {
        let _ = daemon.enable_interface(IfKind::LoopbackV4);
    }
    let id = ctx.identity.lock().clone();
    let registered = service_info(&id).and_then(|info| daemon.register(info).ok());
    let browse = daemon.browse(SERVICE_TYPE).ok();
    let (Some(()), Some(events)) = (registered, browse) else {
        let _ = daemon.shutdown();
        return;
    };
    std::thread::spawn(move || {
        while !ctx.shutdown.load(Ordering::Relaxed) {
            match events.recv_timeout(Duration::from_millis(500)) {
                Ok(ServiceEvent::ServiceResolved(svc)) => on_resolved(&ctx, &svc),
                Ok(_) | Err(mdns_sd::RecvTimeoutError::Timeout) => {}
                Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = daemon.shutdown();
    });
}

fn service_info(id: &Identity) -> Option<ServiceInfo> {
    let props: HashMap<String, String> = [
        ("id", id.node_id.clone()),
        ("name", id.name.clone()),
        ("tcp", id.tcp_port.to_string()),
        ("udp", id.udp_port.to_string()),
        ("ws", id.ws_port.to_string()),
        ("quic", id.quic_port.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let label = dns_label(&id.node_id);
    ServiceInfo::new(
        SERVICE_TYPE,
        &label,
        &format!("{label}.local."),
        (),
        id.quic_port,
        props,
    )
    .ok()
    .map(ServiceInfo::enable_addr_auto) // publish every interface's address
}

/// A DNS label from a node id: [a-z0-9-], at most 63 bytes.
fn dns_label(node_id: &str) -> String {
    let s: String = node_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    if s.is_empty() {
        "mydomain".into()
    } else {
        s
    }
}

fn on_resolved<P: Platform>(ctx: &NetCtx<P>, svc: &ResolvedService) {
    let Some((beacon, ip)) = beacon_from(svc) else {
        return;
    };
    if beacon.node_id == ctx.identity.lock().node_id {
        return;
    }
    let snapshot = {
        let mut map = ctx.peers.lock();
        merge_beacon(&mut map, &beacon, ip).then(|| map.values().cloned().collect())
    };
    if let Some(peers) = snapshot {
        ctx.sink.emit(CoreEvent::PeersUpdated { peers });
    }
}

/// The advertised record as a beacon plus the best address to reach it:
/// a private (LAN) IPv4 first, then any IPv4, then IPv6.
fn beacon_from(svc: &ResolvedService) -> Option<(Beacon, IpAddr)> {
    let txt = |k: &str| svc.get_property_val_str(k);
    let port = |k: &str| txt(k).and_then(|v| v.parse::<u16>().ok()).unwrap_or(0);
    let node_id = txt("id").filter(|s| !s.is_empty())?.to_string();
    let addrs: Vec<IpAddr> = svc.addresses.iter().map(|a| a.to_ip_addr()).collect();
    Some((
        Beacon {
            name: txt("name").unwrap_or(&node_id).to_string(),
            node_id,
            tcp_port: port("tcp"),
            udp_port: port("udp"),
            ws_port: port("ws"),
            quic_port: port("quic"),
            reply: true,
        },
        best_address(&addrs)?,
    ))
}

fn best_address(addrs: &[IpAddr]) -> Option<IpAddr> {
    let rank = |ip: &IpAddr| match ip {
        IpAddr::V4(v4) if v4.is_private() => 0,
        IpAddr::V4(v4) if !v4.is_loopback() => 1,
        IpAddr::V4(_) => 2,
        IpAddr::V6(_) => 3,
    };
    addrs.iter().min_by_key(|ip| rank(ip)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_become_valid_dns_labels() {
        assert_eq!(dns_label("2e281de12c933122"), "2e281de12c933122");
        assert_eq!(dns_label("Mac_Book.local"), "mac-book-local");
        assert_eq!(dns_label(&"x".repeat(80)).len(), 63);
        assert_eq!(dns_label(""), "mydomain");
    }

    #[test]
    fn prefers_a_lan_ipv4_address() {
        let ips = |v: &[&str]| {
            v.iter()
                .map(|s| s.parse().unwrap())
                .collect::<Vec<IpAddr>>()
        };
        assert_eq!(
            best_address(&ips(&["fe80::1", "127.0.0.1", "192.168.0.7", "8.8.8.8"])),
            Some("192.168.0.7".parse().unwrap())
        );
        assert_eq!(
            best_address(&ips(&["fe80::1", "127.0.0.1"])),
            Some("127.0.0.1".parse().unwrap())
        );
        assert_eq!(best_address(&[]), None);
    }
}
