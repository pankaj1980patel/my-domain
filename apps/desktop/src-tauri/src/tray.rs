// The menu-bar (macOS) / system-tray icon: open the window, ring a device,
// send it a file, jump to its page, or quit. Closing the window hides it to
// the tray instead of quitting; "Quit" here really exits.
//
// The device submenus follow the peer list and link state: the event sink
// calls `refresh` on PeersUpdated / PeerConnected / PeerDisconnected, which
// rebuilds the menu at most once per `DEBOUNCE`.
//
// Phones (devices whose status says `"platform": "android"`, as remembered
// by `shortcut`) get quick actions in their submenu: ring, send the
// clipboard, and locate (the answer arrives in the phone's status as
// `location {lat, lon, acc, at}`, and the map opens then). Statuses come in
// through `on_status`, which rebuilds the menu when something it shows
// changed.
//
// "Recent clips" lists the last few clips received as clipboard (kept by
// `clip_history`); choosing one puts it on this computer's clipboard. It's
// rebuilt (through `refresh`) whenever the clipboard history changes.
//
// During a call taken on this Mac (`calls`), the menu starts with Answer /
// Decline, or Hang up and "Use phone / this Mac for audio". While this
// computer's sound plays on a phone (`computer_audio`), a Stop for it.
//
// A left click opens the quick panel (`quick_panel`) instead of the menu
// while its Settings switch is on (macOS and Windows; Linux trays report
// no clicks, so the menu's "Quick panel" item opens it there). A right
// click shows the menu.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::quick_panel::{self, Anchor, Area};
use crate::{notify, shortcut, Eng};

const TRAY_ID: &str = "main";
const DEBOUNCE: Duration = Duration::from_millis(500);
/// How long after "Locate" an arriving position still opens the map.
const LOCATE_WAIT_SECS: u64 = 5 * 60;

// Menu labels (the Rust side has no i18n).
const LABEL_OPEN_APP: &str = "Open my-domain";
const LABEL_QUICK_PANEL: &str = "Quick panel";
const LABEL_RING: &str = "Ring";
const LABEL_SEND: &str = "Send a file…";
const LABEL_SEND_FOLDER: &str = "Send a folder…";
const LABEL_OPEN: &str = "Open";
const LABEL_QUIT: &str = "Quit";
const TOOLTIP_QUIET: &str = "my-domain \u{b7} Quiet: your phone is on Do Not Disturb";
const LABEL_NO_DEVICES: &str = "No devices yet";
const CONNECTED_SUFFIX: &str = " · connected";
const NOT_CONNECTED_SUFFIX: &str = " · not connected";
const LOCATE_OFF_SUFFIX: &str = " (off on the phone)";
const LABEL_RECENT_CLIPS: &str = "Recent clips";
const LABEL_NO_CLIPS: &str = "Nothing received yet";
/// How many received clips "Recent clips" lists.
const RECENT_CLIPS: usize = 5;
/// The most characters of a clip a menu line shows.
const CLIP_PREVIEW: usize = 40;

// Menu item ids; device items are `<action>:<node_id>`.
const ID_OPEN_APP: &str = "app:open";
const ID_QUICK_PANEL: &str = "app:panel";
const ID_QUIT: &str = "app:quit";
const ID_NONE: &str = "app:none";

/// Set by "Quit" so closing the window then really exits.
static QUITTING: AtomicBool = AtomicBool::new(false);
/// A rebuild is already scheduled.
static PENDING: AtomicBool = AtomicBool::new(false);
/// What the phones' latest statuses say (node id -> status).
static PHONES: Mutex<BTreeMap<String, PhoneStatus>> = Mutex::new(BTreeMap::new());
/// Phones asked "where are you?" and not answered yet.
static ASKED: Mutex<BTreeMap<String, Asked>> = Mutex::new(BTreeMap::new());

/// A reported position (`location` in a phone's status).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    lat: f64,
    lon: f64,
    /// When the phone took it (unix seconds).
    at: u64,
}

/// The parts of a phone's status the tray uses.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PhoneStatus {
    /// `location_sharing: true`: the phone answers "Locate".
    sharing: bool,
    place: Option<Place>,
}

