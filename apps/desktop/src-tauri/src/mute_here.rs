// "Mute here": while this computer's sound plays on a phone
// (`computer_audio.rs`), its own speakers can be muted, so the sound comes
// out of the phone only. A switch per phone, remembered.
//
// What gets muted, per OS, and why it doesn't silence the capture too:
// - macOS: the default output device's mute (CoreAudio,
//   kAudioDevicePropertyMute). The capture is ScreenCaptureKit, which
//   takes the apps' audio before it reaches the output device, so the
//   device's mute or volume isn't in it. (Apple's design; not checked on
//   every Mac here, so the guard below stands by.) An output with no mute
//   control (some HDMI and USB devices) can't be muted: the switch says so.
// - Linux: the default sink's mute (`pactl set-sink-mute @DEFAULT_SINK@`).
//   The capture is that sink's monitor; PipeWire keeps monitors before the
//   sink's volume and mute (its default `monitor.channel-volumes = false`).
//   Plain PulseAudio may apply the mute to the monitor too: the guard
//   catches that.
// - Windows: not offered. WASAPI loopback is taken after the endpoint's
//   mute and volume, and a per-app session mute is applied before the mix
//   the loopback takes, so either would silence what the phone hears.
//
// The guard: if the capture had sound in the moments before muting and
// has none for `SILENT_FOR` after, muting here silenced it as well: undo
// it at once and say so ("can't mute here on this computer").
//
// Restoring: the mute is undone when the last phone with the switch on
// stops (Stop, a phone gone, the link lost, the capture ended). What this
// app changed is written down before it changes it ("muted by us", and
// whether it was muted already), so after a crash the next start puts it
// back. If it was muted before, or the user unmuted it meanwhile, it's
// left alone.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager};

/// prefs.json: the switch for a phone.
const KEY_SWITCH: &str = "mute_here:";
/// prefs.json: "1" while this app has the output muted, "0" when it found
/// it muted already (then nothing to undo).
const KEY_ACTIVE: &str = "mute_here_active";
/// After muting, this long without sound means the capture went silent.
pub const SILENT_FOR: Duration = Duration::from_secs(3);
/// Sound this recent before muting counts as "it had sound".
pub const LOUD_WITHIN: Duration = Duration::from_secs(2);
/// A sample this loud (of 32767) is sound, not silence.
pub const LOUD: i16 = 64;

// --- the pure part ---

/// What was found when muting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    /// It was muted already: nothing to undo.
    pub was_muted: bool,
}

impl Saved {
    fn text(self) -> &'static str {
        if self.was_muted {
            "0"
        } else {
            "1"
        }
    }
    fn parse(s: &str) -> Option<Saved> {
        match s {
            "1" => Some(Saved { was_muted: false }),
            "0" => Some(Saved { was_muted: true }),
            _ => None,
        }
    }
}

/// Muting, with the output `muted` now: what to set (`None`: leave it),
/// and what to remember.
pub fn begin(muted: bool) -> (Option<bool>, Saved) {
    if muted {
        (None, Saved { was_muted: true })
    } else {
        (Some(true), Saved { was_muted: false })
    }
}

/// Undoing, with what was remembered and the output `muted` now: what to
/// set. Only what this app did is undone; an output muted before, or
/// unmuted by hand since, is left alone.
pub fn end(saved: Option<Saved>, muted: bool) -> Option<bool> {
    match saved {
        Some(Saved { was_muted: false }) if muted => Some(false),
        _ => None,
    }
}

/// Whether the output should be muted: some phone playing this computer's
/// sound has the switch on.
pub fn wanted(playing_to: &[String], switch_on: impl Fn(&str) -> bool) -> bool {
    playing_to.iter().any(|n| switch_on(n))
}

/// Watches the capture's level around muting.
#[derive(Default, Debug)]
pub struct Guard {
    loud_at: Option<Instant>,
    muted_at: Option<Instant>,
    loud_since: bool,
}

impl Guard {
    /// A captured frame, `loud` or not.
    pub fn heard(&mut self, now: Instant, loud: bool) {
        if loud {
            self.loud_at = Some(now);
            if self.muted_at.is_some() {
                self.loud_since = true;
            }
        }
    }

