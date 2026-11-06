// Wi-Fi Direct for big files (mdcore `features::wifi_direct`): a phone near
// this computer sends or receives a big file over a Wi-Fi Direct group
// when the two aren't on the same network.
//
//  - Windows only: this computer hosts the group (a Wi-Fi Direct
//    advertisement with "legacy" settings, `Windows.Devices.WiFiDirect`),
//    under the name and passphrase the core suggests; the phone joins it.
//    A Mac has no public API for it, and Linux would need NetworkManager's
//    P2P device over D-Bus: both say they can't (caps 0) and keep the
//    usual routes;
//  - "Use Wi-Fi Direct for big files when nearby" in Settings, on by
//    default (`<app config dir>/prefs.json`, key `wifi_direct`, in the
//    settings backup);
//  - the transfer row says "via Wi-Fi Direct" (`transfers.rs`), and the
//    connection map says so too (from the core);
//  - where Wi-Fi Direct can't be used (a Mac, Linux, a Windows PC without
//    it), the phone's hotspot can (`hotspot.rs`): this computer says it can
//    join one (`CAN_JOIN_HOTSPOT`) unless that's set to "never".

use std::sync::atomic::{AtomicBool, Ordering};

use mdcore::features::wifi_direct::{CAN_HOST, CAN_JOIN_HOTSPOT};
use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Manager};

const KEY: &str = "wifi_direct";

/// The switch: on unless it was turned off.
pub fn pref_on(saved: Option<&str>) -> bool {
    saved != Some("off")
}

/// What this computer can do (the core's `CAN_*` bits): host a Wi-Fi
/// Direct group (the switch on, and Windows' API there), join a phone's
/// hotspot (it has Wi-Fi, and that isn't set to "never").
pub fn caps(wifi_direct: bool, windows_api: bool, hotspot: bool) -> u64 {
    let mut c = 0;
    if wifi_direct && windows_api {
        c |= CAN_HOST;
    }
    if hotspot {
        c |= CAN_JOIN_HOTSPOT;
    }
    c
}

/// The "Use Wi-Fi Direct for big files when nearby" switch.
pub struct WifiDirectPref {
    kv: Option<FileKv>,
    on: AtomicBool,
}

impl WifiDirectPref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let on = pref_on(kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref());
        WifiDirectPref {
            kv,
            on: AtomicBool::new(on),
        }
    }

    pub fn on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    fn set(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
                eprintln!("[wifi-direct] couldn't save the setting: {e}");
            }
        }
    }
}

/// Tell the core what this computer can do and the user's switch (at
/// start, once the engine is up).
pub fn init(app: &AppHandle) {
    let app = app.clone();
    // Asking the OS for its Wi-Fi card runs a tool: off the main thread.
    std::thread::spawn(move || refresh(&app));
}

/// Tell the core again (a setting changed).
fn refresh(app: &AppHandle) {
    let on = app.try_state::<WifiDirectPref>().is_none_or(|p| p.on());
    let hotspot =
        crate::hotspot::mode(app) != crate::hotspot::Mode::Never && crate::hotspot::supported();
    let Some(engine) = app.try_state::<crate::Eng>() else {
        return;
    };
    // The core's own switch stays on: what's offered is in the caps.
    engine.set_wifi_direct_enabled(true);
    engine.set_wifi_direct_caps(caps(on, supported(), hotspot));
}

