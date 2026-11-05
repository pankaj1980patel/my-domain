// "Let my other devices wake this computer" (Wake-on-LAN). While on, this
// computer's status lists its wired and Wi-Fi network cards that are up:
//
//   "wol": [{"mac": "aa:bb:…", "broadcast": "192.168.0.255",
//            "iface": "en0", "wired": true}, …]
//
// and the other devices remember it (mdcore `features::wake`), so they can
// send the magic packet while it sleeps. Off by default (`<app config
// dir>/prefs.json`, key `wake_lan`); off, the status says `wol: 1` only
// (this computer still sends wake-ups for the others) and they forget it.
//
// The cards are read with the OS's own calls, no extra crates: macOS and
// Linux `getifaddrs` (a card's address from its AF_LINK / AF_PACKET entry,
// its network's broadcast from its IPv4 one), Windows
// `GetAdaptersAddresses`. The computer itself must be set to wake for
// network access (BIOS/UEFI, the adapter's power settings, macOS "Wake for
// network access", Linux `ethtool -s <card> wol g`): Settings says how, and
// on macOS reads `pmset -g` to tell whether it's on ("womp").

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};

use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Manager};

const KEY: &str = "wake_lan";

/// The "Let my other devices wake this computer" switch.
pub struct WakePref {
    kv: Option<FileKv>,
    on: AtomicBool,
}

impl WakePref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let on = kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref() == Some("on");
        WakePref {
            kv,
            on: AtomicBool::new(on),
        }
    }

    pub fn allowed(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    pub fn set_allowed(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
                eprintln!("[wol] couldn't save the setting: {e}");
            }
        }
    }
}

/// One network card this computer can be woken through.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, specta::Type)]
pub struct WolIface {
    /// `aa:bb:cc:dd:ee:ff`.
    pub mac: String,
    /// Its IPv4 network's broadcast address.
    pub broadcast: String,
    /// The OS's name for it (`en0`, `eth0`, `Ethernet`).
    pub iface: String,
    /// Wired (Wi-Fi otherwise): waking over Wi-Fi works on few computers.
    pub wired: bool,
}

/// What Settings shows.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WakeSetting {
    pub on: bool,
    /// "macos", "windows" or "linux": which hint to show.
    pub os: &'static str,
    /// macOS: whether "Wake for network access" is on (`pmset -g` womp);
    /// `None` where it can't be told.
    pub womp: Option<bool>,
    /// The cards it would list now.
    pub ifaces: Vec<WolIface>,
}

/// The cards, into the status, while the switch is on (an empty list when
/// none could be read now: the others keep what they knew).
pub fn extend(app: &AppHandle, status: &mut serde_json::Value) {
    if !app.try_state::<WakePref>().is_some_and(|p| p.allowed()) {
        return;
    }
    status["wol"] = serde_json::to_value(interfaces()).unwrap_or_else(|_| serde_json::json!([]));
}

/// The switch and what it would share.
pub fn setting(app: &AppHandle) -> WakeSetting {
    WakeSetting {
        on: app.try_state::<WakePref>().is_some_and(|p| p.allowed()),
        os: if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        },
        womp: womp(),
        ifaces: interfaces(),
    }
}

/// The "Let my other devices wake this computer" switch, how to set this
/// computer to wake, and the cards it would share.
#[tauri::command]
#[specta::specta]
pub async fn wake_setting(app: AppHandle) -> WakeSetting {
    // Reading the cards and `pmset` takes a moment: off the main thread.
    tauri::async_runtime::spawn_blocking(move || setting(&app))
        .await
        .unwrap_or(WakeSetting {
            on: false,
            os: "linux",
            womp: None,
            ifaces: Vec::new(),
        })
}

/// Let your other devices wake this computer, or stop it (they forget how
/// at their next look at it).
#[tauri::command]
#[specta::specta]
pub async fn set_wake_setting(on: bool, app: AppHandle) -> WakeSetting {
    if let Some(pref) = app.try_state::<WakePref>() {
        pref.set_allowed(on);
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::device::refresh_status(&app);
        setting(&app)
    })
    .await
    .unwrap_or(WakeSetting {
        on,
        os: "linux",
        womp: None,
        ifaces: Vec::new(),
    })
}

/// Whether another device said it may be woken (kept across restarts).
#[tauri::command]
#[specta::specta]
pub fn can_wake(node_id: String, engine: tauri::State<crate::Eng>) -> bool {
    engine.can_wake(&node_id)
}

