// "Show my phone's battery in the menu bar" (Settings, off by default): the
// phone's battery next to the menu-bar / tray icon, at a glance.
//
//  - macOS and Linux: the icon's title ("82%", "⚡82%" while charging);
//    Linux shows it as the indicator's label (the tooltip isn't shown
//    there). Windows trays have no title: the tooltip says it
//    ("my-domain · Pixel 8: 82%, charging"); the tooltip says it on the
//    others too;
//  - which phone: the one chosen in Settings, or (none chosen, or it's gone)
//    the one heard from most recently;
//  - low (≤ `LOW` % and not charging): with "Show a red dot when it's low"
//    on (the default), the icon gets a small red dot, drawn onto the app's
//    icon here (`badge`);
//  - from the phone's status (`battery`, `charging`), as it comes
//    (`on_status`); a phone that's been disconnected for more than
//    `STALE_SECS` stops showing (a minute's tick and link changes notice);
//  - kept in `<app config dir>/prefs.json`: `tray_battery` and
//    `tray_battery_badge` ("on"/"off"), `tray_battery_phone` (a node id, or
//    nothing for "whichever").

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::image::Image;
use tauri::{AppHandle, Manager, State};

use crate::{notify, Eng};

const KEY_ON: &str = "tray_battery";
const KEY_BADGE: &str = "tray_battery_badge";
const KEY_PHONE: &str = "tray_battery_phone";
/// At or below this (and not charging) the battery is low.
pub const LOW: u8 = 20;
/// A phone disconnected for longer than this stops showing.
pub const STALE_SECS: u64 = 30 * 60;
/// How often staleness is checked.
const TICK: Duration = Duration::from_secs(60);
/// The badge: red, ringed in white so it shows on dark and light bars.
const RED: [u8; 4] = [229, 57, 53, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A phone's latest battery reading.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    level: u8,
    charging: bool,
    /// When it was last heard from or seen connected (unix seconds).
    seen: u64,
}

/// What the tray shows, when it shows anything.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    title: String,
    tip: String,
    /// The red dot.
    low: bool,
}

/// The latest readings (node id -> reading).
static READINGS: Mutex<BTreeMap<String, Reading>> = Mutex::new(BTreeMap::new());
/// What's on the tray now (`None`: nothing).
static SHOWING: Mutex<Option<Shown>> = Mutex::new(None);
/// The app's icon with the red dot, made once.
static BADGED: OnceLock<Option<Image<'static>>> = OnceLock::new();

/// The Settings switches.
pub struct TrayBatteryPref {
    kv: Option<FileKv>,
    on: AtomicBool,
    badge: AtomicBool,
    phone: Mutex<String>,
}

impl TrayBatteryPref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
        TrayBatteryPref {
            on: AtomicBool::new(get(KEY_ON).as_deref() == Some("on")),
            badge: AtomicBool::new(get(KEY_BADGE).as_deref() != Some("off")),
            phone: Mutex::new(get(KEY_PHONE).unwrap_or_default()),
            kv,
        }
    }

    fn phone(&self) -> String {
        self.phone.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn save(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                eprintln!("[tray] couldn't save the battery setting: {e}");
            }
        }
    }
}

/// "82%", or "⚡82%" while charging.
fn title(level: u8, charging: bool) -> String {
    if charging {
        format!("\u{26a1}{level}%")
    } else {
        format!("{level}%")
    }
}

/// Low: at or below `LOW` and not charging.
fn is_low(level: u8, charging: bool) -> bool {
    level <= LOW && !charging
}

/// Whether a reading still shows: the phone is connected, or was within
/// `STALE_SECS`.
fn fresh(r: &Reading, connected: bool, now: u64) -> bool {
    connected || now.saturating_sub(r.seen) <= STALE_SECS
}

/// The tooltip's line: "Pixel 8: 82%", with ", charging" or ", low".
fn tip(name: &str, r: &Reading) -> String {
    let how = if r.charging {
        ", charging"
    } else if is_low(r.level, r.charging) {
        ", low"
    } else {
        ""
    };
    format!("{name}: {}%{how}", r.level)
}