/// A "Locate" waiting for its answer.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Asked {
    /// When it was asked (unix seconds).
    at: u64,
    /// The position known when it was asked: the same one coming back isn't
    /// the answer.
    before: Option<Place>,
}

/// Create the tray icon (after the Engine is managed, so devices show).
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("my-domain")
        .show_menu_on_left_click(!quick_panel::enabled(app))
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(|tray, ev| on_click(tray.app_handle(), ev));
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// A click on the icon: the left button's release opens or closes the quick
/// panel while its switch is on (the menu shows by itself otherwise).
fn on_click(app: &AppHandle, ev: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        rect,
        position,
        ..
    } = ev
    {
        if !quick_panel::enabled(app) {
            return;
        }
        let p = rect.position.to_physical::<f64>(1.0);
        let s = rect.size.to_physical::<f64>(1.0);
        let anchor = if s.width > 0.0 && s.height > 0.0 {
            Anchor::Icon(Area {
                x: p.x,
                y: p.y,
                w: s.width,
                h: s.height,
            })
        } else {
            Anchor::Point(position.x, position.y)
        };
        quick_panel::toggle(app, anchor);
    }
}

/// A left click shows the menu (`true`) or leaves it to the quick panel.
/// Linux trays always show the menu.
pub fn set_left_click_menu(app: &AppHandle, on: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Err(e) = tray.set_show_menu_on_left_click(on) {
            eprintln!("[tray] couldn't change the left click: {e}");
        }
    }
}

/// Whether "Quit" was chosen (the window's close then isn't intercepted).
pub fn quitting() -> bool {
    QUITTING.load(Ordering::Relaxed)
}

/// Rebuild the device submenus soon; calls within `DEBOUNCE` collapse into one.
pub fn refresh(app: &AppHandle) {
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(DEBOUNCE);
        PENDING.store(false, Ordering::Release);
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(tray) = handle.tray_by_id(TRAY_ID) else {
                return;
            };
            match build_menu(&handle) {
                Ok(menu) => {
                    if let Err(e) = tray.set_menu(Some(menu)) {
                        eprintln!("[tray] couldn't update the menu: {e}");
                    }
                }
                Err(e) => eprintln!("[tray] couldn't build the menu: {e}"),
            }
        });
    });
}

/// The tooltip says when pop-ups are held because a phone is on Do Not
/// Disturb (`quiet`).
pub fn set_quiet(app: &AppHandle, on: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            let tip = if on { TOOLTIP_QUIET } else { "my-domain" };
            if let Err(e) = tray.set_tooltip(Some(tip)) {
                eprintln!("[tray] couldn't update the tooltip: {e}");
            }
        }
    });
}

/// Bring the main window forward.
pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// A device's status arrived (`CoreEvent::DeviceStatus`): remember what a
/// phone's says, open the map if it answers a "Locate", and rebuild the menu
/// when something it shows changed.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(st) = phone_status(status) else {
        return;
    };
    let changed = match PHONES.lock() {
        Ok(mut phones) => {
            let old = phones.insert(from.to_string(), st);
            old.map(|o| (o.sharing, o.place.is_some())) != Some((st.sharing, st.place.is_some()))
        }
        Err(_) => false,
    };
    if changed {
        refresh(app);
    }
    let Some(place) = st.place else {
        return;
    };
    let answered = match ASKED.lock() {
        Ok(mut asked) => match asked.get(from) {
            Some(a) if now().saturating_sub(a.at) > LOCATE_WAIT_SECS => {
                asked.remove(from);
                false
            }
            Some(a) if answers(a, &place) => {
                asked.remove(from);
                true
            }
            _ => false,
        },
        Err(_) => false,
    };
    if answered {
        let (app, from) = (app.clone(), from.to_string());
        std::thread::spawn(move || {
            let name = notify::device_name(&app, &from);
            match crate::open_map(place.lat, place.lon, app.clone()) {
                Ok(()) => notify::post_always(&app, &format!("Found {name}"), "It's on the map."),
                Err(e) => notify::post_always(
                    &app,
                    &format!("Couldn't show {name} on a map"),
                    &e.to_string(),
                ),
            }
        });
    }
}