/// Wake another computer: its magic packet from here, and from your other
/// devices on its network.
#[tauri::command]
#[specta::specta]
pub fn wake_device(
    node_id: String,
    engine: tauri::State<crate::Eng>,
) -> Result<mdcore::model::WakeReport, mdcore::CoreError> {
    // A few UDP sends and queued messages: never blocks.
    engine.wake_device(&node_id)
}

/// `aa:bb:cc:dd:ee:ff` for a 6-byte card address (not all zeros).
fn mac_text(bytes: &[u8]) -> Option<String> {
    if bytes.len() != 6 || bytes.iter().all(|b| *b == 0) {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// The broadcast address of `ip`'s network, `prefix` bits long (Windows
/// gives the prefix, not the broadcast).
#[cfg(any(windows, test))]
fn broadcast_of(ip: Ipv4Addr, prefix: u8) -> Option<Ipv4Addr> {
    if !(8..=30).contains(&prefix) {
        return None;
    }
    let host = u32::MAX >> prefix;
    Some(Ipv4Addr::from(u32::from(ip) | host))
}

// --- macOS and Linux: getifaddrs ----------------------------------------------

/// Each card that's up with an IPv4 broadcast, its address and whether it's
/// wired; virtual ones (bridges, tunnels, containers) left out.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(unsafe_code)]
fn interfaces() -> Vec<WolIface> {
    use std::collections::HashMap;
    use std::ffi::CStr;

    let mut macs: HashMap<String, String> = HashMap::new();
    let mut nets: Vec<(String, Ipv4Addr)> = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `head` with a list we free below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Vec::new();
    }
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: a node of the list getifaddrs gave, alive until freed.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: a NUL-terminated name from the OS.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        if !physical(&name) {
            continue;
        }
        // SAFETY: ifa_addr is non-null; every sockaddr starts with its family.
        let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        if family == libc::AF_INET {
            let flags = ifa.ifa_flags as i32;
            let wanted = libc::IFF_UP | libc::IFF_RUNNING | libc::IFF_BROADCAST;
            if flags & wanted != wanted || flags & libc::IFF_LOOPBACK != 0 {
                continue;
            }
            let b = broadcast_addr(ifa);
            if b.is_null() {
                continue;
            }
            // SAFETY: non-null; checked to be an IPv4 address before use.
            let sin = unsafe { &*(b as *const libc::sockaddr_in) };
            if i32::from(sin.sin_family) == libc::AF_INET {
                let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
                if !ip.is_unspecified() && ip != Ipv4Addr::BROADCAST {
                    nets.push((name, ip));
                }
            }
        } else if let Some(mac) = link_mac(ifa, family) {
            macs.insert(name, mac);
        }
    }
    // SAFETY: the list getifaddrs gave, freed once.
    unsafe { libc::freeifaddrs(head) };

    let mut out: Vec<WolIface> = Vec::new();
    for (iface, bcast) in nets {
        let Some(mac) = macs.get(&iface) else {
            continue;
        };
        let t = WolIface {
            mac: mac.clone(),
            broadcast: bcast.to_string(),
            wired: wired(&iface),
            iface,
        };
        if !out.contains(&t) {
            out.push(t);
        }
    }
    // Wired first: it's the one that wakes on most computers.
    out.sort_by_key(|t| !t.wired);
    out
}

/// The IPv4 broadcast address of an entry (macOS keeps it in `ifa_dstaddr`).
#[cfg(target_os = "macos")]
fn broadcast_addr(ifa: &libc::ifaddrs) -> *const libc::sockaddr {
    ifa.ifa_dstaddr
}

/// The IPv4 broadcast address of an entry (Linux: `ifa_ifu`).
#[cfg(target_os = "linux")]
fn broadcast_addr(ifa: &libc::ifaddrs) -> *const libc::sockaddr {
    ifa.ifa_ifu
}

/// An Ethernet-type card's address, from its AF_LINK entry.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn link_mac(ifa: &libc::ifaddrs, family: i32) -> Option<String> {
    /// `IFT_ETHER`: Ethernet and Wi-Fi alike.
    const IFT_ETHER: u8 = 6;
    if family != libc::AF_LINK {
        return None;
    }
    // SAFETY: an AF_LINK entry's address is a sockaddr_dl.
    let dl = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_dl) };
    if dl.sdl_type != IFT_ETHER || dl.sdl_alen != 6 {
        return None;
    }
    // The address follows the name in sdl_data (which may run past its
    // declared 12 bytes, within sdl_len).
    let start = usize::from(dl.sdl_nlen);
    if 8 + start + 6 > usize::from(dl.sdl_len) {
        return None;
    }
    // SAFETY: within the sockaddr_dl's own length, checked above.
    let bytes =
        unsafe { std::slice::from_raw_parts(dl.sdl_data.as_ptr().cast::<u8>().add(start), 6) };
    mac_text(bytes)
}

