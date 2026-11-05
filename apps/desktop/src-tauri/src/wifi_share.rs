// Share this computer's Wi-Fi with your phone: read the network this
// computer is on and its saved password, then send the phone's own `wifi
// {op: "connect", ssid, password}` action (apps/android WifiNetworks.kt),
// end-to-end encrypted like every action.
//
// The password stays out of the way:
//  - it's read here, in Rust, and sent straight from here, so it never
//    reaches the web UI;
//  - it's never shown, logged or saved by my-domain (only held in memory
//    until the phone has the message);
//  - the OS asks first: macOS shows its own keychain prompt each time, and
//    saying no there stops it. Linux reads it from NetworkManager, which
//    may ask for your password too.
//
// The network name comes from the OS's tools: on macOS `networksetup`, then
// `ipconfig getsummary` (newer macOS hides the name from apps without
// Location access: then there's nothing to share and the button isn't
// shown); on Linux `nmcli`. An open network is sent without a password.
// Elsewhere there's no Wi-Fi to share.

use tauri::{AppHandle, Manager};

use crate::Eng;
use mdcore::CoreError;

#[cfg_attr(windows, allow(dead_code))]
const SAID_NO: &str = "The password wasn't shared: you said no on this computer";
const NO_PASSWORD: &str = "There's no saved password for this Wi-Fi on this computer";
const NO_WIFI: &str = "This computer isn't on Wi-Fi";

/// The name of the Wi-Fi network this computer is on, if it can tell.
#[tauri::command]
#[specta::specta]
pub async fn current_wifi_name() -> Option<String> {
    tauri::async_runtime::spawn_blocking(current_wifi)
        .await
        .ok()
        .flatten()
}

/// Send this computer's Wi-Fi (name and password) to `node_id` so it joins
/// it. Returns the network's name. Async: the OS may wait on a prompt.
#[tauri::command]
#[specta::specta]
pub async fn share_wifi_to(node_id: String, app: AppHandle) -> Result<String, CoreError> {
    let (ssid, password) = tauri::async_runtime::spawn_blocking(|| {
        let ssid = current_wifi().ok_or(NO_WIFI)?;
        let password = password(&ssid)?;
        Ok::<_, &'static str>((ssid, password))
    })
    .await
    .map_err(|e| CoreError::Other(e.to_string()))?
    .map_err(|e| CoreError::Other(e.into()))?;
    let mut args = serde_json::json!({ "op": "connect", "ssid": ssid });
    if let Some(p) = password {
        args["password"] = p.into();
    }
    app.state::<Eng>()
        .run_action(&node_id, "wifi", &args.to_string())?;
    Ok(ssid)
}

#[cfg_attr(not(unix), allow(dead_code))]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// --- macOS -------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn wifi_device() -> Option<String> {
    parse_wifi_device(&run("networksetup", &["-listallhardwareports"])?)
}

#[cfg(target_os = "macos")]
pub fn current_wifi() -> Option<String> {
    let dev = wifi_device().unwrap_or_else(|| "en0".into());
    run("networksetup", &["-getairportnetwork", &dev])
        .as_deref()
        .and_then(parse_airport_network)
        .or_else(|| {
            run("ipconfig", &["getsummary", &dev])
                .as_deref()
                .and_then(parse_ipconfig_ssid)
        })
}

/// The saved password (`None`: an open network). macOS asks the user first.
#[cfg(target_os = "macos")]
pub fn password(ssid: &str) -> Result<Option<String>, &'static str> {
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-D",
            "AirPort network password",
            "-a",
            ssid,
            "-w",
        ])
        .output()
        .map_err(|_| NO_PASSWORD)?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        return Ok(Some(p).filter(|p| !p.is_empty()));
    }
    match security_failure(out.status.code(), &String::from_utf8_lossy(&out.stderr)) {
        Failure::SaidNo => Err(SAID_NO),
        // No saved password: fine if the network is open.
        Failure::NotFound => {
            let open = run("system_profiler", &["SPAirPortDataType"])
                .as_deref()
                .and_then(parse_profiler_open);
            if open == Some(true) {
                Ok(None)
            } else {
                Err(NO_PASSWORD)
            }
        }
    }
}

