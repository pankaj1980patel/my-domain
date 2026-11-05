//! Wake a sleeping computer from another of your devices (Wake-on-LAN).
//!
//! A computer whose owner lets the others wake it lists its wired and Wi-Fi
//! network cards in its status:
//!
//!   "wol": [{"mac": "aa:bb:…", "broadcast": "192.168.0.255",
//!            "iface": "en0", "wired": true}, …]
//!
//! The core remembers the last list per device, across restarts (a sleeping
//! computer sends no status). Waking it sends the "magic packet" (six 0xFF
//! bytes, then the address sixteen times) as a UDP broadcast, ports 9 and
//! 7, to each card's network broadcast and to 255.255.255.255, from this
//! device; and asks each other device of yours that's linked now, says it
//! can (`wol` in its status or `addr_update`), and is on one of those
//! networks to send it too:
//!
//!   wake_relay { from, mac, broadcast }   waker → a device on that network
//!
//! So a phone away from home can wake the computer through another computer
//! at home. Older apps don't know `wake_relay` (they'd show it as chat), so
//! it only goes to a device that said `wol`.
//!
//! What a status says about it:
//!   - `wol: [...]`: it may be woken (this list; an empty one keeps what was
//!     known: it's on, but couldn't read its cards just now);
//!   - `wol: 1`: it relays, but may not be woken (its switch is off): what
//!     was known is forgotten;
//!   - an app from before: `"wake": {"mac": …}` (kept, without a network),
//!     or nothing.
//!
//! Works only on computers set to wake for network access (BIOS/UEFI "Wake
//! on LAN"; macOS "Wake for network access"), from their own network.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::model::WakeReport;
use crate::platform::Platform;
use crate::transport::send_control_to;

use super::{ControlHandler, ControlMsg};

/// The status / `addr_update` field: a list of cards, or `1`.
pub const FLAG: &str = "wol";
/// The ports a magic packet goes to (discard and echo, by custom).
pub const PORTS: [u16; 2] = [9, 7];
/// Most cards kept per device.
const MAX_TARGETS: usize = 8;

const KEY_PREFIX: &str = "wol:";
/// What an app from before kept: one address, no network.
const LEGACY_PREFIX: &str = "wake_mac:";
const CAPS_PREFIX: &str = "wol_caps:";

/// One network card of a computer, as its status says it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WolTarget {
    pub mac: String,
    /// Its network's broadcast address ("" when unknown).
    #[serde(default)]
    pub broadcast: String,
    #[serde(default)]
    pub iface: String,
    #[serde(default)]
    pub wired: bool,
}

/// A network card address from `aa:bb:cc:dd:ee:ff` (or with `-`).
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.trim().split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (b, p) in mac.iter_mut().zip(parts) {
        if p.len() != 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    // All zeros or all ones isn't a card.
    (mac != [0; 6] && mac != [0xff; 6]).then_some(mac)
}

/// The magic packet for `mac`.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut p = [0xffu8; 102];
    for chunk in p[6..].chunks_mut(6) {
        chunk.copy_from_slice(&mac);
    }
    p
}

/// How many low bits of `bcast` are host bits: its run of trailing ones,
/// when that makes a network from a /8 to a /30.
fn host_bits(bcast: Ipv4Addr) -> Option<u32> {
    let n = u32::from(bcast).trailing_ones();
    (2..=24).contains(&n).then_some(n)
}

/// A local network's broadcast address: on a private, link-local (or, for
/// tests, loopback) network. Never a public address: a relay must not be
/// made to send packets out to the internet.
pub fn is_lan_broadcast(bcast: Ipv4Addr) -> bool {
    host_bits(bcast).is_some()
        && (bcast.is_private() || bcast.is_link_local() || bcast.is_loopback())
}

/// Whether `ip` is on the network whose broadcast address is `bcast`
/// (taken as wide as its trailing ones allow: 192.168.0.255 is a /24).
pub fn on_network(ip: Ipv4Addr, bcast: Ipv4Addr) -> bool {
    let Some(n) = host_bits(bcast) else {
        return false;
    };
    let mask = u32::MAX << n;
    ip != bcast && u32::from(ip) & mask == u32::from(bcast) & mask
}

/// What a status says to do with what's known about waking its device.
#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    /// Keep it as it is.
    Keep,
    /// Forget it: it may not be woken now.
    Forget,
    /// These cards, from now on.
    Set(Vec<WolTarget>),
}

