//! The platform abstraction. Each client (desktop, android, …) implements
//! `Platform`; the Engine is generic over it. This hides every real divergence
//! between platforms behind one trait so the networking/feature code is shared.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};

use crate::capabilities::{Clipboard, Notifier};

/// How to choose the multicast interface(s) and the /24 sweep base for LAN
/// discovery.
#[derive(Clone)]
pub enum IfaceMode {
    /// Desktop: enumerate all non-loopback IPv4 interfaces (via `if_addrs`) and
    /// join multicast on each.
    All { v4_addrs: Vec<Ipv4Addr> },
    /// Android: pin to one Wi-Fi interface IP (`set_multicast_if_v4`) because the
    /// default route is cellular.
    Single(Ipv4Addr),
}

/// Best-effort local IPv4 via the "connect to 8.8.8.8" trick. Shared default.
pub fn detect_local_ip() -> String {
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            Ok(s.local_addr()?.ip().to_string())
        })
        .unwrap_or_else(|_| "0.0.0.0".to_string())
}

/// This device's global IPv6 address, if it has one: the source address the OS
/// would pick to reach a public IPv6 host (a UDP "connect" sends nothing).
/// Only global unicast (2000::/3) counts: link-local and ULA addresses can't
/// be reached from another network.
pub fn detect_local_ipv6() -> Option<String> {
    let s = UdpSocket::bind("[::]:0").ok()?;
    s.connect("[2001:4860:4860::8888]:80").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V6(v6) if is_global_v6(&v6) => Some(v6.to_string()),
        _ => None,
    }
}

/// Global unicast IPv6 (2000::/3), the only kind worth publishing.
pub fn is_global_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xe000) == 0x2000
}

/// Everything the Engine needs from its host platform. Capability accessors
/// return `None` when the platform doesn't support that feature.
pub trait Platform: Send + Sync + 'static {
    // --- stable identity ---
    /// Stable, hardware-derived device id (machine-uid / ANDROID_ID). Empty
    /// string => the Engine falls back to a random UUID.
    fn device_id(&self) -> String;
    /// Human-readable device name (hostname / user-supplied).
    fn device_name(&self) -> String;

    /// Platform tag for the registry: "desktop" | "android" | "ios" | "mac".
    fn platform_kind(&self) -> &'static str {
        "unknown"
    }

    /// This device can share its mouse and keyboard with your other
    /// computers, and be controlled by theirs ("Share mouse and keyboard",
    /// `features::kvm`): a computer. Default: no (a phone).
    fn shares_input(&self) -> bool {
        false
    }

    // --- network ---
    /// Interface strategy for multicast + LAN sweep. Re-queried on network change.
    fn iface_mode(&self) -> IfaceMode;
    /// Current best local IPv4 (string). Default uses the 8.8.8.8 trick.
    fn local_ip(&self) -> String {
        detect_local_ip()
    }
    /// Current global IPv6 address, if any (see `detect_local_ipv6`).
    fn local_ipv6(&self) -> Option<String> {
        detect_local_ipv6()
    }

    // --- session persistence (server_url, username, QUIC port, peers) ---
    fn kv_get(&self, _key: &str) -> Option<String> {
        None
    }
    fn kv_set(&self, _key: &str, _value: &str) {}
    /// Where a feature keeps a file of its own too big for `kv` (e.g.
    /// `notes.json`), beside the kv store. `None`: it goes in `kv`.
    fn data_file(&self, _name: &str) -> Option<std::path::PathBuf> {
        None
    }

    // --- secrets (session token, derived device key) ---
    /// Read a secret from the OS secure store (Keychain, Android Keystore,
    /// Secret Service). The default keeps nothing, so a restart signs out.
    fn secret_get(&self, _key: &str) -> Option<String> {
        None
    }
    /// Store (`Some`) or delete (`None`) a secret.
    fn secret_set(&self, _key: &str, _value: Option<&str>) {}

    // --- push (roadmap: FCM) ---
    fn fcm_token(&self) -> Option<String> {
        None
    }

    // --- capabilities (absent => feature degrades gracefully) ---
    fn clipboard(&self) -> Option<&dyn Clipboard> {
        None
    }
    fn notifier(&self) -> Option<&dyn Notifier> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_global_unicast_ipv6_counts() {
        let g = |s: &str| is_global_v6(&s.parse().unwrap());
        assert!(g("2409:4040:1234::1"));
        assert!(g("2001:db8::1")); // documentation range, but 2000::/3
        assert!(!g("fe80::1"), "link-local");
        assert!(!g("fd12::1"), "unique-local");
        assert!(!g("::1"), "loopback");
        assert!(!g("::ffff:1.2.3.4"), "v4-mapped");
    }
}