/// Which phone shows: the chosen one if it has a reading, else the one
/// heard from most recently.
fn pick<'a>(
    chosen: &str,
    readings: &'a BTreeMap<String, Reading>,
) -> Option<(&'a str, &'a Reading)> {
    if let Some((node, r)) = readings.get_key_value(chosen) {
        return Some((node.as_str(), r));
    }
    readings
        .iter()
        .max_by_key(|(_, r)| r.seen)
        .map(|(n, r)| (n.as_str(), r))
}

/// What to show for the readings, or nothing.
fn shown(
    chosen: &str,
    badge: bool,
    readings: &BTreeMap<String, Reading>,
    live: &[String],
    now: u64,
    name: impl Fn(&str) -> String,
) -> Option<Shown> {
    let (node, r) = pick(chosen, readings)?;
    let connected = live.iter().any(|l| l == node);
    if !fresh(r, connected, now) {
        return None;
    }
    Some(Shown {
        title: title(r.level, r.charging),
        tip: tip(&name(node), r),
        low: badge && is_low(r.level, r.charging),
    })
}

/// `rgba` (`w` × `h`) with a red dot, ringed in white, in its top-right
/// corner.
pub fn badge(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = rgba.to_vec();
    let (wf, hf) = (w as f32, h as f32);
    let r = (wf.min(hf) * 0.2).max(2.0);
    let ring = (r / 4.0).max(1.0);
    let (cx, cy) = (wf - r - ring, r + ring);
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            let colour = if d <= r {
                RED
            } else if d <= r + ring {
                WHITE
            } else {
                continue;
            };
            let i = ((y * w + x) * 4) as usize;
            if let Some(px) = out.get_mut(i..i + 4) {
                px.copy_from_slice(&colour);
            }
        }
    }
    out
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A device's status arrived: a phone's battery is noted, and the tray
/// follows.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    if status.get("platform").and_then(|p| p.as_str()) != Some("android") {
        return;
    }
    let Some(level) = status.get("battery").and_then(|b| b.as_u64()) else {
        return;
    };
    let charging = status.get("charging").and_then(|c| c.as_bool()) == Some(true);
    if let Ok(mut r) = READINGS.lock() {
        r.insert(
            from.to_string(),
            Reading {
                level: level.min(100) as u8,
                charging,
                seen: now(),
            },
        );
    }
    update(app);
}

/// The tip the tooltip adds, while something shows.
pub fn tip_line() -> Option<String> {
    SHOWING.lock().ok()?.as_ref().map(|s| s.tip.clone())
}

/// Work out what the tray should show and change it if that's different.
/// Also called on link changes and every `TICK`.
pub fn update(app: &AppHandle) {
    let Some(pref) = app.try_state::<TrayBatteryPref>() else {
        return;
    };
    let next = if pref.on.load(Ordering::Relaxed) {
        let live: Vec<String> = app
            .try_state::<Eng>()
            .map(|e| e.links().into_iter().map(|(id, _)| id).collect())
            .unwrap_or_default();
        let now = now();
        let readings = match READINGS.lock() {
            Ok(mut r) => {
                // Connected now counts as seen now.
                for (node, reading) in r.iter_mut() {
                    if live.contains(node) {
                        reading.seen = now;
                    }
                }
                r.clone()
            }
            Err(_) => BTreeMap::new(),
        };
        shown(
            &pref.phone(),
            pref.badge.load(Ordering::Relaxed),
            &readings,
            &live,
            now,
            |n| notify::device_name(app, n),
        )
    } else {
        None
    };
    let Ok(mut showing) = SHOWING.lock() else {
        return;
    };
    if *showing == next {
        return;
    }
    let low_changed =
        showing.as_ref().is_some_and(|s| s.low) != next.as_ref().is_some_and(|s| s.low);
    let title = next.as_ref().map(|s| s.title.clone());
    let low = next.as_ref().is_some_and(|s| s.low);
    *showing = next;
    drop(showing);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id(crate::tray::TRAY_ID) else {
            return;
        };
        // Windows has no title (the tooltip says it there).
        if let Err(e) = tray.set_title(title.as_deref()) {
            eprintln!("[tray] couldn't show the battery: {e}");
        }
        if low_changed {
            let icon = if low {
                badged(&handle)
            } else {
                handle.default_window_icon().cloned()
            };
            if let Some(icon) = icon {
                if let Err(e) = tray.set_icon(Some(icon)) {
                    eprintln!("[tray] couldn't change the icon: {e}");
                }
            }
        }
    });
    crate::tray::refresh_tooltip(app);
}

