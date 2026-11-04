//! Phone calls over Bluetooth on Linux and Windows: this computer as the
//! Hands-Free unit (HFP "HF", like a car kit) for a phone already paired
//! with it.
//!
//! - [`engine`]: the HFP AT dialogue, pure (no I/O), unit-tested anywhere;
//! - [`pcm`]: the SCO link's audio as plain samples, pure;
//! - [`Hfp`]: on Linux, the real thing: a BlueZ profile registered over
//!   D-Bus (`org.bluez.ProfileManager1.RegisterProfile`, HFP HF UUID,
//!   client role) whose RFCOMM channel runs the engine, plus the SCO audio
//!   sockets. On Windows: an RFCOMM socket (Winsock `AF_BTH`) to the
//!   phone's Audio Gateway service running the same engine, for call
//!   control only: Windows gives SCO (call audio) to kernel-mode profile
//!   drivers alone, so the audio stays on the phone (see `windows/mod.rs`).
//!   Elsewhere it's a stub that says calls need Linux or Windows.
//!
//! Audio is CVSD only (8 kHz): wideband mSBC needs an SBC codec, and no
//! pure-Rust one is in the build, so codec negotiation isn't advertised
//! (the engine supports it, see [`engine::Config::with_msbc`]).
//!
//! It never changes system configuration. When another program already
//! plays this role (PipeWire's `hfp_hf`, oFono), [`Hfp::problem`] says so
//! and how to turn that off ([`conflict_message`]).

pub mod engine;
pub mod pcm;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Hfp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::Hfp;

#[cfg(not(any(target_os = "linux", windows)))]
mod stub;
#[cfg(not(any(target_os = "linux", windows)))]
pub use stub::Hfp;

pub use engine::{Engine, Event};
pub use pcm::ScoPcm;

use std::sync::Arc;

/// A paired phone offering the Hands-Free Audio Gateway service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gateway {
    /// `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    pub name: String,
    /// Connected over Bluetooth at all (by anything) right now.
    pub connected: bool,
}

/// What [`Hfp`] tells the app (from its own thread, in order).
#[derive(Clone)]
pub enum HfEvent {
    /// From the phone over the hands-free link ([`Event::Ready`] means
    /// the link is up).
    At(Event),
    /// The hands-free link went.
    Disconnected,
    /// The audio link opened (its samples) or closed (`None`).
    Audio(Option<Arc<ScoPcm>>),
    /// Connecting failed; why, for a person.
    ConnectFailed(String),
    /// A standing problem for Settings changed (`None`: cleared).
    Problem(Option<String>),
}

impl std::fmt::Debug for HfEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HfEvent::At(e) => write!(f, "At({e:?})"),
            HfEvent::Disconnected => write!(f, "Disconnected"),
            HfEvent::Audio(a) => write!(f, "Audio({})", a.is_some()),
            HfEvent::ConnectFailed(m) => write!(f, "ConnectFailed({m:?})"),
            HfEvent::Problem(p) => write!(f, "Problem({p:?})"),
        }
    }
}

/// HFP Hands-Free unit.
pub const HFP_HF_UUID: &str = "0000111e-0000-1000-8000-00805f9b34fb";
/// HFP Audio Gateway (the phone).
pub const HFP_AG_UUID: &str = "0000111f-0000-1000-8000-00805f9b34fb";

/// `AA:BB:CC:DD:EE:FF` from a BlueZ device path
/// (`/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`).
pub fn address_from_path(path: &str) -> Option<String> {
    let dev = path.rsplit('/').next()?.strip_prefix("dev_")?;
    let addr = dev.replace('_', ":").to_ascii_uppercase();
    parse_address(&addr).map(|_| addr)
}

/// A Bluetooth address as the kernel's `bdaddr_t` (bytes reversed).
pub fn parse_address(addr: &str) -> Option<[u8; 6]> {
    let mut out = [0u8; 6];
    let mut parts = addr.trim().split(':');
    for slot in out.iter_mut().rev() {
        let p = parts.next()?;
        if p.len() != 2 {
            return None;
        }
        *slot = u8::from_str_radix(p, 16).ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// A Bluetooth address as Windows' `BTH_ADDR` (`0xAABBCCDDEEFF`).
pub fn address_to_u64(addr: &str) -> Option<u64> {
    let b = parse_address(addr)?;
    Some(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], 0, 0,
    ]))
}

