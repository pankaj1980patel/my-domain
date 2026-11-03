// The desktop's side of the quick device actions (mdcore `features::device`):
// it reports this computer's battery to the core, rings (window to the front
// plus an OS notification; the UI plays the sound) and opens links a sibling
// sends, and runs its volume / mute actions. No extra crates: the battery comes from `pmset` (macOS) or sysfs
// (Linux); the notification goes through `crate::notify`.

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

/// Push the status now (after a setting that changes it).
pub fn refresh_status(app: &AppHandle) {
    push_status(app);
}

/// my-domain may post input events (Privacy & Security > Accessibility).
#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    slides::mac::trusted()
}

/// Battery, volume, mute and the actions this computer takes, to the core.
fn push_status(app: &AppHandle) {
    let mut status = serde_json::json!({});
    if let Some((battery, charging)) = read_battery() {
        status["battery"] = battery.into();
        status["charging"] = charging.into();
    }
    let downloads = app.try_state::<crate::DownloadDir>().map(|d| d.0.clone());
    if let Some((free, total)) = downloads.as_deref().and_then(read_storage) {
        status["storage"] = serde_json::json!({ "free": free, "total": total });
    }
    crate::sysinfo::extend(&mut status);
    let mut actions = Vec::new();
    if let Some((volume, muted)) = sound::read() {
        status["volume"] = volume.into();
        status["muted"] = muted.into();
        actions.extend(["volume", "mute"]);
    }
    if sound::has_media() {
        actions.push("media");
    }
    if screen::can_lock() {
        actions.push("lock");
    }
    // Searches open in the browser here (YouTube, Spotify's web player, the web).
    actions.push("search");
    status["search_apps"] = serde_json::json!(["youtube", "spotify", "web"]);
    match slides::ready() {
        Ok(()) => actions.push("slides"),
        // Tell the phone why there's no slide remote, so it can say so.
        Err(why) => status["slides_needs"] = why.into(),
    }
    // The phone's touchpad: listed while allowed, with what's missing if
    // it can't work yet (the phone says so; the first try here tells the
    // user too).
    if app
        .try_state::<Eng>()
        .is_some_and(|e| e.remote_input_allowed())
    {
        actions.push("input");
        if let Err(why) = crate::remote_input::ready() {
            status["input_needs"] = why.into();
        }
    }
    // Your phone sees this screen: listed while allowed, with what's
    // missing if snapshots can't be taken yet.
    if app.try_state::<Eng>().is_some_and(|e| e.screen_allowed()) {
        actions.push("screen");
        if let Err(why) = crate::screen_share::ready() {
            status["screen_needs"] = why.into();
        }
    }
    if awake::available() {
        actions.push("keep_awake");
        if let Some(until) = awake::until() {
            status["awake_until"] = until.into();
        }
    }
    if !actions.is_empty() {
        status["actions"] = actions.into();
    }
    if let Some(engine) = app.try_state::<Eng>() {
        engine.set_device_status(&status.to_string());
    }
}

/// Tell the core what's playing (Linux: any MPRIS player; macOS: Spotify
/// or Music), every few seconds, but only while some device has looked at
/// this computer lately: nobody would hear it otherwise, and each read
/// runs a command.
pub fn spawn_now_playing(app: AppHandle) {
    std::thread::Builder::new()
        .name("now-playing".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(3));
            let Some(engine) = app.try_state::<Eng>() else {
                continue;
            };
            if !engine.now_playing_watched() {
                continue;
            }
            let media = sound::now_playing().map(|m| m.to_string());
            engine.set_now_playing(media.as_deref().unwrap_or(""));
        })
        .ok();
}

/// One line of `state<TAB>title<TAB>artist<TAB>album` (playerctl's
/// "Playing"/"Paused", or AppleScript's "playing"/"paused") into the
/// core's now-playing object, from player `app`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn parse_now_playing(line: &str, app: &str) -> Option<serde_json::Value> {
    let mut cols = line.split('\t');
    let state = match cols.next()?.trim().to_ascii_lowercase().as_str() {
        "playing" => "playing",
        "paused" => "paused",
        _ => return None,
    };
    let title = cols.next()?.trim();
    if title.is_empty() {
        return None;
    }
    let mut m = serde_json::json!({ "title": title, "state": state, "app": app });
    for (key, v) in ["artist", "album"].into_iter().zip(cols) {
        if !v.trim().is_empty() {
            m[key] = v.trim().into();
        }
    }
    Some(m)
}