/// The app's icon with the red dot (made the first time it's needed).
fn badged(app: &AppHandle) -> Option<Image<'static>> {
    BADGED
        .get_or_init(|| {
            let icon = app.default_window_icon()?;
            let (w, h) = (icon.width(), icon.height());
            Some(Image::new_owned(badge(icon.rgba(), w, h), w, h))
        })
        .clone()
}

/// Notice phones going stale (disconnected for a while).
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("tray-battery".into())
        .spawn(move || loop {
            std::thread::sleep(TICK);
            update(&app);
        })
        .ok();
}

/// A phone the setting can pick.
#[derive(Serialize, specta::Type)]
pub struct TrayPhone {
    pub node: String,
    pub name: String,
}

/// What Settings shows.
#[derive(Serialize, specta::Type)]
pub struct TrayBatterySetting {
    pub on: bool,
    /// The red dot when it's low.
    pub badge: bool,
    /// The chosen phone ("" for whichever was heard from last).
    pub phone: String,
    /// Your phones.
    pub phones: Vec<TrayPhone>,
}

/// The menu-bar battery setting, and your phones to pick from.
#[tauri::command]
#[specta::specta]
pub fn tray_battery_setting(app: AppHandle, pref: State<TrayBatteryPref>) -> TrayBatterySetting {
    let mut nodes = crate::shortcut::known_phones(&app);
    if let Ok(r) = READINGS.lock() {
        nodes.extend(r.keys().cloned());
    }
    nodes.sort();
    nodes.dedup();
    let mut phones: Vec<TrayPhone> = nodes
        .into_iter()
        .map(|node| TrayPhone {
            name: notify::device_name(&app, &node),
            node,
        })
        .collect();
    phones.sort_by_key(|p| p.name.to_lowercase());
    TrayBatterySetting {
        on: pref.on.load(Ordering::Relaxed),
        badge: pref.badge.load(Ordering::Relaxed),
        phone: pref.phone(),
        phones,
    }
}

