// The phone's hotspot for big files (mdcore `features::wifi_direct`,
// method "hotspot"): a phone near this computer, not on the same network
// (or with no network at all: the offer can come over Bluetooth), starts
// its local-only hotspot; this computer leaves its Wi-Fi, joins it, the
// file goes over a QUIC link to the phone there (same device-key check),
// and then this computer goes back to the Wi-Fi it was on.
//
//  - "Use the phone’s hotspot for big files" in Settings: "ask" each time
//    (the default), "always", or "never" (`<app config dir>/prefs.json`,
//    key `phone_hotspot`, in the settings backup). "ask" shows "Join
//    <phone>’s hotspot for this transfer? This computer leaves <Wi-Fi>
//    until it’s done." in the window (`hotspot-ask`), answered with
//    `hotspot_consent`;
//  - joining: macOS `networksetup` (no CoreWLAN binding in the tree), Linux
//    `nmcli`, Windows `netsh wlan` with a profile made for it and deleted
//    after. The name and passphrase go as separate arguments (never through
//    a shell), and are never logged or kept;
//  - going back: the network it was on is remembered (macOS: its name, the
//    Keychain has its password; Linux: the NetworkManager connection;
//    Windows: the profile) and rejoined after the transfer, and after
//    every failure on the way (`Plan`);
//  - nothing checks for internet: a hotspot without it is the point.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

const KEY: &str = "phone_hotspot";
/// How long joining may take before it's given up (the core waits about
/// twice its setup time).
const JOIN_WAIT: Duration = Duration::from_secs(20);

/// "Use the phone’s hotspot for big files".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Ask,
    Always,
    Never,
}

impl Mode {
    pub fn parse(saved: Option<&str>) -> Mode {
        match saved {
            Some("always") => Mode::Always,
            Some("never") => Mode::Never,
            _ => Mode::Ask,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Mode::Ask => "ask",
            Mode::Always => "always",
            Mode::Never => "never",
        }
    }
}

pub struct HotspotPref {
    kv: Option<FileKv>,
    mode: Mutex<Mode>,
}

impl HotspotPref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let mode = Mode::parse(kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref());
        HotspotPref {
            kv,
            mode: Mutex::new(mode),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode.lock().map(|m| *m).unwrap_or(Mode::Ask)
    }

    pub fn set(&self, mode: Mode) {
        if let Ok(mut m) = self.mode.lock() {
            *m = mode;
        }
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(KEY, mode.as_str()) {
                eprintln!("[hotspot] couldn't save the setting: {e}");
            }
        }
    }
}

pub fn mode(app: &AppHandle) -> Mode {
    app.try_state::<HotspotPref>()
        .map(|p| p.mode())
        .unwrap_or(Mode::Ask)
}

// --- going there and back: the rules -----------------------------------------

/// Where joining a hotspot is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Leaving the Wi-Fi it was on (`prev`) for the hotspot.
    Joining {
        prev: Option<String>,
    },
    Joined {
        prev: Option<String>,
    },
}

/// What happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Join,
    Joined,
    Failed,
    Leave,
}

/// What to do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    Nothing,
    /// Join the hotspot.
    Connect,
    /// Tell the core where the phone is.
    Report,
    /// Go back to the Wi-Fi it was on (none: it wasn't on any; just leave
    /// the hotspot).
    Restore(Option<String>),
}

/// The next phase and what to do. Every failure and every leave after a
/// join started goes back to the previous Wi-Fi.
pub fn step(phase: &Phase, s: Step, prev_now: Option<String>) -> (Phase, Act) {
    match (phase, s) {
        (Phase::Idle, Step::Join) => (Phase::Joining { prev: prev_now }, Act::Connect),
        (Phase::Joining { prev }, Step::Joined) => {
            (Phase::Joined { prev: prev.clone() }, Act::Report)
        }
        (Phase::Joining { prev } | Phase::Joined { prev }, Step::Failed | Step::Leave) => {
            (Phase::Idle, Act::Restore(prev.clone()))
        }
        (p, _) => (p.clone(), Act::Nothing),
    }
}

