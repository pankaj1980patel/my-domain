//! Where this computer is, for "find my computer", on Linux and Windows
//! (macOS uses CoreLocation in the desktop app itself).
//!
//! - Linux: GeoClue2 over the system D-Bus. A client gets our desktop id
//!   (GeoClue's agent, e.g. GNOME's, asks the user and checks the desktop's
//!   Location Services switch), is started, and its `Location` is read once
//!   it has one; then it's stopped and deleted.
//! - Windows: `Windows.Devices.Geolocation.Geolocator`; Windows' own
//!   location privacy settings apply.
//!
//! Only on request, never in the background: nothing here runs until the
//! app calls [`fix`], and it stops the location service again before
//! returning. Blocking: call it off the UI thread.
//!
//! The parts that don't touch the OS (timestamp conversion, error texts)
//! are plain functions in [`geoclue`] and [`winloc`], tested everywhere.

use std::time::Duration;

/// A position: (latitude, longitude, accuracy in metres, unix seconds).
pub type Fix = (f64, f64, f64, u64);

/// How long to wait for a position.
pub const FIX_WAIT: Duration = Duration::from_secs(15);

/// Said when no position came in time (as on macOS).
pub const NO_FIX_YET: &str = "no position yet; try again in a moment";

/// This computer can tell where it is at all: Windows always (the privacy
/// settings are checked when asked); Linux when GeoClue is on the system
/// bus or can be started by it (checked at most once a minute).
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(windows)]
    {
        true
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        false
    }
}

/// Where this computer is now, asking the OS's location service (blocking,
/// up to [`FIX_WAIT`]). `desktop_id` is the app's desktop file name without
/// `.desktop` (GeoClue's agent uses it to ask and to remember the answer).
/// The error is a sentence for a notification.
pub fn fix(desktop_id: &str) -> Result<Fix, String> {
    #[cfg(target_os = "linux")]
    {
        linux::fix(desktop_id)
    }
    #[cfg(windows)]
    {
        let _ = desktop_id;
        windows_loc::fix()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = desktop_id;
        Err("finding this computer's location isn't supported here yet".into())
    }
}

/// GeoClue2: names, and the parts that don't need the bus.
pub mod geoclue {
    pub const SERVICE: &str = "org.freedesktop.GeoClue2";
    pub const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
    pub const MANAGER: &str = "org.freedesktop.GeoClue2.Manager";
    pub const CLIENT: &str = "org.freedesktop.GeoClue2.Client";
    pub const LOCATION: &str = "org.freedesktop.GeoClue2.Location";
    /// GClueAccuracyLevel: EXACT (GPS or Wi-Fi, whatever is best).
    pub const ACCURACY_EXACT: u32 = 8;

    /// Said when GeoClue isn't there to ask.
    pub const MISSING: &str = "Location isn't available on this computer: install and enable GeoClue (the geoclue-2.0 package, with its location service running), then try again.";
    /// Said when GeoClue (or its agent) said no.
    pub const DENIED: &str = "Location access for my-domain was refused: turn on Location Services in your desktop's privacy settings (GNOME: Settings \u{2192} Privacy \u{2192} Location) and allow my-domain. Without a desktop location agent, GeoClue needs my-domain allowed in /etc/geoclue/geoclue.conf.";

    /// A `Location`'s `Timestamp` (seconds, microseconds since the epoch) as
    /// whole unix seconds; `None` for GeoClue's "unknown" (0, 0).
    pub fn timestamp((secs, usecs): (u64, u64)) -> Option<u64> {
        if secs == 0 && usecs == 0 {
            return None;
        }
        Some(secs + usecs / 1_000_000)
    }

    /// The sentence for a failure: `name` is the D-Bus error name when
    /// there is one, `detail` its message (or the transport error).
    pub fn error_text(name: Option<&str>, detail: &str) -> String {
        match name {
            Some(
                "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.SpawnServiceNotFound"
                | "org.freedesktop.DBus.Error.SpawnExecFailed"
                | "org.freedesktop.DBus.Error.SpawnChildExited"
                | "org.freedesktop.DBus.Error.UnknownObject"
                | "org.freedesktop.DBus.Error.UnknownInterface"
                | "org.freedesktop.DBus.Error.UnknownMethod",
            ) => MISSING.into(),
            Some(
                "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.DBus.Error.AuthFailed",
            ) => DENIED.into(),
            Some(_) => format!("Location isn't available on this computer: GeoClue said: {detail}"),
            // No bus at all (or it went away).
            None => format!("{MISSING} ({detail})"),
        }
    }