/// Read a status (see the module's notes).
pub fn update_from(status: &Value) -> Update {
    match &status[FLAG] {
        Value::Array(list) => {
            let mut seen = HashSet::new();
            let targets: Vec<WolTarget> = list
                .iter()
                .filter_map(|t| serde_json::from_value::<WolTarget>(t.clone()).ok())
                .filter_map(clean)
                .filter(|t| seen.insert((t.mac.clone(), t.broadcast.clone())))
                .take(MAX_TARGETS)
                .collect();
            if targets.is_empty() {
                Update::Keep
            } else {
                Update::Set(targets)
            }
        }
        Value::Null => match status["wake"]["mac"].as_str().and_then(legacy) {
            Some(t) => Update::Set(vec![t]),
            None => Update::Keep,
        },
        _ => Update::Forget,
    }
}

/// An app from before's `wake.mac`.
fn legacy(mac: &str) -> Option<WolTarget> {
    clean(WolTarget {
        mac: mac.to_string(),
        broadcast: String::new(),
        iface: String::new(),
        wired: false,
    })
}

/// A card as it's kept: a real address (lowercase), a local broadcast or
/// none, a short name.
fn clean(t: WolTarget) -> Option<WolTarget> {
    parse_mac(&t.mac)?;
    let broadcast = t
        .broadcast
        .trim()
        .parse::<Ipv4Addr>()
        .ok()
        .filter(|b| is_lan_broadcast(*b))
        .map(|b| b.to_string())
        .unwrap_or_default();
    Some(WolTarget {
        mac: t.mac.trim().to_ascii_lowercase().replace('-', ":"),
        broadcast,
        iface: t
            .iface
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect(),
        wired: t.wired,
    })
}

/// Whether a `wol` field (status or `addr_update`) says its device relays.
fn says_relays(flag: &Value) -> bool {
    flag.as_u64().is_some_and(|v| v >= 1) || flag.is_array()
}

/// This device's status gets `wol: 1` (it relays), unless the host put the
/// cards there already (which says so too).
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.wake_relay && !status[FLAG].is_array() {
        status[FLAG] = 1.into();
    }
}

/// `node`'s status: remember how to wake it, and whether it relays.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, status: &Value) {
    if node.is_empty() {
        return;
    }
    remember_caps(ctx, node, &status[FLAG]);
    let key = format!("{KEY_PREFIX}{node}");
    match update_from(status) {
        Update::Keep => {}
        Update::Forget => {
            if ctx.platform.kv_get(&key).is_some_and(|v| !v.is_empty()) {
                ctx.platform.kv_set(&key, "");
            }
            if ctx
                .platform
                .kv_get(&format!("{LEGACY_PREFIX}{node}"))
                .is_some_and(|v| !v.is_empty())
            {
                ctx.platform.kv_set(&format!("{LEGACY_PREFIX}{node}"), "");
            }
        }
        Update::Set(targets) => {
            let json = serde_json::to_string(&targets).unwrap_or_default();
            if ctx.platform.kv_get(&key).as_deref() != Some(json.as_str()) {
                ctx.platform.kv_set(&key, &json);
            }
        }
    }
}

/// `node` said (`flag`: the `wol` of its status or `addr_update`) whether
/// it sends magic packets for the others. Kept across restarts.
pub fn remember_caps<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    let on = says_relays(flag);
    if node.is_empty() || relays(ctx, node) == on {
        return;
    }
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
}

/// Whether `node` said it relays.
pub fn relays<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .as_deref()
        == Some("1")
}

/// The cards `node` last said it may be woken through.
pub fn known<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Vec<WolTarget> {
    let saved = ctx
        .platform
        .kv_get(&format!("{KEY_PREFIX}{node}"))
        .and_then(|j| serde_json::from_str::<Vec<WolTarget>>(&j).ok())
        .unwrap_or_default();
    if !saved.is_empty() {
        return saved.into_iter().filter_map(clean).collect();
    }
    ctx.platform
        .kv_get(&format!("{LEGACY_PREFIX}{node}"))
        .as_deref()
        .and_then(legacy)
        .into_iter()
        .collect()
}

/// Where this device sends `t`'s packet: its network's broadcast (or, not
/// knowing it, a /24 guess from `my_ip`) and 255.255.255.255.
pub fn destinations(t: &WolTarget, my_ip: Option<Ipv4Addr>) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    match t.broadcast.parse::<Ipv4Addr>() {
        Ok(b) => out.push(b),
        Err(_) => {
            if let Some(ip) = my_ip.filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) {
                let [a, b, c, _] = ip.octets();
                out.push(Ipv4Addr::new(a, b, c, 255));
            }
        }
    }
    out.push(Ipv4Addr::BROADCAST);
    out
}