/// `AA:BB:CC:DD:EE:FF` from Windows' `BTH_ADDR`.
pub fn address_from_u64(addr: u64) -> String {
    let b = addr.to_be_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// A paired device worth offering for calls: it lists the Hands-Free Audio
/// Gateway service, or its Class of Device says "phone" (major class 2;
/// Windows' list of a device's services isn't always filled in).
pub fn offers_calls(class_of_device: u32, has_gateway_service: bool) -> bool {
    has_gateway_service || (class_of_device >> 8) & 0x1F == 0x02
}

/// Windows: what to tell a person when the phone turns the hands-free link
/// down, usually because something on this computer already has it.
pub const WINDOWS_CONFLICT: &str = "Your phone turned this computer down as its hands-free. \
Usually that\u{2019}s because Windows already has that role for it: Phone Link\u{2019}s Calls \
(or another calling app) holds it. Turn calls off there (Phone Link \u{2192} Settings \u{2192} \
Features \u{2192} Calls) or remove the phone from that app, then press Connect here. Also check \
that your phone allows calls for this computer (Android: Bluetooth \u{2192} this computer \
\u{2192} Phone calls).";

/// How a failed connect to the phone's hands-free service should be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The phone is away or off: worth trying again later.
    Away,
    /// Something else has the hands-free link (a standing problem).
    Taken,
    /// Anything else: say it, don't retry by ourselves.
    Other,
}

/// Why connecting to the phone's hands-free service failed, from the
/// Winsock error (`WSAGetLastError`; the codes are listed in "Bluetooth
/// and connect" in the Win32 docs).
pub fn windows_connect_error(code: i32) -> (String, ConnectFailure) {
    match code {
        // PAGE_TIMEOUT / another radio-level failure: out of range or off.
        10060 | 10051 => (
            "Your phone didn\u{2019}t answer. Is its Bluetooth on, and is it near this computer?"
                .into(),
            ConnectFailure::Away,
        ),
        // The phone answered DM (channel refused), or the channel is taken.
        10064 | 10048 | 10061 => (WINDOWS_CONFLICT.into(), ConnectFailure::Taken),
        10013 => (
            "Bluetooth authentication with your phone failed. Remove the phone in Windows \
Settings \u{2192} Bluetooth & devices, pair it again, then connect here."
                .into(),
            ConnectFailure::Other,
        ),
        // WSASERVICE_NOT_FOUND / bad address: no Audio Gateway service.
        10108 | 10049 => (
            "Your phone doesn\u{2019}t offer calls over Bluetooth to this computer (no hands-free \
service found)."
                .into(),
            ConnectFailure::Other,
        ),
        10050 | 10047 | 10022 => (
            "Bluetooth isn\u{2019}t working on this computer right now. Turn it on in Windows \
Settings \u{2192} Bluetooth & devices."
                .into(),
            ConnectFailure::Other,
        ),
        // The link dropped while it was being set up.
        10054 | 10101 | 10058 | 10053 => (
            "Your phone dropped the connection. Try again.".into(),
            ConnectFailure::Away,
        ),
        other => (
            format!("Couldn\u{2019}t connect to your phone for calls (Windows error {other})."),
            ConnectFailure::Other,
        ),
    }
}

/// Who else is handling phone calls on this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    PipeWire,
    /// oFono (PulseAudio's way to do hands-free).
    Ofono,
    Unknown,
}

/// Who it probably is, from the running programs' names (`/proc/*/comm`)
/// and whether `org.ofono` is on the system bus.
pub fn likely_owner<'a>(programs: impl IntoIterator<Item = &'a str>, ofono_on_bus: bool) -> Owner {
    let mut pipewire = false;
    for p in programs {
        match p.trim() {
            "ofonod" => return Owner::Ofono,
            "wireplumber" | "pipewire" | "pipewire-media-session" => pipewire = true,
            _ => {}
        }
    }
    if ofono_on_bus {
        Owner::Ofono
    } else if pipewire {
        Owner::PipeWire
    } else {
        Owner::Unknown
    }
}