/// A sibling asked this computer to run an action (`volume {step}`,
/// `mute {on}`, `media {key}`, `lock` or `slides {key}`: next, previous,
/// black); then the status is pushed again
/// so it shows the new level.
pub fn on_action(app: &AppHandle, name: &str, args: &serde_json::Value) {
    match name {
        "volume" => sound::step(if args["step"].as_i64().unwrap_or(1) < 0 {
            -10
        } else {
            10
        }),
        "mute" => sound::mute(args["on"].as_bool()),
        "media" => sound::media(args["key"].as_str().unwrap_or_default()),
        "lock" => screen::lock(),
        "search" => {
            let q = args["query"].as_str().unwrap_or_default();
            if let Some(url) = search_url(args["app"].as_str().unwrap_or_default(), q) {
                on_open_url(app, &url);
            }
            return;
        }
        "slides" => {
            if let Err(why) = slides::press(args["key"].as_str().unwrap_or_default()) {
                crate::notify::post(app, "my-domain can't change slides", why);
            }
        }
        "keep_awake" => {
            let minutes = args["minutes"].as_u64().unwrap_or(0);
            let secs = minutes.min(u64::from(mdcore::engine::MAX_AWAKE_MINUTES)) * 60;
            if let Err(why) = awake::keep(secs) {
                eprintln!("[device] keep awake: {why}");
                crate::notify::post(app, "my-domain can't keep this computer awake", &why);
            }
        }
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
        let v = run(
            "osascript",
            &["-e", "output volume of (get volume settings)"],
        )?;
        let m = run(
            "osascript",
            &["-e", "output muted of (get volume settings)"],
        )?;
        Some((v.parse::<u8>().ok()?.min(100), m == "true"))
    }

    #[cfg(target_os = "macos")]
    pub fn step(by: i32) {
        if let Some((v, _)) = read() {
            let to = (i32::from(v) + by).clamp(0, 100).to_string();
            let _ = run(
                "osascript",
                &["-e", &format!("set volume output volume {to}")],
            );
        }
    }

    #[cfg(target_os = "macos")]
    pub fn mute(on: Option<bool>) {
        let on = on.unwrap_or_else(|| !read().is_some_and(|(_, m)| m));
        let _ = run(
            "osascript",
            &["-e", &format!("set volume output muted {on}")],
        );
    }

    #[cfg(target_os = "linux")]
    pub fn read() -> Option<(u8, bool)> {
        let v = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let pct = v
            .split('/')
            .nth(1)?
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<u32>()
            .ok()?;
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

    /// Media keys for whatever is playing, through MPRIS (`playerctl`).
    #[cfg(target_os = "linux")]
    pub fn has_media() -> bool {
        run("playerctl", &["--version"]).is_some()
    }

    #[cfg(target_os = "linux")]
    pub fn media(key: &str) {
        let cmd = match key {
            "play_pause" => "play-pause",
            "next" => "next",
            "previous" => "previous",
            _ => return,
        };
        match pick_player() {
            Some(p) => run("playerctl", &["--player", &p, cmd]),
            None => run("playerctl", &[cmd]),
        };
    }

    /// The player you're listening to, else one you paused: plain
    /// `playerctl` takes the first one listed, maybe an idle browser.
    #[cfg(target_os = "linux")]
    fn pick_player() -> Option<String> {
        let players = run("playerctl", &["--list-all"]).unwrap_or_default();
        let status = |p: &str| run("playerctl", &["--player", p, "status"]).unwrap_or_default();
        ["Playing", "Paused"].iter().find_map(|want| {
            players
                .lines()
                .find(|p| status(p) == *want)
                .map(str::to_string)
        })
    }

    /// What's playing, through MPRIS: `{title, artist, album, app, state}`.
    #[cfg(target_os = "linux")]
    pub fn now_playing() -> Option<serde_json::Value> {
        let player = pick_player()?;
        let fmt = "{{status}}\t{{xesam:title}}\t{{xesam:artist}}\t{{xesam:album}}";
        let line = run(
            "playerctl",
            &["--player", &player, "metadata", "--format", fmt],
        )?;
        // "spotify", "vlc", "firefox.instance_1_23": the app is the first part.
        let app = player.split('.').next().unwrap_or_default();
        super::parse_now_playing(&line, app)
    }

    /// What's playing in Spotify or Music, through AppleScript (no private
    /// frameworks: other players, and browsers, aren't seen). Only asks an
    /// app that's already running, so it never starts one. The first time,
    /// macOS asks whether my-domain may control that app.
    #[cfg(target_os = "macos")]
    pub fn now_playing() -> Option<serde_json::Value> {
        let mut paused = None;
        for app in ["Spotify", "Music"] {
            let script = format!(
                "if application \"{app}\" is running then\n\
                 tell application \"{app}\"\n\
                 try\n\
                 return (player state as string) & tab & (name of current track) & tab & (artist of current track) & tab & (album of current track)\n\
                 end try\n\
                 end tell\n\
                 end if\n\
                 return \"\""
            );
            let Some(line) = run("osascript", &["-e", &script]) else {
                continue;
            };
            match super::parse_now_playing(&line, app) {
                Some(m) if m["state"] == "playing" => return Some(m),
                Some(m) => paused = paused.or(Some(m)),
                None => {}
            }
        }
        paused
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn now_playing() -> Option<serde_json::Value> {
        None
    }

    // macOS has no command for media keys; posting the key events needs
    // the Accessibility permission. Not offered yet.
    #[cfg(not(target_os = "linux"))]
    pub fn has_media() -> bool {
        false
    }

    #[cfg(not(target_os = "linux"))]
    pub fn media(_key: &str) {}

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn read() -> Option<(u8, bool)> {
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn step(_by: i32) {}

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn mute(_on: Option<bool>) {}
}

/// Presentation remote: the keys a slide clicker sends, typed into whatever
/// window is in front (PowerPoint, Keynote, Impress, Google Slides, a PDF
/// viewer). `next` / `previous` are Page Down / Page Up, `black` is B.
mod slides {
    /// Whether keys can be sent here; else what's missing, for the user.
    #[cfg(target_os = "linux")]
    pub fn ready() -> Result<(), &'static str> {
        let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
        // Wayland: wtype; X11 (or an X app under Wayland): xdotool.
        let wtype = wayland && has("wtype", &["-h"]);
        let xdotool = std::env::var_os("DISPLAY").is_some() && has("xdotool", &["version"]);
        if wtype || xdotool {
            Ok(())
        } else if wayland {
            Err("wtype")
        } else {
            Err("xdotool")
        }
    }

    #[cfg(target_os = "linux")]
    fn has(cmd: &str, args: &[&str]) -> bool {
        std::process::Command::new(cmd).args(args).output().is_ok()
    }

    #[cfg(target_os = "linux")]
    pub fn press(key: &str) -> Result<(), &'static str> {
        let (x, w) = match key {
            "next" => ("Next", "Next"),
            "previous" => ("Prior", "Prior"),
            "black" => ("b", "b"),
            _ => return Ok(()),
        };
        ready().map_err(|_| {
            "Install xdotool (X11) or wtype (Wayland) to change slides from your phone."
        })?;
        let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland");
        let ok = if wayland && has("wtype", &["-h"]) {
            run("wtype", &["-k", w])
        } else {
            run("xdotool", &["key", x])
        };
        if ok {
            Ok(())
        } else {
            Err("The key couldn't be sent.")
        }
    }

    #[cfg(target_os = "linux")]
    fn run(cmd: &str, args: &[&str]) -> bool {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Key events through CoreGraphics. macOS only delivers them once the
    /// user has allowed my-domain under Privacy & Security > Accessibility;
    /// until then this says so instead of offering the remote.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub(super) mod mac {
        use std::ffi::c_void;

        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }

        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGEventCreateKeyboardEvent(
                source: *const c_void,
                keycode: u16,
                key_down: bool,
            ) -> *mut c_void;
            fn CGEventPost(tap: u32, event: *mut c_void);
        }

        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            fn CFRelease(cf: *const c_void);
        }

        /// `kCGHIDEventTap`: as if typed on the keyboard.
        const HID_EVENT_TAP: u32 = 0;

        pub fn trusted() -> bool {
            // SAFETY: no arguments; only reads this process's permission.
            unsafe { AXIsProcessTrusted() }
        }

        /// Press and release virtual key `code`.
        pub fn tap(code: u16) -> bool {
            for down in [true, false] {
                // SAFETY: a null source is allowed (the default source);
                // the event is checked for null, posted, then released
                // exactly once (it was created here, so we own it).
                unsafe {
                    let ev = CGEventCreateKeyboardEvent(std::ptr::null(), code, down);
                    if ev.is_null() {
                        return false;
                    }
                    CGEventPost(HID_EVENT_TAP, ev);
                    CFRelease(ev);
                }
            }
            true
        }
    }

    #[cfg(target_os = "macos")]
    pub fn ready() -> Result<(), &'static str> {
        if mac::trusted() {
            Ok(())
        } else {
            Err("accessibility")
        }
    }

    #[cfg(target_os = "macos")]
    pub fn press(key: &str) -> Result<(), &'static str> {
        // Virtual key codes: Page Down 121, Page Up 116, B 11.
        let code = match key {
            "next" => 121,
            "previous" => 116,
            "black" => 11,
            _ => return Ok(()),
        };
        let needs = "Allow my-domain under System Settings > Privacy & Security > Accessibility to change slides from your phone.";
        if !mac::trusted() {
            return Err(needs);
        }
        if mac::tap(code) {
            Ok(())
        } else {
            Err(needs)
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn ready() -> Result<(), &'static str> {
        Err("unsupported")
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn press(_key: &str) -> Result<(), &'static str> {
        Err("Changing slides isn't supported on this computer yet.")
    }
}

