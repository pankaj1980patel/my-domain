// Phone calls on this computer over Bluetooth, like a car kit: the computer
// takes the Hands-Free (HFP "HF") role for a phone you've already paired in
// the system's Bluetooth settings, so an incoming call shows here (number,
// and the name when the phone sends one), you answer, decline, hang up,
// mute this computer's microphone, dial or redial, send keypad tones, and
// move the call audio between the phone and this computer.
//
// This computer stays in charge:
//  - off until "Calls on this computer" is turned on in Settings, with a
//    phone picked there (`<app config dir>/prefs.json`, keys `calls`,
//    `calls_device`, `calls_answer_audio`);
//  - it never pairs: only phones already paired are offered (those whose
//    SDP record has the Hands-Free Audio Gateway service, 0x111F);
//  - Contacts aren't read: a caller is the name the phone sends, or the
//    number.
//
// Everything above the Bluetooth layer is shared: the call state (`Machine`,
// pure), the commands, events, tray items and the audio bridge (`audio`).
// The Bluetooth layer is a backend module per OS, picked as `backend`:
//  - macOS: `mac`, IOBluetooth's `IOBluetoothHandsFreeDevice`, everything
//    on the main thread, where its delegate calls arrive;
//  - Linux: `linux`, the `md-hfp` crate (a BlueZ HFP profile over D-Bus,
//    the AT dialogue, SCO sockets) on threads of its own;
//  - Windows: `windows`, the same `md-hfp` API over a Winsock RFCOMM socket
//    to the phone's Audio Gateway service: call control only, the audio
//    stays on the phone (Windows opens SCO for kernel-mode drivers alone;
//    its own hands-free role, Phone Link's, needs package identity and a
//    restricted capability; see md-hfp's `windows/mod.rs`);
//  - anything else: `none`, every command says "not available"
//    (`available()` is false).
//
// THE BACKEND SEAM. A backend module provides exactly these (all errors are
// sentences for a person):
//   exec(app, f) -> Result<T, String>   run a backend call where it must
//                                       run, wait (bounded) for its result
//   run_later(app, f)                   the same, fire and forget
//   gateways() -> Result<Vec<CallDevice>, String>   paired phones (0x111F)
//   connect(addr) / disconnect()        the hands-free link to a phone; the
//                                       outcome arrives as events, not here
//   answer(on_computer) / end_call() / dial(number) / redial() / dtmf(c)
//   set_muted(on)                       sets MIC_MUTED and feeds Ev::Muted
//   audio_to_phone() / audio_to_mac()   move the audio link (ask the phone)
//   connect_sco()                       open the audio link directly
//   phone_end() -> Option<audio::PhoneEnd>  what to bridge once it's open
//   device_name() / known_name() -> String  the connected phone (any thread
//                                       for known_name; "" when none)
//   cached_name(Option<&str>) -> Option<String>     a paired phone's name
//   problem() -> Option<String>         a standing issue for Settings
// and reports what happens by calling `feed(Ev::...)` (any thread):
// Connected(bool) when the service-level link comes or goes, the phone's
// indicators (Call, Setup, Held, Signal, Battery), Incoming / CallInfo for
// the caller, Sco(bool) when the audio link opens or closes. Another OS's
// backend adds its `mod` with the same functions and one more line in the
// `use ... as backend` list below.
//
// Call audio: the phone sends it over a SCO link. On macOS, when that link
// opens, the Mac shows it as a CoreAudio device named after the phone
// (input: the caller's voice; output: what the caller hears). `audio` below bridges it
// with a small pass-through (cpal, i.e. CoreAudio AudioUnits): the phone's
// device -> the chosen output, the chosen input -> the phone's device
// (zeroed while muted), resampled between the rates (SCO is 8/16 kHz).
// "Chosen" is a device you pick (built-in speaker/mic, AirPods, a USB
// headset, a display...) or the system default. The system default
// devices are never switched, so nothing needs restoring after a call or a
// crash: the bridge's streams simply end with the process. If the phone's
// device never shows up (within a few seconds of SCO opening), nothing is
// bridged and macOS's own routing is left alone.
//
// The device list stays live (a CoreAudio listener on
// kAudioHardwarePropertyDevices -> `calls-audio-devices` to the UI); when the
// device in use disappears mid-call the bridge restarts on the built-in
// one, else the default (`pick_device`).
//
// Audio transfer: closing the SCO link (keeping the service-level link, so
// the controls here still work) moves the audio to the phone; opening it
// again brings it back. The phone can do the same from its side; the
// delegate's SCO opened/closed calls keep `audio` in the state current.
//
// On Linux the SCO link is a socket this app holds (Linux lets apps carry
// call audio): `md-hfp` hands its samples over (8 kHz CVSD) and `audio`
// bridges them to the chosen speaker and microphone with the same
// resampling and device choice. Linux has no device-change listener here
// (ALSA device probing during a call would fight the bridge's own devices).
//
// On Windows no app can hold the SCO link, so there is no bridge: the call
// audio stays on the phone (`computer_audio_refused()` is always true there,
// so every call targets the phone and Settings says why), and this computer
// is the call's remote: ringing, caller, answer, decline, hang up, dial,
// redial, keypad. Mute is the phone's own (no microphone here is in the call).

#![cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use mdcore::kv::FileKv;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use mac as backend;
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
use none as backend;
// `self::`: the module below, never a `windows` crate.
#[cfg(target_os = "windows")]
use self::windows as backend;

const KEY_ON: &str = "calls";
const KEY_DEVICE: &str = "calls_device";
const KEY_ANSWER_AUDIO: &str = "calls_answer_audio";
/// "refused" once macOS turned down carrying call audio on this Mac.
const KEY_MAC_AUDIO: &str = "calls_mac_audio";
/// The longest number `call_dial` accepts.
const MAX_NUMBER: usize = 32;
const NOT_AVAILABLE: &str =
    "Calls on this computer aren\u{2019}t available here (macOS, Linux and Windows only).";

/// The app, for delegate callbacks (they carry no context).
static APP: OnceLock<AppHandle> = OnceLock::new();
/// The call as the phone last told it.
static MACHINE: Mutex<Machine> = Mutex::new(Machine::new());
/// This computer's microphone is muted for the call (read by the audio bridge).
static MIC_MUTED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// Where a call's audio goes: the phone, or this computer with a chosen output
/// and input (`None`: the system's default one).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AudioTarget {
    pub phone: bool,
    pub output: Option<String>,
    pub input: Option<String>,
}

/// Where this call's audio should go on this computer (reset from Settings at
/// each new call, changed from the in-call picker).
static CALL_TARGET: Mutex<Option<AudioTarget>> = Mutex::new(None);

fn call_target() -> AudioTarget {
    let t = CALL_TARGET
        .lock()
        .ok()
        .and_then(|t| t.clone())
        .unwrap_or_default();
    if computer_audio_refused() {
        // Nowhere else to put it: this computer can't carry it.
        return AudioTarget { phone: true, ..t };
    }
    t
}

/// This computer can't carry call audio: never on Windows (no app may hold
/// the SCO link there), or macOS turned it down here before.
fn computer_audio_refused() -> bool {
    if cfg!(target_os = "windows") {
        return true;
    }
    APP.get()
        .and_then(|app| {
            app.try_state::<CallsPrefs>()
                .map(|p| p.get().mac_audio_refused)
        })
        .unwrap_or(false)
}

/// macOS refused the call's audio link (`kIOReturnUnsupported`): remember
/// it, keep the audio on the phone, and say so once.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn on_mac_audio_refused() {
    let Some(app) = APP.get() else {
        return;
    };
    let Some(prefs) = app.try_state::<CallsPrefs>() else {
        return;
    };
    if prefs.get().mac_audio_refused {
        return;
    }
    eprintln!("[calls] macOS won\u{2019}t carry call audio here: keeping it on the phone");
    prefs.update(|p| p.mac_audio_refused = true);
    crate::notify::post_always(
        app,
        "Call audio stays on your phone",
        "macOS doesn\u{2019}t let apps carry phone-call audio on this Mac. Answer, hang up, mute and the keypad still work from here; talk on the phone or a headset paired with it.",
    );
    if let Ok(m) = MACHINE.lock().map(|m| m.clone()) {
        emit_state(app, &m);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PrefsInner {
    on: bool,
    /// The phone's Bluetooth address (as IOBluetooth writes it).
    device: Option<String>,
    /// Where the audio goes when a call is answered here.
    answer: AudioTarget,
    /// macOS refused to carry call audio on this Mac (seen on a call): the
    /// audio stays on the phone and the Mac is only the call's remote.
    mac_audio_refused: bool,
}

/// "Calls on this computer": the switch, the phone, where answered audio goes.
pub struct CallsPrefs {
    kv: Option<FileKv>,
    inner: Mutex<PrefsInner>,
}

impl CallsPrefs {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("prefs.json")));
        let get = |k: &str| kv.as_ref().and_then(|kv| kv.get(k));
        let inner = PrefsInner {
            on: get(KEY_ON).as_deref() == Some("on"),
            device: get(KEY_DEVICE).filter(|d| !d.is_empty()),
            answer: get(KEY_ANSWER_AUDIO)
                .and_then(|a| serde_json::from_str(&a).ok())
                .unwrap_or_default(),
            mac_audio_refused: get(KEY_MAC_AUDIO).as_deref() == Some("refused"),
        };
        CallsPrefs {
            kv,
            inner: Mutex::new(inner),
        }
    }

    fn get(&self) -> PrefsInner {
        self.inner.lock().map(|p| p.clone()).unwrap_or_default()
    }

    fn update(&self, f: impl FnOnce(&mut PrefsInner)) {
        let Ok(mut p) = self.inner.lock() else {
            return;
        };
        f(&mut p);
        if let Some(kv) = &self.kv {
            let answer = serde_json::to_string(&p.answer).unwrap_or_default();
            let saved = kv
                .set(KEY_ON, if p.on { "on" } else { "off" })
                .and_then(|()| kv.set(KEY_DEVICE, p.device.as_deref().unwrap_or("")))
                .and_then(|()| kv.set(KEY_ANSWER_AUDIO, &answer))
                .and_then(|()| {
                    kv.set(
                        KEY_MAC_AUDIO,
                        if p.mac_audio_refused { "refused" } else { "" },
                    )
                });
            if let Err(e) = saved {
                eprintln!("[calls] couldn't save the setting: {e}");
            }
        }
    }
}

/// Calls can work on this computer at all.
pub fn available() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    ))
}

// ---------------------------------------------------------------------------
// The call state (pure)
// ---------------------------------------------------------------------------

/// Where a call stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    /// Ringing here.
    Incoming,
    /// Dialling, or the other side is ringing.
    Outgoing,
    Active,
}

/// Where the call's audio is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioRoute {
    /// No call.
    None,
    /// This computer's speaker and microphone (the SCO link is open).
    Mac,
    /// The phone's own earpiece or speaker.
    Phone,
}

/// What the phone (or this app) said happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Ev {
    /// The service-level link came up (true) or went (false).
    Connected(bool),
    /// The "call" indicator: a call is in progress.
    Call(bool),
    /// The "callsetup" indicator: 0 none, 1 incoming, 2 dialling, 3 alerting.
    Setup(u8),
    /// The "callheld" indicator: 0 none, 1 held + active, 2 held only.
    Held(u8),
    Signal(u8),
    Battery(u8),
    /// The caller's number (CLIP).
    Incoming(String),
    /// A line of the current call list.
    CallInfo {
        number: Option<String>,
        name: Option<String>,
    },
    /// The SCO (audio) link opened or closed.
    Sco(bool),
    /// This computer's microphone muted or not.
    Muted(bool),
}

