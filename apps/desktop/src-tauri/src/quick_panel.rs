// The quick panel: a small borderless window that drops from the menu-bar
// (macOS) / system-tray icon, with your devices and a few buttons each,
// the last clips and transfers, and a way into the main window.
//
//  - left click on the icon opens it, anchored to the icon (and closes it
//    when it's open); right click keeps the tray menu. That's the usual
//    menu-bar convention on macOS too (Dropbox, 1Password and the like: the
//    popover on a click, the menu on a right / Control click) and the
//    taskbar one on Windows;
//  - on Linux the tray (AppIndicator) reports no clicks: a click always
//    shows the menu. Its first item, "Quick panel", opens the panel there
//    (near the pointer where the desktop says where it is, else the
//    bottom-right corner). The item is on every OS's menu;
//  - it hides when it loses focus, or on Esc (the UI calls
//    `quick_panel_hide`); it's created once and kept, hidden;
//  - "Open the quick panel when clicking the menu-bar icon" in Settings
//    turns the left click back into the menu; on by default. In
//    `<app config dir>/prefs.json`, key `quick_panel` ("on" / "off").
//
// The window is labelled `quick-panel`; the UI bundle draws the panel
// instead of the app when it finds itself in that window.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewUrl};

use crate::clip_history::ClipRecord;

/// The panel window's label.
pub const LABEL: &str = "quick-panel";
const PREF_KEY: &str = "quick_panel";
/// The panel's size, in logical pixels.
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 540.0;
/// The gap kept from the icon and the screen's edges, in logical pixels.
const MARGIN: f64 = 8.0;
/// A click on the icon this soon after the panel hid (by losing focus to
/// that very click) closes it rather than opening it again.
const REOPEN_GRACE_MS: u64 = 350;
/// What the panel hears each time it's shown (it refreshes then).
const SHOWN_EVENT: &str = "quick-panel-shown";
/// What the main window hears from the panel's footer.
const OPEN_EVENT: &str = "quick-open";
/// The most recent clips the panel asks for.
const MAX_CLIPS: u32 = 10;

/// When the panel last hid, in ms since 1970.
static HIDDEN_AT: AtomicU64 = AtomicU64::new(0);

/// The Settings switch.
pub struct QuickPanelPref {
    kv: Option<FileKv>,
    on: AtomicBool,
}

impl QuickPanelPref {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let on = kv.as_ref().and_then(|kv| kv.get(PREF_KEY)).as_deref() != Some("off");
        QuickPanelPref {
            kv,
            on: AtomicBool::new(on),
        }
    }
}

/// Whether a left click on the icon opens the panel (else the menu).
pub fn enabled(app: &AppHandle) -> bool {
    app.try_state::<QuickPanelPref>()
        .is_none_or(|p| p.on.load(Ordering::Relaxed))
}

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// What the panel is placed next to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    /// The tray icon's rectangle (macOS, Windows).
    Icon(Area),
    /// Where the pointer is (no icon rectangle: Linux).
    Point(f64, f64),
    /// Nothing known: the work area's bottom-right corner.
    Corner,
}

/// Where the panel's top-left corner goes (physical pixels): next to the
/// icon on the side facing the screen (below a top menu bar, above a
/// bottom taskbar, beside a side one), centred on it, then kept `margin`
/// inside the work area. A panel bigger than the work area keeps its
/// top-left corner in view.
pub fn place(w: f64, h: f64, anchor: Anchor, work: Area, margin: f64) -> (f64, f64) {
    let (left, top) = (work.x + margin, work.y + margin);
    let right = work.x + work.w - w - margin;
    let bottom = work.y + work.h - h - margin;
    let (x, y) = match anchor {
        Anchor::Corner => (right, bottom),
        Anchor::Point(px, py) => beside(
            w,
            h,
            Area {
                x: px,
                y: py,
                w: 0.0,
                h: 0.0,
            },
            work,
            margin,
        ),
        Anchor::Icon(icon) => beside(w, h, icon, work, margin),
    };
    // `max` last: a panel that doesn't fit stays at the top-left.
    (x.min(right).max(left), y.min(bottom).max(top))
}

