//! First-run setup and the health check on the computer (the core's
//! `features::onboarding` and `features::health`): what only this app can
//! read (macOS permissions, a screenshot tool on Linux, free space where
//! received files go) goes to the core, which adds the server, pushes,
//! links, keys and versions and rates each line. Each permission's fix
//! opens the OS's own page for it: nothing is ever granted from here.

// The macOS readings and prompts are only called there; their pure parts
// are tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::net::Ipv4Addr;
use std::path::Path;

use mdcore::features::health::{HealthReport, PermissionFact, PlatformFacts};
use mdcore::features::onboarding::Onboarding;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

pub fn this_os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// What was read here (none: can't be read on this OS).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readings {
    /// macOS Privacy & Security › Accessibility (input from your phone,
    /// shared mouse and keyboard, paste for you).
    pub accessibility: Option<bool>,
    /// Snapshots of the screen can be taken: Ok, or why not (macOS
    /// "screen_recording"; Linux "grim" / "imagemagick", a tool to
    /// install).
    pub screen: Option<Result<(), String>>,
    /// macOS Bluetooth (`bluetooth_state`).
    pub bluetooth: Option<&'static str>,
    /// macOS Notifications (`notification_state`); none outside an .app
    /// bundle, where macOS's notification center can't be asked.
    pub notifications: Option<&'static str>,
    /// macOS Local Network, inferred (`local_network_state`); none: not
    /// probed. Always "Ask now"-able once probed: no API says whether
    /// its prompt was answered.
    pub local_network: Option<&'static str>,
}

fn fact(id: &str, features: &[&str], state: &str, critical: bool) -> PermissionFact {
    PermissionFact {
        id: id.into(),
        features: features.iter().map(|s| s.to_string()).collect(),
        state: state.into(),
        critical,
        can_ask: false,
    }
}

fn askable(mut f: PermissionFact, can_ask: bool) -> PermissionFact {
    f.can_ask = can_ask && f.state != "granted";
    f
}

/// `CBManager.authorization` (CBManagerAuthorization) as a fact's state:
/// 0 notDetermined, 1 restricted, 2 denied, 3 allowedAlways.
pub fn bluetooth_state(auth: i64) -> &'static str {
    match auth {
        0 => "not_determined",
        1 | 2 => "denied",
        3 => "granted",
        _ => "unknown",
    }
}

/// `UNNotificationSettings.authorizationStatus` as a fact's state:
/// 0 notDetermined, 1 denied, 2 authorized, 3 provisional, 4 ephemeral.
pub fn notification_state(status: i64) -> &'static str {
    match status {
        0 => "not_determined",
        1 => "denied",
        2..=4 => "granted",
        _ => "unknown",
    }
}

/// macOS's errno when Local Network is off for this app: a send to an
/// address on this network fails with "No route to host".
pub const EHOSTUNREACH_MAC: i32 = 65;
/// "Host is down": the packet went out but nothing answered ARP, so the
/// network itself is allowed.
pub const EHOSTDOWN_MAC: i32 = 64;

/// Local Network from what sending a datagram to each address on this
/// network did (`Err` holds the errno) and whether one of your devices
/// is linked over this network. A send that went out wins; "No route to
/// host" while the card is up means macOS blocks it; else a local link;
/// else unknown (nothing to try).
pub fn local_network_state(sends: &[Result<(), i32>], local_link: bool) -> &'static str {
    if sends
        .iter()
        .any(|s| matches!(s, Ok(()) | Err(EHOSTDOWN_MAC)))
    {
        "granted"
    } else if sends.contains(&Err(EHOSTUNREACH_MAC)) {
        "denied"
    } else if local_link {
        "granted"
    } else {
        "unknown"
    }
}

/// Where to send the Local Network probe: the broadcast address of each
/// private IPv4 network this computer is on (`(ip, netmask)`), then the
/// devices linked over this network that sit on one of them.
pub fn probe_targets(nets: &[(Ipv4Addr, Ipv4Addr)], peers: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for (ip, mask) in nets {
        if !ip.is_private() || u32::from(*mask) == u32::MAX || u32::from(*mask) == 0 {
            continue;
        }
        let b = Ipv4Addr::from(u32::from(*ip) | !u32::from(*mask));
        if !out.contains(&b) {
            out.push(b);
        }
    }
    for p in peers {
        let on_net = nets.iter().any(|(ip, mask)| {
            u32::from(*ip) & u32::from(*mask) == u32::from(*p) & u32::from(*mask)
        });
        if on_net && *p != Ipv4Addr::UNSPECIFIED && !out.contains(p) {
            out.push(*p);
        }
    }
    out
}