/// What to tell the UI.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Incoming {
        number: String,
        name: Option<String>,
    },
    Active,
    Ended,
    /// held / muted / signal / battery / audio / phase changed.
    State,
}

/// The call, from the phone's indicators.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    connected: bool,
    call: bool,
    setup: u8,
    held: u8,
    signal: Option<u8>,
    battery: Option<u8>,
    sco: bool,
    muted: bool,
    number: Option<String>,
    name: Option<String>,
    /// When the call became active (unix ms).
    since_ms: Option<f64>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub const fn new() -> Self {
        Machine {
            connected: false,
            call: false,
            setup: 0,
            held: 0,
            signal: None,
            battery: None,
            sco: false,
            muted: false,
            number: None,
            name: None,
            since_ms: None,
        }
    }

    pub fn phase(&self) -> Phase {
        if !self.connected {
            Phase::Idle
        } else if self.call {
            Phase::Active
        } else {
            match self.setup {
                1 => Phase::Incoming,
                2 | 3 => Phase::Outgoing,
                _ => Phase::Idle,
            }
        }
    }

    pub fn audio(&self) -> AudioRoute {
        match (self.phase(), self.sco) {
            (Phase::Idle, _) => AudioRoute::None,
            (_, true) => AudioRoute::Mac,
            (_, false) => AudioRoute::Phone,
        }
    }

    fn shown(&self) -> (Phase, AudioRoute, u8, bool, Option<u8>, Option<u8>) {
        (
            self.phase(),
            self.audio(),
            self.held,
            self.muted,
            self.signal,
            self.battery,
        )
    }

    /// Take in one event (at `now_ms`); what to tell the UI.
    pub fn apply(&mut self, ev: Ev, now_ms: f64) -> Vec<Out> {
        let before = self.shown();
        let caller = (self.number.clone(), self.name.clone());
        match ev {
            Ev::Connected(on) => {
                self.connected = on;
                if !on {
                    self.call = false;
                    self.setup = 0;
                    self.held = 0;
                    self.sco = false;
                }
            }
            Ev::Call(on) => self.call = on,
            Ev::Setup(s) => self.setup = s.min(3),
            Ev::Held(h) => self.held = h.min(2),
            Ev::Signal(s) => self.signal = Some(s.min(5)),
            Ev::Battery(b) => self.battery = Some(b.min(5)),
            Ev::Incoming(number) => {
                // The number can come before the setup indicator does.
                if !self.call && self.setup == 0 {
                    self.setup = 1;
                }
                if !number.is_empty() {
                    self.number = Some(number);
                }
            }
            Ev::CallInfo { number, name } => {
                if let Some(n) = number.filter(|n| !n.is_empty()) {
                    self.number = Some(n);
                }
                if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                    self.name = Some(n.trim().to_string());
                }
            }
            Ev::Sco(open) => self.sco = open,
            Ev::Muted(m) => self.muted = m,
        }
        let mut out = Vec::new();
        let (was, now) = (before.0, self.phase());
        if was != now {
            match now {
                Phase::Incoming => out.push(self.incoming()),
                Phase::Active => {
                    self.since_ms = Some(now_ms);
                    out.push(Out::Active);
                }
                Phase::Idle => {
                    self.number = None;
                    self.name = None;
                    self.since_ms = None;
                    self.muted = false;
                    out.push(Out::Ended);
                }
                Phase::Outgoing => {}
            }
        } else if now == Phase::Incoming && caller != (self.number.clone(), self.name.clone()) {
            out.push(self.incoming());
        }
        if before != self.shown() {
            out.push(Out::State);
        }
        out
    }

    fn incoming(&self) -> Out {
        Out::Incoming {
            number: self.number.clone().unwrap_or_default(),
            name: self.name.clone(),
        }
    }

    /// Who's on the line, for a person: the name, else the number.
    pub fn caller(&self) -> Option<String> {
        self.name.clone().or_else(|| self.number.clone())
    }
}

/// `IOBluetoothHandsFreeCallStatus` in the current call list: 0 active,
/// 1 held, 2 dialling, 3 alerting, 4 incoming, 5 waiting.
pub fn call_status_phase(status: i32) -> Phase {
    match status {
        0 | 1 => Phase::Active,
        2 | 3 => Phase::Outgoing,
        4 | 5 => Phase::Incoming,
        _ => Phase::Idle,
    }
}

/// A number to dial: digits, `*`, `#` and a leading `+`; spaces, dashes,
/// dots, slashes and brackets are dropped; anything else is refused.
pub fn sanitize_number(input: &str) -> Result<String, String> {
    let mut out = String::new();
    for c in input.trim().chars() {
        match c {
            '0'..='9' | '*' | '#' => out.push(c),
            '+' if out.is_empty() => out.push(c),
            '+' => return Err("A + can only start the number.".into()),
            ' ' | '-' | '.' | '/' | '(' | ')' | '\u{a0}' => {}
            _ => return Err(format!("\u{201c}{c}\u{201d} can\u{2019}t be dialled.")),
        }
    }
    if out.is_empty() || out == "+" {
        return Err("Type a number to call.".into());
    }
    if out.len() > MAX_NUMBER {
        return Err("That number is too long.".into());
    }
    Ok(out)
}

/// One keypad tone: 0-9, `*`, `#`, A-D.
pub fn dtmf_digit(input: &str) -> Option<char> {
    let mut chars = input.trim().chars();
    let c = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() {
        return None;
    }
    matches!(c, '0'..='9' | '*' | '#' | 'A'..='D').then_some(c)
}

/// The tray's call items for this state: (id after `call:`, label).
pub fn tray_entries(m: &Machine) -> Vec<(&'static str, String)> {
    let who = m.caller().unwrap_or_else(|| "unknown number".into());
    match m.phase() {
        Phase::Idle => Vec::new(),
        Phase::Incoming => vec![
            ("answer", format!("Answer call from {who}")),
            ("decline", "Decline call".into()),
        ],
        Phase::Outgoing | Phase::Active => {
            let mut v = vec![("hangup", format!("Hang up ({who})"))];
            match m.audio() {
                AudioRoute::Mac => v.push(("tophone", "Use phone for audio".into())),
                _ => v.push(("tomac", "Use this computer for audio".into())),
            }
            v
        }
    }
}

// ---------------------------------------------------------------------------
// Feeding events and telling the UI
// ---------------------------------------------------------------------------

/// The `call-state` event.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CallStateEvent {
    pub phase: Phase,
    pub held: bool,
    pub muted: bool,
    pub signal: Option<u8>,
    pub battery: Option<u8>,
    pub audio: AudioRoute,
    /// The devices in use on this computer while the audio's here.
    pub output: Option<String>,
    pub input: Option<String>,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// An event from the phone (main thread on macOS): update the state, then
/// tell the UI, the tray and, for a new call, the OS.
fn feed(ev: Ev) {
    let Some(app) = APP.get() else {
        return;
    };
    let sco = match &ev {
        Ev::Sco(open) => Some(*open),
        _ => None,
    };
    let (was, outs, m) = match MACHINE.lock() {
        Ok(mut m) => {
            let was = m.phase();
            let outs = m.apply(ev, now_ms());
            (was, outs, m.clone())
        }
        Err(_) => return,
    };
    if was == Phase::Idle && m.phase() != Phase::Idle {
        // A new call: its audio goes where Settings says, until changed.
        let answer = app
            .try_state::<CallsPrefs>()
            .map(|p| p.get().answer)
            .unwrap_or_default();
        if let Ok(mut t) = CALL_TARGET.lock() {
            *t = Some(answer);
        }
    }
    eprintln!(
        "[calls] {:?} -> {:?} (audio link {})",
        was,
        m.phase(),
        if m.sco { "open" } else { "closed" }
    );
    match sco {
        Some(true) => {
            if let Some(phone) = backend::phone_end() {
                audio::start(phone, call_target());
            }
        }
        Some(false) => audio::stop(),
        None => {}
    }
    // A call dialling or under way with its audio meant for this computer,
    // and no audio link yet: ask for it. The phone doesn't open one by
    // itself for a call placed from here or answered on the phone.
    if available()
        && matches!(m.phase(), Phase::Outgoing | Phase::Active)
        && was != m.phase()
        && !m.sco
        && !call_target().phone
    {
        request_mac_audio(app);
    }
    let mut tray = false;
    for o in outs {
        match o {
            Out::Incoming { number, name } => {
                let _ = app.emit(
                    "call-incoming",
                    serde_json::json!({ "number": number, "name": name }),
                );
                let who = name.unwrap_or(if number.is_empty() {
                    "Unknown number".into()
                } else {
                    number
                });
                crate::notify::post_always(app, "Incoming call", &who);
                crate::tray::show_window(app);
                tray = true;
            }
            Out::Active => {
                let _ = app.emit(
                    "call-active",
                    serde_json::json!({ "since_ms": m.since_ms, "number": m.number, "name": m.name }),
                );
                tray = true;
            }
            Out::Ended => {
                MIC_MUTED.store(false, Ordering::Relaxed);
                audio::stop();
                let _ = app.emit("call-ended", serde_json::json!({}));
                tray = true;
            }
            Out::State => {
                emit_state(app, &m);
                tray = true;
            }
        }
    }
    if tray {
        crate::tray::refresh(app);
    }
}

/// The devices the call audio is using on this computer: (output, input).
fn in_use() -> (Option<String>, Option<String>) {
    audio::in_use()
}

fn emit_state(app: &AppHandle, m: &Machine) {
    let (output, input) = in_use();
    let _ = app.emit(
        "call-state",
        CallStateEvent {
            phase: m.phase(),
            held: m.held > 0,
            muted: m.muted,
            signal: m.signal,
            battery: m.battery,
            audio: m.audio(),
            output,
            input,
        },
    );
}

/// The audio bridge changed devices (started, moved, fell back).
fn audio_changed() {
    let Some(app) = APP.get() else {
        return;
    };
    if let Ok(m) = MACHINE.lock().map(|m| m.clone()) {
        emit_state(app, &m);
    }
}

/// The tray's call items right now (empty with no call). No "use this
/// computer for audio" where it can't carry it (Windows, or macOS refused).
pub fn tray_items() -> Vec<(&'static str, String)> {
    let mut v = MACHINE.lock().map(|m| tray_entries(&m)).unwrap_or_default();
    if computer_audio_refused() {
        v.retain(|(id, _)| *id != "tomac");
    }
    v
}

/// A tray call item was chosen (main thread).
pub fn tray_action(app: &AppHandle, what: &str) {
    if !available() {
        return;
    }
    let r = match what {
        "answer" => backend::answer(!call_target().phone),
        "decline" | "hangup" => backend::end_call(),
        "tophone" => set_call_audio(AudioTarget {
            phone: true,
            ..call_target()
        }),
        "tomac" => set_call_audio(AudioTarget {
            phone: false,
            ..call_target()
        }),
        _ => Ok(()),
    };
    if let Err(e) = r {
        crate::notify::post_always(app, "Couldn\u{2019}t do that on the call", &e);
    }
}