/// This computer can host a group.
pub fn supported() -> bool {
    #[cfg(windows)]
    {
        win::available()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The core asks: host a group ("host"), join one ("join", never asked of
/// a computer: it only hosts) or leave it ("leave"). Off the core's
/// thread.
#[allow(clippy::too_many_arguments)]
pub fn on_request(
    app: &AppHandle,
    sid: &str,
    node: &str,
    method: &str,
    op: &str,
    ssid: Option<&str>,
    passphrase: Option<&str>,
    host_ip: Option<&str>,
) {
    if method == "hotspot" {
        match op {
            "consent" => crate::hotspot::on_consent(app, sid, node),
            "join" => crate::hotspot::on_join(
                app,
                sid,
                ssid.unwrap_or_default(),
                passphrase.unwrap_or_default(),
                host_ip,
            ),
            "leave" => crate::hotspot::on_leave(sid),
            _ => {
                if let Some(e) = app.try_state::<crate::Eng>() {
                    e.wifi_direct_failed(sid, "a computer doesn't host a hotspot");
                }
            }
        }
        return;
    }
    let (app, sid, op) = (app.clone(), sid.to_string(), op.to_string());
    let (ssid, passphrase) = (
        ssid.unwrap_or_default().to_string(),
        passphrase.unwrap_or_default().to_string(),
    );
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<crate::Eng>() else {
            return;
        };
        match op.as_str() {
            "host" => match host(&sid, &ssid, &passphrase) {
                Ok(()) => engine.wifi_direct_hosted(&sid, &ssid, &passphrase, ""),
                Err(e) => engine.wifi_direct_failed(&sid, &e),
            },
            "leave" => leave(&sid),
            _ => engine.wifi_direct_failed(&sid, "this computer only hosts"),
        }
    });
}

fn host(sid: &str, ssid: &str, passphrase: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        win::host(sid, ssid, passphrase)
    }
    #[cfg(not(windows))]
    {
        let _ = (sid, ssid, passphrase);
        Err("Wi-Fi Direct isn't available on this computer".into())
    }
}

fn leave(sid: &str) {
    #[cfg(windows)]
    win::leave(sid);
    #[cfg(not(windows))]
    let _ = sid;
}

/// What Settings shows.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WifiDirectSetting {
    pub on: bool,
    /// This computer can use it (Windows with Wi-Fi Direct).
    pub supported: bool,
    /// "Use the phone’s hotspot for big files".
    pub hotspot: crate::hotspot::Mode,
    /// This computer has Wi-Fi to join a hotspot with.
    pub hotspot_supported: bool,
}

