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
}

impl Default for ConnectRungs {
    fn default() -> Self {
        ConnectRungs {
            lan_udp: true,
            direct_ws: true,
            signaled_ws: true,
            punch: true,
            webrtc: true,
        }
    }
}

impl ConnectRungs {
    /// Every rung off; switch on the ones you want.
    pub fn none() -> Self {
        ConnectRungs {
            lan_udp: false,
            direct_ws: false,
            signaled_ws: false,
            punch: false,
            webrtc: false,
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
}

impl Default for CoreConfig {
    fn default() -> Self {
        CoreConfig {
            stun_servers: vec!["stun.l.google.com:19302".into()],
            rungs: ConnectRungs::default(),
        }
    }
}

impl CoreConfig {
    /// The first configured STUN server that resolves to an IPv4 address.
    pub fn stun_server(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        self.stun_servers
            .iter()
            .filter_map(|s| s.to_socket_addrs().ok())
            .flatten()
            .find(|a| a.is_ipv4())
    }
}