/// An Ethernet-type card's address, from its AF_PACKET entry.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn link_mac(ifa: &libc::ifaddrs, family: i32) -> Option<String> {
    if family != libc::AF_PACKET {
        return None;
    }
    // SAFETY: an AF_PACKET entry's address is a sockaddr_ll.
    let ll = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_ll) };
    if ll.sll_hatype != libc::ARPHRD_ETHER || ll.sll_halen != 6 {
        return None;
    }
    mac_text(&ll.sll_addr[..6])
}

/// A real card: macOS names Ethernet and Wi-Fi ports `en…` (bridges,
/// tunnels and AirDrop's link have other names).
#[cfg(target_os = "macos")]
fn physical(name: &str) -> bool {
    name.starts_with("en")
}

/// A real card: Linux gives one a `device` (bridges, containers' and VPNs'
/// virtual ones have none).
#[cfg(target_os = "linux")]
fn physical(name: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{name}/device")).exists()
}

/// macOS: not one of the Wi-Fi ports `networksetup` lists (read once).
#[cfg(target_os = "macos")]
fn wired(name: &str) -> bool {
    use std::sync::OnceLock;
    static WIFI: OnceLock<Vec<String>> = OnceLock::new();
    let wifi = WIFI.get_or_init(|| {
        std::process::Command::new("networksetup")
            .arg("-listallhardwareports")
            .output()
            .ok()
            .map(|o| wifi_ports(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    });
    !wifi.iter().any(|w| w == name)
}

/// Linux: a Wi-Fi card has `wireless` in sysfs.
#[cfg(target_os = "linux")]
fn wired(name: &str) -> bool {
    !std::path::Path::new(&format!("/sys/class/net/{name}/wireless")).exists()
}

/// The devices of the Wi-Fi ports in `networksetup -listallhardwareports`
/// ("Hardware Port: Wi-Fi" then "Device: en0").
#[cfg(any(target_os = "macos", test))]
fn wifi_ports(out: &str) -> Vec<String> {
    let mut ports = Vec::new();
    let mut is_wifi = false;
    for line in out.lines() {
        if let Some(port) = line.strip_prefix("Hardware Port:") {
            let port = port.trim();
            is_wifi = port == "Wi-Fi" || port == "AirPort";
        } else if let Some(dev) = line.strip_prefix("Device:") {
            if is_wifi {
                ports.push(dev.trim().to_string());
            }
            is_wifi = false;
        }
    }
    ports
}

// --- Windows: GetAdaptersAddresses ----------------------------------------------

/// Each Ethernet or Wi-Fi adapter that's up with an IPv4 address: its
/// address, its network's broadcast (from the prefix) and its name.
#[cfg(windows)]
#[allow(unsafe_code)]
fn interfaces() -> Vec<WolIface> {
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    // u64s: the list must be 8-byte aligned.
    let mut buf: Vec<u64> = Vec::new();
    let mut ok = false;
    for _ in 0..3 {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: `buf` holds `size` bytes, as the call is told.
        let rc = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET.0),
                flags,
                None,
                Some(buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>()),
                &mut size,
            )
        };
        if rc == NO_ERROR.0 {
            ok = true;
            break;
        }
        if rc != ERROR_BUFFER_OVERFLOW.0 {
            break;
        }
    }
    if !ok {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    while !cur.is_null() {
        // SAFETY: a node of the list the call wrote into `buf`.
        let a = unsafe { &*cur };
        cur = a.Next;
        let wired = a.IfType == IF_TYPE_ETHERNET_CSMACD;
        if !(wired || a.IfType == IF_TYPE_IEEE80211) || a.OperStatus != IfOperStatusUp {
            continue;
        }
        let len = (a.PhysicalAddressLength as usize).min(a.PhysicalAddress.len());
        let Some(mac) = mac_text(&a.PhysicalAddress[..len]) else {
            continue;
        };
        // SAFETY: the OS's NUL-terminated name, or null.
        let iface = unsafe { a.FriendlyName.to_string() }.unwrap_or_default();
        let mut u = a.FirstUnicastAddress;
        while !u.is_null() {
            // SAFETY: a node of the adapter's address list.
            let ua = unsafe { &*u };
            u = ua.Next;
            let sa = ua.Address.lpSockaddr;
            if sa.is_null()
                || ua.Address.iSockaddrLength < std::mem::size_of::<SOCKADDR_IN>() as i32
            {
                continue;
            }
            // SAFETY: a sockaddr of at least SOCKADDR_IN's size; its family checked.
            let sin = unsafe { &*sa.cast::<SOCKADDR_IN>() };
            if sin.sin_family != AF_INET {
                continue;
            }
            // SAFETY: every member of IN_ADDR's union is the same 4 bytes.
            let ip = Ipv4Addr::from(u32::from_be(unsafe { sin.sin_addr.S_un.S_addr }));
            if ip.is_loopback() || ip.is_link_local() {
                continue;
            }
            if let Some(b) = broadcast_of(ip, ua.OnLinkPrefixLength) {
                let t = WolIface {
                    mac: mac.clone(),
                    broadcast: b.to_string(),
                    iface: iface.clone(),
                    wired,
                };
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out.sort_by_key(|t: &WolIface| !t.wired);
    out
}

// --- macOS: "Wake for network access" -------------------------------------------

/// Whether macOS will wake for network access (`pmset -g`: `womp 1`).
#[cfg(target_os = "macos")]
fn womp() -> Option<bool> {
    let out = std::process::Command::new("pmset")
        .arg("-g")
        .output()
        .ok()?;
    parse_womp(&String::from_utf8_lossy(&out.stdout))
}

/// Elsewhere it can't be told without being an administrator.
#[cfg(not(target_os = "macos"))]
fn womp() -> Option<bool> {
    None
}

/// The ` womp  1` line of `pmset -g`.
#[cfg(any(target_os = "macos", test))]
fn parse_womp(out: &str) -> Option<bool> {
    out.lines().find_map(|l| {
        let mut words = l.split_whitespace();
        (words.next() == Some("womp")).then(|| words.next() == Some("1"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_addresses_are_written_out() {
        assert_eq!(
            mac_text(&[0x3c, 0x22, 0xfb, 0x0a, 0xbb, 0xcc]).as_deref(),
            Some("3c:22:fb:0a:bb:cc")
        );
        assert_eq!(mac_text(&[0; 6]), None);
        assert_eq!(mac_text(&[1, 2, 3]), None);
    }

    #[test]
    fn a_networks_broadcast_comes_from_its_prefix() {
        let ip = Ipv4Addr::new(192, 168, 0, 176);
        assert_eq!(broadcast_of(ip, 24), Some(Ipv4Addr::new(192, 168, 0, 255)));
        assert_eq!(broadcast_of(ip, 23), Some(Ipv4Addr::new(192, 168, 1, 255)));
        assert_eq!(
            broadcast_of(Ipv4Addr::new(10, 1, 2, 3), 16),
            Some(Ipv4Addr::new(10, 1, 255, 255))
        );
        assert_eq!(broadcast_of(ip, 32), None);
        assert_eq!(broadcast_of(ip, 0), None);
    }

    #[test]
    fn reads_whether_the_mac_wakes_for_network_access() {
        let on = "System-wide power settings:\nCurrently in use:\n standby              1\n womp                 1\n networkoversleep     0\n";
        assert_eq!(parse_womp(on), Some(true));
        assert_eq!(
            parse_womp(&on.replace("womp                 1", "womp 0")),
            Some(false)
        );
        assert_eq!(parse_womp(" standby 1\n"), None);
    }

    #[test]
    fn this_computers_cards_read_as_real_ones() {
        // Whatever this machine has (CI's may have none): each one sane.
        for t in interfaces() {
            assert!(mdcore::features::wake::parse_mac(&t.mac).is_some(), "{t:?}");
            let b: Ipv4Addr = t.broadcast.parse().unwrap_or(Ipv4Addr::UNSPECIFIED);
            assert!(!b.is_unspecified() && !b.is_loopback(), "{t:?}");
            assert!(!t.iface.is_empty(), "{t:?}");
        }
    }

    #[test]
    fn finds_the_wifi_ports() {
        let out = "\nHardware Port: Ethernet\nDevice: en1\nEthernet Address: aa\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: bb\n\nHardware Port: Thunderbolt Bridge\nDevice: bridge0\n";
        assert_eq!(wifi_ports(out), vec!["en0".to_string()]);
    }
}