/// Keeping the computer awake for a while ("don't sleep while I copy this"):
/// macOS `caffeinate -d -i -t <secs>`, Linux `systemd-inhibit` around a
/// `sleep <secs>`. One at a time: a new request replaces the old one, 0
/// minutes stops it; the child ends by itself when the time is up.
mod awake {
    use std::process::{Child, Command, Stdio};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The running inhibitor and when it ends (unix seconds).
    static CURRENT: Mutex<Option<(Child, u64)>> = Mutex::new(None);

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    /// The command that keeps the machine awake for `secs`.
    pub(super) fn command(secs: u64) -> Option<(String, Vec<String>)> {
        let secs = secs.to_string();
        if cfg!(target_os = "macos") {
            Some((
                "caffeinate".into(),
                vec!["-d".into(), "-i".into(), "-t".into(), secs],
            ))
        } else if cfg!(target_os = "linux") {
            Some((
                "systemd-inhibit".into(),
                vec![
                    "--what=idle:sleep".into(),
                    "--who=my-domain".into(),
                    "--why=Kept awake from another of your devices".into(),
                    "--mode=block".into(),
                    "sleep".into(),
                    secs,
                ],
            ))
        } else {
            None
        }
    }

    /// The tool is there (macOS always has `caffeinate`).
    pub fn available() -> bool {
        match command(1) {
            Some((cmd, _)) if cmd == "caffeinate" => true,
            Some((cmd, _)) => Command::new(cmd)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success()),
            None => false,
        }
    }

    /// When the current keep-awake ends, if one is running.
    pub fn until() -> Option<u64> {
        let mut cur = CURRENT.lock().ok()?;
        let running = match cur.as_mut() {
            Some((child, until)) => matches!(child.try_wait(), Ok(None)) && *until > now(),
            None => false,
        };
        if !running {
            *cur = None;
        }
        cur.as_ref().map(|(_, until)| *until)
    }

    /// Stay awake for `secs` from now (0 = stop).
    /// Keep awake for `secs` (0 stops). `Err` says why it couldn't, for
    /// the user: e.g. Linux refuses a sleep inhibitor to an app outside the
    /// logged-in desktop session, and `systemd-inhibit` then exits at once.
    pub fn keep(secs: u64) -> Result<(), String> {
        let Ok(mut cur) = CURRENT.lock() else {
            return Ok(());
        };
        if let Some((mut child, _)) = cur.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if secs == 0 {
            return Ok(());
        }
        let Some((cmd, args)) = command(secs) else {
            return Err("Keeping awake isn't supported on this computer.".into());
        };
        let mut child = Command::new(&cmd)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't run {cmd}: {e}"))?;
        // A refused inhibitor ends right away; a working one runs on.
        std::thread::sleep(std::time::Duration::from_millis(400));
        if let Ok(Some(status)) = child.try_wait() {
            if !status.success() {
                return Err(format!(
                    "{cmd} was refused (the system may only allow this to apps \
                     started from your desktop session)."
                ));
            }
        }
        *cur = Some((child, now() + secs));
        Ok(())
    }
}