/// The tray's view of a status, or `None` when it isn't a phone's.
fn phone_status(status: &serde_json::Value) -> Option<PhoneStatus> {
    if status.get("platform").and_then(|p| p.as_str()) != Some("android") {
        return None;
    }
    let sharing = status.get("location_sharing").and_then(|v| v.as_bool()) == Some(true);
    let place = status.get("location").and_then(|l| {
        Some(Place {
            lat: l.get("lat")?.as_f64()?,
            lon: l.get("lon")?.as_f64()?,
            at: l.get("at").and_then(|a| a.as_u64()).unwrap_or(0),
        })
    });
    Some(PhoneStatus { sharing, place })
}

/// Whether a reported position answers a "Locate": taken since the asking
/// (give or take a minute of clock difference), or simply a new one.
fn answers(asked: &Asked, place: &Place) -> bool {
    place.at + 60 >= asked.at || asked.before != Some(*place)
}

/// A device's submenu title: its name and whether it's connected.
fn device_label(name: &str, connected: bool) -> String {
    let suffix = if connected {
        CONNECTED_SUFFIX
    } else {
        NOT_CONNECTED_SUFFIX
    };
    format!("{name}{suffix}")
}

/// "Locate {name}", and whether it can be chosen: not when the phone's
/// status says location sharing is off. With no status yet it's offered
/// (the phone asks its owner to turn sharing on if needed).
fn locate_label(name: &str, status: Option<&PhoneStatus>) -> (String, bool) {
    match status {
        Some(st) if !st.sharing => (format!("Locate {name}{LOCATE_OFF_SUFFIX}"), false),
        _ => (format!("Locate {name}"), true),
    }
}

/// One of your devices, as the menu shows it.
struct Device {
    node: String,
    name: String,
    connected: bool,
    /// Set for a phone: what its latest status said, if one came.
    phone: Option<Option<PhoneStatus>>,
}