// --- Linux -------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub fn current_wifi() -> Option<String> {
    parse_nmcli_active_ssid(&run("nmcli", &["-t", "-f", "active,ssid", "dev", "wifi"])?)
}

/// The saved password (`None`: an open network), from NetworkManager's
/// active Wi-Fi connection (or the one named after the network).
#[cfg(target_os = "linux")]
pub fn password(ssid: &str) -> Result<Option<String>, &'static str> {
    let name = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "NAME,TYPE,DEVICE",
            "connection",
            "show",
            "--active",
        ],
    )
    .as_deref()
    .and_then(parse_nmcli_wifi_connection)
    .unwrap_or_else(|| ssid.to_string());
    let get = |field: &str| {
        let out = std::process::Command::new("nmcli")
            .args(["-s", "-g", field, "connection", "show", &name])
            .output()
            .map_err(|_| NO_PASSWORD)?;
        if out.status.success() {
            Ok(unescape_nmcli(
                String::from_utf8_lossy(&out.stdout).trim_end(),
            ))
        } else if String::from_utf8_lossy(&out.stderr)
            .to_lowercase()
            .contains("not authorized")
        {
            Err(SAID_NO)
        } else {
            Err(NO_PASSWORD)
        }
    };
    if get("802-11-wireless-security.key-mgmt")?.is_empty() {
        return Ok(None);
    }
    let p = get("802-11-wireless-security.psk")?;
    if p.is_empty() {
        Err(NO_PASSWORD)
    } else {
        Ok(Some(p))
    }
}

// --- elsewhere ---------------------------------------------------------------

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn current_wifi() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn password(_ssid: &str) -> Result<Option<String>, &'static str> {
    Err(NO_PASSWORD)
}

// --- parsing (kept apart so it's tested on every OS) ---------------------------

/// A name macOS hides or a blank one isn't a name.
fn real_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name != "<redacted>").then(|| name.to_string())
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

/// `networksetup -getairportnetwork <dev>`: `Current Wi-Fi Network: <name>`
/// (anything else, e.g. "You are not associated…", is no name).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_airport_network(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| {
            l.trim_start()
                .strip_prefix("Current Wi-Fi Network: ")
                .or_else(|| l.trim_start().strip_prefix("Current AirPort Network: "))
        })
        .and_then(real_name)
}

/// `ipconfig getsummary <dev>`: the `SSID : <name>` line (not `BSSID`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_ipconfig_ssid(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.trim_start().strip_prefix("SSID : "))
        .and_then(real_name)
}

/// `system_profiler SPAirPortDataType`: whether the current network's
/// `Security:` is `None`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_profiler_open(out: &str) -> Option<bool> {
    out.lines()
        .skip_while(|l| l.trim() != "Current Network Information:")
        .take_while(|l| !l.trim().starts_with("Other Local Wi-Fi Networks:"))
        .find_map(|l| l.trim().strip_prefix("Security:"))
        .map(|s| s.trim() == "None")
}

/// Why `security find-generic-password` failed: exit 44 or "could not be
/// found" is no saved item; anything else (a "Deny" or a cancelled prompt,
/// exit 128 / "User canceled") counts as a no.
#[derive(Debug, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Failure {
    NotFound,
    SaidNo,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn security_failure(code: Option<i32>, stderr: &str) -> Failure {
    if code == Some(44) || stderr.contains("could not be found") {
        Failure::NotFound
    } else {
        Failure::SaidNo
    }
}

/// `nmcli -t -f active,ssid dev wifi`: the `yes:<ssid>` line.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_nmcli_active_ssid(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.strip_prefix("yes:"))
        .map(unescape_nmcli)
        .and_then(|s| real_name(&s))
}

/// `nmcli -t -f NAME,TYPE,DEVICE connection show --active`: the name of the
/// first Wi-Fi connection.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_nmcli_wifi_connection(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let f = split_nmcli(l);
        (f.len() >= 2 && (f[1] == "802-11-wireless" || f[1] == "wifi") && !f[0].is_empty())
            .then(|| f[0].clone())
    })
}