/// Running from an .app bundle (`…/X.app/Contents/MacOS/x`): `tauri dev`
/// runs a bare binary, where macOS's notification center can't be used.
pub fn in_app_bundle(exe: &Path) -> bool {
    exe.to_string_lossy().contains(".app/Contents/MacOS/")
}

fn on_off(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "granted",
        Some(false) => "denied",
        None => "unknown",
    }
}

/// Each permission or setting the features need on `os`, and how it
/// stands. One that couldn't be read is "unknown": shown grey, with its
/// page. Bluetooth and Notifications never asked offer macOS's prompt;
/// Local Network offers "Ask now" whenever it isn't seen working.
pub fn permission_facts(os: Os, r: &Readings) -> Vec<PermissionFact> {
    let mut out = Vec::new();
    match os {
        Os::Mac => {
            out.push(askable(
                fact(
                    "local_network",
                    &[],
                    r.local_network.unwrap_or("unknown"),
                    false,
                ),
                r.local_network.is_some(),
            ));
            let n = r.notifications.unwrap_or("unknown");
            out.push(askable(
                fact("notifications", &["notifications"], n, false),
                n == "not_determined",
            ));
            out.push(fact(
                "accessibility",
                &["media", "clipboard"],
                on_off(r.accessibility),
                false,
            ));
            let screen = match &r.screen {
                Some(Ok(())) => "granted",
                Some(Err(_)) => "denied",
                None => "unknown",
            };
            out.push(fact("screen_recording", &["media"], screen, true));
            let b = r.bluetooth.unwrap_or("unknown");
            out.push(askable(
                fact("bluetooth", &["media"], b, false),
                b == "not_determined",
            ));
        }
        Os::Windows => {
            out.push(fact("notifications", &["notifications"], "unknown", false));
            out.push(fact("location", &["find"], "unknown", false));
        }
        Os::Linux => {
            // A screenshot tool to install, when one is missing.
            if let Some(Err(tool)) = &r.screen {
                if tool == "grim" || tool == "imagemagick" {
                    out.push(fact(&format!("tool_{tool}"), &["media"], "denied", false));
                }
            }
        }
    }
    out
}

/// The OS page where the user turns `id` on (none: no page to open).
pub fn settings_url(os: Os, id: &str) -> Option<&'static str> {
    Some(match (os, id) {
        (Os::Mac, "accessibility") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        (Os::Mac, "screen_recording") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        (Os::Mac, "local_network") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork"
        }
        (Os::Mac, "bluetooth") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        (Os::Mac, "notifications") => {
            "x-apple.systempreferences:com.apple.preference.notifications"
        }
        (Os::Windows, "notifications") => "ms-settings:notifications",
        (Os::Windows, "location") => "ms-settings:privacy-location",
        _ => return None,
    })
}

/// Bytes to whole MB, for the report.
pub fn to_mb(bytes: u64) -> u32 {
    u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX)
}

/// This computer's private IPv4 networks, as (address, netmask).
fn private_nets() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) if !v4.ip.is_loopback() && v4.ip.is_private() => {
                Some((v4.ip, v4.netmask))
            }
            _ => None,
        })
        .collect()
}

/// Your devices linked over this network: their addresses, and whether
/// there's one at all.
fn local_links(app: &AppHandle) -> (Vec<Ipv4Addr>, bool) {
    let Some(eng) = app.try_state::<Eng>() else {
        return (Vec::new(), false);
    };
    let map = eng.links_map();
    let links: Vec<_> = map
        .devices
        .iter()
        .filter_map(|d| d.link.as_ref())
        .filter(|l| l.addr_kind.as_deref() == Some("local"))
        .collect();
    let ips = links
        .iter()
        .filter_map(|l| l.remote.as_deref())
        .filter_map(|r| {
            r.parse::<std::net::SocketAddr>()
                .map(|s| s.ip())
                .or_else(|_| r.parse::<std::net::IpAddr>())
                .ok()
        })
        .filter_map(|ip| match ip {
            std::net::IpAddr::V4(v4) => Some(v4),
            std::net::IpAddr::V6(_) => None,
        })
        .collect();
    (ips, !links.is_empty())
}