    /// Whether the bus lists GeoClue as running or startable.
    pub fn listed<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
        names.any(|n| n == SERVICE)
    }
}

/// Windows.Devices.Geolocation: the parts that don't need Windows.
pub mod winloc {
    use std::time::Duration;

    /// HRESULT E_ACCESSDENIED: the app (or location altogether) is off in
    /// the privacy settings.
    pub const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
    /// HRESULT_FROM_WIN32(ERROR_TIMEOUT): no position within the timeout.
    pub const E_TIMEOUT: i32 = 0x8007_05B4_u32 as i32;
    /// E_ABORT: the request was cancelled (also seen on a timeout).
    pub const E_ABORT: i32 = 0x8000_4004_u32 as i32;

    /// GeolocationAccessStatus values.
    pub const ACCESS_UNSPECIFIED: i32 = 0;
    pub const ACCESS_ALLOWED: i32 = 1;
    pub const ACCESS_DENIED: i32 = 2;

    /// Said when Windows' privacy settings keep the location from us.
    pub const DENIED: &str = "Location access for my-domain is off: turn on Location services and \u{201c}Let desktop apps access your location\u{201d} in Windows Settings \u{2192} Privacy & security \u{2192} Location.";

    /// Seconds between 1601-01-01 (FILETIME) and 1970-01-01.
    const EPOCH_DIFF_SECS: i64 = 11_644_473_600;
    const TICKS_PER_SEC: i64 = 10_000_000;

    /// A `DateTime.UniversalTime` (100 ns ticks since 1601-01-01 UTC) as unix
    /// seconds (0 for anything before 1970).
    pub fn unix_secs(universal_time: i64) -> u64 {
        let secs = universal_time.div_euclid(TICKS_PER_SEC) - EPOCH_DIFF_SECS;
        u64::try_from(secs).unwrap_or(0)
    }

    /// A duration as a `TimeSpan` (100 ns ticks).
    pub fn ticks(d: Duration) -> i64 {
        i64::try_from(d.as_nanos() / 100).unwrap_or(i64::MAX)
    }

    /// The sentence for what `RequestAccessAsync` said, if it's a no.
    /// Unspecified (e.g. not asked from a packaged app) is left to the
    /// position request itself, which fails with E_ACCESSDENIED if need be.
    pub fn access_text(status: i32) -> Option<String> {
        (status == ACCESS_DENIED).then(|| DENIED.to_string())
    }