/// The unclamped spot next to `icon`.
fn beside(w: f64, h: f64, icon: Area, work: Area, margin: f64) -> (f64, f64) {
    let (cx, cy) = (icon.x + icon.w / 2.0, icon.y + icon.h / 2.0);
    let centred_x = cx - w / 2.0;
    let centred_y = cy - h / 2.0;
    if icon.y + icon.h <= work.y {
        // A top bar (the macOS menu bar, a top panel): below it.
        (centred_x, icon.y + icon.h + margin)
    } else if icon.y >= work.y + work.h {
        // A bottom taskbar: above it.
        (centred_x, icon.y - h - margin)
    } else if icon.x + icon.w <= work.x {
        // A taskbar on the left: to its right.
        (icon.x + icon.w + margin, centred_y)
    } else if icon.x >= work.x + work.w {
        // On the right: to its left.
        (icon.x - w - margin, centred_y)
    } else if cy < work.y + work.h / 2.0 {
        // Inside the work area (a point, an auto-hiding bar): open away
        // from the nearer edge.
        (centred_x, icon.y + icon.h + margin)
    } else {
        (centred_x, icon.y - h - margin)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The icon was clicked (or "Quick panel" chosen): hide the panel if it's
/// showing (or just hid because of this click), else show it at `anchor`.
/// On the main thread (tray and menu events are).
pub fn toggle(app: &AppHandle, anchor: Anchor) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if w.is_visible().unwrap_or(false) {
            hide(app);
            return;
        }
    }
    if now_ms().saturating_sub(HIDDEN_AT.load(Ordering::Relaxed)) < REOPEN_GRACE_MS {
        return;
    }
    show(app, anchor);
}

/// Show the panel at `anchor`, creating it the first time.
pub fn show(app: &AppHandle, anchor: Anchor) {
    let window = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => match tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
            .title("my-domain")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .shadow(true)
            .visible(false)
            .build()
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[quick panel] couldn't open it: {e}");
                return;
            }
        },
    };
    // The screen the icon (or pointer) is on, else the main one.
    let point = match anchor {
        Anchor::Icon(a) => Some((a.x + a.w / 2.0, a.y + a.h / 2.0)),
        Anchor::Point(x, y) => Some((x, y)),
        Anchor::Corner => None,
    };
    let monitor = point
        .and_then(|(x, y)| app.monitor_from_point(x, y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let wa = m.work_area();
        let work = Area {
            x: wa.position.x as f64,
            y: wa.position.y as f64,
            w: wa.size.width as f64,
            h: wa.size.height as f64,
        };
        let (x, y) = place(WIDTH * scale, HEIGHT * scale, anchor, work, MARGIN * scale);
        let _ = window.set_position(PhysicalPosition::new(x.round(), y.round()));
    }
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.emit_to(LABEL, SHOWN_EVENT, ());
}

/// Hide the panel (it stays, ready for the next click).
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        if w.is_visible().unwrap_or(false) {
            HIDDEN_AT.store(now_ms(), Ordering::Relaxed);
            let _ = w.hide();
        }
    }
}

/// The anchor for "Quick panel" from the menu: the icon where the OS says
/// where it is, else the pointer, else the corner.
pub fn menu_anchor(app: &AppHandle, icon: Option<tauri::Rect>) -> Anchor {
    if let Some(r) = icon {
        let p = r.position.to_physical::<f64>(1.0);
        let s = r.size.to_physical::<f64>(1.0);
        if s.width > 0.0 && s.height > 0.0 {
            return Anchor::Icon(Area {
                x: p.x,
                y: p.y,
                w: s.width,
                h: s.height,
            });
        }
    }
    match app.cursor_position() {
        Ok(p) if p.x != 0.0 || p.y != 0.0 => Anchor::Point(p.x, p.y),
        _ => Anchor::Corner,
    }
}

/// Whether a left click on the menu-bar icon opens the quick panel.
#[tauri::command]
#[specta::specta]
pub fn quick_panel_enabled(pref: State<QuickPanelPref>) -> bool {
    pref.on.load(Ordering::Relaxed)
}

/// Turn the quick panel on the icon's left click on or off (off: the menu).
#[tauri::command]
#[specta::specta]
pub fn set_quick_panel_enabled(on: bool, app: AppHandle, pref: State<QuickPanelPref>) {
    pref.on.store(on, Ordering::Relaxed);
    if let Some(kv) = &pref.kv {
        if let Err(e) = kv.set(PREF_KEY, if on { "on" } else { "off" }) {
            eprintln!("[quick panel] couldn't save the setting: {e}");
        }
    }
    crate::tray::set_left_click_menu(&app, !on);
    if !on {
        hide(&app);
    }
}

/// Hide the quick panel (Esc, or after it opened something).
#[tauri::command]
#[specta::specta]
pub fn quick_panel_hide(app: AppHandle) {
    hide(&app);
}

/// The panel's footer: bring the main window forward at `place` ("home",
/// "settings" or "search", with `query`), and hide the panel.
#[tauri::command]
#[specta::specta]
pub fn quick_panel_open_main(place: String, query: Option<String>, app: AppHandle) {
    hide(&app);
    crate::tray::show_window(&app);
    let _ = app.emit_to(
        "main",
        OPEN_EVENT,
        serde_json::json!({ "place": place, "query": query }),
    );
}