/// Send one byte to the discard port of each address on this network
/// (`probe_targets`): each send's outcome, `Err` the errno. Nothing
/// listens there; it's how macOS's Local Network block shows.
fn probe_sends(targets: &[Ipv4Addr]) -> Vec<Result<(), i32>> {
    let Ok(sock) = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
        return Vec::new();
    };
    let _ = sock.set_broadcast(true);
    targets
        .iter()
        .map(|t| {
            sock.send_to(&[0], (*t, 9))
                .map(|_| ())
                .map_err(|e| e.raw_os_error().unwrap_or(-1))
        })
        .collect()
}

/// Local Network as it stands now (macOS).
fn read_local_network(app: &AppHandle) -> &'static str {
    let (peers, linked) = local_links(app);
    let targets = probe_targets(&private_nets(), &peers);
    local_network_state(&probe_sends(&targets), linked)
}

fn readings(app: &AppHandle) -> Readings {
    #[cfg(target_os = "macos")]
    {
        Readings {
            accessibility: Some(crate::device::accessibility_trusted()),
            screen: Some(crate::screen_share::ready().map_err(String::from)),
            bluetooth: mac::bluetooth_auth().map(bluetooth_state),
            notifications: mac::notification_status().map(notification_state),
            local_network: Some(read_local_network(app)),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Readings {
            screen: Some(crate::screen_share::ready().map_err(String::from)),
            ..Default::default()
        }
    }
}

fn facts(app: &AppHandle) -> PlatformFacts {
    let disk = app
        .try_state::<crate::DownloadDir>()
        .and_then(|d| crate::device::storage_of(&d.0));
    PlatformFacts {
        permissions: permission_facts(this_os(), &readings(app)),
        disk_free_mb: disk.map(|(free, _)| to_mb(free)),
        disk_total_mb: disk.map(|(_, total)| to_mb(total)),
    }
}

// --- commands ---

/// The health check. `check_server` asks the server too (a few seconds),
/// so it runs off the UI's thread.
#[tauri::command]
#[specta::specta]
pub async fn health_check(check_server: bool, app: AppHandle) -> Result<HealthReport, CoreError> {
    tauri::async_runtime::spawn_blocking(move || {
        let facts = facts(&app);
        app.state::<Eng>().health(&facts, check_server)
    })
    .await
    .map_err(|e| CoreError::Other(e.to_string()))
}

/// Open the OS page for permission `id` (the health check's and setup's
/// "Open settings"). False: this OS has no such page.
#[tauri::command]
#[specta::specta]
pub fn open_permission_page(id: String, app: AppHandle) -> Result<bool, CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let Some(url) = settings_url(this_os(), &id) else {
        return Ok(false);
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(true)
}

/// Show the OS's own prompt for permission `id` (the "Allow" / "Ask now"
/// fix), wait until it's answered (up to a minute) and say how it stands
/// now ("granted", "denied", "not_determined", "unknown"). Nothing is
/// granted from here: only macOS's prompt, answered by the user.
#[tauri::command]
#[specta::specta]
pub async fn request_permission(id: String, app: AppHandle) -> Result<String, CoreError> {
    tauri::async_runtime::spawn_blocking(move || ask(&id, &app))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))
}

#[cfg(target_os = "macos")]
fn ask(id: &str, app: &AppHandle) -> String {
    use std::time::Duration;
    const WAIT: Duration = Duration::from_secs(60);
    match id {
        "bluetooth" => mac::ask_bluetooth(WAIT)
            .map(bluetooth_state)
            .unwrap_or("unknown"),
        "notifications" => mac::ask_notifications(WAIT)
            .map(notification_state)
            .unwrap_or("unknown"),
        "local_network" => {
            // A local-network action: a Bonjour browse and a datagram to
            // this network bring up the prompt if it hasn't been
            // answered. It can't be waited for: look again a little later.
            let _ = read_local_network(app);
            mdcore::mdns::browse_briefly(Duration::from_secs(3));
            read_local_network(app)
        }
        _ => "unknown",
    }
    .to_string()
}

#[cfg(not(target_os = "macos"))]
fn ask(_id: &str, _app: &AppHandle) -> String {
    "unknown".into()
}

