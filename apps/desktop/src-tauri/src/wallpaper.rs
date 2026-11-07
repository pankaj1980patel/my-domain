// "Share my wallpaper with my devices" (mdcore `features::wallpaper`): this
// computer's desktop picture, as a small JPEG (at most 320 px on the long
// side, about 40 KB) and its two or three dominant colours, so your other
// devices can draw it in this computer's tile. On by default; the switch
// lives in `<app config dir>/prefs.json` (key `share_wallpaper`, in the
// settings backup). Off: nothing is read or sent, and your other devices
// are told to forget the one they had.
//
// Where the picture is:
//  - macOS: NSWorkspace's desktop picture for the main screen (objc2, on
//    the main thread), made small with `sips` (it reads HEIC too);
//  - Windows: SystemParametersInfo's wallpaper path, else
//    %APPDATA%\Microsoft\Windows\Themes\TranscodedWallpaper;
//  - Linux: GNOME's `picture-uri` (`picture-uri-dark` in dark style), else
//    KDE Plasma's image (best effort).
// Other formats than JPEG and PNG are made small by `sips` on macOS only;
// elsewhere a picture that can't be read shares nothing.
//
// It's looked at on start, every few minutes and when the switch changes;
// the file's path, size and time say whether it changed, and the core
// sends only a look whose hash changed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use mdcore::kv::FileKv;
use mdcore::model::Wallpaper;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

const KEY: &str = "share_wallpaper";
/// How often the wallpaper is looked at.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// The thumbnail's long side, in pixels.
const SIDE: u32 = 320;
/// The thumbnail's most bytes.
const MAX_BYTES: usize = 40 * 1024;

/// Set to look again now (the switch changed).
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
/// The file last shared: path, size and modified time.
static LAST: Mutex<Option<String>> = Mutex::new(None);

fn prefs(app: &AppHandle) -> Option<FileKv> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")))
}

/// The switch as saved: on unless turned off.
pub fn saved(value: Option<&str>) -> bool {
    value != Some("off")
}

fn enabled(app: &AppHandle) -> bool {
    saved(prefs(app).and_then(|kv| kv.get(KEY)).as_deref())
}

/// Look now, then every few minutes (and when the switch changes).
pub fn init(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("wallpaper".into())
        .spawn(move || loop {
            check(&app);
            let (lock, cvar) = &WAKE;
            let Ok(mut woken) = lock.lock() else { return };
            if !*woken {
                woken = match cvar.wait_timeout(woken, EVERY) {
                    Ok((w, _)) => w,
                    Err(_) => return,
                };
            }
            *woken = false;
        })
        .ok();
}

fn wake() {
    let (lock, cvar) = &WAKE;
    if let Ok(mut w) = lock.lock() {
        *w = true;
        cvar.notify_all();
    }
}

fn set_last(v: Option<String>) {
    if let Ok(mut last) = LAST.lock() {
        *last = v;
    }
}

fn check(app: &AppHandle) {
    let Some(engine) = app.try_state::<Eng>() else {
        return;
    };
    if !enabled(app) {
        set_last(None);
        engine.clear_wallpaper();
        return;
    }
    let candidates = candidates(app);
    let Some(sig) = candidates.iter().find_map(|p| signature(p)) else {
        // Can't tell where it is: leave what's shared.
        return;
    };
    if LAST.lock().ok().and_then(|l| l.clone()).as_deref() == Some(sig.as_str()) {
        return;
    }
    match candidates.iter().find_map(|p| look(p)) {
        Some((jpeg, colors)) => engine.set_wallpaper(colors, Some(jpeg)),
        // A picture that can't be read: don't leave the old one showing.
        None => engine.clear_wallpaper(),
    }
    set_last(Some(sig));
}

/// What says the file changed: its path, size and modified time.
fn signature(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let at = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some(format!("{}|{}|{at}", path.display(), m.len()))
}

/// The thumbnail (JPEG) and dominant colours of the picture at `path`.
fn look(path: &Path) -> Option<(Vec<u8>, Vec<String>)> {
    let bytes = readable(path)?;
    let img = image::load_from_memory(&bytes).ok()?;
    small(&img)
}

/// The picture's bytes in a format read here (JPEG or PNG).
#[cfg(target_os = "macos")]
fn readable(path: &Path) -> Option<Vec<u8>> {
    // sips reads whatever macOS does (HEIC, TIFF...): a JPEG a little
    // bigger than needed, finished below like any other.
    let out = std::env::temp_dir().join(format!("my-domain-wallpaper-{}.jpg", std::process::id()));
    let ok = std::process::Command::new("sips")
        .args([
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "90",
            "-Z",
            "640",
        ])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let bytes = ok.then(|| std::fs::read(&out).ok()).flatten();
    let _ = std::fs::remove_file(&out);
    bytes
}

