//! Signaling protocol exchanged between a user's devices via the server's
//! FCM-backed `/signal` relay. These messages coordinate connection setup when
//! a direct path isn't already available; the resulting data channel still uses
//! the existing E2EE transports (the server never sees message content or keys).
//!
//! `sid` correlates a negotiation; punch timing is relative (`start_in_ms`) to
//! avoid wall-clock skew.

use serde::{Deserialize, Serialize};

/// Delivers a signal to another of the user's devices. The default (no
/// signaler set) is the registry server's `/signal` relay over HTTP, which the
/// server forwards as an FCM push. `Engine::set_signaler` swaps in another
/// path: an in-memory hub in tests, a WebSocket relay later.
pub trait Signaler: Send + Sync + 'static {
    fn send(&self, from: &str, to: &str, signal: &Signal) -> crate::Result<()>;
}

/// An ICE-style connection candidate.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Candidate {
    pub ip: String,
    pub port: u16,
    /// "host" (LAN) | "srflx" (STUN reflexive)
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Signal {
    /// "open your WS server and tell me when it's reachable".
    StartWs {
        sid: String,
    },
    /// Sent after `ws_open` is live; the initiator dials this endpoint.
    WsReady {
        sid: String,
        ip: String,
        ws_port: u16,
    },
    /// Couldn't open a reachable WS (e.g. also firewalled).
    WsUnavailable {
        sid: String,
        reason: String,
    },

    /// UDP hole punch: exchange candidates and a relative start time.
    PunchOffer {
        sid: String,
        candidates: Vec<Candidate>,
        start_in_ms: u64,
    },
    PunchAnswer {
        sid: String,
        candidates: Vec<Candidate>,
        start_in_ms: u64,
    },
    /// Both sides saw each other's probes; promote to the data channel.
    PunchReady {
        sid: String,
    },
    /// Punch failed → escalate to TURN.
    PunchFailed {
        sid: String,
        reason: String,
    },

    /// "Here are my QUIC addresses; tell me yours." We dial the answered
    /// addresses at once; the peer dials ours if we haven't connected within
    /// a second (we can't reach it but it can reach us). Carries fresh
    /// addresses too (a peer that changed networks, or a registry without
    /// QUIC ports).
    QuicOffer {
        sid: String,
        /// `ip:port` socket addresses of our QUIC endpoint (IPv6, router
        /// mapping, LAN IPv4).
        addrs: Vec<String>,
    },
    QuicAnswer {
        sid: String,
        addrs: Vec<String>,
    },

    /// "Join the server relay and say hello to me": the last-resort path when
    /// nothing direct works (see transport/relay.rs). The reply is the peer's
    /// `Hello` arriving through the relay, not another signal.
    RelayOffer {
        sid: String,
    },

    /// WebRTC data-channel offer/answer. The `sdp` is a serialized str0m
    /// SdpOffer/SdpAnswer that already embeds the host + reflexive candidates
    /// (no trickle ICE), so the exchange is one message each way.
    SdpOffer {
        sid: String,
        sdp: String,
    },
    SdpAnswer {
        sid: String,
        sdp: String,
    },

    /// Abort / teardown.
    Bye {
        sid: String,
    },
}