    pub fn muted(&mut self, now: Instant) {
        self.muted_at = Some(now);
        self.loud_since = false;
    }

    pub fn unmuted(&mut self) {
        self.muted_at = None;
        self.loud_since = false;
    }

    /// The capture had sound just before muting and none since, for
    /// `SILENT_FOR`: muting silenced it.
    pub fn silenced(&self, now: Instant) -> bool {
        let Some(m) = self.muted_at else {
            return false;
        };
        !self.loud_since
            && now.duration_since(m) >= SILENT_FOR
            && self
                .loud_at
                .is_some_and(|l| l <= m && m.duration_since(l) <= LOUD_WITHIN)
    }
}

/// The loudest sample of a frame (s16le).
pub fn peak(pcm: &[u8]) -> i16 {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b).saturating_abs())
        .max()
        .unwrap_or(0)
}

// --- state ---

struct State {
    guard: Guard,
    /// The guard found muting silences the capture: not offered again
    /// until the app restarts.
    broken: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    guard: Guard {
        loud_at: None,
        muted_at: None,
        loud_since: false,
    },
    broken: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn prefs(app: &AppHandle) -> Option<FileKv> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")))
}

fn switch_on(kv: &Option<FileKv>, node: &str) -> bool {
    kv.as_ref()
        .and_then(|kv| kv.get(&format!("{KEY_SWITCH}{node}")))
        .is_some_and(|v| v == "1")
}

fn saved(kv: &Option<FileKv>) -> Option<Saved> {
    kv.as_ref()
        .and_then(|kv| kv.get(KEY_ACTIVE))
        .and_then(|v| Saved::parse(&v))
}

/// Mute here for one phone, for its tile.
#[derive(Clone, Debug, Default, serde::Serialize, specta::Type)]
pub struct MuteHere {
    /// This computer can mute its output for this (not on Windows; not an
    /// output without a mute control; not after muting silenced the
    /// capture).
    pub supported: bool,
    /// The switch for this phone.
    pub on: bool,
    /// This app has the output muted now.
    pub active: bool,
    /// Why it can't, if it can't: "windows", "silences" (muting silenced
    /// the phone too, so it was undone), or the output's own words.
    pub why: Option<String>,
}

/// Mute (`true`) or unmute the output; `Err` when it can't.
fn set_muted(on: bool) -> Result<(), String> {
    backend::set_muted(on)
}

/// Bring the output in line with what's playing where: called whenever
/// the phones playing this computer's sound change, and when a switch
/// changes.
pub fn sync(app: &AppHandle) {
    let kv = prefs(app);
    let playing = crate::computer_audio::playing_to();
    let want = wanted(&playing, |n| switch_on(&kv, n)) && state().broken.is_none();
    let had = saved(&kv);
    if want && had.is_none() {
        let muted = match backend::muted() {
            Ok(m) => m,
            Err(e) => {
                // No mute on this output (or no pactl): the switch says why.
                state().broken = Some(e);
                emit(app);
                return;
            }
        };
        let (set, keep) = begin(muted);
        // Written down first: a crash after muting still finds it.
        if let Some(kv) = &kv {
            let _ = kv.set(KEY_ACTIVE, keep.text());
        }
        if let Some(on) = set {
            if let Err(e) = set_muted(on) {
                eprintln!("[mute-here] can't mute: {e}");
                if let Some(kv) = &kv {
                    let _ = kv.remove(KEY_ACTIVE);
                }
                state().broken = Some(e);
                emit(app);
                return;
            }
        }
        state().guard.muted(Instant::now());
        emit(app);
    } else if !want && had.is_some() {
        restore(app);
    }
}

/// Undo what this app muted (on stop, and at start after a crash).
pub fn restore(app: &AppHandle) {
    let kv = prefs(app);
    let had = saved(&kv);
    if had.is_none() {
        return;
    }
    if let Ok(muted) = backend::muted() {
        if let Some(on) = end(had, muted) {
            if let Err(e) = set_muted(on) {
                eprintln!("[mute-here] can't unmute: {e}");
            }
        }
    }
    if let Some(kv) = &kv {
        let _ = kv.remove(KEY_ACTIVE);
    }
    state().guard.unmuted();
    emit(app);
}