/// sid -> phase (one at a time in practice).
static JOBS: Mutex<Vec<(String, Phase)>> = Mutex::new(Vec::new());

fn phase_of(sid: &str) -> Phase {
    JOBS.lock()
        .ok()
        .and_then(|j| j.iter().find(|(s, _)| s == sid).map(|(_, p)| p.clone()))
        .unwrap_or(Phase::Idle)
}

fn set_phase(sid: &str, p: Phase) {
    if let Ok(mut j) = JOBS.lock() {
        j.retain(|(s, _)| s != sid);
        if p != Phase::Idle {
            j.push((sid.to_string(), p));
        }
    }
}

fn advance(sid: &str, s: Step, prev_now: Option<String>) -> Act {
    let (next, act) = step(&phase_of(sid), s, prev_now);
    set_phase(sid, next);
    act
}

// --- the core's requests -------------------------------------------------------

/// What the window shows for "ask".
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct HotspotAsk {
    pub sid: String,
    pub node_id: String,
    /// The Wi-Fi this computer would leave (none: not on one).
    pub wifi: Option<String>,
}

/// The core asks this computer's user (op "consent").
pub fn on_consent(app: &AppHandle, sid: &str, node: &str) {
    let Some(engine) = app.try_state::<crate::Eng>() else {
        return;
    };
    match mode(app) {
        Mode::Always => engine.wifi_direct_consent(sid, true),
        Mode::Never => engine.wifi_direct_consent(sid, false),
        Mode::Ask => {
            let ask = HotspotAsk {
                sid: sid.to_string(),
                node_id: node.to_string(),
                wifi: wifi::current().map(|c| c.ssid),
            };
            crate::tray::show_window(app);
            let _ = app.emit("hotspot-ask", ask);
        }
    }
}

