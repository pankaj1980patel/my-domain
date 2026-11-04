// Where this computer is, for your phone (find my computer): the `locate`
// action puts `location {lat, lon, acc, at}` in this computer's status, the
// same shape the phone uses (apps/android PhoneLocation.kt).
//
// This computer stays in charge, as the phone does:
//  - off until "Let my phone see where this computer is" is turned on in
//    Settings (`<app config dir>/prefs.json`, key `share_location`); asked
//    while off, a notification says so and nothing is shared;
//  - a notification each time it's shared;
//  - the OS's own location permission still applies (macOS asks once);
//  - only on request, never in the background.
//
// macOS only for now (CoreLocation); elsewhere `available()` is false and
// the action isn't listed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use mdcore::kv::FileKv;
use tauri::{AppHandle, Manager};

const KEY: &str = "share_location";

/// The "Let my phone see where this computer is" switch, and the last
/// position shared.
pub struct LocationPref {
    kv: Option<FileKv>,
    on: AtomicBool,
    last: Mutex<Option<serde_json::Value>>,
}

impl LocationPref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let on = kv.as_ref().and_then(|kv| kv.get(KEY)).as_deref() == Some("on");
        LocationPref {
            kv,
            on: AtomicBool::new(on),
            last: Mutex::new(None),
        }
    }

    pub fn allowed(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    pub fn set_allowed(&self, on: bool) {
        self.on.store(on, Ordering::Relaxed);
        if !on {
            if let Ok(mut last) = self.last.lock() {
                *last = None;
            }
        }
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
                eprintln!("[location] couldn't save the setting: {e}");
            }
        }
    }
}

/// This computer can tell where it is at all.
pub fn available() -> bool {
    cfg!(target_os = "macos")
}

/// Sharing state and the last position, into the status.
pub fn extend(app: &AppHandle, status: &mut serde_json::Value) {
    let Some(pref) = app.try_state::<LocationPref>() else {
        return;
    };
    if !pref.allowed() {
        return;
    }
    status["location_sharing"] = true.into();
    if let Some(l) = pref.last.lock().ok().and_then(|l| l.clone()) {
        status["location"] = l;
    }
}

/// A sibling asked where this computer is: find out (off this thread) and
/// push it in the status; `done` runs afterwards (to push the status).
pub fn locate(app: &AppHandle, done: impl FnOnce(&AppHandle) + Send + 'static) {
    let allowed = app.try_state::<LocationPref>().is_some_and(|p| p.allowed());
    if !allowed {
        crate::notify::post(
            app,
            "Your phone asked where this computer is",
            "Turn on \u{201c}Let my phone see where this computer is\u{201d} in my-domain\u{2019}s Settings to share it.",
        );
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || match fix(&app) {
        Ok((lat, lon, acc, at)) => {
            if let Some(pref) = app.try_state::<LocationPref>() {
                if !pref.allowed() {
                    return;
                }
                if let Ok(mut last) = pref.last.lock() {
                    *last =
                        Some(serde_json::json!({ "lat": lat, "lon": lon, "acc": acc, "at": at }));
                }
            }
            crate::notify::post(
                &app,
                "Location shared with your phone",
                "You let your phone see where this computer is. Change it in my-domain\u{2019}s Settings.",
            );
            done(&app);
        }
        Err(why) => {
            eprintln!("[location] {why}");
            crate::notify::post(&app, "my-domain can't tell where this computer is", &why);
        }
    });
}

/// A position: (lat, lon, accuracy in m, unix seconds).
#[cfg(target_os = "macos")]
type Fix = (f64, f64, f64, f64);

/// How long to wait for a fresh position before settling for the last one.
#[cfg(target_os = "macos")]
const FIX_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// (lat, lon, accuracy in m, unix seconds) from CoreLocation: started on
/// the main thread (it delivers there), polled from here.
#[cfg(target_os = "macos")]
fn fix(app: &AppHandle) -> Result<(f64, f64, f64, u64), String> {
    let on_main = |f: fn() -> Result<Option<Fix>, String>| {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| e.to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "no answer from the location service".to_string())?
    };
    on_main(|| mac::start().map(|()| None))?;
    let asked = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let start = std::time::Instant::now();
    let mut best = None;
    while start.elapsed() < FIX_WAIT {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(l) = on_main(mac::read)? {
            let fresh = l.3 >= asked - 1.0;
            best = Some(l);
            if fresh {
                break;
            }
        }
    }
    let _ = on_main(|| {
        mac::stop();
        Ok(None)
    });
    let (lat, lon, acc, at) = best.ok_or("no position yet; try again in a moment")?;
    Ok((lat, lon, acc, at.max(0.0) as u64))
}

#[cfg(not(target_os = "macos"))]
fn fix(_app: &AppHandle) -> Result<(f64, f64, f64, u64), String> {
    Err("finding this computer's location isn't supported here yet".into())
}

/// CoreLocation, used only on the main thread (where it delivers updates).
#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};

    thread_local! {
        static MANAGER: RefCell<Option<Retained<CLLocationManager>>> = const { RefCell::new(None) };
    }

    const DENIED: &str = "Location access for my-domain is off: turn it on in System Settings \u{2192} Privacy & Security \u{2192} Location Services.";

    /// Start updates (asking for permission the first time). Main thread.
    pub fn start() -> Result<(), String> {
        // SAFETY: CoreLocation calls on the main thread, with a manager kept
        // alive in this thread's slot.
        unsafe {
            if !CLLocationManager::locationServicesEnabled_class() {
                return Err("Location Services are off on this computer: turn them on in System Settings \u{2192} Privacy & Security.".into());
            }
            MANAGER.with(|m| {
                let mut m = m.borrow_mut();
                let mgr = m.get_or_insert_with(|| CLLocationManager::new());
                let status = mgr.authorizationStatus();
                if status == CLAuthorizationStatus::Denied
                    || status == CLAuthorizationStatus::Restricted
                {
                    return Err(DENIED.to_string());
                }
                if status == CLAuthorizationStatus::NotDetermined {
                    mgr.requestWhenInUseAuthorization();
                }
                mgr.startUpdatingLocation();
                Ok(())
            })
        }
    }

    /// The newest position: (lat, lon, accuracy m, unix seconds), if any
    /// yet; Err once the user said no to the permission prompt. Main thread.
    pub fn read() -> Result<Option<super::Fix>, String> {
        MANAGER.with(|m| {
            let m = m.borrow();
            let Some(mgr) = m.as_ref() else {
                return Ok(None);
            };
            // SAFETY: as in `start`.
            unsafe {
                let status = mgr.authorizationStatus();
                if status == CLAuthorizationStatus::Denied
                    || status == CLAuthorizationStatus::Restricted
                {
                    return Err(DENIED.to_string());
                }
                Ok(mgr.location().map(|loc| {
                    let c = loc.coordinate();
                    (
                        c.latitude,
                        c.longitude,
                        loc.horizontalAccuracy(),
                        loc.timestamp().timeIntervalSince1970(),
                    )
                }))
            }
        })
    }

    /// Stop updates. Main thread.
    pub fn stop() {
        MANAGER.with(|m| {
            if let Some(mgr) = m.borrow().as_ref() {
                // SAFETY: as in `start`.
                unsafe { mgr.stopUpdatingLocation() };
            }
        });
    }
}