/// Send `mac`'s packet to each of `dests`, ports 9 and 7: the
/// `ip:port`s it went to.
fn send_packets(mac: [u8; 6], dests: &[Ipv4Addr]) -> Vec<String> {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
        return Vec::new();
    };
    if sock.set_broadcast(true).is_err() {
        return Vec::new();
    }
    let packet = magic_packet(mac);
    let mut sent = Vec::new();
    for ip in dests {
        for port in PORTS {
            let to = SocketAddrV4::new(*ip, port);
            if sock.send_to(&packet, to).is_ok() {
                sent.push(to.to_string());
            }
        }
    }
    sent
}

/// The IPv4 addresses `peer` is known by: its LAN address and its link's.
fn peer_ips<P: Platform>(ctx: &NetCtx<P>, peer: &str) -> Vec<Ipv4Addr> {
    let mut ips = Vec::new();
    if let Some(ip) = ctx
        .peers
        .lock()
        .get(peer)
        .and_then(|p| p.ip.parse::<Ipv4Addr>().ok())
    {
        ips.push(ip);
    }
    if let Some(std::net::SocketAddr::V4(a)) = ctx.p2p_conns.lock().get(peer).and_then(|c| c.addr) {
        ips.push(*a.ip());
    }
    ips
}

/// Wake `node`: send its packet from here, and ask the devices of yours on
/// its network to send it too.
pub fn wake<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<WakeReport> {
    let targets = known(ctx, node);
    if targets.is_empty() {
        return Err(CoreError::InvalidInput(
            "this computer never said it may be woken".into(),
        ));
    }
    let (me, my_ip) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.ip.parse::<Ipv4Addr>().ok())
    };
    let mut report = WakeReport::default();
    for t in &targets {
        let Some(mac) = parse_mac(&t.mac) else {
            continue;
        };
        for to in send_packets(mac, &destinations(t, my_ip)) {
            if !report.sent.contains(&to) {
                report.sent.push(to);
            }
        }
    }
    // The other linked devices that relay, on one of its networks.
    let linked: Vec<String> = ctx.p2p_conns.lock().keys().cloned().collect();
    for peer in linked {
        if peer == node || peer == me || !relays(ctx, &peer) {
            continue;
        }
        let ips = peer_ips(ctx, &peer);
        let mut asked = false;
        for t in &targets {
            let Ok(bcast) = t.broadcast.parse::<Ipv4Addr>() else {
                continue;
            };
            if !ips.iter().any(|ip| on_network(*ip, bcast)) {
                continue;
            }
            let msg = serde_json::json!({
                "type": "wake_relay",
                "from": me,
                "mac": t.mac,
                "broadcast": t.broadcast,
            })
            .to_string();
            send_control_to(ctx, &peer, &msg);
            asked = true;
        }
        if asked {
            report.relays.push(peer);
        }
    }
    if report.sent.is_empty() && report.relays.is_empty() {
        return Err(CoreError::NoLink(
            "couldn't send the wake-up on this network".into(),
        ));
    }
    Ok(report)
}

pub struct WakeRelayHandler;