/// Terse nmcli output: fields split on `:`, with `\:` and `\\` escaped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn split_nmcli(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unescape_nmcli(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_share_finds_the_wifi_device() {
        let ports = "Hardware Port: Ethernet\nDevice: en0\nEthernet Address: aa\n\n\
                     Hardware Port: Wi-Fi\nDevice: en1\nEthernet Address: bb\n";
        assert_eq!(parse_wifi_device(ports), Some("en1".into()));
        assert_eq!(
            parse_wifi_device("Hardware Port: Thunderbolt Bridge\nDevice: bridge0\n"),
            None
        );
    }

    #[test]
    fn wifi_share_reads_networksetup() {
        assert_eq!(
            parse_airport_network("Current Wi-Fi Network: Office 5G\n"),
            Some("Office 5G".into())
        );
        assert_eq!(
            parse_airport_network("You are not associated with an AirPort network.\n"),
            None
        );
        assert_eq!(
            parse_airport_network("Current Wi-Fi Network: <redacted>\n"),
            None
        );
        assert_eq!(parse_airport_network("Current Wi-Fi Network: \n"), None);
    }

    #[test]
    fn wifi_share_reads_ipconfig_summary() {
        let out = "<dictionary> {\n  BSSID : 11:22:33:44:55:66\n  InterfaceType : WiFi\n  \
                   LinkStatusActive : TRUE\n  SSID : Office\n  Security : WPA2_PSK\n}\n";
        assert_eq!(parse_ipconfig_ssid(out), Some("Office".into()));
        let hidden = "<dictionary> {\n  BSSID : <redacted>\n  SSID : <redacted>\n}\n";
        assert_eq!(parse_ipconfig_ssid(hidden), None);
        assert_eq!(
            parse_ipconfig_ssid("<dictionary> {\n  InterfaceType : WiFi\n}\n"),
            None
        );
    }

    #[test]
    fn wifi_share_tells_an_open_network() {
        let profile = |sec: &str| {
            format!(
                "Wi-Fi:\n  Interfaces:\n    en0:\n      Status: Connected\n      \
                 Current Network Information:\n        Office:\n          \
                 PHY Mode: 802.11ax\n          Security: {sec}\n      \
                 Other Local Wi-Fi Networks:\n        Cafe:\n          Security: None\n"
            )
        };
        assert_eq!(parse_profiler_open(&profile("None")), Some(true));
        assert_eq!(parse_profiler_open(&profile("WPA2 Personal")), Some(false));
        // Only the other networks say None: not ours.
        let other_only = "Wi-Fi:\n  Other Local Wi-Fi Networks:\n    Cafe:\n      Security: None\n";
        assert_eq!(parse_profiler_open(other_only), None);
    }

    #[test]
    fn wifi_share_reads_keychain_failures() {
        let missing = "security: SecKeychainSearchCopyNext: The specified item could not be found in the keychain.";
        assert_eq!(security_failure(Some(44), missing), Failure::NotFound);
        assert_eq!(security_failure(Some(1), missing), Failure::NotFound);
        assert_eq!(
            security_failure(Some(128), "security: User canceled the operation."),
            Failure::SaidNo
        );
        assert_eq!(security_failure(None, ""), Failure::SaidNo);
    }

    #[test]
    fn wifi_share_reads_nmcli() {
        assert_eq!(
            parse_nmcli_active_ssid("no:Cafe\nyes:Home\\:5G\nno:Other\n"),
            Some("Home:5G".into())
        );
        assert_eq!(parse_nmcli_active_ssid("no:Cafe\n"), None);
        assert_eq!(parse_nmcli_active_ssid("yes:\n"), None);

        let active = "Wired connection 1:802-3-ethernet:enp3s0\n\
                      Home\\:5G 1:802-11-wireless:wlp2s0\nlo:loopback:lo\n";
        assert_eq!(
            parse_nmcli_wifi_connection(active),
            Some("Home:5G 1".into())
        );
        assert_eq!(parse_nmcli_wifi_connection("lo:loopback:lo\n"), None);

        assert_eq!(unescape_nmcli("pa\\:ss\\\\word"), "pa:ss\\word");
    }
}