#[cfg(not(target_os = "macos"))]
fn readable(path: &Path) -> Option<Vec<u8>> {
    let m = std::fs::metadata(path).ok()?;
    // A wallpaper is a few MB at most; anything far bigger isn't read.
    (m.len() <= 64 * 1024 * 1024)
        .then(|| std::fs::read(path).ok())
        .flatten()
}

/// Made small (fits `SIDE`), as a JPEG of at most `MAX_BYTES`, and its
/// colours.
pub fn small(img: &image::DynamicImage) -> Option<(Vec<u8>, Vec<String>)> {
    let thumb = img.thumbnail(SIDE, SIDE).to_rgb8();
    let colors = mdcore::features::wallpaper::dominant_colors(thumb.as_raw(), 3);
    for quality in [72, 60, 48, 36, 26] {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
            .encode_image(&thumb)
            .ok()?;
        if out.len() <= MAX_BYTES {
            return Some((out, colors));
        }
    }
    None
}

// --- where the picture is ------------------------------------------------------

#[cfg(target_os = "macos")]
fn candidates(app: &AppHandle) -> Vec<PathBuf> {
    // AppKit wants the main thread.
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = app.run_on_main_thread(move || {
        let _ = tx.send(mac::desktop_picture());
    });
    if asked.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(Duration::from_secs(5))
        .ok()
        .flatten()
        .into_iter()
        .collect()
}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;

    /// `[[NSWorkspace sharedWorkspace] desktopImageURLForScreen:[NSScreen
    /// mainScreen]].path`.
    pub fn desktop_picture() -> Option<PathBuf> {
        let ws_cls = AnyClass::get(c"NSWorkspace")?;
        let screen_cls = AnyClass::get(c"NSScreen")?;
        // SAFETY: class methods and getters with these exact names and
        // object (or nil) results, called on the main thread.
        unsafe {
            let ws: Option<Retained<AnyObject>> = msg_send![ws_cls, sharedWorkspace];
            let screen: Option<Retained<AnyObject>> = msg_send![screen_cls, mainScreen];
            let (ws, screen) = (ws?, screen?);
            let url: Option<Retained<AnyObject>> =
                msg_send![&*ws, desktopImageURLForScreen: &*screen];
            let path: Option<Retained<NSString>> = msg_send![&*url?, path];
            let path = path?.to_string();
            (!path.is_empty()).then(|| PathBuf::from(path))
        }
    }
}

#[cfg(windows)]
fn candidates(_app: &AppHandle) -> Vec<PathBuf> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut out = Vec::new();
    let mut buf = [0u16; 1024];
    // SAFETY: the buffer is as long as said, and only read back.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            buf.len() as u32,
            Some(buf.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if ok {
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let p = String::from_utf16_lossy(&buf[..end]);
        if !p.trim().is_empty() {
            out.push(PathBuf::from(p.trim()));
        }
    }
    // The copy Windows keeps (a JPEG whatever the original was).
    if let Some(appdata) = std::env::var_os("APPDATA") {
        out.push(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Themes")
                .join("TranscodedWallpaper"),
        );
    }
    out
}