/// Your devices, sorted by name. A device is a phone if `shortcut` has seen
/// it say so (also before a restart) or a status here says so.
fn devices(app: &AppHandle) -> Vec<Device> {
    let Some(engine) = app.try_state::<Eng>() else {
        return Vec::new();
    };
    let live: Vec<String> = engine.links().into_iter().map(|(id, _)| id).collect();
    let known = shortcut::known_phones(app);
    let statuses = PHONES.lock().map(|p| p.clone()).unwrap_or_default();
    let mut out: Vec<Device> = engine
        .get_peers()
        .into_iter()
        .map(|p| {
            let status = statuses.get(&p.node_id).copied();
            let phone = phone_entry(&p.node_id, &known, status);
            Device {
                connected: live.contains(&p.node_id),
                name: crate::device_labels::display_name(app, &p.node_id, &p.name),
                node: p.node_id,
                phone,
            }
        })
        .collect();
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

/// For a phone, its latest status if one came (`Some(None)` when none has);
/// `None` when the device isn't known to be a phone.
fn phone_entry(
    node: &str,
    known: &[String],
    status: Option<PhoneStatus>,
) -> Option<Option<PhoneStatus>> {
    (status.is_some() || known.iter().any(|k| k == node)).then_some(status)
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let mut menu = MenuBuilder::new(app);
    // A call on this Mac (calls.rs): answer / decline, or hang up and move
    // the audio, at the top while it lasts.
    let call = crate::calls::tray_items();
    for (id, label) in &call {
        menu = menu.text(format!("call:{id}"), label);
    }
    if !call.is_empty() {
        menu = menu.separator();
    }
    // This computer's sound playing on a phone (computer_audio.rs): its
    // Stop, at the top while it lasts.
    let sound = crate::computer_audio::tray_items(app);
    for (id, label) in &sound {
        menu = menu.text(format!("csound:{id}"), label);
    }
    if !sound.is_empty() {
        menu = menu.separator();
    }
    let mut menu = menu
        .text(ID_QUICK_PANEL, LABEL_QUICK_PANEL)
        .text(ID_OPEN_APP, LABEL_OPEN_APP)
        .separator();
    let devices = devices(app);
    if devices.is_empty() {
        let none = MenuItemBuilder::with_id(ID_NONE, LABEL_NO_DEVICES)
            .enabled(false)
            .build(app)?;
        menu = menu.item(&none);
    }
    for d in devices {
        let (node, name) = (&d.node, &d.name);
        let mut sub = SubmenuBuilder::new(app, device_label(name, d.connected));
        match d.phone {
            Some(status) => {
                let (locate, can_locate) = locate_label(name, status.as_ref());
                let locate = MenuItemBuilder::with_id(format!("locate:{node}"), locate)
                    .enabled(can_locate)
                    .build(app)?;
                sub = sub
                    .text(format!("ring:{node}"), format!("Ring {name}"))
                    .text(format!("clip:{node}"), format!("Send clipboard to {name}"))
                    .item(&locate);
                if status.is_some_and(|s| s.place.is_some()) {
                    sub = sub.text(format!("map:{node}"), format!("Show {name} on a map"));
                }
                sub = sub.separator();
            }
            None => sub = sub.text(format!("ring:{node}"), LABEL_RING),
        }
        let sub = sub
            .text(format!("send:{node}"), LABEL_SEND)
            .text(format!("folder:{node}"), LABEL_SEND_FOLDER)
            .text(format!("open:{node}"), LABEL_OPEN)
            .build()?;
        menu = menu.item(&sub);
    }
    let mut clips = SubmenuBuilder::new(app, LABEL_RECENT_CLIPS);
    let recent = crate::clip_history::recent_received(app, RECENT_CLIPS);
    if recent.is_empty() {
        let none = MenuItemBuilder::with_id(ID_NONE, LABEL_NO_CLIPS)
            .enabled(false)
            .build(app)?;
        clips = clips.item(&none);
    }
    for c in recent {
        let device = c
            .from_node
            .as_deref()
            .map(|n| notify::device_name(app, n))
            .unwrap_or_default();
        clips = clips.text(format!("rclip:{}", c.id), clip_label(&c.text, &device));
    }
    menu.separator()
        .item(&clips.build()?)
        .separator()
        .text(ID_QUIT, LABEL_QUIT)
        .build()
}

/// A "Recent clips" line: a short preview of the text, and where it came from.
fn clip_label(text: &str, device: &str) -> String {
    let text = crate::clip_history::preview(text, CLIP_PREVIEW);
    if device.is_empty() {
        text
    } else {
        format!("{text} \u{2014} {device}")
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        ID_OPEN_APP => show_window(app),
        ID_QUICK_PANEL => {
            let icon = app
                .tray_by_id(TRAY_ID)
                .and_then(|t| t.rect().ok().flatten());
            quick_panel::show(app, quick_panel::menu_anchor(app, icon));
        }
        ID_QUIT => {
            QUITTING.store(true, Ordering::Relaxed);
            app.exit(0);
        }
        _ => match id.split_once(':') {
            Some(("ring", node)) => ring(app, node),
            Some(("send", node)) => send_file(app, node),
            Some(("folder", node)) => send_folder(app, node),
            Some(("clip", node)) => send_clipboard(app, node),
            Some(("rclip", id)) => copy_clip(app, id),
            // Menu events arrive on the main thread, where IOBluetooth lives.
            Some(("call", what)) => crate::calls::tray_action(app, what),
            Some(("csound", what)) => crate::computer_audio::tray_action(app, what),
            Some(("locate", node)) => locate(app, node),
            Some(("map", node)) => show_on_map(app, node),
            Some(("open", node)) => {
                show_window(app);
                let _ = app.emit("open-device", serde_json::json!({ "node_id": node }));
            }
            _ => {}
        },
    }
}

/// Ring a device; off the main thread (engine send, notification).
pub(crate) fn ring(app: &AppHandle, node: &str) {
    let (app, node) = (app.clone(), node.to_string());
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        let name = notify::device_name(&app, &node);
        let connected = engine.links().iter().any(|(id, _)| *id == node);
        match engine.ring(&node) {
            Ok(_) if connected => {
                notify::post_always(&app, "my-domain", &format!("Ringing {name}"))
            }
            Ok(_) => notify::post_always(
                &app,
                "my-domain",
                &format!("Will ring when {name} is reachable"),
            ),
            Err(e) => notify::post_always(&app, "my-domain", &format!("Couldn't ring {name}: {e}")),
        }
    });
}

