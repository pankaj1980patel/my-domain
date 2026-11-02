//! Ask the home router to forward our QUIC port (UPnP-IGD, else PCP /
//! NAT-PMP, see `FirstOf`). When the router
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
use parking_lot::Mutex;

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

/// PCP (RFC 6887), falling back to NAT-PMP (RFC 6886): what Apple routers,
/// many ISP boxes and OpenWrt speak when UPnP is off. The gateway is assumed
/// to be the LAN's `.1` (true for most home networks; a miss just means no
/// mapping). Holds its mapping so renewals refresh it instead of requesting
/// a new one (PCP ties a mapping to a nonce).
#[derive(Debug, Default)]
pub struct PcpMapper {
    current: Mutex<Option<crab_nat::PortMapping>>,
}

/// The usual home-router address on our subnet: a.b.c.1.
pub fn guess_gateway(local: &Ipv4Addr) -> Ipv4Addr {
    let o = local.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 1)
}

impl PortMapper for PcpMapper {
    fn map_udp(&self, local: SocketAddrV4, lease_secs: u32) -> Option<SocketAddrV4> {
        use crab_nat::{
            InternetProtocol, PortMapping, PortMappingOptions, PortMappingType, TimeoutConfig,
        };
        use std::num::NonZeroU16;
        let gw = guess_gateway(local.ip());
        let timeouts = TimeoutConfig {
            initial_timeout: Duration::from_millis(250),
            max_retries: 2,
            max_retry_timeout: Some(Duration::from_secs(1)),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        let previous = self.current.lock().take();
        let mapping = rt.block_on(async {
            match previous {
                Some(mut m) if m.internal_port().get() == local.port() => {
                    m.renew().await.ok().map(|()| m)
                }
                _ => PortMapping::new(
                    gw.into(),
                    IpAddr::V4(*local.ip()),
                    InternetProtocol::Udp,
                    NonZeroU16::new(local.port())?,
                    PortMappingOptions {
                        external_port: NonZeroU16::new(local.port()),
                        lifetime_seconds: Some(lease_secs),
                        timeout_config: Some(timeouts),
                    },
                )
                .await
                .ok(),
            }
        })?;
        let external_ip = match mapping.mapping_type() {
            PortMappingType::Pcp {
                external_ip: IpAddr::V4(v4),
                ..
            } => Some(v4),
            PortMappingType::Pcp { .. } => None,
            PortMappingType::NatPmp => rt
                .block_on(crab_nat::natpmp::external_address(
                    gw.into(),
                    Some(timeouts),
                ))
                .ok(),
        };
        match external_ip.filter(is_public_v4) {
            Some(ip) => {
                let public = SocketAddrV4::new(ip, mapping.external_port().get());
                *self.current.lock() = Some(mapping);
                Some(public)
            }
            None => {
                let _ = rt.block_on(mapping.try_drop());
                None
            }
        }
    }

    fn unmap_udp(&self, external_port: u16) {
        let Some(m) = self.current.lock().take() else {
            return;
        };
        if m.external_port().get() != external_port {
            *self.current.lock() = Some(m);
            return;
        }
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(m.try_drop());
        }
    }
}

/// Try several mappers in order (UPnP, then PCP / NAT-PMP); remember which
/// one succeeded so renewals and the unmap go to it.
#[derive(Debug)]
pub struct FirstOf {
    mappers: Vec<std::sync::Arc<dyn PortMapper>>,
    winner: Mutex<Option<usize>>,
}

impl FirstOf {
    pub fn new(mappers: Vec<std::sync::Arc<dyn PortMapper>>) -> Self {
        FirstOf {
            mappers,
            winner: Mutex::new(None),
        }
    }

    /// The default chain: UPnP-IGD, then PCP / NAT-PMP.
    pub fn standard() -> Self {
        Self::new(vec![
            std::sync::Arc::new(UpnpMapper),
            std::sync::Arc::new(PcpMapper::default()),
        ])
    }
}

impl PortMapper for FirstOf {
    fn map_udp(&self, local: SocketAddrV4, lease_secs: u32) -> Option<SocketAddrV4> {
        // The one that worked last time first, then the rest in order.
        let last = *self.winner.lock();
        let order = last
            .into_iter()
            .chain((0..self.mappers.len()).filter(|i| Some(*i) != last));
        for i in order {
            if let Some(public) = self.mappers[i].map_udp(local, lease_secs) {
                *self.winner.lock() = Some(i);
                return Some(public);
            }
        }
        *self.winner.lock() = None;
        None
    }

    fn unmap_udp(&self, external_port: u16) {
        if let Some(i) = self.winner.lock().take() {
            self.mappers[i].unmap_udp(external_port);
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
    use std::sync::Arc;

    /// A scripted mapper: answers `public` (or refuses) and logs calls.
    #[derive(Debug)]
    struct Scripted {
        public: Option<SocketAddrV4>,
        log: Arc<Mutex<Vec<String>>>,
        name: &'static str,
    }
    impl PortMapper for Scripted {
        fn map_udp(&self, _: SocketAddrV4, _: u32) -> Option<SocketAddrV4> {
            self.log.lock().push(format!("{} map", self.name));
            self.public
        }
        fn unmap_udp(&self, port: u16) {
            self.log.lock().push(format!("{} unmap {port}", self.name));
        }
    }

    #[test]
    fn first_of_falls_through_and_unmaps_with_the_one_that_worked() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let public: SocketAddrV4 = "152.67.7.50:41000".parse().unwrap();
        let chain = FirstOf::new(vec![
            Arc::new(Scripted {
                public: None,
                log: log.clone(),
                name: "upnp",
            }),
            Arc::new(Scripted {
                public: Some(public),
                log: log.clone(),
                name: "pcp",
            }),
        ]);
        let local: SocketAddrV4 = "192.168.0.5:41000".parse().unwrap();
        assert_eq!(chain.map_udp(local, 60), Some(public));
        // Renewal asks the winner first (and stops there).
        assert_eq!(chain.map_udp(local, 60), Some(public));
        chain.unmap_udp(41000);
        assert_eq!(
            *log.lock(),
            vec!["upnp map", "pcp map", "pcp map", "pcp unmap 41000"]
        );
    }

    #[test]
    fn the_gateway_guess_is_dot_one_on_our_subnet() {
        assert_eq!(
            guess_gateway(&"192.168.0.105".parse().unwrap()),
            "192.168.0.1".parse::<Ipv4Addr>().unwrap()
        );
        assert_eq!(
            guess_gateway(&"10.20.30.40".parse().unwrap()),
            "10.20.30.1".parse::<Ipv4Addr>().unwrap()
        );
    }

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