fn fallback(on: bool, hotspot: crate::hotspot::Mode) -> WifiDirectSetting {
    WifiDirectSetting {
        on,
        supported: false,
        hotspot,
        hotspot_supported: false,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn wifi_direct_setting(app: AppHandle) -> WifiDirectSetting {
    tauri::async_runtime::spawn_blocking(move || setting(&app))
        .await
        .unwrap_or_else(|_| fallback(true, crate::hotspot::Mode::Ask))
}

fn setting(app: &AppHandle) -> WifiDirectSetting {
    WifiDirectSetting {
        on: app.try_state::<WifiDirectPref>().is_none_or(|p| p.on()),
        supported: supported(),
        hotspot: crate::hotspot::mode(app),
        hotspot_supported: crate::hotspot::supported(),
    }
}

/// "Use the phone’s hotspot for big files": ask, always or never.
#[tauri::command]
#[specta::specta]
pub async fn set_phone_hotspot(mode: crate::hotspot::Mode, app: AppHandle) -> WifiDirectSetting {
    if let Some(pref) = app.try_state::<crate::hotspot::HotspotPref>() {
        pref.set(mode);
    }
    tauri::async_runtime::spawn_blocking(move || {
        refresh(&app);
        setting(&app)
    })
    .await
    .unwrap_or_else(|_| fallback(true, mode))
}

/// Turn "Use Wi-Fi Direct for big files when nearby" on or off.
#[tauri::command]
#[specta::specta]
pub async fn set_wifi_direct(on: bool, app: AppHandle) -> WifiDirectSetting {
    if let Some(pref) = app.try_state::<WifiDirectPref>() {
        pref.set(on);
    }
    tauri::async_runtime::spawn_blocking(move || {
        refresh(&app);
        setting(&app)
    })
    .await
    .unwrap_or_else(|_| fallback(on, crate::hotspot::Mode::Ask))
}

/// Windows: a Wi-Fi Direct advertisement that makes this computer a group
/// owner, with "legacy" settings (a name and passphrase) the phone joins
/// with. One per request, kept until the core says leave.
#[cfg(windows)]
mod win {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use windows::core::HSTRING;
    use windows::Devices::WiFiDirect::{
        WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
        WiFiDirectAdvertisementPublisherStatus,
    };
    use windows::Security::Credentials::PasswordCredential;

    /// How long the group may take to start.
    const START_WAIT: Duration = Duration::from_secs(8);

    /// The groups up now, by request.
    static GROUPS: Mutex<Vec<(String, WiFiDirectAdvertisementPublisher)>> = Mutex::new(Vec::new());

    fn text(e: &windows::core::Error) -> String {
        format!("{} ({:#x})", e.message(), e.code().0)
    }

    pub fn available() -> bool {
        WiFiDirectAdvertisementPublisher::new().is_ok()
    }

    pub fn host(sid: &str, ssid: &str, passphrase: &str) -> Result<(), String> {
        let publisher = WiFiDirectAdvertisementPublisher::new().map_err(|e| text(&e))?;
        let adv = publisher.Advertisement().map_err(|e| text(&e))?;
        adv.SetIsAutonomousGroupOwnerEnabled(true)
            .map_err(|e| text(&e))?;
        adv.SetListenStateDiscoverability(
            WiFiDirectAdvertisementListenStateDiscoverability::Normal,
        )
        .map_err(|e| text(&e))?;
        let legacy = adv.LegacySettings().map_err(|e| text(&e))?;
        legacy.SetIsEnabled(true).map_err(|e| text(&e))?;
        legacy.SetSsid(&HSTRING::from(ssid)).map_err(|e| text(&e))?;
        let cred = PasswordCredential::new().map_err(|e| text(&e))?;
        cred.SetPassword(&HSTRING::from(passphrase))
            .map_err(|e| text(&e))?;
        legacy.SetPassphrase(&cred).map_err(|e| text(&e))?;
        publisher.Start().map_err(|e| text(&e))?;
        let deadline = Instant::now() + START_WAIT;
        loop {
            let status = publisher.Status().map_err(|e| text(&e))?;
            if status == WiFiDirectAdvertisementPublisherStatus::Started {
                break;
            }
            if status == WiFiDirectAdvertisementPublisherStatus::Aborted
                || Instant::now() >= deadline
            {
                let _ = publisher.Stop();
                return Err("the Wi-Fi Direct group didn't start".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Ok(mut g) = GROUPS.lock() {
            g.push((sid.to_string(), publisher));
        }
        Ok(())
    }

    pub fn leave(sid: &str) {
        let gone: Vec<WiFiDirectAdvertisementPublisher> = match GROUPS.lock() {
            Ok(mut g) => {
                let (gone, kept): (Vec<_>, Vec<_>) = g.drain(..).partition(|(s, _)| s == sid);
                *g = kept;
                gone.into_iter().map(|(_, p)| p).collect()
            }
            Err(_) => Vec::new(),
        };
        for p in gone {
            let _ = p.Stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_direct_is_on_unless_turned_off() {
        assert!(pref_on(None));
        assert!(pref_on(Some("on")));
        assert!(!pref_on(Some("off")));
        assert!(pref_on(Some("junk")));
    }

    #[test]
    fn wifi_direct_caps_say_host_only() {
        assert_eq!(caps(true, true, false), CAN_HOST);
        assert_eq!(caps(false, true, false), 0, "switched off");
        assert_eq!(caps(true, false, false), 0);
        // A Mac: only a phone's hotspot.
        assert_eq!(caps(true, false, true), CAN_JOIN_HOTSPOT);
        assert_eq!(caps(true, true, true), CAN_HOST | CAN_JOIN_HOTSPOT);
        // Not on a Mac or Linux.
        if !cfg!(windows) {
            assert!(!supported());
        }
    }
}