/// Change the menu-bar battery setting; the tray follows at once.
#[tauri::command]
#[specta::specta]
pub fn set_tray_battery(
    on: bool,
    badge: bool,
    phone: String,
    app: AppHandle,
    pref: State<TrayBatteryPref>,
) {
    pref.on.store(on, Ordering::Relaxed);
    pref.badge.store(badge, Ordering::Relaxed);
    pref.save(KEY_ON, if on { "on" } else { "off" });
    pref.save(KEY_BADGE, if badge { "on" } else { "off" });
    let phone = phone.trim().to_string();
    pref.save(KEY_PHONE, &phone);
    if let Ok(mut p) = pref.phone.lock() {
        *p = phone;
    }
    update(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading(level: u8, charging: bool, seen: u64) -> Reading {
        Reading {
            level,
            charging,
            seen,
        }
    }

    #[test]
    fn tray_battery_titles() {
        assert_eq!(title(82, false), "82%");
        assert_eq!(title(82, true), "\u{26a1}82%");
        assert_eq!(title(100, true), "\u{26a1}100%");
        assert_eq!(title(5, false), "5%");
    }

    #[test]
    fn tray_battery_low() {
        assert!(is_low(20, false));
        assert!(is_low(3, false));
        assert!(!is_low(21, false));
        // Charging isn't low, however empty.
        assert!(!is_low(5, true));
    }

    #[test]
    fn tray_battery_tips() {
        assert_eq!(tip("Pixel", &reading(82, false, 0)), "Pixel: 82%");
        assert_eq!(tip("Pixel", &reading(82, true, 0)), "Pixel: 82%, charging");
        assert_eq!(tip("Pixel", &reading(12, false, 0)), "Pixel: 12%, low");
        assert_eq!(tip("Pixel", &reading(12, true, 0)), "Pixel: 12%, charging");
    }

    #[test]
    fn tray_battery_goes_stale_after_half_an_hour_away() {
        let r = reading(50, false, 1_000);
        assert!(fresh(&r, false, 1_000 + STALE_SECS));
        assert!(!fresh(&r, false, 1_000 + STALE_SECS + 1));
        // Connected is never stale.
        assert!(fresh(&r, true, 1_000 + 10 * STALE_SECS));
        // A clock set back doesn't hide it.
        assert!(fresh(&r, false, 0));
    }

    #[test]
    fn tray_battery_which_phone() {
        let mut m = BTreeMap::new();
        m.insert("a".to_string(), reading(10, false, 100));
        m.insert("b".to_string(), reading(90, true, 200));
        assert_eq!(pick("a", &m).map(|(n, _)| n), Some("a"));
        // None chosen, or the chosen one has no reading: the latest heard.
        assert_eq!(pick("", &m).map(|(n, _)| n), Some("b"));
        assert_eq!(pick("gone", &m).map(|(n, _)| n), Some("b"));
        assert_eq!(pick("", &BTreeMap::new()).map(|(n, _)| n), None);
    }

    #[test]
    fn tray_battery_what_shows() {
        let mut m = BTreeMap::new();
        m.insert("p".to_string(), reading(15, false, 1_000));
        let name = |_: &str| "Pixel".to_string();
        let live = vec!["p".to_string()];
        let s = shown("", true, &m, &live, 1_000, name).unwrap();
        assert_eq!(s.title, "15%");
        assert_eq!(s.tip, "Pixel: 15%, low");
        assert!(s.low);
        // The dot only with its switch on.
        assert!(!shown("", false, &m, &live, 1_000, name).unwrap().low);
        // Disconnected for over half an hour: nothing.
        assert_eq!(shown("", true, &m, &[], 1_000 + STALE_SECS + 1, name), None);
        assert!(shown("", true, &m, &[], 1_000 + STALE_SECS, name).is_some());
        // Charging: no dot.
        m.insert("p".to_string(), reading(15, true, 1_000));
        let s = shown("p", true, &m, &live, 1_000, name).unwrap();
        assert_eq!(s.title, "\u{26a1}15%");
        assert!(!s.low);
        assert_eq!(shown("", true, &BTreeMap::new(), &live, 0, name), None);
    }

    #[test]
    fn tray_battery_badge_pixels() {
        let (w, h) = (32u32, 32u32);
        let grey = [10u8, 20, 30, 40];
        let src: Vec<u8> = grey
            .iter()
            .copied()
            .cycle()
            .take((w * h * 4) as usize)
            .collect();
        let out = badge(&src, w, h);
        assert_eq!(out.len(), src.len());
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [out[i], out[i + 1], out[i + 2], out[i + 3]]
        };
        // r = 6.4, ring = 1.6: the dot's centre is at (24, 8).
        assert_eq!(px(24, 8), RED);
        assert_eq!(px(24, 1), WHITE, "the ring above the dot");
        // The rest of the icon is untouched.
        assert_eq!(px(0, 0), grey);
        assert_eq!(px(0, h - 1), grey);
        assert_eq!(px(w - 1, h - 1), grey);
        assert_eq!(px(16, 16), grey);
        // Only the top-right corner changed.
        for y in 0..h {
            for x in 0..w {
                if px(x, y) != grey {
                    assert!(x >= w / 2 && y < h / 2, "({x}, {y})");
                }
            }
        }
        // A tiny icon still gets a dot, and nothing panics.
        let tiny = badge(&[0u8; 4 * 4 * 4], 4, 4);
        assert!(tiny.chunks(4).any(|p| p == RED));
    }
}