/// A frame captured (s16le): watch for muting silencing the capture.
pub fn heard(app: &AppHandle, pcm: &[u8]) {
    let now = Instant::now();
    let silenced = {
        let mut s = state();
        s.guard.heard(now, peak(pcm) >= LOUD);
        s.guard.silenced(now)
    };
    if silenced {
        eprintln!("[mute-here] the capture went silent with the output muted: undoing");
        state().broken = Some(SILENCES.into());
        restore(app);
    }
}

/// Why it can't (the window says it in words): muting silenced the
/// capture too, or Windows (its capture is taken after the mute).
const SILENCES: &str = "silences";
const NOT_WINDOWS: &str = "windows";

fn emit(app: &AppHandle) {
    let _ = app.emit("mute-here", ());
}

/// At start: undo a mute left by a crash.
pub fn init(app: &AppHandle) {
    restore(app);
}

#[tauri::command]
#[specta::specta]
pub fn mute_here(node_id: String, app: AppHandle) -> MuteHere {
    let kv = prefs(&app);
    let why = if cfg!(windows) {
        Some(NOT_WINDOWS.to_string())
    } else {
        state().broken.clone()
    };
    MuteHere {
        supported: why.is_none(),
        on: switch_on(&kv, &node_id),
        active: saved(&kv).is_some_and(|s| !s.was_muted),
        why,
    }
}

#[tauri::command]
#[specta::specta]
pub fn set_mute_here(node_id: String, on: bool, app: AppHandle) -> Result<(), String> {
    let kv = prefs(&app).ok_or("no settings folder")?;
    kv.set(
        &format!("{KEY_SWITCH}{node_id}"),
        if on { "1" } else { "0" },
    )
    .map_err(|e| e.to_string())?;
    if on && cfg!(windows) {
        return Err(NOT_WINDOWS.into());
    }
    sync(&app);
    emit(&app);
    Ok(())
}

