//! How good is the link to a peer? RTT and packet loss, measured by QUIC on
//! every connection, turned into a plain-language rating for the UI
//! ("Excellent", "Good", "Weak connection").

use serde::Serialize;

use super::TransportKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum Quality {
    Excellent,
    Good,
    Weak,
    /// Not measured on this kind of link.
    Unknown,
}

impl Quality {
    /// What the UI shows.
    pub fn label(self) -> &'static str {
        match self {
            Quality::Excellent => "Excellent",
            Quality::Good => "Good",
            Quality::Weak => "Weak connection",
            Quality::Unknown => "Connected",
        }
    }
}

/// One live link, as the UI and the path-choosing logic see it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LinkStats {
    pub node_id: String,
    pub transport: TransportKind,
    /// Peer-to-peer (not through the server's relay).
    pub direct: bool,
    pub rtt_ms: Option<u32>,
    /// Packets lost, as a percentage of packets sent (QUIC links).
    pub loss_pct: Option<f32>,
    pub quality: Quality,
    pub label: &'static str,
}

/// Rate a link. Thresholds: under 60 ms and 1 % loss feels instant;
/// under 250 ms and 5 % is fine for chat, files and remote control; worse
/// is noticeable. Relayed links are capped at Good (every byte crosses the
/// server, and that's worth knowing).
pub fn rate(rtt_ms: Option<u32>, loss_pct: Option<f32>, transport: TransportKind) -> Quality {
    let Some(rtt) = rtt_ms else {
        return Quality::Unknown;
    };
    let loss = loss_pct.unwrap_or(0.0);
    let q = if rtt <= 60 && loss < 1.0 {
        Quality::Excellent
    } else if rtt <= 250 && loss < 5.0 {
        Quality::Good
    } else {
        Quality::Weak
    };
    if transport.is_fallback() && q == Quality::Excellent {
        Quality::Good
    } else {
        q
    }
}

/// Loss as a percentage of sent packets (`None` before anything was sent).
pub fn loss_pct(lost: u64, sent: u64) -> Option<f32> {
    (sent > 0).then(|| (lost as f64 * 100.0 / sent as f64) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use TransportKind::{Quic, Relay};

    #[test]
    fn ratings_follow_rtt_and_loss() {
        assert_eq!(rate(Some(8), Some(0.0), Quic), Quality::Excellent);
        assert_eq!(rate(Some(120), Some(0.5), Quic), Quality::Good);
        assert_eq!(
            rate(Some(40), Some(3.0), Quic),
            Quality::Good,
            "loss alone drops it"
        );
        assert_eq!(rate(Some(400), Some(0.0), Quic), Quality::Weak);
        assert_eq!(rate(Some(30), Some(9.0), Quic), Quality::Weak);
        assert_eq!(rate(None, None, Quic), Quality::Unknown);
    }

    #[test]
    fn relayed_links_are_never_excellent() {
        assert_eq!(rate(Some(10), Some(0.0), Relay), Quality::Good);
        assert_eq!(rate(Some(400), Some(0.0), Relay), Quality::Weak);
    }

    #[test]
    fn loss_is_a_percentage_of_sent_packets() {
        assert_eq!(loss_pct(0, 0), None);
        assert_eq!(loss_pct(1, 200), Some(0.5));
        assert_eq!(loss_pct(5, 100), Some(5.0));
    }
}
