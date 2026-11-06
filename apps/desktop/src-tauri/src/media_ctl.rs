//! Now playing and media controls on the desktop (mdcore `features::media`).
//!
//! - This computer's side: what plays here (Linux: any MPRIS player through
//!   `playerctl`; macOS: Spotify or Music through AppleScript) is read every
//!   few seconds while some device watches this computer, with its position,
//!   length and volume, and handed to the core. A `media_cmd` from one of
//!   your devices (`CoreEvent::MediaCommand`) plays, pauses, skips, seeks or
//!   sets that player's volume, then what plays is read again at once so
//!   the device sees what it did. Every command runs as an argument vector,
//!   never through a shell; AppleScript gets only numbers and a fixed app
//!   name. Windows has no command-line way to reach players: nothing.
//! - The watching side: Tauri commands for the Now playing tile on another
//!   device's page (`media_command`, `media_watch`, `media_supported`).

use std::process::Command;
use std::time::Duration;

use mdcore::CoreError;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::Eng;

/// The volume scale this computer reports (`volume_max`).
pub const VOLUME_MAX: u64 = 100;

// --- the watching side: commands ---------------------------------------------

/// Control what plays on device `node_id`: `action` "play", "pause",
/// "play_pause", "next", "previous", "seek" (`value`: ms), "volume_up",
/// "volume_down", "volume_set" (`value`: level on its `volume_max` scale)
/// or "session" (`session`: the one to show). Needs a live link: without
/// one it starts connecting and says so. (`value` is a number in
/// TypeScript: whole ms up to a week, far inside what a double holds.)
#[tauri::command]
#[specta::specta]
pub fn media_command(
    node_id: String,
    action: String,
    value: Option<f64>,
    session: Option<String>,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    let value = value.filter(|v| v.is_finite()).map(|v| v.round() as i64);
    engine.media_command(&node_id, &action, value, session.as_deref())
}

/// Ask device `node_id` to keep this computer posted on what it plays for
/// the next few minutes (the tile calls it again while it's open). False:
/// no live link now, nothing sent.
#[tauri::command]
#[specta::specta]
pub fn media_watch(node_id: String, engine: State<Eng>) -> Result<bool, CoreError> {
    engine.media_watch(&node_id)
}

/// Whether device `node_id` takes media commands.
#[tauri::command]
#[specta::specta]
pub fn media_supported(node_id: String, engine: State<Eng>) -> bool {
    engine.media_supported(&node_id)
}

// --- this computer's side ------------------------------------------------------

/// Tell the core what's playing, every few seconds, but only while some
/// device has looked at this computer lately: nobody would hear it
/// otherwise, and each read runs a command.
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
            report(&engine);
        })
        .ok();
}

fn report(engine: &Eng) {
    let media = now_playing().map(|m| m.to_string());
    engine.set_now_playing(media.as_deref().unwrap_or(""));
}

/// One of your devices asked to control what plays here (already checked
/// by the core). Off the event thread: the players' tools may be slow.
pub fn on_command(app: &AppHandle, from: &str, action: &str, value: Option<i64>) {
    let (app, from, action) = (app.clone(), from.to_string(), action.to_string());
    std::thread::spawn(move || {
        if let Err(why) = run_command(&action, value) {
            eprintln!("[media] {action} from {from}: {why}");
        }
        // The device sees what its command did.
        std::thread::sleep(Duration::from_millis(250));
        if let Some(engine) = app.try_state::<Eng>() {
            report(&engine);
        }
    });
}

#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// One line of `state<TAB>title<TAB>artist<TAB>album` and, when the player
/// has them, `<TAB>position ms<TAB>length ms<TAB>volume 0-100` (playerctl's
/// "Playing"/"Paused", or AppleScript's "playing"/"paused") into the core's
/// now-playing object, from player `app`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub fn parse_now_playing(line: &str, app: &str) -> Option<Value> {
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
    let mut m = json!({ "title": title, "state": state, "app": app });
    for key in ["artist", "album"] {
        match cols.next().map(str::trim) {
            Some(v) if !v.is_empty() => m[key] = v.into(),
            _ => {}
        }
    }
    let num = |s: Option<&str>| {
        s.map(str::trim)
            .and_then(|s| s.replace(',', ".").parse::<f64>().ok())
            .filter(|f| f.is_finite() && *f >= 0.0)
    };
    let (position, duration, volume) = (num(cols.next()), num(cols.next()), num(cols.next()));
    if let Some(d) = duration.filter(|d| *d >= 1.0) {
        m["duration"] = (d as u64).into();
        m["can_seek"] = true.into();
    }
    if let Some(p) = position {
        m["position"] = (p as u64).into();
    }
    if let Some(v) = volume {
        m["volume"] = (v.round() as u64).min(VOLUME_MAX).into();
        m["volume_max"] = VOLUME_MAX.into();
    }
    // Both kinds of player skip either way.
    m["can_next"] = true.into();
    m["can_prev"] = true.into();
    Some(m)
}

