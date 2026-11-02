// The menu-bar (macOS) / system-tray icon: open the window, ring a device,
// send it a file, jump to its page, or quit. Closing the window hides it to
// the tray instead of quitting; "Quit" here really exits.
//
// The device submenus follow the peer list and link state: the event sink
// calls `refresh` on PeersUpdated / PeerConnected / PeerDisconnected, which
// rebuilds the menu at most once per `DEBOUNCE`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

use crate::{notify, Eng};

const TRAY_ID: &str = "main";
const DEBOUNCE: Duration = Duration::from_millis(500);

// Menu labels (the Rust side has no i18n).
const LABEL_OPEN_APP: &str = "Open my-domain";
const LABEL_RING: &str = "Ring";
const LABEL_SEND: &str = "Send a file…";
const LABEL_OPEN: &str = "Open";
const LABEL_QUIT: &str = "Quit";
const LABEL_NO_DEVICES: &str = "No devices yet";
const CONNECTED_SUFFIX: &str = " — connected";

// Menu item ids; device items are `<action>:<node_id>`.
const ID_OPEN_APP: &str = "app:open";
const ID_QUIT: &str = "app:quit";
const ID_NONE: &str = "app:none";

/// Set by "Quit" so closing the window then really exits.
static QUITTING: AtomicBool = AtomicBool::new(false);
/// A rebuild is already scheduled.
static PENDING: AtomicBool = AtomicBool::new(false);

/// Create the tray icon (after the Engine is managed, so devices show).
pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("my-domain")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
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

/// Bring the main window forward.
pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// One entry per device: (node id, display name, connected), sorted by name.
fn devices(app: &AppHandle) -> Vec<(String, String, bool)> {
    let Some(engine) = app.try_state::<Eng>() else {
        return Vec::new();
    };
    let live: Vec<String> = engine.links().into_iter().map(|(id, _)| id).collect();
    let mut out: Vec<(String, String, bool)> = engine
        .get_peers()
        .into_iter()
        .map(|p| {
            let connected = live.contains(&p.node_id);
            (p.node_id, notify::nice_name(&p.name), connected)
        })
        .collect();
    out.sort_by_key(|(_, name, _)| name.to_lowercase());
    out
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let mut menu = MenuBuilder::new(app)
        .text(ID_OPEN_APP, LABEL_OPEN_APP)
        .separator();
    let devices = devices(app);
    if devices.is_empty() {
        let none = MenuItemBuilder::with_id(ID_NONE, LABEL_NO_DEVICES)
            .enabled(false)
            .build(app)?;
        menu = menu.item(&none);
    }
    for (node, name, connected) in devices {
        let label = if connected {
            format!("{name}{CONNECTED_SUFFIX}")
        } else {
            name
        };
        let sub = SubmenuBuilder::new(app, label)
            .text(format!("ring:{node}"), LABEL_RING)
            .text(format!("send:{node}"), LABEL_SEND)
            .text(format!("open:{node}"), LABEL_OPEN)
            .build()?;
        menu = menu.item(&sub);
    }
    menu.separator().text(ID_QUIT, LABEL_QUIT).build()
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        ID_OPEN_APP => show_window(app),
        ID_QUIT => {
            QUITTING.store(true, Ordering::Relaxed);
            app.exit(0);
        }
        _ => match id.split_once(':') {
            Some(("ring", node)) => ring(app, node),
            Some(("send", node)) => send_file(app, node),
            Some(("open", node)) => {
                show_window(app);
                let _ = app.emit("open-device", serde_json::json!({ "node_id": node }));
            }
            _ => {}
        },
    }
}

/// Ring a device; off the main thread (engine send, notification).
fn ring(app: &AppHandle, node: &str) {
    let (app, node) = (app.clone(), node.to_string());
    std::thread::spawn(move || {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        let name = notify::device_name(&app, &node);
        let connected = engine.links().iter().any(|(id, _)| *id == node);
        match engine.ring(&node) {
            Ok(_) if connected => notify::post(&app, "my-domain", &format!("Ringing {name}")),
            Ok(_) => notify::post(
                &app,
                "my-domain",
                &format!("Will ring when {name} is reachable"),
            ),
            Err(e) => notify::post(&app, "my-domain", &format!("Couldn't ring {name}: {e}")),
        }
    });
}

/// Pick a file with the native dialog, then send it.
fn send_file(app: &AppHandle, node: &str) {
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
            notify::post(&handle, "my-domain", &body);
        });
    });
}
