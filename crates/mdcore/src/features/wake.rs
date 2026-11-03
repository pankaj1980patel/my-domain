//! Wake a sleeping computer from the phone (Wake-on-LAN): a computer says
//! its network card's address in its status (`"wake": {"mac": "aa:bb:…"}`);
//! the core remembers it, since a sleeping computer sends no status; waking
//! broadcasts the "magic packet" (six 0xFF bytes, then the address sixteen
//! times) on the local network, UDP ports 9 and 7.
//!
//! Works only from the same network, and only on computers set to wake for
//! network access (BIOS/UEFI "Wake on LAN"; macOS "Wake for network
//! access").

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::platform::Platform;

const KEY_PREFIX: &str = "wake_mac:";

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

/// Remember the address `node` said in its status, if any.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, status: &serde_json::Value) {
    let Some(mac) = status["wake"]["mac"].as_str() else {
        return;
    };
    if parse_mac(mac).is_none() {
        return;
    }
    let key = format!("{KEY_PREFIX}{node}");
    let mac = mac.trim().to_ascii_lowercase();
    if ctx.platform.kv_get(&key).as_deref() != Some(mac.as_str()) {
        ctx.platform.kv_set(&key, &mac);
    }
}

/// The address `node` last said, if it ever did.
pub fn known<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<[u8; 6]> {
    ctx.platform
        .kv_get(&format!("{KEY_PREFIX}{node}"))
        .as_deref()
        .and_then(parse_mac)
}

/// Broadcast the magic packet for `node` on the local network.
pub fn wake<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    let mac = known(ctx, node)
        .ok_or_else(|| CoreError::InvalidInput("this device never said how to wake it".into()))?;
    let packet = magic_packet(mac);
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    // The all-ones broadcast, and this network's own (a /24 guess from our
    // address) for routers that drop the first.
    let mut targets = vec![Ipv4Addr::BROADCAST];
    if let Ok(ip) = ctx.identity.lock().ip.parse::<Ipv4Addr>() {
        let [a, b, c, _] = ip.octets();
        if !ip.is_loopback() && !ip.is_unspecified() {
            targets.push(Ipv4Addr::new(a, b, c, 255));
        }
    }
    let mut sent = false;
    for ip in targets {
        for port in [9, 7] {
            sent |= sock.send_to(&packet, SocketAddrV4::new(ip, port)).is_ok();
        }
    }
    if sent {
        Ok(())
    } else {
        Err(CoreError::NoLink("couldn't send on this network".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(&p[..6], &[0xff; 6]);
        assert!(p[6..].chunks(6).all(|c| c == mac));
        assert_eq!(p[6..].chunks(6).count(), 16);
    }
}
