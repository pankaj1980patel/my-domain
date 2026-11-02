//! Engine configuration, fixed at start.
//!
//! Before this existed, values like the STUN server were constants buried in
//! the transport files, so tests had to reach Google's STUN server over the
//! internet and couldn't skip connect-ladder rungs. Pass a `CoreConfig` to
//! `Engine::start_with_config`; `Engine::start` uses `CoreConfig::default()`.
//!
//! Node analogy: the options object you pass to a library's constructor.

/// Which connect-ladder rungs `Engine::connect` may try (see
/// `learnings/01-architecture.md`). All enabled by default. Disabling rungs
/// lets tests exercise a specific path, and lets a user force one when
/// debugging ("only try hole punching").
#[derive(Clone, Debug)]
pub struct ConnectRungs {
    /// 0. Direct QUIC to the peer's advertised address (reliable, encrypted,
    ///    multiplexed; preferred whenever it works).
    pub quic: bool,
    /// 1. UDP ping on the same LAN.
    pub lan_udp: bool,
    /// 2. Direct WebSocket to an address the peer advertises as reachable.
    pub direct_ws: bool,
    /// 3. Ask the peer (via a signal) to open its WebSocket for us.
    pub signaled_ws: bool,
    /// 4. STUN + UDP hole punch.
    pub punch: bool,
    /// 5. WebRTC data channel (needs the `webrtc` cargo feature).
    pub webrtc: bool,
    /// 6. Relay through the registry server's WebSocket `/relay`.
    pub relay: bool,
}

impl Default for ConnectRungs {
    fn default() -> Self {
        ConnectRungs {
            quic: true,
            lan_udp: true,
            direct_ws: true,
            signaled_ws: true,
            punch: true,
            webrtc: true,
            relay: true,
        }
    }
}

impl ConnectRungs {
    /// Every rung off; switch on the ones you want (usable in `const`s).
    pub const NONE: Self = ConnectRungs {
        quic: false,
        lan_udp: false,
        direct_ws: false,
        signaled_ws: false,
        punch: false,
        webrtc: false,
        relay: false,
    };

    /// Every rung off; switch on the ones you want.
    pub fn none() -> Self {
        Self::NONE
    }
}

/// How often idle links are pinged, and how long silence means "dead".
///
/// The interval must stay under the NAT mapping timeout of typical routers
/// (often 30 s): a mapping that expires closes a punched path. Every ping
/// also wakes a phone's radio, so it shouldn't be any shorter than needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeepAlive {
    /// `None`: never ping (links then die after `idle_timeout` of silence).
    pub interval: Option<std::time::Duration>,
    pub idle_timeout: std::time::Duration,
}

impl Default for KeepAlive {
    fn default() -> Self {
        KeepAlive {
            interval: Some(std::time::Duration::from_secs(20)),
            idle_timeout: std::time::Duration::from_secs(60),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CoreConfig {
    /// STUN servers (`host:port`) used to learn our public address for hole
    /// punching and WebRTC. Empty = no STUN (LAN/host candidates only), which
    /// is what loopback tests want.
    pub stun_servers: Vec<String>,
    pub rungs: ConnectRungs,
    /// When sending to a peer with no live link, first open a QUIC link to it
    /// (if it advertises a QUIC port) and send over that, instead of a lone
    /// UDP datagram / TCP connection. The link is then reused.
    pub auto_quic: bool,
    /// Relay WebSocket URL. `None` = derive from the server URL
    /// (`http://host:port` -> `ws://host:port/relay`). Tests point this at a
    /// fake relay.
    pub relay_url: Option<String>,
    /// Ask the home router to forward our QUIC port (UPnP). `None` = don't
    /// (tests, or networks where it's unwanted).
    pub port_mapper: Option<std::sync::Arc<dyn crate::nat::portmap::PortMapper>>,
    /// Advertise and browse via mDNS / DNS-SD on the LAN.
    pub mdns: bool,
    /// Also run mDNS on the loopback interface (tests on one machine).
    pub mdns_loopback: bool,
    /// Link keep-alive (QUIC and the connectionless-link heartbeat).
    pub keepalive: KeepAlive,
}

impl Default for CoreConfig {
    fn default() -> Self {
        CoreConfig {
            // Two independent providers: NAT classification needs answers
            // from two different destinations.
            stun_servers: vec![
                "stun.l.google.com:19302".into(),
                "stun.cloudflare.com:3478".into(),
            ],
            rungs: ConnectRungs::default(),
            auto_quic: true,
            relay_url: None,
            port_mapper: Some(std::sync::Arc::new(crate::nat::portmap::UpnpMapper)),
            mdns: true,
            mdns_loopback: false,
            keepalive: KeepAlive::default(),
        }
    }
}

impl CoreConfig {
    /// The first configured STUN server that resolves to an IPv4 address.
    pub fn stun_server(&self) -> Option<std::net::SocketAddr> {
        self.stun_addrs().into_iter().next()
    }

    /// Every configured STUN server, resolved: one IPv4 address per entry,
    /// duplicates dropped (NAT classification needs distinct destinations).
    pub fn stun_addrs(&self) -> Vec<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        let mut out = Vec::new();
        for s in &self.stun_servers {
            let first_v4 = s
                .to_socket_addrs()
                .ok()
                .and_then(|mut it| it.find(|a| a.is_ipv4()));
            if let Some(a) = first_v4 {
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        out
    }
}