/// Locking the screen ("I walked away and left it open").
mod screen {
    use std::process::Command;

    fn ok(cmd: &str, args: &[&str]) -> bool {
        Command::new(cmd)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// The display sleeps; with "require password after sleep" on (the
    /// default), that locks the Mac.
    #[cfg(target_os = "macos")]
    pub fn can_lock() -> bool {
        true
    }

    #[cfg(target_os = "macos")]
    pub fn lock() {
        ok("pmset", &["displaysleepnow"]);
    }

    /// Through logind (works on every desktop that locks on its signal),
    /// then the freedesktop screensaver as a fallback.
    #[cfg(target_os = "linux")]
    pub fn can_lock() -> bool {
        ok("loginctl", &["--version"])
    }

    #[cfg(target_os = "linux")]
    pub fn lock() {
        let sessions = graphical_sessions();
        for id in &sessions {
            ok("loginctl", &["lock-session", id]);
        }
        // Desktops (GNOME, KDE, Xfce...) lock on logind's signal and say so.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let locked = sessions.iter().any(|id| {
            Command::new("loginctl")
                .args(["show-session", id, "-p", "LockedHint", "--value"])
                .output()
                .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
        });
        if locked || ok("xdg-screensaver", &["lock"]) {
            return;
        }
        // A bare window manager (i3, sway...) has nobody listening: run its
        // locker ourselves. It stays up until unlocked, so don't wait.
        for locker in ["i3lock", "swaylock", "slock"] {
            if let Ok(mut child) = Command::new(locker).spawn() {
                std::thread::spawn(move || child.wait());
                return;
            }
        }
    }

    /// This user's sessions on a screen (X11 or Wayland). Not just
    /// `XDG_SESSION_ID`: an app started from a terminal over ssh would
    /// lock that text session instead.
    #[cfg(target_os = "linux")]
    fn graphical_sessions() -> Vec<String> {
        let me = Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let list = Command::new("loginctl")
            .args(["list-sessions", "--no-legend"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        list.lines()
            .filter_map(|l| {
                let mut cols = l.split_whitespace();
                let (id, uid) = (cols.next()?, cols.next()?);
                (uid == me).then(|| id.to_string())
            })
            .filter(|id| {
                Command::new("loginctl")
                    .args(["show-session", id, "-p", "Type", "--value"])
                    .output()
                    .is_ok_and(|o| {
                        let t = String::from_utf8_lossy(&o.stdout);
                        matches!(t.trim(), "x11" | "wayland")
                    })
            })
            .collect()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn can_lock() -> bool {
        false
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn lock() {}
}

/// (percent 0-100, plugged in), or `None` without a battery.
#[cfg(target_os = "macos")]
fn read_battery() -> Option<(u8, bool)> {
    let out = std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .ok()?;
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

/// (free, total) bytes of the disk holding `dir`, from `df` (macOS and
/// Linux); `None` elsewhere or if it can't be read.
#[cfg(unix)]
fn read_storage(dir: &std::path::Path) -> Option<(u64, u64)> {
    let out = std::process::Command::new("df")
        .arg("-kP")
        .arg(dir)
        .output()
        .ok()?;
    parse_df(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(unix))]
fn read_storage(_dir: &std::path::Path) -> Option<(u64, u64)> {
    None
}

/// `df -kP <dir>`:
///   Filesystem   1024-blocks      Used Available Capacity Mounted on
///   /dev/disk3s5   482797652 400000000  70000000      86% /System/Volumes/Data
/// The numbers are the three before the `NN%` column (the mount point may
/// hold spaces). Returns (available, total) in bytes.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_df(out: &str) -> Option<(u64, u64)> {
    let cols: Vec<&str> = out.lines().nth(1)?.split_whitespace().collect();
    let pct = cols.iter().position(|c| c.ends_with('%'))?;
    let num = |i: usize| cols.get(i)?.parse::<u64>().ok();
    let (total, free) = (num(pct.checked_sub(3)?)?, num(pct - 1)?);
    Some((free * 1024, total * 1024))
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
    let name = crate::notify::device_name(app, from);
    // Asked for on purpose: shown even while pop-ups are held for Do Not
    // Disturb.
    crate::notify::post_urgent(
        app,
        "my-domain",
        &format!("{name} is looking for this computer"),
    );
}

/// The web page that searches `app` for `query` (a computer has no apps for
/// these: the browser does it).
fn search_url(app: &str, query: &str) -> Option<String> {
    let q = urlencode(query.trim());
    if q.is_empty() {
        return None;
    }
    Some(match app {
        "youtube" => format!("https://www.youtube.com/results?search_query={q}"),
        "spotify" => format!("https://open.spotify.com/search/{q}"),
        "web" => format!("https://duckduckgo.com/?q={q}"),
        _ => return None,
    })
}

/// Percent-encode everything but unreserved characters.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{parse_df, parse_now_playing, parse_pmset, search_url};

    #[test]
    fn search_pages() {
        assert_eq!(
            search_url("youtube", " miles davis & co "),
            Some("https://www.youtube.com/results?search_query=miles%20davis%20%26%20co".into())
        );
        assert_eq!(
            search_url("spotify", "Kind of Blue").as_deref(),
            Some("https://open.spotify.com/search/Kind%20of%20Blue")
        );
        assert!(search_url("web", "\u{e9}t\u{e9}")
            .unwrap()
            .ends_with("%C3%A9t%C3%A9"));
        assert_eq!(search_url("tiktok", "x"), None);
        assert_eq!(search_url("youtube", "  "), None);
    }

    #[test]
    fn reads_now_playing() {
        let m = parse_now_playing("Playing\tSo What\tMiles Davis\tKind of Blue", "spotify");
        assert_eq!(
            m,
            Some(serde_json::json!({
                "title": "So What", "artist": "Miles Davis", "album": "Kind of Blue",
                "app": "spotify", "state": "playing",
            }))
        );
        let paused = parse_now_playing("paused\tTrack 1\t\t", "Music").unwrap();
        assert_eq!(paused["state"], "paused");
        assert!(paused.get("artist").is_none());
        assert_eq!(parse_now_playing("Stopped\tx\ty\tz", "vlc"), None);
        assert_eq!(parse_now_playing("Playing\t\t\t", "vlc"), None);
        assert_eq!(parse_now_playing("", "vlc"), None);
    }

    #[test]
    fn keep_awake_runs_the_os_tool_for_the_time_asked() {
        let cmd = super::awake::command(1800);
        if cfg!(target_os = "macos") {
            let (c, args) = cmd.expect("macOS has caffeinate");
            assert_eq!(c, "caffeinate");
            assert_eq!(args, ["-d", "-i", "-t", "1800"]);
        } else if cfg!(target_os = "linux") {
            let (c, args) = cmd.expect("Linux uses systemd-inhibit");
            assert_eq!(c, "systemd-inhibit");
            assert!(args.iter().any(|a| a == "--what=idle:sleep"));
            assert_eq!(args[args.len() - 2..], ["sleep", "1800"]);
        }
    }

    #[test]
    fn reads_df() {
        let linux = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/nvme0n1p4 209715200 140509184 69206016 68% /home/my disk\n";
        assert_eq!(
            parse_df(linux),
            Some((69_206_016 * 1024, 209_715_200 * 1024))
        );
        // Inode columns after the capacity (macOS without -P) are ignored.
        let mac = "Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on\n/dev/disk3s5 482797652 400000000 70000000 86% 1 2 0% /System/Volumes/Data\n";
        assert_eq!(parse_df(mac), Some((70_000_000 * 1024, 482_797_652 * 1024)));
        assert_eq!(parse_df("Filesystem\n"), None);
    }

    #[test]
    fn reads_pmset() {
        let on_ac = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=22610019)\t82%; charging; 0:54 remaining present: true\n";
        assert_eq!(parse_pmset(on_ac), Some((82, true)));
        let on_battery = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t100%; discharged; 5:00 remaining present: true\n";
        assert_eq!(parse_pmset(on_battery), Some((100, false)));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n"), None);
    }
}