/// Ask the phone to send the call's audio to this computer (where backend
/// calls run); if no audio link has opened two seconds later, open one
/// directly.
fn request_mac_audio(app: &AppHandle) {
    if let Err(e) = backend::audio_to_mac() {
        eprintln!("[calls] couldn't ask for the call audio: {e}");
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let still = MACHINE
            .lock()
            .map(|m| !m.sco && m.phase() != Phase::Idle)
            .unwrap_or(false);
        if still && !call_target().phone {
            eprintln!("[calls] no audio link yet: opening one");
            backend::run_later(&app, || {
                if let Err(e) = backend::connect_sco() {
                    eprintln!("[calls] couldn't open the audio link: {e}");
                }
            });
        }
    });
}

/// Send this call's audio somewhere (where backend calls run): to the
/// phone (closes the audio link), or to this computer on the given devices
/// (opens the link, or moves the running bridge).
fn set_call_audio(target: AudioTarget) -> Result<(), String> {
    if let Ok(mut t) = CALL_TARGET.lock() {
        *t = Some(target.clone());
    }
    let sco = MACHINE.lock().map(|m| m.sco).unwrap_or(false);
    if target.phone {
        backend::audio_to_phone()
    } else if sco {
        if let Some(phone) = backend::phone_end() {
            audio::start(phone, target);
        }
        Ok(())
    } else {
        backend::audio_to_mac()
    }
}

