// Verification codes on the computer: a code your phone found in a
// notification (mdcore `features::otp_relay`) shows here as "Code 482913
// from Bank" with Copy: an OS notification (a Copy button on macOS and
// Windows; Linux pops up a plain one) and a toast in the window. The quick
// panel shows the latest code still valid.
//
// "Copy automatically" (off by default; `<app config dir>/prefs.json`, key
// `otp_auto_copy`) puts it on the clipboard at once. Copying, either way,
// marks it concealed (macOS `org.nspasteboard.ConcealedType`; Windows
// excluded from monitoring, the clipboard history and the cloud; Linux the
// KDE password-manager hint), so clipboard managers skip it; the clipboard
// sync is told first (`Engine::clipboard_written_here`), so it never goes
// to your other devices or into the clipboard history. After `CLEAR_AFTER`
// the clipboard is cleared if it still holds the code.
//
// A code is never logged, never put in Activity, the notification history
// or search: lib.rs hands `VerificationCode` here and to the window only.
// The core forgets it after five minutes; what's kept here for clearing
// the clipboard is wiped once that's done.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use mdcore::features::otp_relay::{now_ms, wipe, VerificationCode};
use mdcore::kv::FileKv;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::notify::{self, Kind};
use crate::Eng;

const AUTO_KEY: &str = "otp_auto_copy";
/// How long a copied code stays on the clipboard.
pub const CLEAR_AFTER: Duration = Duration::from_secs(90);

struct Prefs {
    kv: Option<FileKv>,
    auto_copy: AtomicBool,
}

static PREFS: OnceLock<Prefs> = OnceLock::new();

/// The code put on the clipboard last, and when (to clear it later).
struct Copied {
    code: String,
    at: Instant,
    /// Which copy this is: a later copy replaces the clearing of an earlier.
    seq: u64,
}

impl Drop for Copied {
    fn drop(&mut self) {
        wipe(&mut self.code);
    }
}

static COPIED: Mutex<Option<Copied>> = Mutex::new(None);
static COPY_SEQ: AtomicU64 = AtomicU64::new(0);

/// Read the setting; before the Engine starts.
pub fn load(app: &AppHandle) {
    let kv = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")));
    let auto = kv.as_ref().and_then(|kv| kv.get(AUTO_KEY)).as_deref() == Some("on");
    let _ = PREFS.set(Prefs {
        kv,
        auto_copy: AtomicBool::new(auto),
    });
}

fn auto_copy_on() -> bool {
    PREFS
        .get()
        .is_some_and(|p| p.auto_copy.load(Ordering::Relaxed))
}

// --- pure parts (tested) ------------------------------------------------------

/// The pop-up's title and text: "Code 482913" / "from HDFC Bank on Pixel".
pub fn texts(code: &str, app: &str, issuer: &str, device: &str) -> (String, String) {
    let from = if !issuer.is_empty() {
        issuer
    } else if !app.is_empty() {
        app
    } else {
        device
    };
    let title = format!("Code {}", spaced(code));
    let body = if from == device {
        format!("from {device}")
    } else {
        format!("from {from} on {device}")
    };
    (title, body)
}

/// A long code in two halves, easier to read: "482 913", "1234 5678".
pub fn spaced(code: &str) -> String {
    let n = code.chars().count();
    if n < 6 {
        return code.to_string();
    }
    let half = n.div_ceil(2);
    let (a, b): (String, String) = (
        code.chars().take(half).collect(),
        code.chars().skip(half).collect(),
    );
    format!("{a} {b}")
}

/// Whether to clear the clipboard now: `CLEAR_AFTER` has passed since the
/// copy at `copied_at`, and it still holds `code` (anything copied since is
/// the user's, left alone).
pub fn should_clear(copied_at: Instant, now: Instant, clipboard: Option<&str>, code: &str) -> bool {
    now.saturating_duration_since(copied_at) >= CLEAR_AFTER
        && clipboard.map(str::trim) == Some(code)
}

/// Milliseconds left until `expires_at` (unix ms), for the window.
pub fn left_ms(expires_at: u64, now: u64) -> u32 {
    u32::try_from(expires_at.saturating_sub(now)).unwrap_or(u32::MAX)
}

// --- the clipboard -------------------------------------------------------------

/// Put `code` on the clipboard marked concealed, so clipboard managers and
/// the OS's clipboard history skip it.
fn write_concealed(code: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("no clipboard: {e}"))?;
    let set = cb.set();
    #[cfg(target_os = "macos")]
    let set = {
        use arboard::SetExtApple;
        set.exclude_from_history()
    };
    #[cfg(windows)]
    let set = {
        use arboard::SetExtWindows;
        set.exclude_from_monitoring()
            .exclude_from_cloud()
            .exclude_from_history()
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let set = {
        use arboard::SetExtLinux;
        // Served until the new owner takes it (or `CLEAR_AFTER`).
        set.exclude_from_history()
            .wait_until(std::time::Instant::now() + CLEAR_AFTER + Duration::from_secs(5))
    };
    set.text(code.to_string()).map_err(|e| e.to_string())
}

fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

fn clear_clipboard() {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.clear();
    }
}

/// Copy `code` to the clipboard (concealed, not synced) and clear it after
/// `CLEAR_AFTER` if it's still there.
fn copy_code(app: &AppHandle, code: &str) -> Result<(), String> {
    // The clipboard sync takes it as seen before it's there: never sent on.
    if let Some(engine) = app.try_state::<Eng>() {
        engine.clipboard_written_here(code);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Linux: the owner must keep serving it; off this thread.
        let code = code.to_string();
        std::thread::spawn(move || {
            let mut code = code;
            if let Err(e) = write_concealed(&code) {
                eprintln!("[otp] couldn't copy the code: {e}");
            }
            wipe(&mut code);
        });
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    write_concealed(code)?;
    let seq = COPY_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut c) = COPIED.lock() {
        *c = Some(Copied {
            code: code.to_string(),
            at: Instant::now(),
            seq,
        });
    }
    std::thread::spawn(move || {
        std::thread::sleep(CLEAR_AFTER);
        let Ok(mut c) = COPIED.lock() else {
            return;
        };
        let Some(copied) = c.as_ref().filter(|c| c.seq == seq) else {
            return; // copied again since: that one clears
        };
        let mut now = read_clipboard();
        if should_clear(copied.at, Instant::now(), now.as_deref(), &copied.code) {
            clear_clipboard();
        }
        if let Some(n) = now.as_mut() {
            wipe(n);
        }
        *c = None;
    });
    Ok(())
}

// --- arriving codes ----------------------------------------------------------------

/// A code arrived from your phone: pop up, and copy it if that's on.
pub fn on_code(app: &AppHandle, c: &VerificationCode) {
    let app = app.clone();
    let c = c.clone();
    std::thread::spawn(move || {
        let device = notify::device_name(&app, &c.from);
        if auto_copy_on() {
            if let Err(e) = copy_code(&app, &c.code) {
                eprintln!("[otp] couldn't copy the code: {e}");
            }
        }
        let (title, body) = texts(&c.code, &c.app, &c.issuer, &device);
        let body = if auto_copy_on() {
            format!("{body} · copied")
        } else {
            body
        };
        if notify::may_show(&app, Kind::Always, &title) && !interactive(&app, &c, &title, &body) {
            notify::show_plain(&app, &title, &body);
        }
    });
}

#[cfg(target_os = "macos")]
fn interactive(app: &AppHandle, c: &VerificationCode, title: &str, body: &str) -> bool {
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};
    let bundle = if tauri::is_dev() {
        "com.apple.Terminal".to_string()
    } else {
        app.config().identifier.clone()
    };
    let _ = mac_notification_sys::set_application(&bundle);
    let mut n = Notification::new();
    n.title(title)
        .message(body)
        .main_button(MainButton::SingleAction("Copy"))
        .close_button("Dismiss");
    match n.send() {
        Ok(NotificationResponse::ActionButton(_)) => {
            if let Err(e) = copy_by_id(app, &c.id) {
                notify::post_always(app, "Couldn't copy the code", &e);
            }
        }
        Ok(NotificationResponse::Click) => crate::tray::show_window(app),
        Ok(_) => {}
        Err(e) => {
            eprintln!("[notify] couldn't show a notification: {e}");
            return false;
        }
    }
    true
}

#[cfg(windows)]
fn interactive(app: &AppHandle, c: &VerificationCode, title: &str, body: &str) -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_string_lossy().to_string()))
        .unwrap_or_default();
    let dev = exe_dir.ends_with("target\\debug") || exe_dir.ends_with("target\\release");
    let aumid = if dev {
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe".to_string()
    } else {
        app.config().identifier.clone()
    };
    let xml = toast_xml(title, body);
    let (app, id) = (app.clone(), c.id.clone());
    let shown = crate::notif_popup::win::show(
        &aumid,
        &xml,
        format!("otp\u{1}{id}"),
        move |arguments: String, _text: Option<String>| {
            if arguments == "copy" {
                let (app, id) = (app.clone(), id.clone());
                std::thread::spawn(move || {
                    if let Err(e) = copy_by_id(&app, &id) {
                        notify::post_always(&app, "Couldn't copy the code", &e);
                    }
                });
            }
        },
    );
    shown.is_ok()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn interactive(_app: &AppHandle, _c: &VerificationCode, _title: &str, _body: &str) -> bool {
    false
}