/// Send this computer's clipboard to a device; off the main thread.
pub(crate) fn send_clipboard(app: &AppHandle, node: &str) {
    let (app, node) = (app.clone(), node.to_string());
    std::thread::spawn(move || {
        if let Some(engine) = app.try_state::<Eng>() {
            shortcut::send_clipboard_to(&app, &engine, &node);
        }
    });
}

/// Put a recent clip back on this computer's clipboard; off the main thread.
fn copy_clip(app: &AppHandle, id: &str) {
    let Ok(id) = id.parse() else {
        return;
    };
    // A picture goes back on the clipboard as the picture.
    if let Some(path) = crate::clip_history::picture_of(app, id) {
        let app = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = crate::clip_image::put_on_clipboard(&path) {
                notify::post_always(&app, "Couldn't copy the picture", &e);
            }
        });
        return;
    }
    let Some(text) = crate::clip_history::text_of(app, id) else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        if let Err(e) = engine.copy_to_clipboard(&text) {
            notify::post_always(&app, "Couldn't copy the clip", &e.to_string());
        }
    });
}

/// Ask a phone where it is (the `locate` action); the map opens when the
/// answer arrives in its status (`on_status`).
pub(crate) fn locate(app: &AppHandle, node: &str) {
    let (app, node) = (app.clone(), node.to_string());
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        let name = notify::device_name(&app, &node);
        if let Ok(mut asked) = ASKED.lock() {
            asked.insert(
                node.clone(),
                Asked {
                    at: now(),
                    before: last_place(&node),
                },
            );
        }
        match engine.run_action(&node, "locate", "") {
            Ok(how) if how == "queued" => notify::post_always(
                &app,
                "my-domain",
                &format!("Will ask where {name} is when it's reachable"),
            ),
            Ok(_) => notify::post_always(&app, "my-domain", &format!("Asking where {name} is")),
            Err(e) => {
                if let Ok(mut asked) = ASKED.lock() {
                    asked.remove(&node);
                }
                notify::post_always(&app, "my-domain", &format!("Couldn't locate {name}: {e}"));
            }
        }
    });
}

/// Open a map at a phone's last reported position.
fn show_on_map(app: &AppHandle, node: &str) {
    let Some(place) = last_place(node) else {
        return;
    };
    if let Err(e) = crate::open_map(place.lat, place.lon, app.clone()) {
        let name = notify::device_name(app, node);
        notify::post_always(
            app,
            &format!("Couldn't show {name} on a map"),
            &e.to_string(),
        );
    }
}

/// The last position a phone reported, if any.
fn last_place(node: &str) -> Option<Place> {
    PHONES.lock().ok()?.get(node)?.place
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pick a file with the native dialog, then send it.
pub(crate) fn send_file(app: &AppHandle, node: &str) {
    use tauri_plugin_dialog::DialogExt;
    let (handle, node) = (app.clone(), node.to_string());
    app.dialog().file().pick_file(move |file| {
        let Some(path) = file.and_then(|f| f.into_path().ok()) else {
            return;
        };
        std::thread::spawn(move || {
            let Some(engine) = handle.try_state::<Eng>() else {
                return;
            };
            let name = notify::device_name(&handle, &node);
            let file = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "a file".to_string());
            let body = match engine.send_file(&node, &path.to_string_lossy()) {
                Ok(_) => format!("Sending {file} to {name}"),
                Err(e) => format!("Couldn't send {file} to {name}: {e}"),
            };
            notify::post_always(&handle, "my-domain", &body);
        });
    });
}

