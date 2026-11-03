// Your phone sees this screen (mdcore `features::screen`): while one of
// your devices watches, a snapshot goes to it about twice a second.
//
// Snapshots come from the OS's own tools, shrunk to `WIDTH` pixels wide:
// macOS `screencapture` + `sips` (needs Screen Recording permission, else
// macOS hands over only the wallpaper); Linux `grim` (Wayland) or
// ImageMagick's `import` (X11). A notice says when a device starts
// watching, so it's never silent.

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

use crate::Eng;

/// Snapshot width in pixels (height follows the screen's shape).
const WIDTH: u32 = 1280;
/// At most one snapshot per this long.
const EVERY: Duration = Duration::from_millis(500);

/// Why snapshots can't be taken here, for the phone to show: macOS
/// "screen_recording" (permission); Linux "grim" or "imagemagick" (a tool to
/// install); "unsupported" elsewhere.
pub fn ready() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        if mac::allowed() {
            Ok(())
        } else {
            Err("screen_recording")
        }
    }
    #[cfg(target_os = "linux")]
    {
        if wayland() {
            if has("grim", &["-h"]) {
                Ok(())
            } else {
                Err("grim")
            }
        } else if has("import", &["-version"]) {
            Ok(())
        } else {
            Err("imagemagick")
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("unsupported")
    }
}

/// A device started (`on`) or stopped watching: say so, and keep sending
/// snapshots while anyone watches.
pub fn on_watch(app: &AppHandle, from: &str, on: bool) {
    if !on {
        return;
    }
    let who = crate::notify::device_name(app, from);
    match ready() {
        Ok(()) => crate::notify::post(
            app,
            &format!("{who} is watching this screen"),
            "Turn it off in my-domain's settings.",
        ),
        Err(why) => {
            crate::notify::post(app, &format!("{who} can't see this screen"), needs(why));
            return;
        }
    }
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("screen-share".into())
        .spawn(move || {
            send_while_watched(&app);
            RUNNING.store(false, Ordering::Release);
        })
        .ok();
}

fn send_while_watched(app: &AppHandle) {
    loop {
        let Some(engine) = app.try_state::<Eng>() else {
            return;
        };
        if engine.screen_watchers().is_empty() {
            return;
        }
        let started = Instant::now();
        match snapshot() {
            Some(jpeg) => {
                if let Err(e) = engine.screen_frame(&jpeg) {
                    eprintln!("[screen] {e}");
                }
            }
            None => eprintln!("[screen] no snapshot"),
        }
        std::thread::sleep(EVERY.saturating_sub(started.elapsed()));
    }
}

/// What to tell the user when snapshots can't be taken.
fn needs(why: &str) -> &'static str {
    match why {
        "screen_recording" => {
            "Allow my-domain in System Settings > Privacy & Security > Screen Recording, then try again."
        }
        "grim" => "Install grim to share this screen from Wayland.",
        "imagemagick" => "Install ImageMagick (its import tool) to share this screen.",
        _ => "Sharing this screen isn't supported here yet.",
    }
}

/// A JPEG of the main screen, `WIDTH` wide.
fn snapshot() -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        let dir = std::env::temp_dir();
        let (full, small) = (dir.join("md-screen.jpg"), dir.join("md-screen-small.jpg"));
        let ok = Command::new("screencapture")
            .args(["-x", "-m", "-t", "jpg"])
            .arg(&full)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return None;
        }
        let ok = Command::new("sips")
            .args(["-Z", &WIDTH.to_string(), "-s", "formatOptions", "45"])
            .arg(&full)
            .arg("--out")
            .arg(&small)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let jpeg = if ok { std::fs::read(&small).ok() } else { None };
        let _ = std::fs::remove_file(&full);
        let _ = std::fs::remove_file(&small);
        jpeg
    }
    #[cfg(target_os = "linux")]
    {
        let out = if wayland() {
            Command::new("grim")
                .args(["-t", "jpeg", "-q", "50", "-s", "0.5", "-"])
                .output()
        } else {
            Command::new("import")
                .args(["-silent", "-window", "root", "-resize"])
                .arg(format!("{WIDTH}x"))
                .args(["-quality", "50", "jpeg:-"])
                .output()
        }
        .ok()?;
        (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland")
}

#[cfg(target_os = "linux")]
fn has(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[cfg(target_os = "macos")]
mod mac {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    /// my-domain may record the screen (Privacy & Security > Screen
    /// Recording). Doesn't ask; macOS asks on the first `screencapture`.
    pub fn allowed() -> bool {
        // SAFETY: takes no arguments and only reads the permission.
        unsafe { CGPreflightScreenCaptureAccess() }
    }
}