/// macOS: Bluetooth's and Notifications' authorization, read and asked.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    use std::sync::mpsc::sync_channel;
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSError;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNNotificationSettings, UNUserNotificationCenter,
    };

    #[link(name = "CoreBluetooth", kind = "framework")]
    extern "C" {}

    extern "C" {
        fn dispatch_queue_create(
            label: *const std::ffi::c_char,
            attr: *mut c_void,
        ) -> *mut AnyObject;
        fn dispatch_release(object: *mut AnyObject);
    }

    /// `+[CBManager authorization]` (macOS 10.15+); none before.
    pub fn bluetooth_auth() -> Option<i64> {
        let cls = AnyClass::get(c"CBManager")?;
        // SAFETY: NSObject's class method, a SEL argument, BOOL back.
        let has: bool = unsafe { msg_send![cls, respondsToSelector: sel!(authorization)] };
        if !has {
            return None;
        }
        // SAFETY: a class property returning CBManagerAuthorization (NSInteger).
        let auth: isize = unsafe { msg_send![cls, authorization] };
        Some(auth as i64)
    }

    /// Never asked: make a CBCentralManager on a queue of its own, which
    /// makes macOS show its Bluetooth prompt, and wait until it's answered
    /// (or `wait` passes). The authorization after.
    pub fn ask_bluetooth(wait: Duration) -> Option<i64> {
        let before = bluetooth_auth()?;
        if before != 0 {
            return Some(before);
        }
        let cls = AnyClass::get(c"CBCentralManager")?;
        // SAFETY: a new serial queue, released below once the manager's gone.
        let queue = unsafe {
            dispatch_queue_create(c"my-domain.bluetooth-ask".as_ptr(), std::ptr::null_mut())
        };
        if queue.is_null() {
            return Some(before);
        }
        let nil: *mut AnyObject = std::ptr::null_mut();
        // SAFETY: alloc + initWithDelegate:queue: (a nil delegate and a
        // dispatch queue, both object pointers); the manager is released
        // when dropped.
        let manager: Option<Retained<AnyObject>> = unsafe {
            let a: Allocated<AnyObject> = msg_send![cls, alloc];
            msg_send![a, initWithDelegate: nil, queue: queue]
        };
        let end = Instant::now() + wait;
        let mut now = before;
        while manager.is_some() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(250));
            now = bluetooth_auth().unwrap_or(now);
            if now != 0 {
                break;
            }
        }
        drop(manager);
        // SAFETY: the queue created above, released once.
        unsafe { dispatch_release(queue) };
        Some(now)
    }

    /// The app's notification authorization (UNAuthorizationStatus); none
    /// outside an .app bundle (macOS's notification center throws there)
    /// or when it doesn't answer in a few seconds.
    pub fn notification_status() -> Option<i64> {
        let exe = std::env::current_exe().ok()?;
        if !super::in_app_bundle(&exe) {
            return None;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (tx, rx) = sync_channel(1);
        let block = RcBlock::new(move |s: NonNull<UNNotificationSettings>| {
            // SAFETY: the settings macOS hands the handler, alive for the call.
            let status = unsafe { s.as_ref() }.authorizationStatus();
            let _ = tx.try_send(status.0 as i64);
        });
        center.getNotificationSettingsWithCompletionHandler(&block);
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }

    /// Never asked: macOS's prompt for alerts, sounds and badges, waited
    /// for (up to `wait`). The status after.
    pub fn ask_notifications(wait: Duration) -> Option<i64> {
        let before = notification_status()?;
        if before != 0 {
            return Some(before);
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (tx, rx) = sync_channel(1);
        let block = RcBlock::new(move |_granted: Bool, _err: *mut NSError| {
            let _ = tx.try_send(());
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge,
            &block,
        );
        let _ = rx.recv_timeout(wait);
        notification_status()
    }
}

/// Open the folder received files go to (the disk check's fix).
#[tauri::command]
#[specta::specta]
pub fn open_downloads(app: AppHandle) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app
        .try_state::<crate::DownloadDir>()
        .map(|d| d.0.clone())
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| CoreError::Other("no downloads folder".into()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Where first-run setup stands on this computer.
#[tauri::command]
#[specta::specta]
pub fn onboarding_state(engine: State<Eng>) -> Onboarding {
    engine.onboarding()
}

/// A tap in first-run setup ("next", "back", "skip", "goto" with `step`,
/// "skip_all", "finish", "restart").
#[tauri::command]
#[specta::specta]
pub fn onboarding_act(
    action: String,
    step: Option<String>,
    engine: State<Eng>,
) -> Result<Onboarding, CoreError> {
    engine.onboarding_act(&action, step.as_deref())
}

/// The feature groups picked in setup.
#[tauri::command]
#[specta::specta]
pub fn set_onboarding_features(features: Vec<String>, engine: State<Eng>) -> Onboarding {
    engine.set_onboarding_features(&features)
}

/// Name this computer (setup's first step). Returns the name as kept.
#[tauri::command]
#[specta::specta]
pub fn set_device_name(name: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.set_device_name(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(f: &[PermissionFact]) -> Vec<(&str, &str)> {
        f.iter()
            .map(|p| (p.id.as_str(), p.state.as_str()))
            .collect()
    }

    #[test]
    fn mac_reads_what_it_can_and_points_at_the_rest() {
        let r = Readings {
            accessibility: Some(false),
            screen: Some(Err("screen_recording".into())),
            ..Default::default()
        };
        let f = permission_facts(Os::Mac, &r);
        assert_eq!(
            states(&f),
            vec![
                ("local_network", "unknown"),
                ("notifications", "unknown"),
                ("accessibility", "denied"),
                ("screen_recording", "denied"),
                ("bluetooth", "unknown"),
            ]
        );
        assert!(
            f.iter()
                .find(|p| p.id == "screen_recording")
                .unwrap()
                .critical
        );
        assert_eq!(
            f.iter().find(|p| p.id == "accessibility").unwrap().features,
            vec!["media", "clipboard"]
        );
        // Nothing probed or readable: none offers a prompt.
        assert!(f.iter().all(|p| !p.can_ask));
        let ok = Readings {
            accessibility: Some(true),
            screen: Some(Ok(())),
            bluetooth: Some("granted"),
            notifications: Some("granted"),
            local_network: Some("granted"),
        };
        let f = permission_facts(Os::Mac, &ok);
        assert!(f.iter().all(|p| p.state == "granted" && !p.can_ask));
        // Every one has a page to open.
        assert!(f.iter().all(|p| settings_url(Os::Mac, &p.id).is_some()));
    }

    #[test]
    fn mac_offers_the_prompt_only_when_it_can_still_show() {
        let fresh = Readings {
            accessibility: Some(true),
            screen: Some(Ok(())),
            bluetooth: Some("not_determined"),
            notifications: Some("not_determined"),
            local_network: Some("unknown"),
        };
        let f = permission_facts(Os::Mac, &fresh);
        let ask: Vec<(&str, &str, bool)> = f
            .iter()
            .map(|p| (p.id.as_str(), p.state.as_str(), p.can_ask))
            .collect();
        assert_eq!(
            ask,
            vec![
                ("local_network", "unknown", true),
                ("notifications", "not_determined", true),
                ("accessibility", "granted", false),
                ("screen_recording", "granted", false),
                ("bluetooth", "not_determined", true),
            ]
        );
        let said_no = Readings {
            bluetooth: Some("denied"),
            notifications: Some("denied"),
            local_network: Some("denied"),
            ..fresh
        };
        let f = permission_facts(Os::Mac, &said_no);
        let can = |id: &str| f.iter().find(|p| p.id == id).unwrap().can_ask;
        // Once answered no, only its page helps; Local Network can always
        // be asked again (nothing says its prompt was answered).
        assert!(!can("bluetooth"));
        assert!(!can("notifications"));
        assert!(can("local_network"));
    }

    #[test]
    fn bluetooth_and_notification_authorization() {
        assert_eq!(bluetooth_state(0), "not_determined");
        assert_eq!(bluetooth_state(1), "denied", "restricted");
        assert_eq!(bluetooth_state(2), "denied");
        assert_eq!(bluetooth_state(3), "granted");
        assert_eq!(bluetooth_state(9), "unknown");
        assert_eq!(notification_state(0), "not_determined");
        assert_eq!(notification_state(1), "denied");
        assert_eq!(notification_state(2), "granted");
        assert_eq!(notification_state(3), "granted", "provisional");
        assert_eq!(notification_state(4), "granted", "ephemeral");
        assert_eq!(notification_state(-1), "unknown");
    }

    #[test]
    fn local_network_from_what_a_send_did() {
        const NET_UNREACH: i32 = 51;
        assert_eq!(local_network_state(&[Ok(())], false), "granted");
        assert_eq!(
            local_network_state(&[Err(EHOSTUNREACH_MAC), Err(EHOSTDOWN_MAC)], false),
            "granted",
            "host down: the packet went out"
        );
        assert_eq!(
            local_network_state(&[Err(EHOSTUNREACH_MAC)], false),
            "denied"
        );
        assert_eq!(
            local_network_state(&[Err(EHOSTUNREACH_MAC)], true),
            "denied",
            "a link the other side opened doesn't prove this side may"
        );
        assert_eq!(local_network_state(&[Err(NET_UNREACH)], false), "unknown");
        assert_eq!(local_network_state(&[], true), "granted");
        assert_eq!(local_network_state(&[], false), "unknown");
    }

    #[test]
    fn probe_goes_to_this_networks_broadcast_and_local_devices() {
        let lan = Ipv4Addr::new(192, 168, 0, 176);
        let mask = Ipv4Addr::new(255, 255, 255, 0);
        let nets = [
            (lan, mask),
            (Ipv4Addr::new(10, 1, 2, 3), Ipv4Addr::new(255, 255, 0, 0)),
            // Not private, or a point-to-point address: left out.
            (Ipv4Addr::new(100, 64, 0, 1), mask),
            (Ipv4Addr::new(192, 168, 9, 9), Ipv4Addr::BROADCAST),
        ];
        let peers = [
            Ipv4Addr::new(192, 168, 0, 42),
            Ipv4Addr::new(172, 16, 0, 5),
            Ipv4Addr::new(192, 168, 0, 42),
        ];
        assert_eq!(
            probe_targets(&nets, &peers),
            vec![
                Ipv4Addr::new(192, 168, 0, 255),
                Ipv4Addr::new(10, 1, 255, 255),
                Ipv4Addr::new(192, 168, 0, 42),
            ]
        );
        assert!(probe_targets(&[], &peers).is_empty());
    }

    #[test]
    fn notifications_only_asked_from_an_app_bundle() {
        assert!(in_app_bundle(Path::new(
            "/Applications/my-domain.app/Contents/MacOS/my-domain"
        )));
        assert!(!in_app_bundle(Path::new(
            "/Users/me/my-domain/target/debug/my-domain"
        )));
    }

    #[test]
    fn info_plist_says_why_for_each_prompt() {
        let plist = include_str!("../Info.plist");
        for key in [
            "NSBluetoothAlwaysUsageDescription",
            "NSLocalNetworkUsageDescription",
            "NSBonjourServices",
        ] {
            assert!(plist.contains(&format!("<key>{key}</key>")), "{key}");
        }
        // The Bonjour type the core browses and advertises.
        let ty = mdcore::mdns::SERVICE_TYPE.trim_end_matches(".local.");
        assert!(plist.contains(&format!("<string>{ty}</string>")), "{ty}");
    }

    #[test]
    fn linux_only_asks_for_a_missing_screenshot_tool() {
        let none = Readings {
            screen: Some(Ok(())),
            ..Default::default()
        };
        assert!(permission_facts(Os::Linux, &none).is_empty());
        let grim = Readings {
            screen: Some(Err("grim".into())),
            ..Default::default()
        };
        assert_eq!(
            states(&permission_facts(Os::Linux, &grim)),
            vec![("tool_grim", "denied")]
        );
        assert_eq!(settings_url(Os::Linux, "tool_grim"), None);
    }

    #[test]
    fn windows_pages() {
        let f = permission_facts(Os::Windows, &Readings::default());
        assert_eq!(
            states(&f),
            vec![("notifications", "unknown"), ("location", "unknown")]
        );
        assert_eq!(
            settings_url(Os::Windows, "location"),
            Some("ms-settings:privacy-location")
        );
        assert_eq!(settings_url(Os::Windows, "accessibility"), None);
    }

    #[test]
    fn megabytes() {
        assert_eq!(to_mb(5 * 1024 * 1024 + 3), 5);
        assert_eq!(to_mb(u64::MAX), u32::MAX);
    }
}