/// Pick a folder and send everything in it to a device (each file as its
/// own transfer); off the main thread.
fn send_folder(app: &AppHandle, node: &str) {
    use tauri_plugin_dialog::DialogExt;
    let (handle, node) = (app.clone(), node.to_string());
    app.dialog().file().pick_folder(move |folder| {
        let Some(path) = folder.and_then(|f| f.into_path().ok()) else {
            return;
        };
        std::thread::spawn(move || {
            let Some(engine) = handle.try_state::<Eng>() else {
                return;
            };
            let name = notify::device_name(&handle, &node);
            let folder = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "the folder".to_string());
            let body = match engine.send_folder(&node, &path.to_string_lossy()) {
                Ok(1) => format!("Sending 1 file to {name}"),
                Ok(n) => format!("Sending {n} files to {name}"),
                Err(e) => format!("Couldn't send {folder} to {name}: {e}"),
            };
            notify::post_always(&handle, "my-domain", &body);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tray_only_phones_have_a_phone_status() {
        assert_eq!(phone_status(&json!({ "platform": "macos" })), None);
        assert_eq!(phone_status(&json!({})), None);
        assert_eq!(
            phone_status(&json!({ "platform": "android" })),
            Some(PhoneStatus {
                sharing: false,
                place: None
            })
        );
        let st = phone_status(&json!({
            "platform": "android",
            "location_sharing": true,
            "location": { "lat": 12.5, "lon": 77.25, "acc": 9.0, "at": 1700 },
        }));
        assert_eq!(
            st,
            Some(PhoneStatus {
                sharing: true,
                place: Some(Place {
                    lat: 12.5,
                    lon: 77.25,
                    at: 1700
                }),
            })
        );
        // A position without coordinates isn't one.
        let st = phone_status(&json!({ "platform": "android", "location": { "at": 5 } }));
        assert_eq!(st.and_then(|s| s.place), None);
    }

    #[test]
    fn tray_which_devices_are_phones() {
        let known = vec!["phone".to_string()];
        let st = PhoneStatus {
            sharing: true,
            place: None,
        };
        // Remembered as a phone, no status yet this run.
        assert_eq!(phone_entry("phone", &known, None), Some(None));
        // A phone's status here is enough.
        assert_eq!(phone_entry("other", &known, Some(st)), Some(Some(st)));
        // A computer.
        assert_eq!(phone_entry("mac", &known, None), None);
    }

    #[test]
    fn tray_labels() {
        assert_eq!(device_label("I2405", true), "I2405 · connected");
        assert_eq!(device_label("I2405", false), "I2405 · not connected");
        let on = PhoneStatus {
            sharing: true,
            place: None,
        };
        let off = PhoneStatus {
            sharing: false,
            place: None,
        };
        assert_eq!(
            locate_label("I2405", Some(&on)),
            ("Locate I2405".into(), true)
        );
        assert_eq!(locate_label("I2405", None), ("Locate I2405".into(), true));
        assert_eq!(
            locate_label("I2405", Some(&off)),
            ("Locate I2405 (off on the phone)".into(), false)
        );
    }

    #[test]
    fn tray_clip_labels() {
        assert_eq!(clip_label("hello", "Pixel"), "hello \u{2014} Pixel");
        assert_eq!(clip_label("hello", ""), "hello");
        let long = "x".repeat(60);
        assert_eq!(
            clip_label(&long, ""),
            format!("{}\u{2026}", "x".repeat(CLIP_PREVIEW))
        );
    }

    #[test]
    fn tray_locate_answer() {
        let old = Place {
            lat: 1.0,
            lon: 2.0,
            at: 100,
        };
        let asked = Asked {
            at: 1000,
            before: Some(old),
        };
        // The same old position coming back isn't the answer.
        assert!(!answers(&asked, &old));
        // A new one is, and so is one taken since the asking.
        assert!(answers(
            &asked,
            &Place {
                lat: 1.5,
                lon: 2.0,
                at: 900
            }
        ));
        let fresh = Asked {
            at: 1000,
            before: None,
        };
        assert!(answers(
            &fresh,
            &Place {
                lat: 1.0,
                lon: 2.0,
                at: 1001
            }
        ));
        let again = Place { at: 1001, ..old };
        assert!(answers(
            &Asked {
                at: 1000,
                before: Some(again)
            },
            &again
        ));
    }
}
