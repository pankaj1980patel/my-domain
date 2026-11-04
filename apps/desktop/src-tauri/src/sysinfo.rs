// More of this computer's state for its status (see `device::push_status`):
// Wi-Fi, Bluetooth, memory, CPU and boot time, read with the OS's own
// command-line tools or counters (no extra crates). Each part is left out
// when it can't be read.
//
//   "wifi":      {"on": bool, "connected": bool, "ssid"?: str, "signal"?: 0-4}
//   "bluetooth": bool (powered)
//   "ram":       {"free": bytes, "total": bytes}
//   "cpu":       0-100, percent busy since the previous push (absent on the first)
//   "boot_at":   unix seconds it started (the core turns it into `uptime_s`)

use serde_json::{json, Value};

/// Add what can be read to `status`.
pub fn extend(status: &mut Value) {
    if let Some(w) = wifi() {
        status["wifi"] = w;
    }
    if let Some(on) = bluetooth() {
        status["bluetooth"] = on.into();
    }
    if let Some((free, total)) = ram() {
        status["ram"] = json!({ "free": free, "total": total });
    }
    if let Some(pct) = cpu() {
        status["cpu"] = pct.into();
    }
    if let Some(at) = boot_at() {
        status["boot_at"] = at.into();
    }
    // How to wake this computer from the phone later (Wake-on-LAN).
    if let Some(mac) = wake_mac() {
        status["wake"] = json!({ "mac": mac });
    }
}

/// CPU time since boot, all cores together: (idle, total) clock ticks.
type Ticks = (u64, u64);

/// How busy the CPU was since the last call: the counters are sampled on
/// each status push (every minute) and the two last samples compared, so
/// nothing waits here. `None` on the first call; a call within a few
/// seconds of the last (a setting changed) repeats the last answer.
fn cpu() -> Option<u8> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<(Instant, Ticks, Option<u8>)>> = Mutex::new(None);
    let mut last = LAST.lock().ok()?;
    if let Some((at, _, pct)) = *last {
        if at.elapsed() < Duration::from_secs(5) {
            return pct;
        }
    }
    let now = cpu_ticks()?;
    let pct = last.and_then(|(_, prev, _)| cpu_busy(prev, now));
    *last = Some((Instant::now(), now, pct));
    pct
}

#[cfg_attr(not(unix), allow(dead_code))]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// --- Linux -------------------------------------------------------------------

/// NetworkManager: the radio, and the network in use with its signal.
#[cfg(target_os = "linux")]
fn wifi() -> Option<Value> {
    let radio = run("nmcli", &["radio", "wifi"])?;
    let on = radio.trim() == "enabled";
    let mut w = json!({ "on": on, "connected": false });
    if on {
        let list = run("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL", "dev", "wifi"])?;
        if let Some((ssid, pct)) = parse_nmcli_active(&list) {
            w["connected"] = true.into();
            if !ssid.is_empty() {
                w["ssid"] = ssid.into();
            }
            w["signal"] = bars(pct).into();
        }
    }
    Some(w)
}

#[cfg(target_os = "linux")]
fn bluetooth() -> Option<bool> {
    parse_bluetoothctl(&run("bluetoothctl", &["show"])?)
}

#[cfg(target_os = "linux")]
fn ram() -> Option<(u64, u64)> {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "linux")]
fn cpu_ticks() -> Option<Ticks> {
    parse_proc_stat(&std::fs::read_to_string("/proc/stat").ok()?)
}

#[cfg(target_os = "linux")]
fn boot_at() -> Option<u64> {
    parse_btime(&std::fs::read_to_string("/proc/stat").ok()?)
}

// --- macOS -------------------------------------------------------------------

/// The Wi-Fi interface's power and address; the network name when macOS
/// still tells it (newer versions hide it without Location access). No
/// signal level: macOS has no tool for it any more.
#[cfg(target_os = "macos")]
fn wifi() -> Option<Value> {
    let dev = parse_wifi_device(&run("networksetup", &["-listallhardwareports"])?)?;
    let power = run("networksetup", &["-getairportpower", &dev])?;
    let on = power.trim_end().ends_with("On");
    let connected =
        on && run("ipconfig", &["getifaddr", &dev]).is_some_and(|ip| !ip.trim().is_empty());
    let mut w = json!({ "on": on, "connected": connected });
    if connected {
        if let Some(ssid) = run("networksetup", &["-getairportnetwork", &dev])
            .as_deref()
            .and_then(|s| s.trim().strip_prefix("Current Wi-Fi Network: "))
        {
            w["ssid"] = ssid.trim().into();
        }
    }
    Some(w)
}