/// playerctl's raw columns (position and length in µs, volume 0.0-1.0)
/// into the common line (ms, 0-100).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn mpris_line(raw: &str) -> String {
    let cols: Vec<&str> = raw.split('\t').collect();
    let conv = |i: usize, scale: f64| {
        cols.get(i)
            .and_then(|s| s.trim().parse::<f64>().ok())
            .map(|f| format!("{}", (f * scale).round() as i64))
            .unwrap_or_default()
    };
    let mut out: Vec<String> = cols.iter().take(4).map(|s| s.to_string()).collect();
    out.resize(4, String::new());
    out.push(conv(4, 0.001));
    out.push(conv(5, 0.001));
    out.push(conv(6, 100.0));
    out.join("\t")
}

/// The `playerctl` arguments (after `--player NAME`) for a command;
/// `None` for one it can't do. `volume` is the current level 0-100, for
/// the steps.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn playerctl_args(
    action: &str,
    value: Option<i64>,
    volume: Option<u64>,
) -> Option<Vec<String>> {
    let v = |s: &str| Some(vec![s.to_string()]);
    match action {
        "play" => v("play"),
        "pause" => v("pause"),
        "play_pause" => v("play-pause"),
        "next" => v("next"),
        "previous" => v("previous"),
        "seek" => Some(vec![
            "position".into(),
            format!("{:.3}", value? as f64 / 1000.0),
        ]),
        "volume_set" => Some(vec![
            "volume".into(),
            format!("{:.2}", value?.clamp(0, VOLUME_MAX as i64) as f64 / 100.0),
        ]),
        "volume_up" | "volume_down" => {
            let step = if action == "volume_up" { 10 } else { -10 };
            let to = (volume.unwrap_or(50) as i64 + step).clamp(0, VOLUME_MAX as i64);
            Some(vec!["volume".into(), format!("{:.2}", to as f64 / 100.0)])
        }
        _ => None,
    }
}

/// The AppleScript for a command to `app` (Spotify or Music only: both
/// take the same words); `None` for one it can't do or another app.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn applescript(app: &str, action: &str, value: Option<i64>) -> Option<String> {
    if !matches!(app, "Spotify" | "Music") {
        return None;
    }
    let body = match action {
        "play" => "play".to_string(),
        "pause" => "pause".to_string(),
        "play_pause" => "playpause".to_string(),
        "next" => "next track".to_string(),
        "previous" => "previous track".to_string(),
        "seek" => format!("set player position to {:.3}", value? as f64 / 1000.0),
        "volume_set" => format!("set sound volume to {}", value?.clamp(0, VOLUME_MAX as i64)),
        "volume_up" => "set sound volume to ((sound volume) + 10)".to_string(),
        "volume_down" => "set sound volume to ((sound volume) - 10)".to_string(),
        _ => return None,
    };
    Some(format!(
        "if application \"{app}\" is running then\ntell application \"{app}\" to {body}\nend if"
    ))
}

/// Do `action` on the player that plays here.
#[cfg(target_os = "linux")]
fn run_command(action: &str, value: Option<i64>) -> Result<(), String> {
    if action == "session" {
        return Ok(()); // one player is shown: nothing to switch
    }
    let player = pick_player().ok_or("nothing plays here")?;
    let volume = now_playing().and_then(|m| m["volume"].as_u64());
    let args = playerctl_args(action, value, volume).ok_or("not a command playerctl takes")?;
    let mut all: Vec<&str> = vec!["--player", &player];
    all.extend(args.iter().map(String::as_str));
    run("playerctl", &all)
        .map(|_| ())
        .ok_or_else(|| "playerctl failed".into())
}

