//! Ask the home router to forward our QUIC port (UPnP-IGD). When the router
//! agrees, the device is reachable from anywhere at `external_ip:port`, with
//! no hole punching and no relay. Many home routers support this; carrier
//! (CGNAT) and office networks usually don't, and then nothing changes.
//!
//! The engine's loop (engine/background.rs) maps the port at start, renews it
//! before the lease runs out, re-maps when the LAN address changes, and
//! removes it on shutdown. It's behind a trait so tests use a fake and never
//! touch a real router.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use igd_next::{search_gateway, PortMappingProtocol, SearchOptions};

/// How long the router keeps a mapping unless we renew it.
pub const LEASE_SECS: u32 = 3600;
const DESCRIPTION: &str = "my-domain";

pub trait PortMapper: Send + Sync + std::fmt::Debug {
    /// Forward UDP `external -> local`; returns the public address on success.
    fn map_udp(&self, local: SocketAddrV4, lease_secs: u32) -> Option<SocketAddrV4>;
    /// Remove a mapping made earlier (best effort).
    fn unmap_udp(&self, external_port: u16);
}

/// The real thing: UPnP Internet Gateway Device protocol.
#[derive(Debug, Default)]
pub struct UpnpMapper;

fn gateway() -> Option<igd_next::Gateway> {
    let mut opts = SearchOptions::default(); // non-exhaustive: mutate a default
    opts.timeout = Some(Duration::from_secs(3));
    search_gateway(opts).ok()
}

impl PortMapper for UpnpMapper {
    fn map_udp(&self, local: SocketAddrV4, lease_secs: u32) -> Option<SocketAddrV4> {
        let gw = gateway()?;
        let IpAddr::V4(external_ip) = gw.get_external_ip().ok()? else {
            return None;
        };
        let local_sa = SocketAddr::V4(local);
        let udp = PortMappingProtocol::UDP;
        // Same port outside as inside if free, else whatever the router picks.
        let port = if gw
            .add_port(udp, local.port(), local_sa, lease_secs, DESCRIPTION)
            .is_ok()
        {
            local.port()
        } else {
            gw.add_any_port(udp, local_sa, lease_secs, DESCRIPTION)
                .ok()?
        };
        // Behind carrier-grade NAT the router's "external" address is itself
        // private, so the mapping can't be reached from the internet.
        if !is_public_v4(&external_ip) {
            let _ = gw.remove_port(udp, port);
            return None;
        }
        Some(SocketAddrV4::new(external_ip, port))
    }

    fn unmap_udp(&self, external_port: u16) {
        if let Some(gw) = gateway() {
            let _ = gw.remove_port(PortMappingProtocol::UDP, external_port);
        }
    }
}

/// Reachable from the internet: not private, CGNAT (100.64/10), loopback,
/// link-local, documentation, or unspecified.
pub fn is_public_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || (o[0] == 100 && (o[1] & 0xc0) == 64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_internet_reachable_ipv4_counts_as_public() {
        let p = |s: &str| is_public_v4(&s.parse().unwrap());
        assert!(p("152.67.7.50"));
        assert!(!p("192.168.0.1"));
        assert!(!p("10.1.2.3"));
        assert!(!p("172.16.0.9"));
        assert!(!p("100.64.0.1"), "carrier-grade NAT");
        assert!(!p("100.127.255.254"), "carrier-grade NAT");
        assert!(p("100.128.0.1"));
        assert!(!p("127.0.0.1"));
        assert!(!p("169.254.1.1"));
        assert!(!p("192.0.2.1"), "documentation");
    }
}