/// `system_profiler` takes a second or two, so its answer is kept for five
/// minutes.
#[cfg(target_os = "macos")]
fn bluetooth() -> Option<bool> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Option<bool>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().ok()?;
    if let Some((at, v)) = *cache {
        if at.elapsed() < Duration::from_secs(300) {
            return v;
        }
    }
    let v = run("system_profiler", &["SPBluetoothDataType"])
        .as_deref()
        .and_then(parse_bt_profile);
    *cache = Some((Instant::now(), v));
    v
}

/// The network card of the default route (`ip route` → `/sys/class/net`).
#[cfg(target_os = "linux")]
fn wake_mac() -> Option<String> {
    let dev = parse_default_dev(&run("ip", &["route", "show", "default"])?)?;
    let mac = std::fs::read_to_string(format!("/sys/class/net/{dev}/address")).ok()?;
    Some(mac.trim().to_string()).filter(|m| m.len() == 17)
}

/// The network card of the default route (`route` → `ifconfig`).
#[cfg(target_os = "macos")]
fn wake_mac() -> Option<String> {
    let dev = parse_route_interface(&run("route", &["-n", "get", "default"])?)?;
    parse_ether(&run("ifconfig", &[&dev])?)
}

#[cfg(target_os = "macos")]
fn ram() -> Option<(u64, u64)> {
    let total = run("sysctl", &["-n", "hw.memsize"])?.trim().parse().ok()?;
    let free = parse_vm_stat(&run("vm_stat", &[])?)?;
    Some((free.min(total), total))
}

/// The kernel's CPU load counters (`host_statistics`, the numbers `top`
/// reads): user, system, idle and nice ticks, all cores together.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn cpu_ticks() -> Option<Ticks> {
    use std::sync::OnceLock;
    extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }
    /// `HOST_CPU_LOAD_INFO`, answered with `CPU_STATE_MAX` (4) counters.
    const HOST_CPU_LOAD_INFO: i32 = 3;
    // Each mach_host_self() takes a port reference: take one, keep it.
    static HOST: OnceLock<u32> = OnceLock::new();
    let host = *HOST.get_or_init(|| unsafe { mach_host_self() });
    let mut ticks = [0u32; 4];
    let mut count = ticks.len() as u32;
    let kr = unsafe {
        host_statistics(
            host,
            HOST_CPU_LOAD_INFO,
            ticks.as_mut_ptr().cast(),
            &mut count,
        )
    };
    (kr == 0 && count == 4).then(|| mach_ticks(ticks))
}

#[cfg(target_os = "macos")]
fn boot_at() -> Option<u64> {
    parse_boottime(&run("sysctl", &["-n", "kern.boottime"])?)
}

// --- elsewhere ---------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn wifi() -> Option<Value> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn bluetooth() -> Option<bool> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn ram() -> Option<(u64, u64)> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn wake_mac() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn cpu_ticks() -> Option<Ticks> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn boot_at() -> Option<u64> {
    None
}

// --- parsing (kept apart so it's tested on every OS) ---------------------------

/// Signal percent → 0-4 bars, the scale Android reports.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn bars(pct: u32) -> u32 {
    (pct.min(100) * 5 / 101).min(4)
}

/// `nmcli -t -f ACTIVE,SSID,SIGNAL dev wifi`: the `yes:<ssid>:<signal>`
/// line (`:` in a name comes as `\:`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_nmcli_active(out: &str) -> Option<(String, u32)> {
    let line = out.lines().find_map(|l| l.strip_prefix("yes:"))?;
    let (ssid, pct) = line.rsplit_once(':')?;
    Some((ssid.replace("\\:", ":"), pct.trim().parse().ok()?))
}

/// `bluetoothctl show`: `Powered: yes|no`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_bluetoothctl(out: &str) -> Option<bool> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("Powered:"))
        .map(|v| v.trim() == "yes")
}

/// `/proc/meminfo`: (MemAvailable, MemTotal) in bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let kb = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    };
    Some((kb("MemAvailable:")? * 1024, kb("MemTotal:")? * 1024))
}