    /// The sentence for a failed call, from its HRESULT and message.
    pub fn error_text(hresult: i32, message: &str) -> String {
        match hresult {
            E_ACCESSDENIED => DENIED.into(),
            E_TIMEOUT | E_ABORT => super::NO_FIX_YET.into(),
            _ => {
                let message = message.trim();
                if message.is_empty() {
                    format!(
                        "Windows couldn't tell where this computer is (0x{:08X}).",
                        hresult as u32
                    )
                } else {
                    format!("Windows couldn't tell where this computer is: {message}")
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use zbus::blocking::{fdo::DBusProxy, Connection, Proxy};
    use zbus::proxy::CacheProperties;
    use zbus::zvariant::OwnedObjectPath;
    use zbus::DBusError;

    use super::geoclue::{self, CLIENT, LOCATION, MANAGER, MANAGER_PATH, SERVICE};
    use super::{Fix, FIX_WAIT, NO_FIX_YET};

    /// The last check, reused for a minute (it's asked on every status).
    static CHECKED: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

    pub fn available() -> bool {
        let Ok(mut checked) = CHECKED.lock() else {
            return false;
        };
        if let Some((at, ok)) = *checked {
            if at.elapsed() < Duration::from_secs(60) {
                return ok;
            }
        }
        let ok = reachable();
        *checked = Some((Instant::now(), ok));
        ok
    }

    /// GeoClue is on the system bus, or the bus can start it.
    fn reachable() -> bool {
        let Ok(conn) = Connection::system() else {
            return false;
        };
        let Ok(dbus) = DBusProxy::new(&conn) else {
            return false;
        };
        if let Ok(name) = SERVICE.try_into() {
            if dbus.name_has_owner(name).unwrap_or(false) {
                return true;
            }
        }
        dbus.list_activatable_names()
            .map(|names| geoclue::listed(names.iter().map(|n| n.as_str())))
            .unwrap_or(false)
    }

    /// A zbus error as the sentence for the notification.
    fn text(e: zbus::Error) -> String {
        match &e {
            zbus::Error::MethodError(name, detail, _) => {
                geoclue::error_text(Some(name.as_str()), detail.as_deref().unwrap_or(""))
            }
            zbus::Error::FDO(f) => {
                geoclue::error_text(Some(f.name().as_str()), f.description().unwrap_or(""))
            }
            other => geoclue::error_text(None, &other.to_string()),
        }
    }

    fn proxy<'a>(
        conn: &Connection,
        path: OwnedObjectPath,
        iface: &'static str,
    ) -> zbus::Result<Proxy<'a>> {
        zbus::blocking::proxy::Builder::<Proxy>::new(conn)
            .destination(SERVICE)?
            .path(path)?
            .interface(iface)?
            .cache_properties(CacheProperties::No)
            .build()
    }

    pub fn fix(desktop_id: &str) -> Result<Fix, String> {
        let conn = Connection::system().map_err(text)?;
        let manager_path = OwnedObjectPath::try_from(MANAGER_PATH).map_err(|e| text(e.into()))?;
        let manager = proxy(&conn, manager_path, MANAGER).map_err(text)?;
        let client_path: OwnedObjectPath = manager.call("GetClient", &()).map_err(text)?;
        let client = proxy(&conn, client_path.clone(), CLIENT).map_err(text)?;
        let result = locate(&conn, &client, desktop_id);
        // Stop, and let GeoClue drop the client (it would anyway once this
        // connection closes).
        let _ = client.call_method("Stop", &());
        let _ = manager.call_method("DeleteClient", &(&client_path,));
        result
    }

    fn locate(conn: &Connection, client: &Proxy<'_>, desktop_id: &str) -> Result<Fix, String> {
        client
            .set_property("DesktopId", desktop_id)
            .map_err(|e| text(e.into()))?;
        client
            .set_property("RequestedAccuracyLevel", geoclue::ACCURACY_EXACT)
            .map_err(|e| text(e.into()))?;
        // The agent asks the user here (or refuses: AccessDenied).
        client.call_method("Start", &()).map_err(text)?;
        // `LocationUpdated` sets the client's `Location` (to "/" until the
        // first one): watch that rather than the signal, with a deadline.
        let start = Instant::now();
        while start.elapsed() < FIX_WAIT {
            let at: OwnedObjectPath = client.get_property("Location").map_err(text)?;
            if at.as_str() != "/" {
                let loc = proxy(conn, at, LOCATION).map_err(text)?;
                let lat: f64 = loc.get_property("Latitude").map_err(text)?;
                let lon: f64 = loc.get_property("Longitude").map_err(text)?;
                let acc: f64 = loc.get_property("Accuracy").map_err(text)?;
                let ts: (u64, u64) = loc.get_property("Timestamp").map_err(text)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                return Ok((lat, lon, acc, geoclue::timestamp(ts).unwrap_or(now)));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        Err(NO_FIX_YET.into())
    }
}

#[cfg(windows)]
mod windows_loc {
    use windows::Devices::Geolocation::{Geolocator, PositionAccuracy};
    use windows::Foundation::TimeSpan;

    use super::winloc;
    use super::{Fix, FIX_WAIT};

    /// A position up to this old is good enough.
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30);

    fn text(e: &windows::core::Error) -> String {
        winloc::error_text(e.code().0, &e.message())
    }

    pub fn fix() -> Result<Fix, String> {
        // Asks the first time where Windows does that; a desktop app may get
        // "unspecified" or an error here, in which case the position request
        // below says whether it's allowed.
        if let Ok(status) = Geolocator::RequestAccessAsync().and_then(|op| op.get()) {
            if let Some(why) = winloc::access_text(status.0) {
                return Err(why);
            }
        }
        let locator = Geolocator::new().map_err(|e| text(&e))?;
        locator
            .SetDesiredAccuracy(PositionAccuracy::High)
            .map_err(|e| text(&e))?;
        let position = locator
            .GetGeopositionAsyncWithAgeAndTimeout(
                TimeSpan {
                    Duration: winloc::ticks(MAX_AGE),
                },
                TimeSpan {
                    Duration: winloc::ticks(FIX_WAIT),
                },
            )
            .and_then(|op| op.get())
            .map_err(|e| text(&e))?;
        let coord = position.Coordinate().map_err(|e| text(&e))?;
        let point = coord
            .Point()
            .and_then(|p| p.Position())
            .map_err(|e| text(&e))?;
        let acc = coord.Accuracy().map_err(|e| text(&e))?;
        let at = coord.Timestamp().map_err(|e| text(&e))?;
        Ok((
            point.Latitude,
            point.Longitude,
            acc,
            winloc::unix_secs(at.UniversalTime),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geoclue_timestamp() {
        assert_eq!(geoclue::timestamp((1_700_000_000, 0)), Some(1_700_000_000));
        assert_eq!(
            geoclue::timestamp((1_700_000_000, 999_999)),
            Some(1_700_000_000)
        );
        assert_eq!(
            geoclue::timestamp((1_700_000_000, 1_500_000)),
            Some(1_700_000_001)
        );
        assert_eq!(geoclue::timestamp((0, 0)), None);
    }

    #[test]
    fn geoclue_errors() {
        assert_eq!(
            geoclue::error_text(Some("org.freedesktop.DBus.Error.ServiceUnknown"), "x"),
            geoclue::MISSING
        );
        assert_eq!(
            geoclue::error_text(
                Some("org.freedesktop.DBus.Error.AccessDenied"),
                "no agent for UID 1000"
            ),
            geoclue::DENIED
        );
        let other = geoclue::error_text(Some("org.freedesktop.DBus.Error.Failed"), "boom");
        assert!(other.ends_with("GeoClue said: boom"), "{other}");
        let nobus = geoclue::error_text(None, "no such file");
        assert!(
            nobus.starts_with(geoclue::MISSING) && nobus.contains("no such file"),
            "{nobus}"
        );
        assert!(geoclue::MISSING.contains("GeoClue"));
    }

    #[test]
    fn geoclue_listed() {
        assert!(geoclue::listed(
            ["org.freedesktop.DBus", "org.freedesktop.GeoClue2"].into_iter()
        ));
        assert!(!geoclue::listed(["org.freedesktop.GeoClue"].into_iter()));
        assert!(!geoclue::listed(std::iter::empty()));
    }

    #[test]
    fn windows_time() {
        // 1970-01-01 in FILETIME ticks.
        assert_eq!(winloc::unix_secs(116_444_736_000_000_000), 0);
        assert_eq!(
            winloc::unix_secs(116_444_736_000_000_000 + 15 * 10_000_000 + 9_999_999),
            15
        );
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(winloc::unix_secs(133_444_736_000_000_000), 1_700_000_000);
        assert_eq!(winloc::unix_secs(0), 0);
        assert_eq!(winloc::unix_secs(-1), 0);
        assert_eq!(
            winloc::ticks(std::time::Duration::from_secs(15)),
            150_000_000
        );
        assert_eq!(winloc::ticks(std::time::Duration::from_micros(1)), 10);
    }

    #[test]
    fn windows_errors() {
        assert_eq!(
            winloc::error_text(winloc::E_ACCESSDENIED, "Access is denied."),
            winloc::DENIED
        );
        assert!(winloc::DENIED.contains("Privacy & security \u{2192} Location"));
        assert_eq!(winloc::error_text(winloc::E_TIMEOUT, ""), NO_FIX_YET);
        assert_eq!(winloc::error_text(winloc::E_ABORT, ""), NO_FIX_YET);
        assert_eq!(
            winloc::error_text(0x8000_4005_u32 as i32, "Unspecified error \r\n"),
            "Windows couldn't tell where this computer is: Unspecified error"
        );
        assert_eq!(
            winloc::error_text(0x8000_4005_u32 as i32, ""),
            "Windows couldn't tell where this computer is (0x80004005)."
        );
        assert_eq!(
            winloc::access_text(winloc::ACCESS_DENIED).as_deref(),
            Some(winloc::DENIED)
        );
        assert_eq!(winloc::access_text(winloc::ACCESS_ALLOWED), None);
        assert_eq!(winloc::access_text(winloc::ACCESS_UNSPECIFIED), None);
    }

    #[test]
    fn unsupported_elsewhere() {
        if cfg!(not(any(target_os = "linux", windows))) {
            assert!(!available());
            assert!(fix("my-domain").is_err());
        }
    }
}