#[cfg(target_os = "macos")]
fn run_command(action: &str, value: Option<i64>) -> Result<(), String> {
    if action == "session" {
        return Ok(());
    }
    let app = now_playing()
        .and_then(|m| m["app"].as_str().map(String::from))
        .ok_or("nothing plays here")?;
    let script = applescript(&app, action, value).ok_or("not a command it takes")?;
    run("osascript", &["-e", &script])
        .map(|_| ())
        .ok_or_else(|| "osascript failed".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn run_command(_action: &str, _value: Option<i64>) -> Result<(), String> {
    Err("no way to reach the players on this computer".into())
}

/// The player you're listening to, else one you paused: plain `playerctl`
/// takes the first one listed, maybe an idle browser.
#[cfg(target_os = "linux")]
pub fn pick_player() -> Option<String> {
    let players = run("playerctl", &["--list-all"]).unwrap_or_default();
    let status = |p: &str| run("playerctl", &["--player", p, "status"]).unwrap_or_default();
    ["Playing", "Paused"].iter().find_map(|want| {
        players
            .lines()
            .find(|p| status(p) == *want)
            .map(str::to_string)
    })
}

/// What's playing, through MPRIS.
#[cfg(target_os = "linux")]
pub fn now_playing() -> Option<Value> {
    let player = pick_player()?;
    let fmt = "{{status}}\t{{xesam:title}}\t{{xesam:artist}}\t{{xesam:album}}\t{{position}}\t{{mpris:length}}\t{{volume}}";
    let line = run(
        "playerctl",
        &["--player", &player, "metadata", "--format", fmt],
    )?;
    // "spotify", "vlc", "firefox.instance_1_23": the app is the first part.
    let app = player.split('.').next().unwrap_or_default();
    parse_now_playing(&mpris_line(&line), app)
}

/// What's playing in Spotify or Music, through AppleScript (no private
/// frameworks: other players, and browsers, aren't seen). Only asks an app
/// that's already running, so it never starts one. The first time, macOS
/// asks whether my-domain may control that app.
#[cfg(target_os = "macos")]
pub fn now_playing() -> Option<Value> {
    let mut paused = None;
    for app in ["Spotify", "Music"] {
        // Spotify gives the length in ms, Music in seconds.
        let length = if app == "Spotify" {
            "(duration of current track)"
        } else {
            "((duration of current track) * 1000)"
        };
        let script = format!(
            "if application \"{app}\" is running then\n\
             tell application \"{app}\"\n\
             try\n\
             return (player state as string) & tab & (name of current track) & tab & (artist of current track) & tab & (album of current track) & tab & (((player position) * 1000) as integer) & tab & ({length} as integer) & tab & (sound volume)\n\
             end try\n\
             end tell\n\
             end if\n\
             return \"\""
        );
        let Some(line) = run("osascript", &["-e", &script]) else {
            continue;
        };
        match parse_now_playing(&line, app) {
            Some(m) if m["state"] == "playing" => return Some(m),
            Some(m) => paused = paused.or(Some(m)),
            None => {}
        }
    }
    paused
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn now_playing() -> Option<Value> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_now_playing() {
        let m = parse_now_playing("Playing\tSo What\tMiles Davis\tKind of Blue", "spotify");
        assert_eq!(
            m,
            Some(json!({
                "title": "So What", "artist": "Miles Davis", "album": "Kind of Blue",
                "app": "spotify", "state": "playing", "can_next": true, "can_prev": true,
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
    fn reads_the_times_and_volume() {
        let m = parse_now_playing("playing\tSo What\tMiles\tKoB\t61500\t562000\t40", "Spotify")
            .unwrap();
        assert_eq!(m["position"], 61_500);
        assert_eq!(m["duration"], 562_000);
        assert_eq!(m["can_seek"], true);
        assert_eq!(
            (m["volume"].clone(), m["volume_max"].clone()),
            (json!(40), json!(100))
        );
        // A stream without a length can't seek; junk is left out.
        let live = parse_now_playing("playing\tRadio\t\t\t1000\t0\tloud", "vlc").unwrap();
        assert!(live.get("duration").is_none() && live.get("can_seek").is_none());
        assert!(live.get("volume").is_none());
        assert_eq!(live["position"], 1000);
        // A comma decimal (a locale) still reads.
        let c = parse_now_playing("paused\tx\t\t\t1,5\t2000,0\t", "Music").unwrap();
        assert_eq!(c["duration"], 2000);
    }

    #[test]
    fn mpris_columns_become_ms_and_percent() {
        assert_eq!(
            mpris_line("Playing\tSo What\tMiles\tKoB\t61500000\t562000000\t0.4"),
            "Playing\tSo What\tMiles\tKoB\t61500\t562000\t40"
        );
        assert_eq!(mpris_line("Paused\tx"), "Paused\tx\t\t\t\t\t");
        let m = parse_now_playing(&mpris_line("Playing\tx\t\t\t\t\t"), "vlc").unwrap();
        assert!(m.get("position").is_none());
    }

    #[test]
    fn playerctl_commands() {
        let a = |action: &str, v: Option<i64>| playerctl_args(action, v, Some(55));
        assert_eq!(a("play_pause", None), Some(vec!["play-pause".to_string()]));
        assert_eq!(a("previous", None), Some(vec!["previous".to_string()]));
        assert_eq!(
            a("seek", Some(90_500)),
            Some(vec!["position".to_string(), "90.500".to_string()])
        );
        assert_eq!(a("seek", None), None);
        assert_eq!(
            a("volume_set", Some(250)),
            Some(vec!["volume".to_string(), "1.00".to_string()])
        );
        assert_eq!(
            a("volume_down", None),
            Some(vec!["volume".to_string(), "0.45".to_string()])
        );
        assert_eq!(
            playerctl_args("volume_up", None, Some(95)),
            Some(vec!["volume".to_string(), "1.00".to_string()])
        );
        assert_eq!(a("session", None), None);
    }

    #[test]
    fn applescript_commands() {
        assert_eq!(
            applescript("Spotify", "next", None).as_deref(),
            Some("if application \"Spotify\" is running then\ntell application \"Spotify\" to next track\nend if")
        );
        assert!(applescript("Music", "seek", Some(61_500))
            .unwrap()
            .contains("set player position to 61.500"));
        assert!(applescript("Music", "volume_set", Some(-4))
            .unwrap()
            .contains("set sound volume to 0"));
        assert_eq!(applescript("Music", "seek", None), None);
        // Only the two players it reads, never a name from the wire.
        assert_eq!(applescript("Finder", "play", None), None);
        assert_eq!(applescript("Spotify\" to quit", "play", None), None);
        assert_eq!(applescript("Spotify", "session", None), None);
    }
}