/// `networksetup -listallhardwareports`: the device after
/// `Hardware Port: Wi-Fi` (or `AirPort` on old systems).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_wifi_device(out: &str) -> Option<String> {
    let mut lines = out.lines();
    while let Some(l) = lines.next() {
        let port = l.trim().strip_prefix("Hardware Port: ").unwrap_or_default();
        if port == "Wi-Fi" || port == "AirPort" {
            return lines
                .next()?
                .trim()
                .strip_prefix("Device: ")
                .map(str::to_string);
        }
    }
    None
}

/// `system_profiler SPBluetoothDataType`: `State: On` (newer macOS) or
/// `Bluetooth Power: On` (older).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_bt_profile(out: &str) -> Option<bool> {
    out.lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("State:")
                .or_else(|| l.strip_prefix("Bluetooth Power:"))
        })
        .map(|v| v.trim() == "On")
}

/// `vm_stat`: free + inactive + speculative pages, in bytes (what can be
/// used without swapping, roughly).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_vm_stat(out: &str) -> Option<u64> {
    let page: u64 = out
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages = |key: &str| -> Option<u64> {
        out.lines()
            .find_map(|l| l.strip_prefix(key))?
            .trim()
            .trim_end_matches('.')
            .parse()
            .ok()
    };
    let n = pages("Pages free:")?
        + pages("Pages inactive:")?
        + pages("Pages speculative:").unwrap_or(0);
    Some(n * page)
}

/// Percent of the time between two samples the CPU wasn't idle, rounded.
/// `None` if no time passed or a counter went back (wrapped, or reset).
fn cpu_busy(prev: Ticks, now: Ticks) -> Option<u8> {
    let total = now.1.checked_sub(prev.1).filter(|t| *t > 0)?;
    let idle = now.0.checked_sub(prev.0)?.min(total);
    Some((((total - idle) * 100 + total / 2) / total).min(100) as u8)
}

/// `/proc/stat`'s first line, `cpu  user nice system idle iowait irq
/// softirq steal guest guest_nice`: idle is idle + iowait; the total is
/// the first eight (guest time is already counted in user).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_stat(text: &str) -> Option<Ticks> {
    let line = text.lines().find_map(|l| l.strip_prefix("cpu "))?;
    let n: Vec<u64> = line
        .split_whitespace()
        .take(8)
        .map(|v| v.parse().ok())
        .collect::<Option<_>>()?;
    let idle = n.get(3)? + n.get(4).unwrap_or(&0);
    Some((idle, n.iter().sum()))
}

/// `btime 1730700000` in `/proc/stat`: when it started, unix seconds.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_btime(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()
}

/// `host_statistics` counters in `CPU_STATE_*` order: user, system, idle,
/// nice.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mach_ticks(t: [u32; 4]) -> Ticks {
    let [user, system, idle, nice] = t.map(u64::from);
    (idle, user + system + idle + nice)
}

/// `sysctl -n kern.boottime`: `{ sec = 1730700000, usec = 1234 } Mon Nov  4 …`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_boottime(out: &str) -> Option<u64> {
    out.split("sec = ")
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
        .filter(|s| *s > 0)
}

/// `dev wlan0` in `ip route show default`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_default_dev(out: &str) -> Option<String> {
    let mut words = out.split_whitespace();
    words.find(|w| *w == "dev")?;
    words.next().map(String::from)
}

/// `interface: en0` in `route -n get default`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_route_interface(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("interface:"))
        .map(|d| d.trim().to_string())
}

