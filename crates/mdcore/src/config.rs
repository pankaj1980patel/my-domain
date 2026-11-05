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
    /// 1. Exchange QUIC addresses through a signal; both sides dial.
    pub signaled_quic: bool,
    /// 3. STUN + UDP hole punch.
    pub punch: bool,
    /// 4. WebRTC data channel (needs the `webrtc` cargo feature).
    pub webrtc: bool,
    /// 2. Relay through the registry server's WebSocket `/relay` (brought up
    ///    before 3 and 4, which then try to upgrade it).
    pub relay: bool,
}

impl Default for ConnectRungs {
    fn default() -> Self {
        ConnectRungs {
            quic: true,
            signaled_quic: true,
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
        signaled_quic: false,
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
    /// Ask the home router to forward our QUIC port (UPnP, else PCP /
    /// NAT-PMP). `None` = don't
    /// (tests, or networks where it's unwanted).
    pub port_mapper: Option<std::sync::Arc<dyn crate::nat::portmap::PortMapper>>,
    /// Automatic LAN discovery: advertise and browse via mDNS / DNS-SD, and
    /// send a beacon when signing in. (`Engine::scan_lan` works either way.)
    pub mdns: bool,
    /// Also run mDNS on the loopback interface (tests on one machine).
    pub mdns_loopback: bool,
    /// Link keep-alive (QUIC and the connectionless-link heartbeat).
    pub keepalive: KeepAlive,
    /// While logged in, keep a WebSocket to the server open, so signals
    /// arrive over it instantly. OFF by default: it would make every device
    /// depend on the server just to be reachable, and the server should only
    /// introduce devices. Without it, the server still uses a device's
    /// WebSocket for a signal when one happens to be open (e.g. a relay
    /// session), and FCM otherwise.
    pub signal_channel: bool,
    /// While the app is in the background (`Engine::set_background`), close
    /// links that carried no messages for this long, so keep-alive pings
    /// stop waking the radio. A send re-opens a link on demand, and a push
    /// reaches a device with none. `None`: never close idle links.
    pub idle_close_after: Option<std::time::Duration>,
    /// Send screen and audio frames as binary records over QUIC (and the
    /// relay, when its server forwards binary frames) to peers that read
    /// them (`transport::media`), and say this end reads them.
    /// On by default; off behaves like an app from before (tests use it to
    /// play an older peer).
    pub bin_media: bool,
    /// Synced folders (`features::folder_sync`): this end says `sync` in
    /// its status and understands the `sync_*` messages. On by default;
    /// off behaves like an app from before (tests use it to play an older
    /// peer).
    pub folder_sync: bool,
    /// How often a synced folder is scanned and compared with the other
    /// device while there's a link to it.
    pub sync_every: std::time::Duration,
    /// Treat every synced folder's disk as one where names that differ
    /// only by letter case are the same file, without asking the disk
    /// (tests: a Linux temp folder tells them apart, a Mac's doesn't).
    /// Off: each folder's disk is asked once.
    pub sync_fold_case: bool,
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
            port_mapper: Some(std::sync::Arc::new(crate::nat::portmap::FirstOf::standard())),
            mdns: true,
            mdns_loopback: false,
            keepalive: KeepAlive::default(),
            signal_channel: false,
            idle_close_after: Some(std::time::Duration::from_secs(180)),
            bin_media: true,
            folder_sync: true,
            sync_every: std::time::Duration::from_secs(30),
            sync_fold_case: false,
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