/// A Windows toast: the code, who it's from, Copy and Dismiss.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn toast_xml(title: &str, body: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual><actions><action content="Copy" arguments="copy"/><action content="Dismiss" arguments="dismiss"/></actions></toast>"#,
        esc(title),
        esc(body)
    )
}

/// Copy code `id` (still valid) to the clipboard.
fn copy_by_id(app: &AppHandle, id: &str) -> Result<(), String> {
    let engine = app.try_state::<Eng>().ok_or("not started")?;
    let codes = engine.verification_codes();
    let c = codes
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "That code has expired.".to_string())?;
    copy_code(app, &c.code)?;
    let _ = app.emit("verification-code-copied", id);
    Ok(())
}

// --- the window -----------------------------------------------------------------------

/// A code shown here, for the window and the quick panel.
#[derive(Clone, Serialize, specta::Type)]
pub struct OtpView {
    pub id: String,
    pub from: String,
    pub code: String,
    pub app: String,
    pub issuer: String,
    /// Milliseconds until it's gone.
    pub expires_in_ms: u32,
}

/// The codes your phone sent that are still valid, newest first.
#[tauri::command]
#[specta::specta]
pub fn verification_codes(engine: State<Eng>) -> Vec<OtpView> {
    let now = now_ms();
    engine
        .verification_codes()
        .iter()
        .map(|c| OtpView {
            id: c.id.clone(),
            from: c.from.clone(),
            code: c.code.clone(),
            app: c.app.clone(),
            issuer: c.issuer.clone(),
            expires_in_ms: left_ms(c.expires_at, now),
        })
        .collect()
}

/// Copy code `id` to the clipboard (concealed; cleared after 90 seconds).
#[tauri::command]
#[specta::specta]
pub fn copy_verification_code(app: AppHandle, id: String) -> Result<(), String> {
    copy_by_id(&app, &id)
}

/// Forget code `id` now.
#[tauri::command]
#[specta::specta]
pub fn dismiss_verification_code(engine: State<Eng>, id: String) -> bool {
    engine.dismiss_verification_code(&id)
}

/// Whether codes are copied as they arrive.
#[tauri::command]
#[specta::specta]
pub fn otp_auto_copy() -> bool {
    auto_copy_on()
}

/// Copy codes as they arrive (off by default).
#[tauri::command]
#[specta::specta]
pub fn set_otp_auto_copy(on: bool) {
    if let Some(p) = PREFS.get() {
        p.auto_copy.store(on, Ordering::Relaxed);
        if let Some(kv) = &p.kv {
            if let Err(e) = kv.set(AUTO_KEY, if on { "on" } else { "off" }) {
                eprintln!("[otp] couldn't save the setting: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clipboard_is_cleared_after_ninety_seconds_if_it_still_holds_the_code() {
        let t = Instant::now();
        let code = "482913";
        assert!(!should_clear(t, t, Some(code), code));
        assert!(!should_clear(
            t,
            t + Duration::from_secs(89),
            Some(code),
            code
        ));
        assert!(should_clear(
            t,
            t + Duration::from_secs(90),
            Some(code),
            code
        ));
        assert!(should_clear(
            t,
            t + Duration::from_secs(300),
            Some(" 482913\n"),
            code
        ));
        // Something copied since is left alone, and so is an empty clipboard.
        assert!(!should_clear(t, t + CLEAR_AFTER, Some("hello"), code));
        assert!(!should_clear(t, t + CLEAR_AFTER, None, code));
        assert_eq!(CLEAR_AFTER, Duration::from_secs(90));
    }

    #[test]
    fn the_pop_up_says_who_its_from() {
        assert_eq!(
            texts("482913", "Messages", "HDFC Bank", "Pixel"),
            ("Code 482 913".into(), "from HDFC Bank on Pixel".into())
        );
        assert_eq!(
            texts("4412", "Messages", "", "Pixel"),
            ("Code 4412".into(), "from Messages on Pixel".into())
        );
        assert_eq!(
            texts("12345678", "", "", "Pixel"),
            ("Code 1234 5678".into(), "from Pixel".into())
        );
        assert_eq!(spaced("AB12CD3"), "AB12 CD3");
    }

    #[test]
    fn time_left_never_goes_negative() {
        assert_eq!(left_ms(10_000, 4_000), 6_000);
        assert_eq!(left_ms(10_000, 12_000), 0);
    }

    #[test]
    fn the_toast_escapes_and_offers_copy() {
        let x = toast_xml("Code 482 913", "from A&B <Bank>");
        assert!(x.contains("from A&amp;B &lt;Bank&gt;"));
        assert!(x.contains(r#"arguments="copy""#));
    }
}