/// `ether aa:bb:cc:dd:ee:ff` in `ifconfig en0`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ether(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("ether "))
        .map(|m| m.trim().to_string())
        .filter(|m| m.len() == 17)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wake_address_is_found() {
        assert_eq!(
            parse_default_dev("default via 192.168.0.1 dev wlp2s0 proto dhcp metric 600")
                .as_deref(),
            Some("wlp2s0")
        );
        assert_eq!(parse_default_dev("default via 10.0.0.1"), None);
        assert_eq!(
            parse_route_interface("   route to: default\n  interface: en0\n      flags: <UP>")
                .as_deref(),
            Some("en0")
        );
        assert_eq!(
            parse_ether("en0: flags=8863<UP>\n\tether 3c:22:fb:aa:bb:cc\n\tinet 10.0.0.2")
                .as_deref(),
            Some("3c:22:fb:aa:bb:cc")
        );
        assert_eq!(parse_ether("lo0: flags=8049<UP,LOOPBACK>"), None);
    }

    #[test]
    fn reads_nmcli() {
        let out = "no:Neighbour:40\nyes:Home\\:5G:72\nno::30\n";
        assert_eq!(parse_nmcli_active(out), Some(("Home:5G".into(), 72)));
        assert_eq!(parse_nmcli_active("no:x:1\n"), None);
        assert_eq!(
            (bars(0), bars(20), bars(50), bars(81), bars(100)),
            (0, 0, 2, 4, 4)
        );
    }

    #[test]
    fn reads_bluetooth() {
        let show = "Controller AA:BB Name\n\tName: laptop\n\tPowered: yes\n\tDiscoverable: no\n";
        assert_eq!(parse_bluetoothctl(show), Some(true));
        assert_eq!(parse_bluetoothctl("\tPowered: no\n"), Some(false));
        assert_eq!(
            parse_bluetoothctl("No default controller available\n"),
            None
        );
        assert_eq!(
            parse_bt_profile("Bluetooth:\n\n      Bluetooth Controller:\n          State: On\n"),
            Some(true)
        );
        assert_eq!(parse_bt_profile("  Bluetooth Power: Off\n"), Some(false));
    }

    #[test]
    fn reads_memory() {
        let meminfo = "MemTotal:       16000000 kB\nMemFree:          500000 kB\nMemAvailable:    8000000 kB\n";
        assert_eq!(
            parse_meminfo(meminfo),
            Some((8_000_000 * 1024, 16_000_000 * 1024))
        );
        let vm = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               10.\nPages active:                            99.\nPages inactive:                          20.\nPages speculative:                        2.\n";
        assert_eq!(parse_vm_stat(vm), Some(32 * 16384));
    }

    #[test]
    fn cpu_busy_is_the_share_of_ticks_not_idle() {
        // 1000 ticks passed, 250 of them idle: 75% busy.
        assert_eq!(cpu_busy((1_000, 4_000), (1_250, 5_000)), Some(75));
        assert_eq!(cpu_busy((0, 0), (300, 300)), Some(0));
        assert_eq!(cpu_busy((0, 0), (0, 300)), Some(100));
        // Rounded to the nearest percent.
        assert_eq!(cpu_busy((0, 0), (2, 3)), Some(33));
        assert_eq!(cpu_busy((0, 0), (1, 3)), Some(67));
        // No time passed, or a counter went back: no answer.
        assert_eq!(cpu_busy((10, 100), (10, 100)), None);
        assert_eq!(cpu_busy((10, 100), (5, 50)), None);
        assert_eq!(cpu_busy((10, 100), (5, 150)), None);
    }

    #[test]
    fn reads_cpu_counters_and_boot_time() {
        let stat = "cpu  100 20 30 800 50 0 0 0 7 0\ncpu0 50 10 15 400 25 0 0 0 0 0\nintr 1 2\nbtime 1730700000\n";
        assert_eq!(parse_proc_stat(stat), Some((850, 1_000)));
        assert_eq!(parse_btime(stat), Some(1_730_700_000));
        // Old kernels with fewer columns still count.
        assert_eq!(parse_proc_stat("cpu 1 2 3 4\n"), Some((4, 10)));
        assert_eq!(parse_proc_stat("cpu0 1 2 3 4\n"), None);
        assert_eq!(mach_ticks([10, 5, 80, 5]), (80, 100));
        assert_eq!(
            parse_boottime("{ sec = 1730700000, usec = 123456 } Mon Nov  4 09:00:00 2026\n"),
            Some(1_730_700_000)
        );
        assert_eq!(parse_boottime(""), None);
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn reads_this_machines_cpu_and_boot_time() {
        let (idle, total) = cpu_ticks().expect("CPU counters");
        assert!(total > 0 && idle <= total);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(boot_at().is_some_and(|b| b > 0 && b <= now));
        assert_eq!(cpu(), None, "no earlier sample yet");
    }

    #[test]
    fn finds_the_wifi_device() {
        let ports = "\nHardware Port: Ethernet\nDevice: en0\nEthernet Address: x\n\nHardware Port: Wi-Fi\nDevice: en1\nEthernet Address: y\n";
        assert_eq!(parse_wifi_device(ports), Some("en1".into()));
        assert_eq!(
            parse_wifi_device("Hardware Port: Thunderbolt Bridge\nDevice: bridge0\n"),
            None
        );
    }
}