/// What to tell a person when another program has the hands-free role.
pub fn conflict_message(owner: Owner) -> String {
    match owner {
        Owner::PipeWire => "Another program (PipeWire) is handling phone calls on this computer. \
Turn its hands-free role off, then turn Calls off and on here. With WirePlumber 0.5, \
save this as ~/.config/wireplumber/wireplumber.conf.d/51-no-phone-calls.conf:\n\n\
monitor.bluez.properties = {\n  bluez5.roles = [ a2dp_sink a2dp_source bap_sink bap_source hfp_ag ]\n}\n\n\
(WirePlumber 0.4: in ~/.config/wireplumber/bluetooth.lua.d/51-no-phone-calls.lua put \
bluez_monitor.properties[\"bluez5.headset-roles\"] = \"[ hsp_ag hfp_ag ]\".) \
Then run: systemctl --user restart wireplumber pipewire"
            .into(),
        Owner::Ofono => "Another program (oFono) is handling phone calls on this computer. \
Stop it, then turn Calls off and on here: sudo systemctl disable --now ofono \
(PulseAudio's headset support for phones uses it; headphones keep working)."
            .into(),
        Owner::Unknown => "Another program is handling phone calls over Bluetooth on this \
computer (often PipeWire's or oFono's hands-free role). Turn that off, then turn Calls off \
and on here."
            .into(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            address_from_path("/org/bluez/hci0/dev_a4_C3_f0_01_02_0F").as_deref(),
            Some("A4:C3:F0:01:02:0F")
        );
        assert_eq!(address_from_path("/org/bluez/hci0"), None);
        assert_eq!(address_from_path("/org/bluez/hci0/dev_A4_C3"), None);
        assert_eq!(
            parse_address("A4:C3:F0:01:02:0F"),
            Some([0x0F, 0x02, 0x01, 0xF0, 0xC3, 0xA4])
        );
        assert_eq!(parse_address("A4:C3:F0:01:02"), None);
        assert_eq!(parse_address("A4:C3:F0:01:02:0F:00"), None);
        assert_eq!(parse_address("A4:C3:F0:01:02:G0"), None);
        assert_eq!(parse_address("A4C3:F0:01:02:0F"), None);
    }

    #[test]
    fn windows_addresses() {
        assert_eq!(address_to_u64("A4:C3:F0:01:02:0F"), Some(0xA4C3_F001_020F));
        assert_eq!(address_to_u64("a4:c3:f0:01:02:0f"), Some(0xA4C3_F001_020F));
        assert_eq!(address_to_u64("A4:C3:F0:01:02"), None);
        assert_eq!(address_from_u64(0xA4C3_F001_020F), "A4:C3:F0:01:02:0F");
        assert_eq!(address_from_u64(1), "00:00:00:00:00:01");
        let a = "00:1A:7D:DA:71:13";
        assert_eq!(address_to_u64(a).map(address_from_u64).as_deref(), Some(a));
    }

    #[test]
    fn which_devices_offer_calls() {
        // A smartphone: major class 2 (phone), minor 3, plus service bits.
        assert!(offers_calls(0x5A_020C, false));
        // Headphones (major 4) and a computer (major 1) don't...
        assert!(!offers_calls(0x24_0404, false));
        assert!(!offers_calls(0x10_010C, false));
        // ...unless they list the Audio Gateway service.
        assert!(offers_calls(0x10_010C, true));
    }

    #[test]
    fn windows_errors() {
        let (m, how) = windows_connect_error(10060);
        assert!(m.contains("didn\u{2019}t answer"));
        assert_eq!(how, ConnectFailure::Away);
        assert_eq!(
            windows_connect_error(10064),
            (WINDOWS_CONFLICT.to_string(), ConnectFailure::Taken)
        );
        assert!(WINDOWS_CONFLICT.contains("Phone Link"));
        assert_eq!(windows_connect_error(10108).1, ConnectFailure::Other);
        assert!(windows_connect_error(12345).0.contains("12345"));
    }

    #[test]
    fn owners_and_messages() {
        assert_eq!(
            likely_owner(["bash", "wireplumber", "pipewire"], false),
            Owner::PipeWire
        );
        assert_eq!(likely_owner(["pipewire", "ofonod"], false), Owner::Ofono);
        assert_eq!(likely_owner(["pulseaudio"], true), Owner::Ofono);
        assert_eq!(likely_owner(["pulseaudio"], false), Owner::Unknown);
        let pw = conflict_message(Owner::PipeWire);
        assert!(pw.contains("bluez5.roles"));
        assert!(!pw.contains("hfp_hf"));
        assert!(conflict_message(Owner::Ofono).contains("oFono"));
    }
}