/// Join the phone's hotspot (op "join"), off the core's thread.
pub fn on_join(app: &AppHandle, sid: &str, ssid: &str, pass: &str, host_ip: Option<&str>) {
    let (app, sid, ssid, pass) = (
        app.clone(),
        sid.to_string(),
        ssid.to_string(),
        pass.to_string(),
    );
    let hint = host_ip.map(str::to_string);
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<crate::Eng>() else {
            return;
        };
        let cur = wifi::current();
        if advance(&sid, Step::Join, cur.as_ref().map(|c| c.prev.clone())) != Act::Connect {
            return;
        }
        let Some(iface) = cur.map(|c| c.iface).or_else(wifi::iface) else {
            restore(&sid, Step::Failed);
            return engine.wifi_direct_failed(&sid, "no Wi-Fi");
        };
        let res = wifi::join(&iface, &ssid, &pass).and_then(|()| {
            let deadline = Instant::now() + JOIN_WAIT;
            loop {
                if let Some(ip) = hint.clone().or_else(|| wifi::gateway(&iface)) {
                    return Ok(ip);
                }
                if Instant::now() >= deadline {
                    return Err("no address on the hotspot".to_string());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        match res {
            Ok(ip) if advance(&sid, Step::Joined, None) == Act::Report => {
                engine.wifi_direct_joined(&sid, &ip)
            }
            Ok(_) => {}
            Err(e) => {
                restore(&sid, Step::Failed);
                engine.wifi_direct_failed(&sid, &e);
            }
        }
        wifi::cleanup(&ssid);
    });
}

/// Done (op "leave"): back to the Wi-Fi it was on.
pub fn on_leave(sid: &str) {
    let sid = sid.to_string();
    std::thread::spawn(move || restore(&sid, Step::Leave));
}

fn restore(sid: &str, s: Step) {
    if let Act::Restore(prev) = advance(sid, s, None) {
        let iface = wifi::iface();
        if let (Some(iface), Some(prev)) = (iface.as_deref(), prev.as_deref()) {
            if let Err(e) = wifi::rejoin(iface, prev) {
                eprintln!("[hotspot] couldn't go back to the previous Wi-Fi: {e}");
            }
        } else if let Some(iface) = iface.as_deref() {
            wifi::disconnect(iface);
        }
    }
}

// --- commands --------------------------------------------------------------

/// The user's answer to `hotspot-ask`.
#[tauri::command]
#[specta::specta]
pub fn hotspot_consent(sid: String, yes: bool, engine: tauri::State<crate::Eng>) {
    engine.wifi_direct_consent(&sid, yes);
}

/// This computer can join a phone's hotspot (it has Wi-Fi).
pub fn supported() -> bool {
    wifi::iface().is_some()
}

// --- the OS ------------------------------------------------------------------

/// Joining and leaving Wi-Fi networks with the OS's own tools.
pub mod wifi {
    use std::process::Command;

    /// The Wi-Fi this computer is on.
    pub struct Current {
        pub iface: String,
        pub ssid: String,
        /// What `rejoin` takes to go back (macOS: the name; Linux: the
        /// NetworkManager connection; Windows: the profile).
        pub prev: String,
    }

    fn run(cmd: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The temporary name a joined hotspot is kept under (Linux, Windows).
    pub const PROFILE: &str = "my-domain-hotspot";

    /// `Hardware Port: Wi-Fi` → its `Device:` (macOS `networksetup
    /// -listallhardwareports`).
    pub fn mac_iface(text: &str) -> Option<String> {
        let mut wifi = false;
        for line in text.lines() {
            if let Some(port) = line.strip_prefix("Hardware Port: ") {
                wifi = port.trim() == "Wi-Fi" || port.trim() == "AirPort";
            } else if let Some(dev) = line.strip_prefix("Device: ") {
                if wifi {
                    return Some(dev.trim().to_string());
                }
            }
        }
        None
    }

    /// `Current Wi-Fi Network: Home` → `Home`.
    pub fn mac_ssid(text: &str) -> Option<String> {
        text.trim()
            .strip_prefix("Current Wi-Fi Network: ")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Split one `nmcli -t` line on its unescaped colons.
    pub fn nm_fields(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut esc = false;
        for c in line.chars() {
            match (esc, c) {
                (true, c) => {
                    cur.push(c);
                    esc = false;
                }
                (false, '\\') => esc = true,
                (false, ':') => out.push(std::mem::take(&mut cur)),
                (false, c) => cur.push(c),
            }
        }
        out.push(cur);
        out
    }

    /// `    Name : Wi-Fi` → ("Name", "Wi-Fi") from `netsh wlan show
    /// interfaces` (English names).
    pub fn netsh_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key)
                .then(|| v.trim())
                .filter(|v| !v.is_empty())
        })
    }

    /// A WPA2 profile for `ssid` (`netsh wlan add profile`), with XML
    /// special characters escaped.
    pub fn netsh_profile(ssid: &str, pass: &str) -> String {
        let x = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;")
        };
        format!(
            r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
<name>{name}</name>
<SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig>
<connectionType>ESS</connectionType>
<connectionMode>manual</connectionMode>
<MSM><security>
<authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>
<sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{pass}</keyMaterial></sharedKey>
</security></MSM>
</WLANProfile>"#,
            name = super::wifi::PROFILE,
            ssid = x(ssid),
            pass = x(pass),
        )
    }

    /// The Wi-Fi card.
    pub fn iface() -> Option<String> {
        if cfg!(target_os = "macos") {
            mac_iface(&run("networksetup", &["-listallhardwareports"])?)
        } else if cfg!(windows) {
            let t = run("netsh", &["wlan", "show", "interfaces"])?;
            netsh_field(&t, "Name").map(str::to_string)
        } else {
            let t = run("nmcli", &["-t", "-f", "DEVICE,TYPE", "device"])?;
            t.lines().find_map(|l| {
                let f = nm_fields(l);
                (f.get(1).map(String::as_str) == Some("wifi")).then(|| f[0].clone())
            })
        }
    }

    /// The Wi-Fi it's on now.
    pub fn current() -> Option<Current> {
        let iface = iface()?;
        if cfg!(target_os = "macos") {
            let ssid = mac_ssid(&run("networksetup", &["-getairportnetwork", &iface])?)?;
            Some(Current {
                iface,
                prev: ssid.clone(),
                ssid,
            })
        } else if cfg!(windows) {
            let t = run("netsh", &["wlan", "show", "interfaces"])?;
            let ssid = netsh_field(&t, "SSID")?.to_string();
            let prev = netsh_field(&t, "Profile").unwrap_or(&ssid).to_string();
            Some(Current { iface, ssid, prev })
        } else {
            let t = run(
                "nmcli",
                &[
                    "-t",
                    "-f",
                    "NAME,TYPE,DEVICE",
                    "connection",
                    "show",
                    "--active",
                ],
            )?;
            let name = t.lines().find_map(|l| {
                let f = nm_fields(l);
                (f.get(2) == Some(&iface)).then(|| f[0].clone())
            })?;
            Some(Current {
                iface,
                ssid: name.clone(),
                prev: name,
            })
        }
    }

    /// Join `ssid` with `pass` (separate arguments, no shell).
    pub fn join(iface: &str, ssid: &str, pass: &str) -> Result<(), String> {
        let failed = || "couldn't join the phone's hotspot".to_string();
        if cfg!(target_os = "macos") {
            let out = Command::new("networksetup")
                .args(["-setairportnetwork", iface, ssid, pass])
                .output()
                .map_err(|_| failed())?;
            // It says why on stdout, and may still exit 0.
            let said = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if !out.status.success() || said.contains("could not") || said.contains("failed") {
                return Err(failed());
            }
            Ok(())
        } else if cfg!(windows) {
            let path = std::env::temp_dir().join(format!("{PROFILE}-{}.xml", std::process::id()));
            std::fs::write(&path, netsh_profile(ssid, pass)).map_err(|_| failed())?;
            let p = format!("filename={}", path.display());
            let i = format!("interface={iface}");
            let added = run("netsh", &["wlan", "add", "profile", &p, &i]);
            let _ = std::fs::remove_file(&path);
            added.ok_or_else(failed)?;
            let n = format!("name={PROFILE}");
            run("netsh", &["wlan", "connect", &n, &i])
                .map(|_| ())
                .ok_or_else(failed)
        } else {
            run(
                "nmcli",
                &[
                    "device", "wifi", "connect", ssid, "password", pass, "ifname", iface, "name",
                    PROFILE,
                ],
            )
            .map(|_| ())
            .ok_or_else(failed)
        }
    }

    /// Go back to `prev` (see `Current::prev`).
    pub fn rejoin(iface: &str, prev: &str) -> Result<(), String> {
        let ok = if cfg!(target_os = "macos") {
            // The Keychain has its password.
            run("networksetup", &["-setairportnetwork", iface, prev]).is_some()
        } else if cfg!(windows) {
            let (n, i) = (format!("name={prev}"), format!("interface={iface}"));
            run("netsh", &["wlan", "connect", &n, &i]).is_some()
        } else {
            run("nmcli", &["connection", "up", "id", prev]).is_some()
        };
        ok.then_some(()).ok_or_else(|| "rejoin failed".to_string())
    }

    /// Leave the hotspot when there's nothing to go back to.
    pub fn disconnect(iface: &str) {
        if cfg!(target_os = "macos") {
            let _ = run("networksetup", &["-setairportpower", iface, "off"]);
            let _ = run("networksetup", &["-setairportpower", iface, "on"]);
        } else if cfg!(windows) {
            let i = format!("interface={iface}");
            let _ = run("netsh", &["wlan", "disconnect", &i]);
        } else {
            let _ = run("nmcli", &["device", "disconnect", iface]);
        }
    }

    /// Forget what joining left behind (the hotspot's saved network).
    pub fn cleanup(ssid: &str) {
        if cfg!(target_os = "macos") {
            if let Some(iface) = iface() {
                let _ = run(
                    "networksetup",
                    &["-removepreferredwirelessnetwork", &iface, ssid],
                );
            }
        } else if cfg!(windows) {
            let n = format!("name={PROFILE}");
            let _ = run("netsh", &["wlan", "delete", "profile", &n]);
        } else {
            let _ = run("nmcli", &["connection", "delete", "id", PROFILE]);
        }
    }

    /// The hotspot's gateway (the phone), once there's an address.
    pub fn gateway(iface: &str) -> Option<String> {
        let text = if cfg!(target_os = "macos") {
            run("ipconfig", &["getoption", iface, "router"])?
        } else if cfg!(windows) {
            return None; // the phone says its address
        } else {
            run("nmcli", &["-g", "IP4.GATEWAY", "device", "show", iface])?
        };
        let ip = text.trim().to_string();
        ip.parse::<std::net::IpAddr>().ok().map(|_| ip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotspot_mode_defaults_to_ask() {
        assert_eq!(Mode::parse(None), Mode::Ask);
        assert_eq!(Mode::parse(Some("always")), Mode::Always);
        assert_eq!(Mode::parse(Some("never")), Mode::Never);
        assert_eq!(Mode::parse(Some("junk")), Mode::Ask);
    }

    #[test]
    fn hotspot_always_goes_back_to_the_previous_wifi() {
        let home = Some("Home".to_string());
        // Join, joined, done: back home.
        let (p, a) = step(&Phase::Idle, Step::Join, home.clone());
        assert_eq!(a, Act::Connect);
        let (p, a) = step(&p, Step::Joined, None);
        assert_eq!(a, Act::Report);
        let (p, a) = step(&p, Step::Leave, None);
        assert_eq!((p, a), (Phase::Idle, Act::Restore(home.clone())));
        // Failing while joining, failing once joined, the core giving up
        // while joining: back home every time.
        for path in [
            vec![Step::Join, Step::Failed],
            vec![Step::Join, Step::Joined, Step::Failed],
            vec![Step::Join, Step::Leave],
        ] {
            let mut p = Phase::Idle;
            let mut last = Act::Nothing;
            for s in path {
                (p, last) = step(&p, s, home.clone());
            }
            assert_eq!(last, Act::Restore(home.clone()));
            assert_eq!(p, Phase::Idle);
        }
        // Not on any Wi-Fi before: just leave the hotspot.
        let (p, _) = step(&Phase::Idle, Step::Join, None);
        assert_eq!(step(&p, Step::Failed, None).1, Act::Restore(None));
        // A leave with nothing joined, or a late "joined": nothing to do.
        assert_eq!(step(&Phase::Idle, Step::Leave, None).1, Act::Nothing);
        assert_eq!(step(&Phase::Idle, Step::Joined, None).1, Act::Nothing);
    }

    #[test]
    fn hotspot_reads_what_the_os_says() {
        let ports = "Hardware Port: Ethernet\nDevice: en1\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: x\n";
        assert_eq!(wifi::mac_iface(ports).as_deref(), Some("en0"));
        assert_eq!(
            wifi::mac_iface("Hardware Port: Ethernet\nDevice: en1\n"),
            None
        );
        assert_eq!(
            wifi::mac_ssid("Current Wi-Fi Network: My Home\n").as_deref(),
            Some("My Home")
        );
        assert_eq!(
            wifi::mac_ssid("You are not associated with an AirPort network.\n"),
            None
        );
        assert_eq!(
            wifi::nm_fields(r"Cafe\: 2G:802-11-wireless:wlan0"),
            vec!["Cafe: 2G", "802-11-wireless", "wlan0"]
        );
        let netsh = "    Name                   : Wi-Fi\n    SSID                   : Home\n    BSSID                  : aa:bb\n    Profile                : Home 5G\n";
        assert_eq!(wifi::netsh_field(netsh, "Name"), Some("Wi-Fi"));
        assert_eq!(wifi::netsh_field(netsh, "SSID"), Some("Home"));
        assert_eq!(wifi::netsh_field(netsh, "Profile"), Some("Home 5G"));
        let xml = wifi::netsh_profile("A&B <x>", "p\"ss'1234");
        assert!(xml.contains("<name>A&amp;B &lt;x&gt;</name>"), "{xml}");
        assert!(xml.contains("<keyMaterial>p&quot;ss&apos;1234</keyMaterial>"));
    }
}