/// At startup: remember the app, watch the audio devices, and reconnect to
/// the chosen phone if calls are on.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    audio::watch_devices();
    let Some(prefs) = app.try_state::<CallsPrefs>() else {
        return;
    };
    let p = prefs.get();
    if !(available() && p.on) {
        return;
    }
    if let Some(addr) = p.device {
        backend::run_later(app, move || {
            if let Err(e) = backend::connect(&addr) {
                eprintln!("[calls] couldn't reconnect to the phone: {e}");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// A paired phone that can take this computer as its hands-free.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CallDevice {
    pub address: String,
    pub name: String,
    /// Connected over Bluetooth at all (by anything) right now.
    pub connected: bool,
}

/// Everything the Settings tile and the in-call bar show.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CallsStatus {
    pub available: bool,
    pub enabled: bool,
    pub device: Option<String>,
    pub device_name: Option<String>,
    /// The hands-free link to the phone is up.
    pub connected: bool,
    pub phase: Phase,
    pub number: Option<String>,
    pub name: Option<String>,
    /// When the call became active (unix ms).
    pub since_ms: Option<f64>,
    pub held: bool,
    pub muted: bool,
    pub signal: Option<u8>,
    pub battery: Option<u8>,
    pub audio: AudioRoute,
    /// The devices in use on this computer while the audio's here.
    pub output: Option<String>,
    pub input: Option<String>,
    /// Where this call's audio was asked to go.
    pub target: AudioTarget,
    /// Where the audio goes when a call is answered here (Settings).
    pub answer: AudioTarget,
    /// This computer can carry call audio (false on Windows, and once macOS
    /// refused it).
    pub mac_audio: bool,
    /// Something keeps calls from working here, and what to do about it
    /// (e.g. another program has the hands-free role on Linux).
    pub problem: Option<String>,
    /// "macos", "linux", ... (the Settings wording differs a little).
    pub os: String,
}

/// The audio devices on this computer (by name), for the pickers.
#[derive(Clone, Debug, Default, Serialize, specta::Type)]
pub struct AudioDevices {
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

fn status_of(app: &AppHandle) -> CallsStatus {
    let p = app
        .try_state::<CallsPrefs>()
        .map(|p| p.get())
        .unwrap_or_default();
    let m = MACHINE.lock().map(|m| m.clone()).unwrap_or_default();
    let device_name = backend::cached_name(p.device.as_deref());
    let (output, input) = in_use();
    CallsStatus {
        output,
        input,
        target: call_target(),
        available: available(),
        enabled: p.on,
        device: p.device,
        device_name,
        connected: m.connected,
        phase: m.phase(),
        number: m.number.clone(),
        name: m.name.clone(),
        since_ms: m.since_ms,
        held: m.held > 0,
        muted: m.muted,
        signal: m.signal,
        battery: m.battery,
        audio: m.audio(),
        answer: p.answer,
        mac_audio: !(cfg!(target_os = "windows") || p.mac_audio_refused),
        problem: if p.on { backend::problem() } else { None },
        os: std::env::consts::OS.to_string(),
    }
}

/// A backend call, run where the backend needs it (macOS: the main thread)
/// and waited for; "not available" where there's no backend.
macro_rules! hf_call {
    ($app:expr, $body:expr) => {{
        backend::exec(&$app, move || $body)
    }};
}

/// The calls switch, phone, link and call right now.
#[tauri::command]
#[specta::specta]
pub fn calls_status(app: AppHandle) -> CallsStatus {
    status_of(&app)
}

/// Paired phones that offer the Hands-Free Audio Gateway service.
#[tauri::command]
#[specta::specta]
pub async fn calls_devices(app: AppHandle) -> Result<Vec<CallDevice>, String> {
    hf_call!(app, backend::gateways())
}

/// Turn "Calls on this computer" on or off (off disconnects).
#[tauri::command]
#[specta::specta]
pub async fn calls_set_enabled(on: bool, app: AppHandle) -> Result<CallsStatus, String> {
    if on && !available() {
        return Err(NOT_AVAILABLE.into());
    }
    if let Some(p) = app.try_state::<CallsPrefs>() {
        p.update(|p| p.on = on);
    }
    if !on && available() {
        hf_call!(app, backend::disconnect())?;
    }
    Ok(status_of(&app))
}

/// Pick the phone (its Bluetooth address), or none.
#[tauri::command]
#[specta::specta]
pub async fn calls_set_device(
    address: Option<String>,
    app: AppHandle,
) -> Result<CallsStatus, String> {
    let address = address.filter(|a| !a.trim().is_empty());
    let old = app.try_state::<CallsPrefs>().and_then(|p| p.get().device);
    if old != address && old.is_some() && available() {
        hf_call!(app, backend::disconnect())?;
    }
    if let Some(p) = app.try_state::<CallsPrefs>() {
        p.update(|p| p.device = address);
    }
    Ok(status_of(&app))
}

/// Where the audio goes when a call is answered here: the phone, or this
/// computer's default or chosen devices.
#[tauri::command]
#[specta::specta]
pub fn calls_set_answer_audio(target: AudioTarget, app: AppHandle) -> CallsStatus {
    if let Some(p) = app.try_state::<CallsPrefs>() {
        p.update(|p| p.answer = target);
    }
    status_of(&app)
}

/// This computer's audio outputs and inputs (the phone's own device left out).
#[tauri::command]
#[specta::specta]
pub async fn calls_audio_devices() -> AudioDevices {
    audio::list()
}

/// Move this call's audio: to the phone, or to this computer on the given
/// output and input (`None`: the system default).
#[tauri::command]
#[specta::specta]
pub async fn call_set_audio(target: AudioTarget, app: AppHandle) -> Result<(), String> {
    hf_call!(app, set_call_audio(target))
}

/// Connect to the chosen phone as its hands-free (the delegate says when).
#[tauri::command]
#[specta::specta]
pub async fn calls_connect(app: AppHandle) -> Result<(), String> {
    let p = app
        .try_state::<CallsPrefs>()
        .map(|p| p.get())
        .unwrap_or_default();
    if !available() {
        return Err(NOT_AVAILABLE.into());
    }
    if !p.on {
        return Err("Turn on \u{201c}Calls on this computer\u{201d} first.".into());
    }
    let Some(addr) = p.device else {
        return Err("Pick your phone first.".into());
    };
    hf_call!(app, backend::connect(&addr))
}

/// Drop the hands-free link to the phone.
#[tauri::command]
#[specta::specta]
pub async fn calls_disconnect(app: AppHandle) -> Result<(), String> {
    hf_call!(app, backend::disconnect())
}

/// Answer the ringing call (audio where Settings says).
#[tauri::command]
#[specta::specta]
pub async fn call_answer(app: AppHandle) -> Result<(), String> {
    let on_mac = !call_target().phone;
    hf_call!(app, backend::answer(on_mac))
}

/// Turn down the ringing call.
#[tauri::command]
#[specta::specta]
pub async fn call_decline(app: AppHandle) -> Result<(), String> {
    hf_call!(app, backend::end_call())
}

/// End the call.
#[tauri::command]
#[specta::specta]
pub async fn call_hang_up(app: AppHandle) -> Result<(), String> {
    hf_call!(app, backend::end_call())
}

/// Call a number through the phone.
#[tauri::command]
#[specta::specta]
pub async fn call_dial(number: String, app: AppHandle) -> Result<(), String> {
    let number = sanitize_number(&number)?;
    hf_call!(app, backend::dial(&number))
}

/// Call the last number again (the phone remembers it).
#[tauri::command]
#[specta::specta]
pub async fn call_redial(app: AppHandle) -> Result<(), String> {
    hf_call!(app, backend::redial())
}

/// One keypad tone during the call.
#[tauri::command]
#[specta::specta]
pub async fn call_dtmf(digit: String, app: AppHandle) -> Result<(), String> {
    let d = dtmf_digit(&digit).ok_or("Only 0-9, *, # and A-D.")?;
    hf_call!(app, backend::dtmf(d))
}

/// Mute or unmute this computer's microphone for the call.
#[tauri::command]
#[specta::specta]
pub async fn call_mute(on: bool, app: AppHandle) -> Result<(), String> {
    hf_call!(app, backend::set_muted(on))
}

/// Move the call audio to the phone (the controls here keep working).
#[tauri::command]
#[specta::specta]
pub async fn call_audio_to_phone(app: AppHandle) -> Result<(), String> {
    let target = AudioTarget {
        phone: true,
        ..call_target()
    };
    hf_call!(app, set_call_audio(target))
}

/// Bring the call audio to this computer (on the devices last chosen).
#[tauri::command]
#[specta::specta]
pub async fn call_audio_to_mac(app: AppHandle) -> Result<(), String> {
    let target = AudioTarget {
        phone: false,
        ..call_target()
    };
    hf_call!(app, set_call_audio(target))
}

// ---------------------------------------------------------------------------
// IOBluetooth (main thread only)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::RefCell;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
    use objc2::{define_class, msg_send, MainThreadMarker, MainThreadOnly};
    use objc2_foundation::{NSArray, NSNumber, NSString};

    use super::{feed, Ev, MIC_MUTED};

    #[link(name = "IOBluetooth", kind = "framework")]
    extern "C" {
        static IOBluetoothHandsFreeCallNumber: &'static NSString;
        static IOBluetoothHandsFreeCallName: &'static NSString;
        static IOBluetoothHandsFreeCallStatus: &'static NSString;
    }

    /// `IOBluetoothHandsFreeDeviceFeatures`: caller id, enhanced call
    /// status (the current call list), three-way calling, remote volume.
    const HF_FEATURES: u32 = (1 << 1) | (1 << 2) | (1 << 4) | (1 << 5);
    /// The Hands-Free Audio Gateway service class.
    const UUID_HFP_AG: u16 = 0x111F;
    /// `kIOReturnSuccess`.
    const OK: i32 = 0;
    /// `kIOReturnUnsupported`: what macOS answers when it won't open the
    /// call's audio link for an app.
    const UNSUPPORTED: i32 = 0xE000_02C7_u32 as i32;

    const NOT_CONNECTED: &str = "Your phone isn\u{2019}t connected for calls. Connect it in Settings \u{2192} Calls on this computer.";

    struct Session {
        address: String,
        name: String,
        hf: Retained<AnyObject>,
        // The framework only keeps a weak (assign) pointer to it.
        _delegate: Retained<Delegate>,
    }

    thread_local! {
        static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    }

    /// Paired phones' names by address (for the Settings status).
    static NAMES: std::sync::Mutex<std::collections::BTreeMap<String, String>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());
    /// The phone connected for calls, readable off the main thread (the
    /// audio device list leaves its device out).
    static CONNECTED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MdHandsFreeDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        impl Delegate {
            #[unsafe(method(handsFree:connected:))]
            fn hf_connected(&self, _d: &AnyObject, status: Option<&NSNumber>) {
                let ok = status.is_none_or(|s| s.intValue() == OK);
                if !ok {
                    eprintln!("[calls] connect failed: {:?}", status.map(|s| s.intValue()));
                    if let Some(app) = super::APP.get() {
                        crate::notify::post_always(
                            app,
                            "Couldn\u{2019}t connect to your phone for calls",
                            "Make sure Bluetooth is on and the phone is nearby, then try Connect again.",
                        );
                    }
                }
                feed(Ev::Connected(ok));
                if ok {
                    // Ask for the call list (caller names, an existing call).
                    current_call_list();
                }
            }

            #[unsafe(method(handsFree:disconnected:))]
            fn hf_disconnected(&self, _d: &AnyObject, _status: Option<&NSNumber>) {
                feed(Ev::Connected(false));
            }

            #[unsafe(method(handsFree:scoConnectionOpened:))]
            fn hf_sco_opened(&self, _d: &AnyObject, status: Option<&NSNumber>) {
                eprintln!("[calls] audio link opened: {:?}", status.map(|s| s.intValue()));
                if status.is_some_and(|s| s.intValue() == UNSUPPORTED) {
                    super::on_mac_audio_refused();
                }
                if status.is_none_or(|s| s.intValue() == OK) {
                    feed(Ev::Sco(true));
                }
            }

            #[unsafe(method(handsFree:scoConnectionClosed:))]
            fn hf_sco_closed(&self, _d: &AnyObject, _status: Option<&NSNumber>) {
                feed(Ev::Sco(false));
            }

            #[unsafe(method(handsFree:isCallActive:))]
            fn hf_call(&self, _d: &AnyObject, v: Option<&NSNumber>) {
                feed(Ev::Call(num(v) != 0));
            }

            #[unsafe(method(handsFree:callSetupMode:))]
            fn hf_setup(&self, _d: &AnyObject, v: Option<&NSNumber>) {
                let mode = num(v);
                feed(Ev::Setup(mode.clamp(0, 3) as u8));
                if mode == 1 {
                    current_call_list();
                }
            }

            #[unsafe(method(handsFree:callHoldState:))]
            fn hf_held(&self, _d: &AnyObject, v: Option<&NSNumber>) {
                feed(Ev::Held(num(v).clamp(0, 2) as u8));
            }

            #[unsafe(method(handsFree:signalStrength:))]
            fn hf_signal(&self, _d: &AnyObject, v: Option<&NSNumber>) {
                feed(Ev::Signal(num(v).clamp(0, 5) as u8));
            }

            #[unsafe(method(handsFree:batteryCharge:))]
            fn hf_battery(&self, _d: &AnyObject, v: Option<&NSNumber>) {
                feed(Ev::Battery(num(v).clamp(0, 5) as u8));
            }

            #[unsafe(method(handsFree:incomingCallFrom:))]
            fn hf_incoming(&self, _d: &AnyObject, number: Option<&NSString>) {
                feed(Ev::Incoming(number.map(|n| n.to_string()).unwrap_or_default()));
            }

            #[unsafe(method(handsFree:currentCall:))]
            fn hf_current_call(&self, _d: &AnyObject, call: Option<&AnyObject>) {
                let Some(call) = call else {
                    return;
                };
                // SAFETY: the framework's own key constants, looked up in
                // the dictionary it handed us.
                let (number, name, status) = unsafe {
                    (
                        string_for(call, IOBluetoothHandsFreeCallNumber),
                        string_for(call, IOBluetoothHandsFreeCallName),
                        number_for(call, IOBluetoothHandsFreeCallStatus),
                    )
                };
                // Only the ringing or current call's caller is shown.
                if status.map(super::call_status_phase) != Some(super::Phase::Idle) {
                    feed(Ev::CallInfo { number, name });
                }
            }
        }
    );

    impl Delegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's init.
            unsafe { msg_send![super(this), init] }
        }
    }

    fn num(v: Option<&NSNumber>) -> i32 {
        v.map(|n| n.intValue()).unwrap_or(0)
    }

    /// `dict[key]` as a string, if it is one.
    unsafe fn string_for(dict: &AnyObject, key: &NSString) -> Option<String> {
        let v: Option<Retained<AnyObject>> = msg_send![dict, objectForKey: key];
        v?.downcast::<NSString>().ok().map(|s| s.to_string())
    }

    /// `dict[key]` as an int, if it's a number.
    unsafe fn number_for(dict: &AnyObject, key: &NSString) -> Option<i32> {
        let v: Option<Retained<AnyObject>> = msg_send![dict, objectForKey: key];
        v?.downcast::<NSNumber>().ok().map(|n| n.intValue())
    }

    fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
        AnyClass::get(name)
            .ok_or_else(|| "Bluetooth calls aren\u{2019}t supported on this Mac.".into())
    }

    fn mtm() -> Result<MainThreadMarker, String> {
        MainThreadMarker::new().ok_or_else(|| "not on the main thread".into())
    }

    /// Paired classic devices offering the Hands-Free Audio Gateway service.
    pub fn gateways() -> Result<Vec<super::CallDevice>, String> {
        mtm()?;
        let cls = class(c"IOBluetoothDevice")?;
        // SAFETY: IOBluetoothDevice's documented class/instance methods,
        // on the main thread.
        unsafe {
            let list: Option<Retained<NSArray<AnyObject>>> = msg_send![cls, pairedDevices];
            let Some(list) = list else {
                return Ok(Vec::new());
            };
            let uuid_cls = class(c"IOBluetoothSDPUUID")?;
            let uuid: Option<Retained<AnyObject>> = msg_send![uuid_cls, uuid16: UUID_HFP_AG];
            let mut out = Vec::new();
            for i in 0..list.count() {
                let dev = list.objectAtIndex(i);
                let mut gateway: bool = msg_send![&*dev, isHandsFreeAudioGateway];
                if !gateway {
                    if let Some(uuid) = &uuid {
                        let rec: Option<Retained<AnyObject>> =
                            msg_send![&*dev, getServiceRecordForUUID: &**uuid];
                        gateway = rec.is_some();
                    }
                }
                if !gateway {
                    continue;
                }
                let addr: Option<Retained<NSString>> = msg_send![&*dev, addressString];
                let name: Option<Retained<NSString>> = msg_send![&*dev, name];
                let connected: bool = msg_send![&*dev, isConnected];
                let Some(addr) = addr.map(|a| a.to_string()) else {
                    continue;
                };
                let name = name.map(|n| n.to_string()).unwrap_or_else(|| addr.clone());
                remember(&addr, &name);
                out.push(super::CallDevice {
                    address: addr,
                    name,
                    connected,
                });
            }
            out.sort_by_key(|d| d.name.to_lowercase());
            Ok(out)
        }
    }

    fn remember(addr: &str, name: &str) {
        if let Ok(mut n) = NAMES.lock() {
            n.insert(addr.to_string(), name.to_string());
        }
    }

    /// A paired phone's name, when seen.
    pub fn cached_name(addr: Option<&str>) -> Option<String> {
        NAMES.lock().ok()?.get(addr?).cloned()
    }

    /// The phone connected for calls (any thread; empty when none).
    pub fn known_name() -> String {
        CONNECTED.lock().map(|n| n.clone()).unwrap_or_default()
    }

    /// The connected phone's name (empty when none).
    pub fn device_name() -> String {
        SESSION.with(|s| {
            s.borrow()
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_default()
        })
    }

    /// Connect to the paired phone at `addr` as its hands-free.
    pub fn connect(addr: &str) -> Result<(), String> {
        let mtm = mtm()?;
        let same = SESSION.with(|s| s.borrow().as_ref().is_some_and(|s| s.address == addr));
        if !same {
            disconnect()?;
            let dev_cls = class(c"IOBluetoothDevice")?;
            let hf_cls = class(c"IOBluetoothHandsFreeDevice")?;
            // SAFETY: documented IOBluetooth calls on the main thread; the
            // delegate is kept alive in SESSION as long as the device object.
            unsafe {
                let dev: Option<Retained<AnyObject>> =
                    msg_send![dev_cls, deviceWithAddressString: &*NSString::from_str(addr)];
                let dev = dev.ok_or("That phone isn\u{2019}t paired with this Mac any more.")?;
                let paired: bool = msg_send![&*dev, isPaired];
                if !paired {
                    return Err(
                        "Pair your phone in System Settings \u{2192} Bluetooth first.".into(),
                    );
                }
                let name: Option<Retained<NSString>> = msg_send![&*dev, name];
                let name = name.map(|n| n.to_string()).unwrap_or_default();
                let delegate = Delegate::new(mtm);
                let alloc: Allocated<AnyObject> = msg_send![hf_cls, alloc];
                let hf: Option<Retained<AnyObject>> =
                    msg_send![alloc, initWithDevice: &*dev, delegate: &*delegate];
                let hf = hf.ok_or("Couldn\u{2019}t set this Mac up as a hands-free.")?;
                let features: u32 = msg_send![&*hf, supportedFeatures];
                let _: () = msg_send![&*hf, setSupportedFeatures: features | HF_FEATURES];
                remember(addr, &name);
                if let Ok(mut c) = CONNECTED.lock() {
                    c.clone_from(&name);
                }
                SESSION.with(|s| {
                    *s.borrow_mut() = Some(Session {
                        address: addr.to_string(),
                        name,
                        hf,
                        _delegate: delegate,
                    })
                });
            }
        }
        with_hf(|hf| unsafe {
            let connected: bool = msg_send![hf, isConnected];
            if !connected {
                let _: () = msg_send![hf, connect];
            }
            Ok(())
        })
    }

    /// Drop the link and forget the device object.
    pub fn disconnect() -> Result<(), String> {
        mtm()?;
        let old = SESSION.with(|s| s.borrow_mut().take());
        if let Some(old) = old {
            // SAFETY: documented calls; the delegate is cleared before both
            // objects go, so no callback reaches a freed delegate.
            unsafe {
                let _: () = msg_send![&*old.hf, disconnect];
                let none: Option<&AnyObject> = None;
                let _: () = msg_send![&*old.hf, setDelegate: none];
            }
            feed(Ev::Connected(false));
        }
        Ok(())
    }

    /// Run `f` with the connected hands-free object.
    fn with_hf<T>(f: impl FnOnce(&AnyObject) -> Result<T, String>) -> Result<T, String> {
        mtm()?;
        SESSION.with(|s| {
            let s = s.borrow();
            let s = s.as_ref().ok_or(NOT_CONNECTED)?;
            f(&s.hf)
        })
    }

    fn with_connected(f: impl FnOnce(&AnyObject)) -> Result<(), String> {
        with_hf(|hf| {
            // SAFETY: IOBluetoothHandsFree's isConnected.
            let connected: bool = unsafe { msg_send![hf, isConnected] };
            if !connected {
                return Err(NOT_CONNECTED.into());
            }
            f(hf);
            Ok(())
        })
    }

    fn current_call_list() {
        // SAFETY: documented; answers come through handsFree:currentCall:.
        let _ = with_connected(|hf| unsafe {
            let _: () = msg_send![hf, currentCallList];
        });
    }

    // SAFETY (all below): documented IOBluetoothHandsFreeDevice methods,
    // called on the main thread with a connected device.

    pub fn answer(on_mac: bool) -> Result<(), String> {
        with_connected(|hf| unsafe {
            if on_mac {
                let _: () = msg_send![hf, acceptCall];
            } else {
                let _: () = msg_send![hf, acceptCallOnPhone];
            }
        })
    }

    pub fn end_call() -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, endCall];
        })
    }

    pub fn dial(number: &str) -> Result<(), String> {
        let n = NSString::from_str(number);
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, dialNumber: &*n];
        })
    }

    pub fn redial() -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, redial];
        })
    }

    pub fn dtmf(c: char) -> Result<(), String> {
        let s = NSString::from_str(&c.to_string());
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, sendDTMF: &*s];
        })
    }

    pub fn set_muted(on: bool) -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, setInputMuted: on];
        })?;
        MIC_MUTED.store(on, std::sync::atomic::Ordering::Relaxed);
        feed(Ev::Muted(on));
        Ok(())
    }

    /// Close the audio link: the phone's earpiece takes over; the
    /// service-level link (and the controls here) stays.
    pub fn audio_to_phone() -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, transferAudioToPhone];
        })
    }

    /// Open the audio link again: this Mac's speaker and microphone.
    pub fn audio_to_mac() -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, transferAudioToComputer];
        })
    }

    /// Open the audio (SCO) link directly, for when the phone didn't move
    /// the audio on request.
    pub fn connect_sco() -> Result<(), String> {
        with_connected(|hf| unsafe {
            let _: () = msg_send![hf, connectSCO];
        })
    }

    /// Run `f` on the main thread (IOBluetooth lives there) and wait for it.
    pub fn exec<T: Send + 'static>(
        app: &tauri::AppHandle,
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| e.to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Bluetooth didn\u{2019}t answer in time.".to_string())?
    }

    /// Run `f` on the main thread later.
    pub fn run_later(app: &tauri::AppHandle, f: impl FnOnce() + Send + 'static) {
        let _ = app.run_on_main_thread(f);
    }

    /// The call audio shows up as a CoreAudio device named after the phone.
    pub fn phone_end() -> Option<super::audio::PhoneEnd> {
        Some(super::audio::PhoneEnd::Device(device_name()))
    }

    /// Nothing stands in the way on macOS (a refused audio link is
    /// `mac_audio_refused`, handled on its own).
    pub fn problem() -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// md-hfp's events (Linux and Windows)