impl<P: Platform> ControlHandler<P> for WakeRelayHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["wake_relay"]
    }

    /// Another device of yours asks this one to send a packet on its
    /// network: only a real address, only to a local broadcast.
    fn handle(&self, _ctx: &NetCtx<P>, msg: &ControlMsg) {
        if msg.sender().is_empty() {
            return;
        }
        let Some(mac) = parse_mac(&msg.str("mac")) else {
            return;
        };
        let Ok(bcast) = msg.str("broadcast").trim().parse::<Ipv4Addr>() else {
            return;
        };
        if !is_lan_broadcast(bcast) {
            return;
        }
        // Off this thread: sending may block a moment.
        std::thread::spawn(move || {
            send_packets(mac, &[bcast, Ipv4Addr::BROADCAST]);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addresses_parse_and_bad_ones_dont() {
        assert_eq!(
            parse_mac("AA:bb:0c:01:02:ff"),
            Some([0xaa, 0xbb, 0x0c, 0x01, 0x02, 0xff])
        );
        assert_eq!(
            parse_mac("aa-bb-0c-01-02-ff"),
            parse_mac("aa:bb:0c:01:02:ff")
        );
        assert_eq!(parse_mac("aa:bb:0c:01:02"), None);
        assert_eq!(parse_mac("aa:bb:0c:01:02:zz"), None);
        assert_eq!(parse_mac("aaa:b:0c:01:02:ff"), None);
        assert_eq!(parse_mac("00:00:00:00:00:00"), None);
        assert_eq!(parse_mac("ff:ff:ff:ff:ff:ff"), None);
    }

    #[test]
    fn the_magic_packet_is_six_ones_then_the_address_sixteen_times() {
        let mac = [1, 2, 3, 4, 5, 6];
        let p = magic_packet(mac);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xff; 6]);
        assert!(p[6..].chunks(6).all(|c| c == mac));
        assert_eq!(p[6..].chunks(6).count(), 16);
        let real = magic_packet(parse_mac("00:1a:2b:3c:4d:5e").unwrap_or_default());
        assert_eq!(&real[6..12], &[0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]);
        assert_eq!(&real[96..], &[0x00, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]);
    }

    #[test]
    fn only_local_broadcasts_count() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap_or(Ipv4Addr::UNSPECIFIED);
        assert!(is_lan_broadcast(ip("192.168.0.255")));
        assert!(is_lan_broadcast(ip("10.0.255.255")));
        assert!(is_lan_broadcast(ip("172.16.3.127")));
        assert!(is_lan_broadcast(ip("169.254.255.255")));
        assert!(!is_lan_broadcast(ip("8.8.8.255")), "public");
        assert!(
            !is_lan_broadcast(ip("192.168.0.10")),
            "a host, not a broadcast"
        );
        assert!(!is_lan_broadcast(ip("255.255.255.255")), "no network");
    }

    #[test]
    fn a_device_is_on_a_network_by_its_broadcast() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap_or(Ipv4Addr::UNSPECIFIED);
        assert!(on_network(ip("192.168.0.176"), ip("192.168.0.255")));
        assert!(!on_network(ip("192.168.1.176"), ip("192.168.0.255")));
        assert!(on_network(ip("10.1.2.3"), ip("10.1.255.255")));
        assert!(!on_network(ip("192.168.0.255"), ip("192.168.0.255")));
        assert!(!on_network(ip("192.168.0.4"), ip("255.255.255.255")));
    }

    #[test]
    fn packets_go_to_the_network_and_everywhere() {
        let t = WolTarget {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            broadcast: "192.168.0.255".into(),
            iface: "en0".into(),
            wired: true,
        };
        let me = Some(Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(
            destinations(&t, me),
            vec![Ipv4Addr::new(192, 168, 0, 255), Ipv4Addr::BROADCAST]
        );
        // From an app from before: no network known, guessed from ours.
        let old = WolTarget {
            broadcast: String::new(),
            ..t
        };
        assert_eq!(
            destinations(&old, me),
            vec![Ipv4Addr::new(10, 0, 0, 255), Ipv4Addr::BROADCAST]
        );
        assert_eq!(destinations(&old, None), vec![Ipv4Addr::BROADCAST]);
    }

    #[test]
    fn a_status_sets_keeps_or_forgets_what_is_known() {
        let set = update_from(&json!({"wol": [
            {"mac": "AA-BB-CC-00-11-22", "broadcast": "192.168.0.255", "iface": "en0", "wired": true},
            {"mac": "aa:bb:cc:00:11:22", "broadcast": "192.168.0.255", "iface": "dup"},
            {"mac": "bad", "broadcast": "192.168.0.255"},
            {"mac": "aa:bb:cc:00:11:33", "broadcast": "8.8.8.255", "iface": "en1"},
        ]}));
        assert_eq!(
            set,
            Update::Set(vec![
                WolTarget {
                    mac: "aa:bb:cc:00:11:22".into(),
                    broadcast: "192.168.0.255".into(),
                    iface: "en0".into(),
                    wired: true,
                },
                // A public "broadcast" isn't kept; the card is.
                WolTarget {
                    mac: "aa:bb:cc:00:11:33".into(),
                    broadcast: String::new(),
                    iface: "en1".into(),
                    wired: false,
                },
            ])
        );
        // Switched on, but nothing read just now: keep the last.
        assert_eq!(update_from(&json!({"wol": []})), Update::Keep);
        // Switched off (it still relays).
        assert_eq!(update_from(&json!({"wol": 1})), Update::Forget);
        // An app from before.
        assert_eq!(
            update_from(&json!({"wake": {"mac": "AA:BB:CC:00:11:22"}})),
            Update::Set(vec![WolTarget {
                mac: "aa:bb:cc:00:11:22".into(),
                broadcast: String::new(),
                iface: String::new(),
                wired: false,
            }])
        );
        assert_eq!(update_from(&json!({"battery": 40})), Update::Keep);
    }

    #[test]
    fn a_list_or_a_one_says_it_relays() {
        assert!(says_relays(&json!(1)));
        assert!(says_relays(&json!([])));
        assert!(!says_relays(&Value::Null));
        assert!(!says_relays(&json!(0)));
    }
}