// --- per OS ---

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod backend {
    use std::ffi::c_void;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    const fn four(c: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*c)
    }
    const SYSTEM_OBJECT: u32 = 1;
    const DEFAULT_OUTPUT: u32 = four(b"dOut");
    const MUTE: u32 = four(b"mute");
    const SCOPE_GLOBAL: u32 = four(b"glob");
    const SCOPE_OUTPUT: u32 = four(b"outp");
    const MAIN: u32 = 0;

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyData(
            id: u32,
            addr: *const Address,
            qual_size: u32,
            qual: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            id: u32,
            addr: *const Address,
            qual_size: u32,
            qual: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> i32;
        fn AudioObjectHasProperty(id: u32, addr: *const Address) -> u8;
    }

    fn get_u32(id: u32, addr: &Address) -> Option<u32> {
        let mut v: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `v` is a u32 and `size` says so; CoreAudio writes at most
        // `size` bytes.
        let st = unsafe {
            AudioObjectGetPropertyData(
                id,
                addr,
                0,
                std::ptr::null(),
                &mut size,
                (&mut v as *mut u32).cast(),
            )
        };
        (st == 0).then_some(v)
    }

    fn output() -> Result<u32, String> {
        let addr = Address {
            selector: DEFAULT_OUTPUT,
            scope: SCOPE_GLOBAL,
            element: MAIN,
        };
        get_u32(SYSTEM_OBJECT, &addr)
            .filter(|d| *d != 0)
            .ok_or_else(|| "no sound output".to_string())
    }

    const MUTE_ADDR: Address = Address {
        selector: MUTE,
        scope: SCOPE_OUTPUT,
        element: MAIN,
    };

    pub fn muted() -> Result<bool, String> {
        let dev = output()?;
        // SAFETY: a plain query with a valid address.
        if unsafe { AudioObjectHasProperty(dev, &MUTE_ADDR) } == 0 {
            return Err("This computer\u{2019}s sound output can\u{2019}t be muted.".into());
        }
        get_u32(dev, &MUTE_ADDR)
            .map(|v| v != 0)
            .ok_or_else(|| "can't read the mute".into())
    }

    pub fn set_muted(on: bool) -> Result<(), String> {
        let dev = output()?;
        let v: u32 = u32::from(on);
        // SAFETY: `v` is a u32 and the size says so.
        let st = unsafe {
            AudioObjectSetPropertyData(
                dev,
                &MUTE_ADDR,
                0,
                std::ptr::null(),
                std::mem::size_of::<u32>() as u32,
                (&v as *const u32).cast(),
            )
        };
        if st == 0 {
            Ok(())
        } else {
            Err(format!(
                "This computer\u{2019}s sound output can\u{2019}t be muted ({st})."
            ))
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use std::process::Command;

    fn pactl(args: &[&str]) -> Result<String, String> {
        let out = Command::new("pactl").args(args).output().map_err(|_| {
            "pactl isn\u{2019}t installed (PulseAudio or PipeWire tools).".to_string()
        })?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn muted() -> Result<bool, String> {
        // "Mute: yes" / "Mute: no" (the word is not translated).
        let out = pactl(&["get-sink-mute", "@DEFAULT_SINK@"])?;
        super::parse_pactl_mute(&out).ok_or_else(|| "can't read the mute".to_string())
    }

    pub fn set_muted(on: bool) -> Result<(), String> {
        pactl(&[
            "set-sink-mute",
            "@DEFAULT_SINK@",
            if on { "1" } else { "0" },
        ])
        .map(|_| ())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod backend {
    pub fn muted() -> Result<bool, String> {
        Err(super::NOT_WINDOWS.into())
    }
    pub fn set_muted(_on: bool) -> Result<(), String> {
        Err(super::NOT_WINDOWS.into())
    }
}

/// `pactl get-sink-mute`'s answer: muted or not.
#[allow(dead_code)]
pub fn parse_pactl_mute(out: &str) -> Option<bool> {
    let v = out.trim().strip_prefix("Mute:")?.trim();
    match v {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_we_muted_is_undone() {
        // Not muted: mute, and undo it after.
        let (set, saved) = begin(false);
        assert_eq!(set, Some(true));
        assert_eq!(end(Some(saved), true), Some(false));
        // Unmuted by hand meanwhile: leave it.
        assert_eq!(end(Some(saved), false), None);
        // Muted already: nothing to set, nothing to undo.
        let (set, saved) = begin(true);
        assert_eq!(set, None);
        assert_eq!(end(Some(saved), true), None);
        // Nothing remembered (never muted, or undone already).
        assert_eq!(end(None, true), None);
    }

    #[test]
    fn what_was_muted_survives_a_restart() {
        for s in [Saved { was_muted: false }, Saved { was_muted: true }] {
            assert_eq!(Saved::parse(s.text()), Some(s));
        }
        assert_eq!(Saved::parse(""), None);
        // After a crash: "1" in prefs means we muted it; undo it.
        assert_eq!(end(Saved::parse("1"), true), Some(false));
        assert_eq!(end(Saved::parse("0"), true), None);
    }

    #[test]
    fn muted_while_a_phone_with_the_switch_plays() {
        let on = |n: &str| n == "phone-a";
        assert!(wanted(&["phone-b".into(), "phone-a".into()], on));
        assert!(!wanted(&["phone-b".into()], on));
        assert!(!wanted(&[], on), "nothing playing: restore");
    }

    #[test]
    fn the_guard_sees_muting_silence_the_capture() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut g = Guard::default();
        g.heard(at(0), true);
        g.heard(at(500), true);
        g.muted(at(600));
        g.heard(at(1000), false);
        assert!(!g.silenced(at(2000)), "not long enough yet");
        assert!(g.silenced(at(3700)));
        // Sound after muting: the capture is fine.
        let mut g = Guard::default();
        g.heard(at(0), true);
        g.muted(at(100));
        g.heard(at(400), true);
        assert!(!g.silenced(at(5000)));
        // Quiet before muting (nothing playing): no verdict.
        let mut g = Guard::default();
        g.heard(at(0), true);
        g.muted(at(5000));
        assert!(!g.silenced(at(9000)));
    }

    #[test]
    fn levels_and_pactl() {
        assert_eq!(peak(&[0, 0, 0x10, 0x80, 0xff, 0x7f]), 32767);
        assert_eq!(peak(&[]), 0);
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no"), Some(false));
        assert_eq!(parse_pactl_mute("nonsense"), None);
    }
}