#[cfg(all(unix, not(target_os = "macos")))]
fn candidates(_app: &AppHandle) -> Vec<PathBuf> {
    let run = |args: &[&str]| -> Option<String> {
        let o = std::process::Command::new("gsettings")
            .args(args)
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let mut out = Vec::new();
    let dark = run(&["get", "org.gnome.desktop.interface", "color-scheme"])
        .is_some_and(|s| s.contains("dark"));
    let keys: &[&str] = if dark {
        &["picture-uri-dark", "picture-uri"]
    } else {
        &["picture-uri"]
    };
    for key in keys {
        if let Some(p) = run(&["get", "org.gnome.desktop.background", key])
            .as_deref()
            .and_then(parse_gsettings_uri)
        {
            out.push(p);
        }
    }
    // KDE Plasma.
    if let Some(home) = std::env::var_os("HOME") {
        let rc = PathBuf::from(home).join(".config/plasma-org.kde.plasma.desktop-appletsrc");
        if let Ok(text) = std::fs::read_to_string(rc) {
            for p in parse_plasma(&text) {
                out.push(plasma_image(p));
            }
        }
    }
    out
}

/// A KDE wallpaper package (a folder) to one of its pictures.
#[cfg(all(unix, not(target_os = "macos")))]
fn plasma_image(p: PathBuf) -> PathBuf {
    if !p.is_dir() {
        return p;
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(p.join("contents/images"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    files.sort();
    // Named by size ("1920x1080.png"): the last is usually the biggest.
    files.pop().unwrap_or(p)
}

/// The path in `gsettings get`'s answer (`'file:///usr/share/a%20b.jpg'`).
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn parse_gsettings_uri(out: &str) -> Option<PathBuf> {
    let v = out.trim().trim_matches('\'').trim_matches('"').trim();
    let path = v.strip_prefix("file://").unwrap_or(v);
    if path.is_empty() || !path.starts_with('/') || path.ends_with(".xml") {
        return None;
    }
    Some(PathBuf::from(percent_decode(path)))
}

/// The `Image=` of Plasma's desktop wallpaper sections, in order.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn parse_plasma(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut in_image = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_image = line.ends_with("[Wallpaper][org.kde.image][General]");
        } else if in_image {
            if let Some(v) = line.strip_prefix("Image=") {
                let path = v.strip_prefix("file://").unwrap_or(v);
                if path.starts_with('/') {
                    out.push(PathBuf::from(percent_decode(path)));
                }
            }
        }
    }
    out
}

/// `%20` and the like back to bytes (bad escapes left as they are).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --- commands ---

/// Whether this computer shares its wallpaper with your other devices.
#[tauri::command]
#[specta::specta]
pub fn share_wallpaper(app: AppHandle) -> bool {
    enabled(&app)
}

/// Share this computer's wallpaper, or stop (your other devices then
/// forget it). Kept.
#[tauri::command]
#[specta::specta]
pub fn set_share_wallpaper(on: bool, app: AppHandle) {
    if let Some(kv) = prefs(&app) {
        if let Err(e) = kv.set(KEY, if on { "on" } else { "off" }) {
            eprintln!("[wallpaper] couldn't save the setting: {e}");
        }
    }
    wake();
}

/// The wallpaper look each of your devices shared (colours, maybe a small
/// picture as a `data:` URL), by node id. Changes come as
/// `wallpaper-changed` events (`{node, wallpaper}`; `wallpaper` null when
/// it stopped sharing).
#[tauri::command]
#[specta::specta]
pub fn wallpapers(engine: State<Eng>) -> BTreeMap<String, Wallpaper> {
    engine.wallpapers()
}

/// This computer's own look, as shared (null: nothing shared).
#[tauri::command]
#[specta::specta]
pub fn own_wallpaper(engine: State<Eng>) -> Option<Wallpaper> {
    engine.own_wallpaper()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_unless_turned_off() {
        assert!(saved(None));
        assert!(saved(Some("on")));
        assert!(!saved(Some("off")));
    }

    #[test]
    fn gnome_uris_become_paths() {
        assert_eq!(
            parse_gsettings_uri("'file:///usr/share/backgrounds/a%20b.jpg'\n"),
            Some(PathBuf::from("/usr/share/backgrounds/a b.jpg"))
        );
        assert_eq!(
            parse_gsettings_uri("'/home/p/x.png'"),
            Some(PathBuf::from("/home/p/x.png"))
        );
        assert_eq!(parse_gsettings_uri("''"), None);
        assert_eq!(
            parse_gsettings_uri("'file:///usr/share/backgrounds/slides.xml'"),
            None
        );
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%e2%82%ac"), "€");
    }

    #[test]
    fn plasma_images_are_found() {
        let rc = "[Containments][1][Wallpaper][org.kde.image][General]\n\
                  Image=file:///usr/share/wallpapers/Next/\n\
                  [Containments][1][General]\nImage=/not/this.png\n\
                  [Containments][2][Wallpaper][org.kde.image][General]\n\
                  Image=/home/p/Pictures/sea.jpg\n";
        assert_eq!(
            parse_plasma(rc),
            vec![
                PathBuf::from("/usr/share/wallpapers/Next/"),
                PathBuf::from("/home/p/Pictures/sea.jpg")
            ]
        );
    }

    #[test]
    fn a_picture_is_made_small() {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(1600, 900, |x, y| {
            image::Rgb([(x / 7) as u8, (y / 4) as u8, 180])
        }));
        let (jpeg, colors) = small(&img).unwrap();
        assert!(jpeg.len() <= MAX_BYTES, "{}", jpeg.len());
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
        let back = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (320, 180));
        assert!(!colors.is_empty() && colors.len() <= 3, "{colors:?}");
    }
}
