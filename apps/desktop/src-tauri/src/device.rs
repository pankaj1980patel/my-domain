// The desktop's side of the quick device actions (mdcore `features::device`):
// it reports this computer's battery to the core, rings (window to the front
// plus an OS notification; the UI plays the sound) and opens links a sibling
// sends, and runs its volume / mute actions. No extra crates: the battery comes from `pmset` (macOS) or sysfs
// (Linux), the notification from `osascript` / `notify-send`.

use std::process::Command;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::Eng;

/// Push this computer's status (battery, charging) to the core now and every
/// 60 s, so a sibling's `request_status` gets a fresh answer. Machines without
/// a battery (or Windows, not read yet) push `{}`.
pub fn spawn_status_reporter(app: AppHandle) {
    std::thread::Builder::new()
        .name("device-status".into())
        .spawn(move || loop {
            push_status(&app);
            std::thread::sleep(Duration::from_secs(60));
        })
        .ok();
}

/// Battery, volume, mute and the actions this computer takes, to the core.
fn push_status(app: &AppHandle) {
    let mut status = serde_json::json!({});
    if let Some((battery, charging)) = read_battery() {
        status["battery"] = battery.into();
        status["charging"] = charging.into();
    }
    if let Some((volume, muted)) = sound::read() {
        status["volume"] = volume.into();
        status["muted"] = muted.into();
        status["actions"] = serde_json::json!(["volume", "mute"]);
    }
    if let Some(engine) = app.try_state::<Eng>() {
        engine.set_device_status(&status.to_string());
    }
}

/// A sibling asked this computer to run an action (`volume {step}` or
/// `mute {on}`); then the status is pushed again so it shows the new level.
pub fn on_action(app: &AppHandle, name: &str, args: &serde_json::Value) {
    match name {
        "volume" => sound::step(if args["step"].as_i64().unwrap_or(1) < 0 { -10 } else { 10 }),
        "mute" => sound::mute(args["on"].as_bool()),
        _ => return,
    }
    push_status(app);
}

/// The system output volume, through the OS's own command-line tools.
mod sound {
    use std::process::Command;

    fn run(cmd: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// (volume 0-100, muted)
    #[cfg(target_os = "macos")]
    pub fn read() -> Option<(u8, bool)> {
        let v = run("osascript", &["-e", "output volume of (get volume settings)"])?;
        let m = run("osascript", &["-e", "output muted of (get volume settings)"])?;
        Some((v.parse::<u8>().ok()?.min(100), m == "true"))
    }

    #[cfg(target_os = "macos")]
    pub fn step(by: i32) {
        if let Some((v, _)) = read() {
            let to = (i32::from(v) + by).clamp(0, 100).to_string();
            let _ = run("osascript", &["-e", &format!("set volume output volume {to}")]);
        }
    }

    #[cfg(target_os = "macos")]
    pub fn mute(on: Option<bool>) {
        let on = on.unwrap_or_else(|| !read().is_some_and(|(_, m)| m));
        let _ = run("osascript", &["-e", &format!("set volume output muted {on}")]);
    }

    #[cfg(target_os = "linux")]
    pub fn read() -> Option<(u8, bool)> {
        let v = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let pct = v.split('/').nth(1)?.trim().trim_end_matches('%').trim().parse::<u32>().ok()?;
        let m = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])?;
        Some((pct.min(100) as u8, m.ends_with("yes")))
    }

    #[cfg(target_os = "linux")]
    pub fn step(by: i32) {
        let arg = format!("{by:+}%");
        let _ = run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &arg]);
    }

    #[cfg(target_os = "linux")]
    pub fn mute(on: Option<bool>) {
        let v = match on {
            Some(true) => "1",
            Some(false) => "0",
            None => "toggle",
        };
        let _ = run("pactl", &["set-sink-mute", "@DEFAULT_SINK@", v]);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn read() -> Option<(u8, bool)> {
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn step(_by: i32) {}

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn mute(_on: Option<bool>) {}
}

/// (percent 0-100, plugged in), or `None` without a battery.
#[cfg(target_os = "macos")]
fn read_battery() -> Option<(u8, bool)> {
    let out = Command::new("pmset").args(["-g", "batt"]).output().ok()?;
    parse_pmset(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(target_os = "linux")]
fn read_battery() -> Option<(u8, bool)> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).map(|s| s.trim().to_string());
        if read("type").ok().as_deref() != Some("Battery") {
            continue;
        }
        let Some(level) = read("capacity").ok().and_then(|c| c.parse::<u8>().ok()) else {
            continue;
        };
        let status = read("status").unwrap_or_default();
        return Some((level.min(100), status == "Charging" || status == "Full"));
    }
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_battery() -> Option<(u8, bool)> {
    None
}

/// `pmset -g batt`:
///   Now drawing from 'AC Power'
///    -InternalBattery-0 (id=22610019) <TAB> 82%; charging; 0:54 remaining present: true
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_pmset(out: &str) -> Option<(u8, bool)> {
    let plugged = out.lines().next()?.contains("'AC Power'");
    let line = out.lines().find(|l| l.contains("InternalBattery"))?;
    let pct = line.split_whitespace().find_map(|w| w.strip_suffix("%;"))?;
    Some((pct.parse::<u8>().ok()?.min(100), plugged))
}

/// A sibling is looking for this computer: bring the window to the front and
/// show an OS notification. The UI hears the same `ring-event` and plays the
/// sound until it is dismissed.
pub fn on_ring(app: &AppHandle, from: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    let name = app
        .try_state::<Eng>()
        .and_then(|e| e.get_peers().into_iter().find(|p| p.node_id == from).map(|p| p.name))
        .unwrap_or_else(|| "One of your devices".to_string());
    notify("my-domain", &format!("{name} is looking for this computer"));
}

/// Open a link a sibling sent, in the default browser. The core only emits
/// http(s) links; checked again here since this hands it to the OS.
pub fn on_open_url(app: &AppHandle, url: &str) {
    if !mdcore::features::device::is_openable(url) {
        return;
    }
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        eprintln!("[device] couldn't open {url}: {e}");
    }
}

/// Best-effort OS notification without a plugin. The text goes in as
/// arguments, never into a script.
fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    let cmd = Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            title,
            body,
        ])
        .spawn();
    #[cfg(target_os = "linux")]
    let cmd = Command::new("notify-send").args([title, body]).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd: std::io::Result<std::process::Child> = {
        let _ = (title, body);
        Err(std::io::ErrorKind::Unsupported.into())
    };
    match cmd {
        // Reap it off-thread so it doesn't linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("[device] no OS notification: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pmset;

    #[test]
    fn reads_pmset() {
        let on_ac = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=22610019)\t82%; charging; 0:54 remaining present: true\n";
        assert_eq!(parse_pmset(on_ac), Some((82, true)));
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t100%; discharged; 5:00 remaining present: true\n";
        assert_eq!(parse_pmset(on_battery), Some((100, false)));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), None);
    }
}