/// A device's quick button: "ring", "clipboard" (send this computer's
/// clipboard), "send_file" (pick one; the panel hides for the dialog),
/// "locate" (phones) or "lock" (computers). Done the tray's way: a
/// notification says how it went.
#[tauri::command]
#[specta::specta]
pub fn quick_panel_action(node_id: String, action: String, app: AppHandle) -> Result<(), String> {
    let node = node_id.as_str();
    match action.as_str() {
        "ring" => crate::tray::ring(&app, node),
        "clipboard" => crate::tray::send_clipboard(&app, node),
        "locate" => crate::tray::locate(&app, node),
        "send_file" => {
            hide(&app);
            crate::tray::send_file(&app, node);
        }
        "lock" => {
            let (app, node) = (app.clone(), node_id.clone());
            std::thread::spawn(move || {
                let name = crate::notify::device_name(&app, &node);
                let result = crate::schedule::run_action(&app, &node, "lock", "");
                crate::shortcuts::sent(&app, &format!("Lock {name}"), &name, result);
            });
        }
        other => return Err(format!("unknown action: {other}")),
    }
    Ok(())
}

/// The last `limit` clips (sent and received, any device), newest first.
#[tauri::command]
#[specta::specta]
pub fn quick_panel_clips(limit: u32, app: AppHandle) -> Vec<ClipRecord> {
    crate::clip_history::recent(&app, limit.min(MAX_CLIPS) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Area = Area {
        x: 0.0,
        y: 25.0,
        w: 1440.0,
        h: 875.0,
    };

    #[test]
    fn quick_panel_below_the_macos_menu_bar() {
        // An icon in the menu bar, above the work area: centred under it,
        // `margin` inside the work area (which starts under the bar).
        let icon = Area {
            x: 1000.0,
            y: 0.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(
            place(360.0, 540.0, Anchor::Icon(icon), SCREEN, 8.0),
            (832.0, 33.0)
        );
    }

    #[test]
    fn quick_panel_kept_inside_the_right_edge() {
        let icon = Area {
            x: 1420.0,
            y: 0.0,
            w: 20.0,
            h: 24.0,
        };
        let (x, y) = place(360.0, 540.0, Anchor::Icon(icon), SCREEN, 8.0);
        assert_eq!(x, 1440.0 - 360.0 - 8.0);
        assert_eq!(y, 33.0);
    }

    #[test]
    fn quick_panel_above_a_bottom_taskbar() {
        // Windows: the work area stops above the taskbar.
        let work = Area {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1040.0,
        };
        let icon = Area {
            x: 1700.0,
            y: 1048.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(
            place(360.0, 540.0, Anchor::Icon(icon), work, 8.0),
            (1532.0, 1040.0 - 540.0 - 8.0)
        );
        // Near the corner it stays on screen.
        let corner = Area { x: 1900.0, ..icon };
        assert_eq!(
            place(360.0, 540.0, Anchor::Icon(corner), work, 8.0).0,
            1920.0 - 360.0 - 8.0
        );
    }

    #[test]
    fn quick_panel_beside_a_side_taskbar() {
        // Taskbar on the left: the work area starts to its right.
        let work = Area {
            x: 60.0,
            y: 0.0,
            w: 1860.0,
            h: 1080.0,
        };
        let icon = Area {
            x: 18.0,
            y: 1000.0,
            w: 24.0,
            h: 24.0,
        };
        let (x, y) = place(360.0, 540.0, Anchor::Icon(icon), work, 8.0);
        assert_eq!(x, 68.0);
        // Centred on the icon would run off the bottom: kept inside.
        assert_eq!(y, 1080.0 - 540.0 - 8.0);
        // On the right.
        let work = Area {
            x: 0.0,
            y: 0.0,
            w: 1860.0,
            h: 1080.0,
        };
        let icon = Area {
            x: 1878.0,
            y: 300.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(
            place(360.0, 540.0, Anchor::Icon(icon), work, 8.0),
            (1860.0 - 360.0 - 8.0, 312.0 - 270.0)
        );
    }

    #[test]
    fn quick_panel_at_the_pointer_or_the_corner() {
        // A pointer near the top: the panel opens below it.
        let (x, y) = place(360.0, 540.0, Anchor::Point(700.0, 30.0), SCREEN, 8.0);
        assert_eq!((x, y), (520.0, 38.0));
        // Near the bottom: above it.
        let (_, y) = place(360.0, 540.0, Anchor::Point(700.0, 880.0), SCREEN, 8.0);
        assert_eq!(y, 880.0 - 540.0 - 8.0);
        // Nothing known: the bottom-right corner.
        assert_eq!(
            place(360.0, 540.0, Anchor::Corner, SCREEN, 8.0),
            (1440.0 - 368.0, 25.0 + 875.0 - 548.0)
        );
    }

    #[test]
    fn quick_panel_bigger_than_the_screen_keeps_its_top_left() {
        let small = Area {
            x: 100.0,
            y: 50.0,
            w: 300.0,
            h: 400.0,
        };
        assert_eq!(
            place(360.0, 540.0, Anchor::Corner, small, 8.0),
            (108.0, 58.0)
        );
        let icon = Area {
            x: 200.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
        };
        assert_eq!(
            place(360.0, 540.0, Anchor::Icon(icon), small, 8.0),
            (108.0, 58.0)
        );
    }
}