// ---------------------------------------------------------------------------

/// md-hfp's events (its dispatcher thread, in order) into the call state
/// (Linux and Windows). `list_calls` asks the phone for its call list.
#[cfg(any(target_os = "linux", target_os = "windows", test))]
#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), allow(dead_code))]
fn md_hfp_event(ev: md_hfp::HfEvent, list_calls: impl Fn()) {
    use md_hfp::{Event, HfEvent};
    match ev {
        HfEvent::At(e) => match e {
            Event::Ready { .. } => feed(Ev::Connected(true)),
            Event::Call(on) => feed(Ev::Call(on)),
            Event::Setup(s) => {
                feed(Ev::Setup(s));
                if s == 1 {
                    // The caller's name, when the phone lists it.
                    list_calls();
                }
            }
            Event::Held(h) => feed(Ev::Held(h)),
            Event::Signal(s) => feed(Ev::Signal(s)),
            Event::Battery(b) => feed(Ev::Battery(b)),
            Event::Caller { number, name } => {
                feed(Ev::Incoming(number.clone()));
                if name.is_some() {
                    feed(Ev::CallInfo {
                        number: Some(number),
                        name,
                    });
                }
            }
            Event::ListedCall(c) => {
                // Only the ringing or current call's caller is shown.
                if call_status_phase(c.status as i32) != Phase::Idle {
                    feed(Ev::CallInfo {
                        number: c.number,
                        name: c.name,
                    });
                }
            }
            Event::Refused { command } => {
                eprintln!("[calls] the phone refused {command}");
            }
            _ => {}
        },
        HfEvent::Disconnected => feed(Ev::Connected(false)),
        HfEvent::Audio(a) => feed(Ev::Sco(a.is_some())),
        HfEvent::ConnectFailed(why) => {
            eprintln!("[calls] connect failed: {why}");
            if let Some(app) = APP.get() {
                crate::notify::post_always(
                    app,
                    "Couldn\u{2019}t connect to your phone for calls",
                    &why,
                );
            }
            feed(Ev::Connected(false));
        }
        HfEvent::Problem(p) => {
            if let Some(p) = &p {
                eprintln!("[calls] {p}");
            }
            if let Some(app) = APP.get() {
                if let Ok(m) = MACHINE.lock().map(|m| m.clone()) {
                    emit_state(app, &m);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: BlueZ through md-hfp (its own threads)
// ---------------------------------------------------------------------------

// Compiled into the macOS test build too (md-hfp's stub `Hfp` has the same
// API there), so this glue is at least type-checked off Linux.
// Also built for macOS tests, so the Mac type-checks it (Windows has no
// audio bridge for its phone end).
#[cfg(any(target_os = "linux", all(test, target_os = "macos")))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux {
    use std::sync::{Arc, Mutex};

    use md_hfp::{HfEvent, Hfp};

    use super::{feed, CallDevice, Ev, MIC_MUTED};

    const NOT_CONNECTED: &str = "Your phone isn\u{2019}t connected for calls. Connect it in Settings \u{2192} Calls on this computer.";

    /// The hands-free unit, started on first use.
    static HFP: Mutex<Option<Arc<Hfp>>> = Mutex::new(None);
    /// Why it couldn't start (BlueZ not running...), for Settings.
    static START_ERROR: Mutex<Option<String>> = Mutex::new(None);

    fn current() -> Option<Arc<Hfp>> {
        HFP.lock().ok()?.clone()
    }

    /// The running unit, starting it if need be.
    fn hfp() -> Result<Arc<Hfp>, String> {
        let mut g = HFP
            .lock()
            .map_err(|_| "Bluetooth calls stopped.".to_string())?;
        if let Some(h) = g.as_ref() {
            return Ok(h.clone());
        }
        let started = Hfp::start(on_event);
        if let Ok(mut e) = START_ERROR.lock() {
            *e = started.as_ref().err().cloned();
        }
        let h = Arc::new(started?);
        *g = Some(h.clone());
        Ok(h)
    }

    /// md-hfp's events into the call state.
    fn on_event(ev: HfEvent) {
        super::md_hfp_event(ev, || {
            if let Some(h) = current() {
                let _ = h.list_calls();
            }
        });
    }

    /// md-hfp blocks only briefly (its worker answers within seconds), so
    /// calls run right here, on the caller's thread.
    pub fn exec<T: Send + 'static>(
        _app: &tauri::AppHandle,
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        f()
    }

    pub fn run_later(_app: &tauri::AppHandle, f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(f);
    }

    pub fn gateways() -> Result<Vec<CallDevice>, String> {
        Ok(hfp()?
            .gateways()?
            .into_iter()
            .map(|g| CallDevice {
                address: g.address,
                name: g.name,
                connected: g.connected,
            })
            .collect())
    }

    pub fn connect(addr: &str) -> Result<(), String> {
        hfp()?.connect(addr)
    }

    pub fn disconnect() -> Result<(), String> {
        match current() {
            Some(h) => h.disconnect(),
            None => Ok(()),
        }
    }

    fn connected() -> Result<Arc<Hfp>, String> {
        current().ok_or_else(|| NOT_CONNECTED.to_string())
    }

    pub fn answer(on_computer: bool) -> Result<(), String> {
        connected()?.answer(on_computer)
    }

    pub fn end_call() -> Result<(), String> {
        connected()?.hang_up()
    }

    pub fn dial(number: &str) -> Result<(), String> {
        connected()?.dial(number)
    }

    pub fn redial() -> Result<(), String> {
        connected()?.redial()
    }

    pub fn dtmf(c: char) -> Result<(), String> {
        connected()?.dtmf(c)
    }

    /// The microphone is ours on Linux: muting just silences what the
    /// bridge sends.
    pub fn set_muted(on: bool) -> Result<(), String> {
        connected()?;
        MIC_MUTED.store(on, std::sync::atomic::Ordering::Relaxed);
        feed(Ev::Muted(on));
        Ok(())
    }

    pub fn audio_to_phone() -> Result<(), String> {
        connected()?.audio_to_phone()
    }

    pub fn audio_to_mac() -> Result<(), String> {
        connected()?.audio_here()
    }

    pub fn connect_sco() -> Result<(), String> {
        connected()?.connect_audio()
    }

    /// The open audio link's samples (none while it's closed).
    pub fn phone_end() -> Option<super::audio::PhoneEnd> {
        current()?.audio().map(super::audio::PhoneEnd::Sco)
    }

    pub fn device_name() -> String {
        current().map(|h| h.device_name()).unwrap_or_default()
    }

    pub fn known_name() -> String {
        device_name()
    }

    pub fn cached_name(addr: Option<&str>) -> Option<String> {
        current()?.cached_name(addr?)
    }

    /// Why calls can't work here right now: BlueZ missing, or another
    /// program with the hands-free role (and how to turn that off).
    pub fn problem() -> Option<String> {
        match current() {
            Some(h) => h.problem(),
            None => START_ERROR.lock().ok()?.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: md-hfp over a Winsock RFCOMM socket (call control only)
// ---------------------------------------------------------------------------

// Like `linux`, compiled into the macOS test build too (against md-hfp's
// stub `Hfp`), so this glue is type-checked off Windows. The audio never
// comes here (see the top of this file): `phone_end` is always None and
// asking for the audio here says why.
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
mod windows {
    use std::sync::{Arc, Mutex};

    use md_hfp::{HfEvent, Hfp};

    use super::CallDevice;

    const NOT_CONNECTED: &str = "Your phone isn\u{2019}t connected for calls. Connect it in Settings \u{2192} Calls on this computer.";
    /// The call's audio (and so its microphone) is on the phone.
    const MUTE_ON_PHONE: &str = "Mute on your phone: the call\u{2019}s audio is there, Windows doesn\u{2019}t let apps carry it to this computer.";

    /// The hands-free unit, started on first use.
    static HFP: Mutex<Option<Arc<Hfp>>> = Mutex::new(None);
    /// Why it couldn't start (Winsock didn't), for Settings.
    static START_ERROR: Mutex<Option<String>> = Mutex::new(None);

    fn current() -> Option<Arc<Hfp>> {
        HFP.lock().ok()?.clone()
    }

    /// The running unit, starting it if need be.
    fn hfp() -> Result<Arc<Hfp>, String> {
        let mut g = HFP
            .lock()
            .map_err(|_| "Bluetooth calls stopped.".to_string())?;
        if let Some(h) = g.as_ref() {
            return Ok(h.clone());
        }
        let started = Hfp::start(on_event);
        if let Ok(mut e) = START_ERROR.lock() {
            *e = started.as_ref().err().cloned();
        }
        let h = Arc::new(started?);
        *g = Some(h.clone());
        Ok(h)
    }

    fn on_event(ev: HfEvent) {
        super::md_hfp_event(ev, || {
            if let Some(h) = current() {
                let _ = h.list_calls();
            }
        });
    }

    /// md-hfp blocks only briefly (its worker answers within seconds; a
    /// connect happens on a thread of its own), so calls run right here.
    pub fn exec<T: Send + 'static>(
        _app: &tauri::AppHandle,
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        f()
    }

    pub fn run_later(_app: &tauri::AppHandle, f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(f);
    }

    pub fn gateways() -> Result<Vec<CallDevice>, String> {
        Ok(hfp()?
            .gateways()?
            .into_iter()
            .map(|g| CallDevice {
                address: g.address,
                name: g.name,
                connected: g.connected,
            })
            .collect())
    }

    pub fn connect(addr: &str) -> Result<(), String> {
        hfp()?.connect(addr)
    }

    pub fn disconnect() -> Result<(), String> {
        match current() {
            Some(h) => h.disconnect(),
            None => Ok(()),
        }
    }

    fn connected() -> Result<Arc<Hfp>, String> {
        current().ok_or_else(|| NOT_CONNECTED.to_string())
    }

    /// The audio stays on the phone whatever `on_computer` says.
    pub fn answer(on_computer: bool) -> Result<(), String> {
        connected()?.answer(on_computer)
    }

    pub fn end_call() -> Result<(), String> {
        connected()?.hang_up()
    }

    pub fn dial(number: &str) -> Result<(), String> {
        connected()?.dial(number)
    }

    pub fn redial() -> Result<(), String> {
        connected()?.redial()
    }

    pub fn dtmf(c: char) -> Result<(), String> {
        connected()?.dtmf(c)
    }

    /// No microphone here is in the call: muting is the phone's.
    pub fn set_muted(_on: bool) -> Result<(), String> {
        connected()?;
        Err(MUTE_ON_PHONE.into())
    }

    pub fn audio_to_phone() -> Result<(), String> {
        connected()?.audio_to_phone()
    }

    /// Says why the audio can't come here (md-hfp's message).
    pub fn audio_to_mac() -> Result<(), String> {
        connected()?.audio_here()
    }

    pub fn connect_sco() -> Result<(), String> {
        connected()?.connect_audio()
    }

    /// Nothing to bridge: no app gets the call audio on Windows.
    pub fn phone_end() -> Option<super::audio::PhoneEnd> {
        None
    }

    pub fn device_name() -> String {
        current().map(|h| h.device_name()).unwrap_or_default()
    }

    pub fn known_name() -> String {
        device_name()
    }

    pub fn cached_name(addr: Option<&str>) -> Option<String> {
        current()?.cached_name(addr?)
    }

    /// Why calls can't work here right now: Windows' own hands-free
    /// (Phone Link's Calls) has the phone, and how to free it.
    pub fn problem() -> Option<String> {
        match current() {
            Some(h) => h.problem(),
            None => START_ERROR.lock().ok()?.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// No backend on this OS: everything says "not available"
// ---------------------------------------------------------------------------

#[cfg(any(
    not(any(target_os = "macos", target_os = "linux", target_os = "windows")),
    test
))]
#[allow(dead_code)]
mod none {
    use super::{CallDevice, NOT_AVAILABLE};

    fn no<T>() -> Result<T, String> {
        Err(NOT_AVAILABLE.into())
    }

    pub fn exec<T: Send + 'static>(
        _app: &tauri::AppHandle,
        _f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        no()
    }

    pub fn run_later(_app: &tauri::AppHandle, _f: impl FnOnce() + Send + 'static) {}

    pub fn gateways() -> Result<Vec<CallDevice>, String> {
        no()
    }

    pub fn connect(_addr: &str) -> Result<(), String> {
        no()
    }

    pub fn disconnect() -> Result<(), String> {
        no()
    }

    pub fn answer(_on_computer: bool) -> Result<(), String> {
        no()
    }

    pub fn end_call() -> Result<(), String> {
        no()
    }

    pub fn dial(_number: &str) -> Result<(), String> {
        no()
    }

    pub fn redial() -> Result<(), String> {
        no()
    }

    pub fn dtmf(_c: char) -> Result<(), String> {
        no()
    }

    pub fn set_muted(_on: bool) -> Result<(), String> {
        no()
    }

    pub fn audio_to_phone() -> Result<(), String> {
        no()
    }

    pub fn audio_to_mac() -> Result<(), String> {
        no()
    }

    pub fn connect_sco() -> Result<(), String> {
        no()
    }

    pub fn phone_end() -> Option<super::audio::PhoneEnd> {
        None
    }

    pub fn device_name() -> String {
        String::new()
    }

    pub fn known_name() -> String {
        String::new()
    }

    pub fn cached_name(_addr: Option<&str>) -> Option<String> {
        None
    }

    pub fn problem() -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// Call audio: the phone's audio link <-> this computer's speaker and mic
// ---------------------------------------------------------------------------

/// Linear resampling of mono samples from one rate to another (call audio is
/// 8 or 16 kHz; a Mac's speaker is usually 44.1 or 48 kHz).
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// Position of the next output sample, in input samples from `last`.
    pos: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from_hz: u32, to_hz: u32) -> Self {
        Resampler {
            step: from_hz.max(1) as f64 / to_hz.max(1) as f64,
            pos: 0.0,
            last: 0.0,
        }
    }

    /// Resample `input`, appending to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        for &s in input {
            // Output samples between `last` (at 0) and `s` (at 1).
            // (A hair under 1, so float steps like 1/6 don't add a sample.)
            while self.pos < 1.0 - 1e-9 {
                let t = self.pos as f32;
                out.push(self.last + (s - self.last) * t);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.last = s;
        }
    }
}

/// Average the channels of interleaved frames into mono.
pub fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    data.chunks(channels)
        .map(|f| f.iter().sum::<f32>() / f.len() as f32)
        .collect()
}

/// The audio device for the phone: its name matches the phone's.
pub fn is_phone_device(device: &str, phone: &str) -> bool {
    let (d, p) = (device.trim().to_lowercase(), phone.trim().to_lowercase());
    !p.is_empty() && (d == p || d.contains(&p))
}

/// This Mac's own speaker or microphone, by CoreAudio's usual names.
pub fn is_builtin(device: &str) -> bool {
    let d = device.to_lowercase();
    [
        "built-in",
        "macbook",
        "imac",
        "mac mini",
        "mac studio",
        "mac pro",
    ]
    .iter()
    .any(|b| d.contains(b))
}

/// The device to use from `available`: the one `wanted` while it's there;
/// with none wanted, the system default; if the wanted one is gone, the
/// built-in one, then the default, then any. Never the phone's own device.
pub fn pick_device(
    wanted: Option<&str>,
    available: &[String],
    default: Option<&str>,
    phone: &str,
) -> Option<String> {
    let usable = |n: &str| !is_phone_device(n, phone) && available.iter().any(|a| a == n);
    let builtin = || {
        available
            .iter()
            .find(|a| is_builtin(a) && usable(a))
            .cloned()
    };
    let default = || default.filter(|d| usable(d)).map(str::to_string);
    match wanted {
        Some(w) if usable(w) => Some(w.to_string()),
        Some(_) => builtin().or_else(default),
        None => default().or_else(builtin),
    }
    .or_else(|| available.iter().find(|a| usable(a)).cloned())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod audio {
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::{
        downmix, is_phone_device, pick_device, AudioDevices, AudioTarget, Resampler, MIC_MUTED,
    };

    /// The phone's end of the call audio, as the backend has it.
    #[derive(Clone)]
    pub enum PhoneEnd {
        /// macOS: the CoreAudio device the system makes for the audio
        /// link, named after the phone.
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        Device(String),
        /// Linux: the audio link's samples (md-hfp holds the socket).
        #[cfg_attr(target_os = "macos", allow(dead_code))]
        Sco(Arc<md_hfp::ScoPcm>),
    }

    /// How often, and how many times, to look for the phone's audio device
    /// after the SCO link opens (it can take a moment to appear). On Linux
    /// the samples are handed over directly (its bridge never waits).
    const FIND_EVERY: Duration = Duration::from_millis(300);
    const FIND_TRIES: u32 = if cfg!(target_os = "macos") { 10 } else { 1 };

    /// A bridge thread; dropping `_stop` ends it (and its streams).
    struct Running {
        gen: u64,
        _stop: mpsc::Sender<()>,
        /// The devices it settled on (None while still starting).
        output: Option<String>,
        input: Option<String>,
    }

    struct State {
        /// The phone's end and where its audio should go, while the SCO
        /// link is open.
        want: Option<(PhoneEnd, AudioTarget)>,
        running: Option<Running>,
        gen: u64,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        want: None,
        running: None,
        gen: 0,
    });

    /// Bridge the phone's audio to `target` (restarting a running bridge).
    /// Doesn't block.
    pub fn start(phone: PhoneEnd, target: AudioTarget) {
        let (tx, rx) = mpsc::channel::<()>();
        let gen = {
            let Ok(mut s) = STATE.lock() else {
                return;
            };
            s.gen += 1;
            s.want = Some((phone.clone(), target.clone()));
            s.running = Some(Running {
                gen: s.gen,
                _stop: tx,
                output: None,
                input: None,
            });
            s.gen
        };
        std::thread::spawn(move || {
            let mut tries = 0;
            let (streams, output, input) = loop {
                if !current(gen) {
                    return;
                }
                match bridge(&phone, &target) {
                    Ok(Some(b)) => break b,
                    Ok(None) if tries < FIND_TRIES => {
                        tries += 1;
                        std::thread::sleep(FIND_EVERY);
                    }
                    Ok(None) => {
                        eprintln!(
                            "[calls] no audio device for the phone; call audio left to macOS"
                        );
                        give_up(gen);
                        return;
                    }
                    Err(e) => {
                        eprintln!("[calls] couldn't route the call audio: {e}");
                        give_up(gen);
                        return;
                    }
                }
            };
            {
                let Ok(mut s) = STATE.lock() else {
                    return;
                };
                match s.running.as_mut() {
                    Some(r) if r.gen == gen => {
                        r.output = Some(output.clone());
                        r.input = Some(input.clone());
                    }
                    _ => return,
                }
            }
            eprintln!("[calls] call audio on {output} / {input}");
            super::audio_changed();
            // Until stopped or replaced (the sender is dropped).
            let _ = rx.recv();
            drop(streams);
        });
    }

    fn current(gen: u64) -> bool {
        STATE
            .lock()
            .is_ok_and(|s| s.running.as_ref().is_some_and(|r| r.gen == gen))
    }

    /// The bridge couldn't start: no bridge, but still wanted (a device
    /// appearing later tries again).
    fn give_up(gen: u64) {
        if let Ok(mut s) = STATE.lock() {
            if s.running.as_ref().is_some_and(|r| r.gen == gen) {
                s.running = None;
            }
        }
        super::audio_changed();
    }

    /// The SCO link closed or the call ended: stop bridging.
    pub fn stop() {
        if let Ok(mut s) = STATE.lock() {
            s.gen += 1;
            s.want = None;
            s.running = None;
        }
    }

    /// The devices in use: (output, input).
    pub fn in_use() -> (Option<String>, Option<String>) {
        STATE
            .lock()
            .ok()
            .and_then(|s| {
                s.running
                    .as_ref()
                    .map(|r| (r.output.clone(), r.input.clone()))
            })
            .unwrap_or((None, None))
    }

    fn names(devices: impl Iterator<Item = cpal::Device>) -> Vec<String> {
        let mut v: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// This computer's outputs and inputs, without the phone's own device.
    pub fn list() -> AudioDevices {
        let host = cpal::default_host();
        let phone = super::backend::known_name();
        let keep = |v: Vec<String>| -> Vec<String> {
            v.into_iter()
                .filter(|n| !is_phone_device(n, &phone))
                .collect()
        };
        AudioDevices {
            outputs: keep(host.output_devices().map(names).unwrap_or_default()),
            inputs: keep(host.input_devices().map(names).unwrap_or_default()),
            default_output: host.default_output_device().and_then(|d| d.name().ok()),
            default_input: host.default_input_device().and_then(|d| d.name().ok()),
        }
    }

    /// Devices came or went: tell the UI, and move a running bridge whose
    /// device went (or retry one that found no phone device).
    #[cfg(target_os = "macos")]
    fn devices_changed() {
        use tauri::Emitter;
        if let Some(app) = super::APP.get() {
            let _ = app.emit("calls-audio-devices", ());
        }
        let Some((want, running)) = STATE.lock().ok().map(|s| {
            let r = s
                .running
                .as_ref()
                .map(|r| (r.output.clone(), r.input.clone()));
            (s.want.clone(), r)
        }) else {
            return;
        };
        let Some((phone, target)) = want else {
            return;
        };
        let healthy = match running {
            None => false,
            // Still starting.
            Some((None, _)) | Some((_, None)) => true,
            Some((Some(out), Some(inp))) => {
                let host = cpal::default_host();
                let outs = host.output_devices().map(names).unwrap_or_default();
                let ins = host.input_devices().map(names).unwrap_or_default();
                outs.contains(&out) && ins.contains(&inp)
            }
        };
        if !healthy {
            start(phone, target);
        }
    }

    /// Listen for audio devices coming and going (once per process).
    #[cfg(target_os = "macos")]
    pub fn watch_devices() {
        use std::ffi::c_void;
        use std::sync::Once;

        #[repr(C)]
        struct PropertyAddress {
            selector: u32,
            scope: u32,
            element: u32,
        }

        type Listener = unsafe extern "C" fn(u32, u32, *const PropertyAddress, *mut c_void) -> i32;

        #[link(name = "CoreAudio", kind = "framework")]
        extern "C" {
            fn AudioObjectAddPropertyListener(
                object: u32,
                address: *const PropertyAddress,
                listener: Listener,
                data: *mut c_void,
            ) -> i32;
        }

        const fn fourcc(s: &[u8; 4]) -> u32 {
            u32::from_be_bytes(*s)
        }

        /// `kAudioObjectSystemObject`.
        const SYSTEM_OBJECT: u32 = 1;

        unsafe extern "C" fn on_devices(
            _object: u32,
            _count: u32,
            _addresses: *const PropertyAddress,
            _data: *mut c_void,
        ) -> i32 {
            // CoreAudio's own thread: do the work elsewhere.
            std::thread::spawn(devices_changed);
            0
        }

        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let address = PropertyAddress {
                selector: fourcc(b"dev#"), // kAudioHardwarePropertyDevices
                scope: fourcc(b"glob"),    // kAudioObjectPropertyScopeGlobal
                element: 0,                // kAudioObjectPropertyElementMain
            };
            // SAFETY: a documented CoreAudio call; the address is copied by
            // CoreAudio, the listener is a plain function with no data.
            let err = unsafe {
                AudioObjectAddPropertyListener(
                    SYSTEM_OBJECT,
                    &address,
                    on_devices,
                    std::ptr::null_mut(),
                )
            };
            if err != 0 {
                eprintln!("[calls] can't watch audio devices ({err})");
            }
        });
    }

    /// No device watching on Linux (see the top of this file): the pickers
    /// refresh when opened.
    #[cfg(not(target_os = "macos"))]
    pub fn watch_devices() {}

    /// A running bridge's streams; dropping it ends them and unhooks the
    /// audio link's sink.
    struct Streams {
        streams: Vec<cpal::Stream>,
        sco: Option<Arc<md_hfp::ScoPcm>>,
    }

    impl Drop for Streams {
        fn drop(&mut self) {
            if let Some(sco) = &self.sco {
                sco.set_sink(None);
            }
            self.streams.clear();
        }
    }

    type Bridge = (Streams, String, String);

    /// Bridge the phone's end to `target`'s devices. `None` while the
    /// phone's device isn't there (macOS).
    fn bridge(phone: &PhoneEnd, target: &AudioTarget) -> Result<Option<Bridge>, String> {
        match phone {
            PhoneEnd::Device(name) => bridge_device(name, target),
            PhoneEnd::Sco(sco) => bridge_sco(sco, target).map(Some),
        }
    }

    /// The speaker and microphone to use: (output name, device, input
    /// name, device), never the phone's own device.
    fn choose(
        host: &cpal::Host,
        target: &AudioTarget,
        phone: &str,
    ) -> Result<(String, cpal::Device, String, cpal::Device), String> {
        let name = |d: &cpal::Device| d.name().unwrap_or_default();
        let outs = host
            .output_devices()
            .map(names)
            .map_err(|e| e.to_string())?;
        let ins = host.input_devices().map(names).map_err(|e| e.to_string())?;
        let def_out = host.default_output_device().map(|d| name(&d));
        let def_in = host.default_input_device().map(|d| name(&d));
        let out_name = pick_device(target.output.as_deref(), &outs, def_out.as_deref(), phone)
            .ok_or("no speaker or headset to play the call on")?;
        let in_name = pick_device(target.input.as_deref(), &ins, def_in.as_deref(), phone)
            .ok_or("no microphone for the call")?;
        let out = host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| name(d) == out_name)
            .ok_or("the speaker went away")?;
        let inp = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| name(d) == in_name)
            .ok_or("the microphone went away")?;
        Ok((out_name, out, in_name, inp))
    }

    /// macOS: four streams, phone device -> output, input -> phone device.
    fn bridge_device(phone: &str, target: &AudioTarget) -> Result<Option<Bridge>, String> {
        let host = cpal::default_host();
        let name = |d: &cpal::Device| d.name().unwrap_or_default();
        let phone_in = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| is_phone_device(&name(d), phone));
        let phone_out = host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| is_phone_device(&name(d), phone));
        let (Some(phone_in), Some(phone_out)) = (phone_in, phone_out) else {
            return Ok(None);
        };
        let (out_name, mac_out, in_name, mac_in) = choose(&host, target, phone)?;
        let mut streams = pipe(&phone_in, &mac_out, false)?;
        streams.extend(pipe(&mac_in, &phone_out, true)?);
        for s in &streams {
            s.play().map_err(|e| e.to_string())?;
        }
        Ok(Some((Streams { streams, sco: None }, out_name, in_name)))
    }

    /// Linux: the audio link's samples -> output; input -> the audio link.
    fn bridge_sco(sco: &Arc<md_hfp::ScoPcm>, target: &AudioTarget) -> Result<Bridge, String> {
        let host = cpal::default_host();
        let (out_name, out, in_name, inp) = choose(&host, target, "")?;
        let out_cfg = out.default_output_config().map_err(|e| e.to_string())?;
        let out_hz = out_cfg.sample_rate().0;
        let ring = Ring::new(out_hz);
        let speaker = play(&out, &out_cfg, ring.clone())?;
        let link = sco.clone();
        let mic = capture(&inp, sco.rate(), true, move |s| link.push_mic(s))?;
        let streams = vec![mic, speaker];
        for s in &streams {
            s.play().map_err(|e| e.to_string())?;
        }
        // Only now, with everything playing: the caller's voice, resampled
        // to the speaker's rate.
        let mut resampler = Resampler::new(sco.rate(), out_hz);
        let mut buf = Vec::new();
        sco.set_sink(Some(Box::new(move |s: &[f32]| {
            buf.clear();
            resampler.process(s, &mut buf);
            ring.push(&buf);
        })));
        Ok((
            Streams {
                streams,
                sco: Some(sco.clone()),
            },
            out_name,
            in_name,
        ))
    }

    /// Mono samples waiting for an output stream: half a second at most,
    /// so a stall doesn't build up delay.
    #[derive(Clone)]
    struct Ring {
        q: Arc<Mutex<VecDeque<f32>>>,
        cap: usize,
    }

    impl Ring {
        fn new(hz: u32) -> Self {
            let cap = (hz / 2) as usize;
            Ring {
                q: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
                cap,
            }
        }

        fn push(&self, samples: &[f32]) {
            if let Ok(mut r) = self.q.lock() {
                r.extend(samples.iter().copied());
                while r.len() > self.cap {
                    r.pop_front();
                }
            }
        }
    }

    /// Input device -> output device; `mic` ones are silenced while muted.
    fn pipe(
        from: &cpal::Device,
        to: &cpal::Device,
        mic: bool,
    ) -> Result<Vec<cpal::Stream>, String> {
        let out_cfg = to.default_output_config().map_err(|e| e.to_string())?;
        let ring = Ring::new(out_cfg.sample_rate().0);
        let writer = ring.clone();
        let input = capture(from, out_cfg.sample_rate().0, mic, move |s| writer.push(s))?;
        let output = play(to, &out_cfg, ring)?;
        Ok(vec![input, output])
    }

    /// Record `from` as mono at `to_hz` (silenced while muted when `mic`)
    /// and hand each piece to `sink`.
    fn capture(
        from: &cpal::Device,
        to_hz: u32,
        mic: bool,
        mut sink: impl FnMut(&[f32]) + Send + 'static,
    ) -> Result<cpal::Stream, String> {
        let in_cfg = from.default_input_config().map_err(|e| e.to_string())?;
        let in_ch = in_cfg.channels() as usize;
        let mut resampler = Resampler::new(in_cfg.sample_rate().0, to_hz);
        let mut buf = Vec::new();
        from.build_input_stream::<f32, _, _>(
            &in_cfg.config(),
            move |data, _| {
                let mut mono = downmix(data, in_ch);
                if mic && MIC_MUTED.load(Ordering::Relaxed) {
                    mono.iter_mut().for_each(|s| *s = 0.0);
                }
                buf.clear();
                resampler.process(&mono, &mut buf);
                sink(&buf);
            },
            |e| eprintln!("[calls] audio input: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
    }

    /// Play `ring` (at `cfg`'s rate) on `to`, the same sample on every
    /// channel; silence when it runs dry.
    fn play(
        to: &cpal::Device,
        cfg: &cpal::SupportedStreamConfig,
        ring: Ring,
    ) -> Result<cpal::Stream, String> {
        let out_ch = cfg.channels() as usize;
        to.build_output_stream::<f32, _, _>(
            &cfg.config(),
            move |data: &mut [f32], _| {
                let mut r = ring.q.lock().ok();
                for frame in data.chunks_mut(out_ch.max(1)) {
                    let s = r.as_mut().and_then(|r| r.pop_front()).unwrap_or(0.0);
                    frame.iter_mut().for_each(|o| *o = s);
                }
            },
            |e| eprintln!("[calls] audio output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
    }
}

/// No audio bridge on this OS (no backend to give it a phone end).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod audio {
    use super::{AudioDevices, AudioTarget};

    pub enum PhoneEnd {}

    pub fn start(phone: PhoneEnd, _target: AudioTarget) {
        match phone {}
    }

    pub fn stop() {}

    pub fn in_use() -> (Option<String>, Option<String>) {
        (None, None)
    }

    pub fn list() -> AudioDevices {
        AudioDevices::default()
    }

    pub fn watch_devices() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> Machine {
        let mut m = Machine::new();
        m.apply(Ev::Connected(true), 0.0);
        m
    }

    #[test]
    fn calls_incoming_answered_and_ended() {
        let mut m = connected();
        assert_eq!(m.phase(), Phase::Idle);
        let out = m.apply(Ev::Setup(1), 1.0);
        assert_eq!(
            out,
            vec![
                Out::Incoming {
                    number: String::new(),
                    name: None
                },
                Out::State
            ]
        );
        // The number arrives after the ringing starts: told again.
        let out = m.apply(Ev::Incoming("+911234".into()), 2.0);
        assert_eq!(
            out,
            vec![Out::Incoming {
                number: "+911234".into(),
                name: None
            }]
        );
        // Same number again: nothing new.
        assert!(m.apply(Ev::Incoming("+911234".into()), 2.5).is_empty());
        let out = m.apply(
            Ev::CallInfo {
                number: Some("+911234".into()),
                name: Some(" Asha ".into()),
            },
            3.0,
        );
        assert_eq!(
            out,
            vec![Out::Incoming {
                number: "+911234".into(),
                name: Some("Asha".into())
            }]
        );
        assert_eq!(m.caller().as_deref(), Some("Asha"));
        // Answered: active from now, setup goes back to 0 meanwhile.
        let out = m.apply(Ev::Call(true), 10.0);
        assert_eq!(out, vec![Out::Active, Out::State]);
        assert_eq!(m.since_ms, Some(10.0));
        assert_eq!(m.apply(Ev::Setup(0), 11.0), vec![]);
        assert_eq!(m.phase(), Phase::Active);
        let out = m.apply(Ev::Call(false), 20.0);
        assert_eq!(out, vec![Out::Ended, Out::State]);
        assert_eq!(
            (m.number.clone(), m.name.clone(), m.since_ms),
            (None, None, None)
        );
    }

    #[test]
    fn calls_declined_or_missed() {
        let mut m = connected();
        m.apply(Ev::Incoming("555".into()), 0.0);
        assert_eq!(m.phase(), Phase::Incoming);
        assert_eq!(m.apply(Ev::Setup(0), 1.0), vec![Out::Ended, Out::State]);
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn calls_outgoing() {
        let mut m = connected();
        assert_eq!(m.apply(Ev::Setup(2), 0.0), vec![Out::State]);
        assert_eq!(m.phase(), Phase::Outgoing);
        assert_eq!(m.apply(Ev::Setup(3), 0.0), vec![]);
        assert_eq!(m.apply(Ev::Call(true), 5.0), vec![Out::Active, Out::State]);
        assert_eq!(m.apply(Ev::Setup(0), 5.0), vec![]);
    }

    #[test]
    fn calls_link_lost_ends_the_call() {
        let mut m = connected();
        m.apply(Ev::Call(true), 0.0);
        m.apply(Ev::Sco(true), 0.0);
        m.apply(Ev::Muted(true), 0.0);
        let out = m.apply(Ev::Connected(false), 1.0);
        assert_eq!(out, vec![Out::Ended, Out::State]);
        assert!(!m.muted && !m.sco && !m.call);
        assert_eq!(m.audio(), AudioRoute::None);
        // Indicators while not connected show no call.
        assert!(m.apply(Ev::Call(true), 2.0).is_empty());
        assert_eq!(m.phase(), Phase::Idle);
    }

    #[test]
    fn calls_audio_transfer() {
        let mut m = connected();
        assert_eq!(m.audio(), AudioRoute::None);
        m.apply(Ev::Call(true), 0.0);
        // Answered on the phone: audio there.
        assert_eq!(m.audio(), AudioRoute::Phone);
        assert_eq!(m.apply(Ev::Sco(true), 1.0), vec![Out::State]);
        assert_eq!(m.audio(), AudioRoute::Mac);
        // The phone took it back (or "Use phone for audio").
        assert_eq!(m.apply(Ev::Sco(false), 2.0), vec![Out::State]);
        assert_eq!(m.audio(), AudioRoute::Phone);
        // The call's still on and still controllable.
        assert_eq!(m.phase(), Phase::Active);
        // SCO without a call (in-band ring tone) isn't call audio.
        let mut m = connected();
        assert!(m.apply(Ev::Sco(true), 0.0).is_empty());
        assert_eq!(m.audio(), AudioRoute::None);
    }

    #[test]
    fn calls_indicator_state() {
        let mut m = connected();
        assert_eq!(m.apply(Ev::Signal(9), 0.0), vec![Out::State]);
        assert_eq!(m.signal, Some(5));
        assert!(m.apply(Ev::Signal(5), 0.0).is_empty());
        assert_eq!(m.apply(Ev::Battery(2), 0.0), vec![Out::State]);
        m.apply(Ev::Call(true), 0.0);
        assert_eq!(m.apply(Ev::Held(1), 0.0), vec![Out::State]);
        assert_eq!(m.apply(Ev::Muted(true), 0.0), vec![Out::State]);
    }

    #[test]
    fn calls_tray_entries() {
        let mut m = connected();
        assert!(tray_entries(&m).is_empty());
        m.apply(Ev::Incoming("555".into()), 0.0);
        let ids: Vec<_> = tray_entries(&m).into_iter().map(|e| e.0).collect();
        assert_eq!(ids, ["answer", "decline"]);
        assert_eq!(tray_entries(&m)[0].1, "Answer call from 555");
        m.apply(Ev::Call(true), 0.0);
        let ids: Vec<_> = tray_entries(&m).into_iter().map(|e| e.0).collect();
        assert_eq!(ids, ["hangup", "tomac"]);
        m.apply(Ev::Sco(true), 0.0);
        let ids: Vec<_> = tray_entries(&m).into_iter().map(|e| e.0).collect();
        assert_eq!(ids, ["hangup", "tophone"]);
    }

    #[test]
    fn calls_call_list_status() {
        assert_eq!(call_status_phase(0), Phase::Active);
        assert_eq!(call_status_phase(1), Phase::Active);
        assert_eq!(call_status_phase(2), Phase::Outgoing);
        assert_eq!(call_status_phase(3), Phase::Outgoing);
        assert_eq!(call_status_phase(4), Phase::Incoming);
        assert_eq!(call_status_phase(5), Phase::Incoming);
        assert_eq!(call_status_phase(9), Phase::Idle);
    }

    #[test]
    fn calls_sanitize_number() {
        assert_eq!(
            sanitize_number(" +91 (80) 1234-5678 ").unwrap(),
            "+918012345678"
        );
        assert_eq!(sanitize_number("*#06#").unwrap(), "*#06#");
        assert_eq!(sanitize_number("1.800.555/0199").unwrap(), "18005550199");
        assert!(sanitize_number("").is_err());
        assert!(sanitize_number("+").is_err());
        assert!(sanitize_number(" - ").is_err());
        assert!(sanitize_number("12+3").is_err());
        assert!(sanitize_number("1800-FLOWERS").is_err());
        assert!(sanitize_number("123;ATD").is_err());
        assert!(sanitize_number(&"1".repeat(MAX_NUMBER + 1)).is_err());
        assert!(sanitize_number(&"1".repeat(MAX_NUMBER)).is_ok());
    }

    #[test]
    fn calls_dtmf_digit() {
        assert_eq!(dtmf_digit("5"), Some('5'));
        assert_eq!(dtmf_digit(" # "), Some('#'));
        assert_eq!(dtmf_digit("*"), Some('*'));
        assert_eq!(dtmf_digit("a"), Some('A'));
        assert_eq!(dtmf_digit("D"), Some('D'));
        assert_eq!(dtmf_digit("E"), None);
        assert_eq!(dtmf_digit("12"), None);
        assert_eq!(dtmf_digit(""), None);
        assert_eq!(dtmf_digit("+"), None);
    }

    #[test]
    fn calls_resampler() {
        // Same rate: the input, one sample behind (linear interpolation).
        let mut r = Resampler::new(8000, 8000);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0, 3.0], &mut out);
        assert_eq!(out, vec![0.0, 1.0, 2.0]);
        // Up 8k -> 48k: six outputs per input.
        let mut r = Resampler::new(8000, 48000);
        let mut out = Vec::new();
        r.process(&[0.0; 100], &mut out);
        assert_eq!(out.len(), 600);
        // Down 48k -> 16k: one per three, across calls too.
        let mut r = Resampler::new(48000, 16000);
        let mut out = Vec::new();
        r.process(&[0.5; 10], &mut out);
        r.process(&[0.5; 20], &mut out);
        assert_eq!(out.len(), 10);
        // Interpolates between samples.
        let mut r = Resampler::new(8000, 16000);
        let mut out = Vec::new();
        r.process(&[0.0, 1.0], &mut out);
        assert_eq!(out, vec![0.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn calls_downmix_and_device_match() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[0.25, 0.75], 1), vec![0.25, 0.75]);
        assert!(is_phone_device("Pixel 8", "pixel 8"));
        assert!(is_phone_device("Pixel 8 Hands-Free", "Pixel 8"));
        assert!(!is_phone_device("MacBook Pro Speakers", "Pixel 8"));
        assert!(!is_phone_device("anything", " "));
        assert!(is_builtin("MacBook Pro Speakers"));
        assert!(is_builtin("MacBook Air Microphone"));
        assert!(is_builtin("Built-in Output"));
        assert!(!is_builtin("AirPods Pro"));
    }

    #[test]
    fn calls_pick_device_and_fallback() {
        let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let devs = list(&[
            "MacBook Pro Speakers",
            "AirPods Pro",
            "LG Display",
            "Pixel 8",
        ]);
        let phone = "Pixel 8";
        // The chosen one while it's there.
        assert_eq!(
            pick_device(Some("AirPods Pro"), &devs, Some("LG Display"), phone).as_deref(),
            Some("AirPods Pro")
        );
        // None chosen: the system default.
        assert_eq!(
            pick_device(None, &devs, Some("LG Display"), phone).as_deref(),
            Some("LG Display")
        );
        // The chosen one went (AirPods out of range): the built-in one.
        let gone = list(&["MacBook Pro Speakers", "LG Display", "Pixel 8"]);
        assert_eq!(
            pick_device(Some("AirPods Pro"), &gone, Some("LG Display"), phone).as_deref(),
            Some("MacBook Pro Speakers")
        );
        // No built-in one (a Mac mini with no speaker): the default.
        let no_builtin = list(&["LG Display", "Pixel 8"]);
        assert_eq!(
            pick_device(Some("AirPods Pro"), &no_builtin, Some("LG Display"), phone).as_deref(),
            Some("LG Display")
        );
        // The default is the phone's own device: never loop the call back.
        assert_eq!(
            pick_device(None, &devs, Some("Pixel 8"), phone).as_deref(),
            Some("MacBook Pro Speakers")
        );
        assert_eq!(
            pick_device(Some("Pixel 8"), &devs, None, phone).as_deref(),
            Some("MacBook Pro Speakers")
        );
        // Nothing else at all.
        assert_eq!(pick_device(None, &list(&["Pixel 8"]), None, phone), None);
        // Neither built-in nor default usable: anything that is.
        assert_eq!(
            pick_device(
                Some("Gone"),
                &list(&["USB Headset", "Pixel 8"]),
                None,
                phone
            )
            .as_deref(),
            Some("USB Headset")
        );
    }
}
