//! Phone calls over Bluetooth on Linux: this computer as the Hands-Free
//! unit (HFP "HF", like a car kit) for a phone already paired with it.
//!
//! - [`engine`]: the HFP AT dialogue, pure (no I/O), unit-tested anywhere;
//! - [`pcm`]: the SCO link's audio as plain samples, pure;
//! - [`Hfp`]: on Linux, the real thing: a BlueZ profile registered over
//!   D-Bus (`org.bluez.ProfileManager1.RegisterProfile`, HFP HF UUID,
//!   client role) whose RFCOMM channel runs the engine, plus the SCO audio
//!   sockets. Elsewhere it's a stub that says calls need Linux.
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

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
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
