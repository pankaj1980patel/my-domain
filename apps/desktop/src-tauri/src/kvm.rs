// Share mouse and keyboard between your computers (mdcore `features::kvm`),
// like Barrier or Synergy:
//
//  - Settings → "Share mouse and keyboard": drag your other computers to
//    the left, right, top or bottom of this one. The core keeps the layout
//    and tells the other computer where this one sits, so both agree;
//  - while the layout has a computer at a side, a thread looks at the
//    pointer about 100 times a second (`POLL_EVERY`). When it reaches that
//    outer edge of this computer's screens (not a corner, and not an edge
//    another screen of this computer continues past), this computer starts
//    controlling the other one: its own mouse and keyboard are captured
//    (`kvm_os`: a CGEventTap on a Mac, low-level hooks on Windows, X11
//    grabs on Linux; the pointer stays parked at the edge) and go to the
//    other computer, which plays them back (enigo) from the matching spot
//    on its facing edge;
//  - pushing the pointer back out across that edge over there brings
//    control back here, at the matching spot; so do the hotkey
//    (Ctrl+Alt+Esc by default, remembered), "Take back" on the controlled
//    computer, the link dropping, or either end going quiet: the core
//    always ends in "idle", and every "idle" lets go of everything here
//    (the capture, or every key and button held on this computer);
//  - while another computer controls this one, a small bar at the top of
//    the screen and the tray say so; "Let other computers control this
//    one" in Settings turns being controlled off;
//  - Cmd ↔ Ctrl between a Mac and the others (`kvm_keys`), and the
//    clipboard follows the active screen (the core; a password manager's
//    concealed copy never does).
//
// macOS: capturing and playing input both need my-domain allowed under
// Privacy & Security > Accessibility; until then one notice says so (once
// per run). Nothing ever asks macOS to grant it. Linux: X11 only (on
// Wayland there's no way for an app to capture or move the pointer), so
// the feature says it's unavailable there.
//
// The parts that decide (geometry, the capture's states, keys) are plain
// functions, tested here; the OS hooks (`kvm_os`) stay thin.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use mdcore::features::kvm::{Side, POS_SCALE};
use mdcore::kv::FileKv;
use mdcore::model::{KvmEvent, KvmNeighbor, KvmState};
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State, WebviewUrl};

use crate::kvm_keys::{self, Hotkey, KeyAction, Keys};
use crate::kvm_os::{self, Captured, Rect};
use crate::Eng;

/// How often the pointer is looked at for an edge.
const POLL_EVERY: Duration = Duration::from_millis(10);
/// With nobody placed around this computer, how often to look again.
const IDLE_POLL: Duration = Duration::from_millis(1000);
/// The screens are read again this often.
const SCREENS_EVERY: Duration = Duration::from_secs(2);
/// After a failed start or a return, the edge does nothing this long.
const COOLDOWN: Duration = Duration::from_millis(400);
const FAILED_COOLDOWN: Duration = Duration::from_secs(2);
/// Within this many pixels of a screen's border counts as at it.
const EDGE_PX: f64 = 1.0;
/// This close to a screen's corner doesn't count (hot corners).
const CORNER_PX: f64 = 8.0;
/// Where the pointer comes in: this far inside the edge.
const INSET_PX: f64 = 3.0;

const ENABLED_KEY: &str = "kvm_enabled";
const HOTKEY_KEY: &str = "kvm_hotkey";
const SWAP_KEY: &str = "kvm_swap_mods";

pub const OVERLAY: &str = "kvm-overlay";

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

fn bounds(screens: &[Rect]) -> Option<Rect> {
    let first = screens.first()?;
    let (mut l, mut t, mut r, mut b) = (first.x, first.y, first.x + first.w, first.y + first.h);
    for s in &screens[1..] {
        l = l.min(s.x);
        t = t.min(s.y);
        r = r.max(s.x + s.w);
        b = b.max(s.y + s.h);
    }
    Some(Rect {
        x: l,
        y: t,
        w: r - l,
        h: b - t,
    })
}

fn inside(screens: &[Rect], x: f64, y: f64) -> bool {
    screens.iter().any(|s| s.contains(x, y))
}

/// Where along the whole desktop's `side` a point is, 0..POS_SCALE.
fn along(b: &Rect, side: Side, x: f64, y: f64) -> u16 {
    let f = match side {
        Side::Left | Side::Right => (y - b.y) / (b.h - 1.0).max(1.0),
        Side::Top | Side::Bottom => (x - b.x) / (b.w - 1.0).max(1.0),
    };
    (f.clamp(0.0, 1.0) * f64::from(POS_SCALE)).round() as u16
}

/// The pointer at (`x`, `y`) is at an outer edge of the screens: one no
/// screen of this computer continues past. Which, and how far along the
/// whole desktop's side. `corners`: a spot near a screen's corner counts
/// too (it doesn't when starting: hot corners live there).
pub fn edge_at(screens: &[Rect], x: f64, y: f64, corners: bool) -> Option<(Side, u16)> {
    let s = screens.iter().find(|s| s.contains(x, y))?;
    let b = bounds(screens)?;
    let (l, t, r, btm) = (s.x, s.y, s.x + s.w - 1.0, s.y + s.h - 1.0);
    let near_end =
        |v: f64, lo: f64, hi: f64| !corners && (v - lo < CORNER_PX || hi - v < CORNER_PX);
    let tries = [
        (
            Side::Left,
            x - l <= EDGE_PX,
            (l - 2.0, y),
            near_end(y, t, btm),
        ),
        (
            Side::Right,
            r - x <= EDGE_PX,
            (r + 2.0, y),
            near_end(y, t, btm),
        ),
        (Side::Top, y - t <= EDGE_PX, (x, t - 2.0), near_end(x, l, r)),
        (
            Side::Bottom,
            btm - y <= EDGE_PX,
            (x, btm + 2.0),
            near_end(x, l, r),
        ),
    ];
    tries
        .into_iter()
        .find(|(_, at, beyond, corner)| *at && !*corner && !inside(screens, beyond.0, beyond.1))
        .map(|(side, ..)| (side, along(&b, side, x, y)))
}

/// The pointer reached an edge just now: it wasn't at that edge at the
/// last look. One left resting at an edge (after coming back, or the
/// hotkey) has to leave it and come back to go over again.
pub fn arrived(before: Option<Side>, now: Option<(Side, u16)>) -> Option<(Side, u16)> {
    now.filter(|(side, _)| before != Some(*side))
}

/// Where to put a pointer resting at an edge when control comes back
/// without crossing it (the hotkey, Stop, a lost link): a little inside.
pub fn off_the_edge(screens: &[Rect], x: f64, y: f64) -> Option<(f64, f64)> {
    const AWAY: f64 = 40.0;
    let (side, _) = edge_at(screens, x, y, true)?;
    Some(match side {
        Side::Left => (x + AWAY, y),
        Side::Right => (x - AWAY, y),
        Side::Top => (x, y + AWAY),
        Side::Bottom => (x, y - AWAY),
    })
}

/// Where the pointer comes in at `side` of these screens, `pos` along the
/// whole desktop's side: just inside the outermost screen there (the
/// nearest one if no screen spans that spot).
pub fn entry_point(screens: &[Rect], side: Side, pos: u16) -> Option<(f64, f64)> {
    let b = bounds(screens)?;
    let f = f64::from(pos.min(POS_SCALE)) / f64::from(POS_SCALE);
    let vertical = matches!(side, Side::Left | Side::Right);
    let want = if vertical {
        b.y + f * (b.h - 1.0)
    } else {
        b.x + f * (b.w - 1.0)
    };
    let span = |s: &Rect| {
        if vertical {
            (s.y, s.y + s.h - 1.0)
        } else {
            (s.x, s.x + s.w - 1.0)
        }
    };
    let dist = |s: &Rect| {
        let (lo, hi) = span(s);
        if want < lo {
            lo - want
        } else if want > hi {
            want - hi
        } else {
            0.0
        }
    };
    // The outermost screen at that side wins among those nearest the spot.
    let outer = |s: &Rect| match side {
        Side::Left => s.x,
        Side::Right => -(s.x + s.w),
        Side::Top => s.y,
        Side::Bottom => -(s.y + s.h),
    };
    let s = screens.iter().min_by(|a, c| {
        dist(a)
            .total_cmp(&dist(c))
            .then(outer(a).total_cmp(&outer(c)))
    })?;
    let (lo, hi) = span(s);
    let a = want.clamp(lo, hi);
    Some(match side {
        Side::Left => (s.x + INSET_PX, a),
        Side::Right => (s.x + s.w - 1.0 - INSET_PX, a),
        Side::Top => (a, s.y + INSET_PX),
        Side::Bottom => (a, s.y + s.h - 1.0 - INSET_PX),
    })
}

/// While controlled: the pointer at (`x`, `y`), just moved by (`dx`,
/// `dy`), was pushed out across `edge` (the one it came in at): how far
/// along. Moving along the edge, or away from it, isn't.
pub fn pushed_out(screens: &[Rect], x: f64, y: f64, dx: i32, dy: i32, edge: Side) -> Option<u16> {
    let outward = match edge {
        Side::Left => dx < 0,
        Side::Right => dx > 0,
        Side::Top => dy < 0,
        Side::Bottom => dy > 0,
    };
    if !outward {
        return None;
    }
    match edge_at(screens, x, y, true) {
        Some((side, pos)) if side == edge => Some(pos),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The capture's states
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// Asked the core to start controlling `node`.
    Starting {
        node: String,
    },
    /// This computer's mouse and keyboard drive `node`.
    Capturing {
        node: String,
    },
    /// `node`'s drive this computer.
    Controlled {
        node: String,
    },
}

#[derive(Debug)]
pub enum Input<'a> {
    /// The pointer reached `edge` with `node` placed there.
    EdgeHit {
        node: &'a str,
        edge: Side,
        pos: u16,
    },
    StartFailed,
    /// The core's state changed.
    Core(&'a KvmState),
    Hotkey,
    CaptureFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    StartCore {
        node: String,
        edge: Side,
        pos: u16,
    },
    BeginCapture {
        node: String,
    },
    /// Let go of this computer's input; put the pointer back at that spot
    /// of that edge if it came back across it.
    EndCapture {
        back: Option<(Side, u16)>,
    },
    StopCore,
    /// Controlled from now: the pointer comes in there.
    EnterHere {
        edge: Side,
        pos: u16,
    },
    /// No longer controlled: let go of every key and button held here.
    ReleaseHere,
}

/// What happens next, from where the capture is and what just happened.
/// Every way out of "capturing" ends the capture; every way out of
/// "controlled" lets go of what's held.
pub fn step(phase: &Phase, input: Input) -> (Phase, Vec<Action>) {
    use Action::*;
    match (phase, input) {
        (Phase::Idle, Input::EdgeHit { node, edge, pos }) => (
            Phase::Starting {
                node: node.to_string(),
            },
            vec![StartCore {
                node: node.to_string(),
                edge,
                pos,
            }],
        ),
        (Phase::Starting { .. }, Input::StartFailed) => (Phase::Idle, vec![]),
        (_, Input::Core(s)) => from_core(phase, s),
        (Phase::Capturing { .. }, Input::Hotkey | Input::CaptureFailed) => {
            (Phase::Idle, vec![EndCapture { back: None }, StopCore])
        }
        // Anything else changes nothing (an edge while controlled, the
        // hotkey with nothing captured).
        (p, _) => (p.clone(), vec![]),
    }
}

fn from_core(phase: &Phase, s: &KvmState) -> (Phase, Vec<Action>) {
    use Action::*;
    let node = s.node_id.clone().unwrap_or_default();
    let edge = s.edge.as_deref().and_then(Side::parse);
    match s.role.as_str() {
        "controlling" => match phase {
            Phase::Capturing { node: n } if *n == node => (phase.clone(), vec![]),
            Phase::Capturing { .. } => (
                Phase::Capturing { node: node.clone() },
                vec![EndCapture { back: None }, BeginCapture { node }],
            ),
            Phase::Controlled { .. } => (
                Phase::Capturing { node: node.clone() },
                vec![ReleaseHere, BeginCapture { node }],
            ),
            _ => (
                Phase::Capturing { node: node.clone() },
                vec![BeginCapture { node }],
            ),
        },
        "controlled" => {
            let enter = EnterHere {
                edge: edge.unwrap_or(Side::Left),
                pos: s.pos.unwrap_or(POS_SCALE / 2),
            };
            match phase {
                Phase::Controlled { node: n } if *n == node => (phase.clone(), vec![]),
                Phase::Capturing { .. } => (
                    Phase::Controlled { node },
                    vec![EndCapture { back: None }, enter],
                ),
                Phase::Controlled { .. } => (Phase::Controlled { node }, vec![ReleaseHere, enter]),
                _ => (Phase::Controlled { node }, vec![enter]),
            }
        }
        _ => match phase {
            Phase::Capturing { .. } => {
                let back = match (s.reason.as_deref(), edge, s.pos) {
                    (Some("returned"), Some(e), Some(p)) => Some((e, p)),
                    _ => None,
                };
                (Phase::Idle, vec![EndCapture { back }])
            }
            Phase::Controlled { .. } => (Phase::Idle, vec![ReleaseHere]),
            _ => (Phase::Idle, vec![]),
        },
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Prefs {
    enabled: bool,
    hotkey: Hotkey,
    swap: bool,
}

fn load_prefs(kv: Option<&FileKv>) -> Prefs {
    let get = |k: &str| kv.and_then(|kv| kv.get(k));
    Prefs {
        enabled: get(ENABLED_KEY).as_deref() != Some("off"),
        hotkey: get(HOTKEY_KEY)
            .and_then(|h| Hotkey::parse(&h))
            .unwrap_or(Hotkey::CtrlAltEsc),
        swap: get(SWAP_KEY).as_deref() != Some("off"),
    }
}

/// The Settings section.
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct KvmSettings {
    /// This computer can share its mouse and keyboard at all.
    pub supported: bool,
    /// Why not, or why it can't capture or play input yet: "wayland",
    /// "display" (no X server), "accessibility" (a Mac not allowed yet).
    pub blocked: Option<String>,
    /// Share this computer's mouse and keyboard with the computers placed
    /// around it.
    pub enabled: bool,
    /// Let your other computers control this one.
    pub allowed: bool,
    pub clip_follows: bool,
    pub swap_mods: bool,
    /// "ctrl+alt+esc", "ctrl+alt+f12" or "scroll_lock".
    pub hotkey: String,
    pub layout: Vec<KvmNeighbor>,
    /// Your other computers that can take part.
    pub computers: Vec<KvmComputer>,
    pub state: KvmState,
    /// "macos", "windows" or "linux".
    pub os: String,
}

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct KvmComputer {
    pub node_id: String,
    pub name: String,
    pub os: String,
    pub linked: bool,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// What the controlled computer's player is told.
enum Play {
    Enter { edge: Side, pos: u16 },
    Events(Vec<KvmEvent>),
    Release,
}

struct Rt {
    app: AppHandle,
    kv: Option<FileKv>,
    prefs: Mutex<Prefs>,
    phase: Mutex<Phase>,
    capture: Mutex<Option<kvm_os::Capture>>,
    cooldown: Mutex<Instant>,
    player: Mutex<Sender<Play>>,
}

static RT: OnceLock<Rt> = OnceLock::new();

fn rt() -> Option<&'static Rt> {
    RT.get()
}

/// Read the settings and start the edge thread and the player.
pub fn init(app: &AppHandle) {
    let kv = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")));
    let prefs = load_prefs(kv.as_ref());
    let (play_tx, play_rx) = mpsc::channel::<Play>();
    let fresh = RT
        .set(Rt {
            app: app.clone(),
            kv,
            prefs: Mutex::new(prefs),
            phase: Mutex::new(Phase::Idle),
            capture: Mutex::new(None),
            cooldown: Mutex::new(Instant::now()),
            player: Mutex::new(play_tx),
        })
        .is_ok();
    if !fresh {
        return;
    }
    let a = app.clone();
    let _ = std::thread::Builder::new()
        .name("kvm-player".into())
        .spawn(move || player(&a, play_rx));
    let a = app.clone();
    let _ = std::thread::Builder::new()
        .name("kvm-edges".into())
        .spawn(move || edges(&a));
}

/// Why this computer can't share its mouse and keyboard now, if it can't.
fn blocked() -> Option<&'static str> {
    if let Some(why) = kvm_os::unsupported() {
        return Some(why);
    }
    crate::remote_input::ready().err()
}

fn engine(app: &AppHandle) -> Option<State<'_, Eng>> {
    app.try_state::<Eng>()
}

fn feed(input: Input) {
    let Some(rt) = rt() else {
        return;
    };
    let actions = {
        let Ok(mut phase) = rt.phase.lock() else {
            return;
        };
        let (next, actions) = step(&phase, input);
        *phase = next;
        actions
    };
    // Never under the lock: the core answers with events that come back
    // here.
    for a in actions {
        run(rt, a);
    }
}

fn set_cooldown(rt: &Rt, d: Duration) {
    if let Ok(mut c) = rt.cooldown.lock() {
        *c = Instant::now() + d;
    }
}

fn run(rt: &Rt, action: Action) {
    let app = &rt.app;
    match action {
        Action::StartCore { node, edge, pos } => {
            let started = engine(app).map(|e| e.kvm_start(&node, edge.as_str(), pos));
            if !matches!(started, Some(Ok(()))) {
                if let Some(Err(e)) = started {
                    eprintln!("[kvm] couldn't start: {e}");
                }
                set_cooldown(rt, FAILED_COOLDOWN);
                feed(Input::StartFailed);
            }
        }
        Action::BeginCapture { node } => {
            let to_mac = engine(app).is_some_and(|e| e.kvm_peer_os(&node) == "macos");
            let (tx, rx) = mpsc::channel::<Captured>();
            let a = app.clone();
            let _ = std::thread::Builder::new()
                .name("kvm-capture".into())
                .spawn(move || forward(&a, rx, to_mac));
            match kvm_os::start_capture(tx) {
                Ok(c) => {
                    if let Ok(mut slot) = rt.capture.lock() {
                        *slot = Some(c);
                    }
                    crate::tray::set_kvm(app, Some(format!("Controlling {}", name(app, &node))));
                }
                Err(e) => {
                    eprintln!("[kvm] couldn't capture this computer's input: {e}");
                    tell_once(app, e);
                    feed(Input::CaptureFailed);
                }
            }
        }
        Action::EndCapture { back } => {
            // Dropping the capture lets go of the input (and the pointer).
            let cap = rt.capture.lock().ok().and_then(|mut c| c.take());
            drop(cap);
            let screens = kvm_os::screens(app);
            let to = match back {
                Some((edge, pos)) => entry_point(&screens, edge, pos),
                None => kvm_os::cursor().and_then(|(x, y)| off_the_edge(&screens, x, y)),
            };
            if let Some((x, y)) = to {
                kvm_os::warp(x, y);
            }
            set_cooldown(rt, COOLDOWN);
            crate::tray::set_kvm(app, None);
        }
        Action::StopCore => {
            if let Some(e) = engine(app) {
                e.kvm_stop("stopped");
            }
        }
        Action::EnterHere { edge, pos } => {
            play(Play::Enter { edge, pos });
            let who = rt
                .phase
                .lock()
                .ok()
                .and_then(|p| match &*p {
                    Phase::Controlled { node } => Some(node.clone()),
                    _ => None,
                })
                .map(|n| name(app, &n))
                .unwrap_or_default();
            crate::tray::set_kvm(app, Some(format!("{who} is controlling this computer")));
            show_overlay(app);
        }
        Action::ReleaseHere => {
            play(Play::Release);
            set_cooldown(rt, COOLDOWN);
            crate::tray::set_kvm(app, None);
            hide_overlay(app);
        }
    }
}

fn name(app: &AppHandle, node: &str) -> String {
    crate::notify::device_name(app, node)
}

fn play(p: Play) {
    if let Some(rt) = rt() {
        if let Ok(tx) = rt.player.lock() {
            let _ = tx.send(p);
        }
    }
}

/// The core said who this computer's mouse and keyboard are with.
pub fn on_changed(_app: &AppHandle, state: &KvmState) {
    feed(Input::Core(state));
}

/// Input from the computer controlling this one.
pub fn on_input(events: Vec<KvmEvent>) {
    play(Play::Events(events));
}

/// The edge thread: looks for the pointer at an edge with a computer there.
fn edges(app: &AppHandle) {
    let mut screens: Vec<Rect> = Vec::new();
    let mut read_at = Instant::now() - SCREENS_EVERY;
    let mut layout: Vec<KvmNeighbor> = Vec::new();
    let mut layout_at = Instant::now() - SCREENS_EVERY;
    let mut before: Option<Side> = None;
    loop {
        let Some(rt) = rt() else {
            std::thread::sleep(IDLE_POLL);
            continue;
        };
        let enabled = rt.prefs.lock().map(|p| p.enabled).unwrap_or(false);
        let idle = rt.phase.lock().map(|p| *p == Phase::Idle).unwrap_or(false);
        if layout_at.elapsed() >= Duration::from_millis(500) {
            layout = engine(app).map(|e| e.kvm_layout()).unwrap_or_default();
            layout_at = Instant::now();
        }
        if !enabled || layout.is_empty() || kvm_os::unsupported().is_some() {
            std::thread::sleep(IDLE_POLL);
            continue;
        }
        std::thread::sleep(POLL_EVERY);
        let cooling = rt
            .cooldown
            .lock()
            .map(|c| Instant::now() < *c)
            .unwrap_or(true);
        if read_at.elapsed() >= SCREENS_EVERY {
            screens = kvm_os::screens(app);
            read_at = Instant::now();
        }
        let hit = kvm_os::cursor().and_then(|(x, y)| edge_at(&screens, x, y, false));
        let fresh = arrived(before, hit);
        before = hit.map(|(side, _)| side);
        if !idle || cooling {
            continue;
        }
        let Some((edge, pos)) = fresh else {
            continue;
        };
        let Some(n) = layout.iter().find(|n| n.side == edge.as_str()) else {
            continue;
        };
        if let Err(why) = crate::remote_input::ready() {
            tell_once(app, why);
            set_cooldown(rt, FAILED_COOLDOWN);
            continue;
        }
        let node = n.node_id.clone();
        feed(Input::EdgeHit {
            node: &node,
            edge,
            pos,
        });
    }
}

/// The capture's worker: what the hooks caught goes to the core (fast,
/// off the hooks' own thread).
fn forward(app: &AppHandle, rx: Receiver<Captured>, to_mac: bool) {
    let mut keys = Keys::default();
    let from_mac = cfg!(target_os = "macos");
    for c in rx {
        let Some(rt) = rt() else {
            return;
        };
        let capturing = rt
            .phase
            .lock()
            .is_ok_and(|p| matches!(*p, Phase::Capturing { .. }));
        if !capturing {
            // The capture ended: nothing more goes.
            return;
        }
        let Some(e) = engine(app) else {
            return;
        };
        let prefs = rt.prefs.lock().map(|p| *p).ok();
        let (hotkey, swap) = prefs.map_or((Hotkey::CtrlAltEsc, true), |p| (p.hotkey, p.swap));
        let ev = match c {
            Captured::Motion { dx, dy } => {
                e.kvm_motion(dx, dy);
                continue;
            }
            Captured::Button { button, down } => KvmEvent::Button { button, down },
            Captured::Wheel { dx, dy } => KvmEvent::Wheel { dx, dy },
            Captured::Key { hid, down } => match keys.key(hid, down, hotkey) {
                KeyAction::Skip => continue,
                KeyAction::Hotkey => {
                    feed(Input::Hotkey);
                    return;
                }
                KeyAction::Send => KvmEvent::Key {
                    code: kvm_keys::map_for(hid, from_mac, to_mac, swap),
                    down,
                },
            },
        };
        let _ = e.kvm_send(vec![ev]);
    }
}

/// The player: plays the controlling computer's input here, in order, and
/// gives control back when the pointer is pushed out the way it came in.
fn player(app: &AppHandle, rx: Receiver<Play>) {
    let mut enigo: Option<Enigo> = None;
    let mut keys: Vec<u16> = Vec::new();
    let mut buttons: Vec<u8> = Vec::new();
    let mut edge: Option<Side> = None;
    let mut screens: Vec<Rect> = Vec::new();
    for p in rx {
        match p {
            Play::Enter { edge: e, pos } => {
                edge = Some(e);
                screens = kvm_os::screens(app);
                if let Some((x, y)) = entry_point(&screens, e, pos) {
                    kvm_os::warp(x, y);
                }
            }
            Play::Release => {
                edge = None;
                if let Some(en) = enigo.as_mut() {
                    release_all(en, &mut keys, &mut buttons);
                }
            }
            Play::Events(events) => {
                let Some(e) = edge else {
                    continue;
                };
                if enigo.is_none() {
                    if let Err(why) = crate::remote_input::ready() {
                        tell_once(app, why);
                        continue;
                    }
                    let settings = Settings {
                        open_prompt_to_get_permissions: false,
                        ..Settings::default()
                    };
                    match Enigo::new(&settings) {
                        Ok(en) => enigo = Some(en),
                        Err(err) => {
                            eprintln!("[kvm] can't control this computer: {err}");
                            tell_once(app, "enigo");
                            continue;
                        }
                    }
                }
                let Some(en) = enigo.as_mut() else {
                    continue;
                };
                for ev in events {
                    if let KvmEvent::Move { dx, dy } = ev {
                        let _ = en.move_mouse(dx, dy, Coordinate::Rel);
                        let Some((x, y)) = kvm_os::cursor() else {
                            continue;
                        };
                        if let Some(pos) = pushed_out(&screens, x, y, dx, dy, e) {
                            let gave_back =
                                engine(app).is_some_and(|eng| eng.kvm_leave(e.as_str(), pos));
                            if gave_back {
                                // The core says "idle" too; let go now.
                                edge = None;
                                release_all(en, &mut keys, &mut buttons);
                                break;
                            }
                        }
                    } else if let Err(err) = play_one(en, ev, &mut keys, &mut buttons) {
                        eprintln!("[kvm] couldn't play an event: {err}");
                    }
                }
            }
        }
    }
}

fn enigo_button(b: u8) -> Option<Button> {
    match b {
        1 => Some(Button::Left),
        2 => Some(Button::Right),
        3 => Some(Button::Middle),
        4 => Some(Button::Back),
        5 => Some(Button::Forward),
        _ => None,
    }
}

/// Press or release this OS's key for a HID usage.
fn press(en: &mut Enigo, hid: u16, dir: Direction) -> Result<(), enigo::InputError> {
    #[cfg(target_os = "macos")]
    if let Some(code) = kvm_keys::to_mac(hid) {
        return en.raw(code, dir);
    }
    #[cfg(target_os = "windows")]
    if let Some(vk) = kvm_keys::to_win(hid) {
        return en.key(enigo::Key::Other(u32::from(vk)), dir);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(code) = kvm_keys::to_evdev(hid) {
        return en.raw(code + 8, dir);
    }
    let _ = (en, hid, dir);
    Ok(())
}

fn play_one(
    en: &mut Enigo,
    ev: KvmEvent,
    keys: &mut Vec<u16>,
    buttons: &mut Vec<u8>,
) -> Result<(), enigo::InputError> {
    match ev {
        KvmEvent::Move { dx, dy } => en.move_mouse(dx, dy, Coordinate::Rel),
        KvmEvent::Button { button, down } => {
            let Some(b) = enigo_button(button) else {
                return Ok(());
            };
            if down {
                buttons.push(button);
                en.button(b, Direction::Press)
            } else {
                buttons.retain(|x| *x != button);
                en.button(b, Direction::Release)
            }
        }
        KvmEvent::Wheel { dx, dy } => {
            if dy != 0 {
                en.scroll(dy, Axis::Vertical)?;
            }
            if dx != 0 {
                en.scroll(dx, Axis::Horizontal)?;
            }
            Ok(())
        }
        KvmEvent::Key { code, down } => {
            if down {
                if !keys.contains(&code) {
                    keys.push(code);
                }
                press(en, code, Direction::Press)
            } else {
                keys.retain(|k| *k != code);
                press(en, code, Direction::Release)
            }
        }
    }
}

/// Let go of every key and button still held here (the session ended).
fn release_all(en: &mut Enigo, keys: &mut Vec<u16>, buttons: &mut Vec<u8>) {
    for k in keys.drain(..).rev() {
        let _ = press(en, k, Direction::Release);
    }
    for b in buttons.drain(..) {
        if let Some(b) = enigo_button(b) {
            let _ = en.button(b, Direction::Release);
        }
    }
}

/// The small bar while another computer controls this one.
fn show_overlay(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        let w = match a.get_webview_window(OVERLAY) {
            Some(w) => w,
            None => match tauri::WebviewWindowBuilder::new(&a, OVERLAY, WebviewUrl::default())
                .title("my-domain")
                .inner_size(420.0, 52.0)
                .resizable(false)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible_on_all_workspaces(true)
                .focused(false)
                .shadow(true)
                .visible(false)
                .build()
            {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[kvm] couldn't show the bar: {e}");
                    return;
                }
            },
        };
        if let Ok(Some(m)) = a.primary_monitor() {
            let scale = m.scale_factor();
            let wa = m.work_area();
            let width = 420.0 * scale;
            let x = f64::from(wa.position.x) + (f64::from(wa.size.width) - width) / 2.0;
            let y = f64::from(wa.position.y) + 12.0 * scale;
            let _ = w.set_position(tauri::PhysicalPosition::new(x.round(), y.round()));
        }
        let _ = w.show();
    });
}

fn hide_overlay(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window(OVERLAY) {
            let _ = w.hide();
        }
    });
}

/// Tell the user, once per run, why sharing the mouse and keyboard does
/// nothing. Never asks macOS for the permission itself.
fn tell_once(app: &AppHandle, why: &str) {
    static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TOLD.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let body = match why {
        "accessibility" => {
            "To share your mouse and keyboard with your other computers, allow my-domain in \
             System Settings > Privacy & Security > Accessibility, then try again."
        }
        "display" | "wayland" => {
            "Sharing your mouse and keyboard needs an X11 session on this computer (Wayland isn't supported)."
        }
        _ => "my-domain couldn't use this computer's mouse and keyboard.",
    };
    crate::notify::post_always(app, "Share mouse and keyboard", body);
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn save(key: &str, value: &str) {
    if let Some(kv) = rt().and_then(|r| r.kv.as_ref()) {
        if let Err(e) = kv.set(key, value) {
            eprintln!("[kvm] couldn't save the setting: {e}");
        }
    }
}

fn update(f: impl FnOnce(&mut Prefs)) {
    if let Some(rt) = rt() {
        if let Ok(mut p) = rt.prefs.lock() {
            f(&mut p);
        }
    }
}

fn settings(engine: &Eng) -> KvmSettings {
    let prefs = rt()
        .and_then(|r| r.prefs.lock().ok().map(|p| *p))
        .unwrap_or_else(|| load_prefs(None));
    let linked = engine.connected_peers();
    let computers = engine
        .get_peers()
        .into_iter()
        .filter(|p| engine.kvm_supported(&p.node_id))
        .map(|p| KvmComputer {
            os: engine.kvm_peer_os(&p.node_id),
            linked: linked.contains(&p.node_id),
            node_id: p.node_id,
            name: p.name,
        })
        .collect();
    KvmSettings {
        supported: engine.kvm_available() && kvm_os::unsupported().is_none(),
        blocked: blocked().map(str::to_string),
        enabled: prefs.enabled,
        allowed: engine.kvm_allowed(),
        clip_follows: engine.kvm_clip_follows(),
        swap_mods: prefs.swap,
        hotkey: prefs.hotkey.id().to_string(),
        layout: engine.kvm_layout(),
        computers,
        state: engine.kvm_state(),
        os: std::env::consts::OS.to_string(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn kvm_settings(engine: State<Eng>) -> KvmSettings {
    settings(&engine)
}

/// Place a computer at a side of this one ("left", "right", "top",
/// "bottom"), or take it off (`None`).
#[tauri::command]
#[specta::specta]
pub fn kvm_place(
    node_id: String,
    side: Option<String>,
    engine: State<Eng>,
) -> Result<KvmSettings, CoreError> {
    engine.kvm_set_neighbor(&node_id, side.as_deref())?;
    Ok(settings(&engine))
}

#[tauri::command]
#[specta::specta]
pub fn kvm_set_enabled(on: bool, engine: State<Eng>) -> KvmSettings {
    update(|p| p.enabled = on);
    save(ENABLED_KEY, if on { "on" } else { "off" });
    if !on && engine.kvm_state().role == "controlling" {
        engine.kvm_stop("stopped");
    }
    settings(&engine)
}

#[tauri::command]
#[specta::specta]
pub fn kvm_set_allowed(on: bool, engine: State<Eng>) -> KvmSettings {
    engine.set_kvm_allowed(on);
    settings(&engine)
}

#[tauri::command]
#[specta::specta]
pub fn kvm_set_clip(on: bool, engine: State<Eng>) -> KvmSettings {
    engine.set_kvm_clip_follows(on);
    settings(&engine)
}

#[tauri::command]
#[specta::specta]
pub fn kvm_set_swap(on: bool, engine: State<Eng>) -> KvmSettings {
    update(|p| p.swap = on);
    save(SWAP_KEY, if on { "on" } else { "off" });
    settings(&engine)
}

#[tauri::command]
#[specta::specta]
pub fn kvm_set_hotkey(hotkey: String, engine: State<Eng>) -> Result<KvmSettings, CoreError> {
    let h =
        Hotkey::parse(&hotkey).ok_or_else(|| CoreError::InvalidInput("unknown hotkey".into()))?;
    update(|p| p.hotkey = h);
    save(HOTKEY_KEY, h.id());
    Ok(settings(&engine))
}

/// "Take back" on the controlled computer, or Stop on the controlling one.
#[tauri::command]
#[specta::specta]
pub fn kvm_stop(engine: State<Eng>) {
    engine.kvm_stop("stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn an_outer_edge_counts_and_one_into_another_screen_doesnt() {
        // A laptop with a second screen to its right, a bit lower.
        let screens = [r(0.0, 0.0, 1440.0, 900.0), r(1440.0, 100.0, 1920.0, 1080.0)];
        // Left edge of the laptop: outer.
        assert_eq!(
            edge_at(&screens, 0.0, 450.0, false).map(|e| e.0),
            Some(Side::Left)
        );
        // The laptop's right edge goes on into the other screen: not an edge.
        assert_eq!(edge_at(&screens, 1439.0, 450.0, false), None);
        // But above where the other screen starts, it is.
        assert_eq!(
            edge_at(&screens, 1439.0, 50.0, false).map(|e| e.0),
            Some(Side::Right)
        );
        // The far right of the second screen.
        let (side, pos) = edge_at(&screens, 3359.0, 1179.0 - 500.0, false).unwrap();
        assert_eq!(side, Side::Right);
        // Halfway down the whole desktop (0..1180) is near 5000.
        assert!((5_700..=5_800).contains(&pos), "{pos}");
        // The middle of a screen, and outside every screen: nothing.
        assert_eq!(edge_at(&screens, 700.0, 450.0, false), None);
        assert_eq!(edge_at(&screens, -5.0, 450.0, false), None);
        // A corner doesn't start anything; it counts while controlled.
        assert_eq!(edge_at(&screens, 0.0, 2.0, false), None);
        assert_eq!(
            edge_at(&screens, 0.0, 2.0, true).map(|e| e.0),
            Some(Side::Left)
        );
        // Top and bottom.
        assert_eq!(
            edge_at(&screens, 700.0, 0.0, false).map(|e| e.0),
            Some(Side::Top)
        );
        assert_eq!(
            edge_at(&screens, 700.0, 899.0, false).map(|e| e.0),
            Some(Side::Bottom)
        );
    }

    #[test]
    fn the_pointer_comes_in_at_the_matching_spot() {
        let one = [r(0.0, 0.0, 1920.0, 1080.0)];
        // A quarter of the way down the left edge, just inside.
        let (x, y) = entry_point(&one, Side::Left, 2_500).unwrap();
        assert_eq!(x, INSET_PX);
        assert!((y - 269.75).abs() < 0.01, "{y}");
        let (x, y) = entry_point(&one, Side::Right, 10_000).unwrap();
        assert_eq!((x, y), (1919.0 - INSET_PX, 1079.0));
        let (x, y) = entry_point(&one, Side::Top, 5_000).unwrap();
        assert_eq!((x, y), (959.5, INSET_PX));
        let (_, y) = entry_point(&one, Side::Bottom, 0).unwrap();
        assert_eq!(y, 1079.0 - INSET_PX);
        // Two screens side by side: coming in at the right lands on the
        // right-hand one; at the left on the left-hand one.
        let two = [r(0.0, 0.0, 1000.0, 800.0), r(1000.0, 0.0, 1000.0, 800.0)];
        assert_eq!(
            entry_point(&two, Side::Right, 5_000).unwrap().0,
            1999.0 - INSET_PX
        );
        assert_eq!(entry_point(&two, Side::Left, 5_000).unwrap().0, INSET_PX);
        // Low down the left side only the taller screen is there: its own
        // left edge is the outer one at that height.
        let stepped = [r(0.0, 0.0, 1000.0, 500.0), r(1000.0, 0.0, 1000.0, 1000.0)];
        let (x, y) = entry_point(&stepped, Side::Left, 10_000).unwrap();
        assert_eq!((x, y), (1000.0 + INSET_PX, 999.0));
        // A spot no screen spans (a gap between two): the nearest, clamped.
        let gap = [r(0.0, 0.0, 1000.0, 400.0), r(0.0, 600.0, 1000.0, 400.0)];
        let (x, y) = entry_point(&gap, Side::Left, 4_500).unwrap();
        assert_eq!((x, y), (INSET_PX, 399.0));
        assert_eq!(entry_point(&[], Side::Left, 1), None);
    }

    #[test]
    fn only_arriving_at_an_edge_goes_over() {
        // Reached the right edge just now: go.
        assert_eq!(
            arrived(None, Some((Side::Right, 5))),
            Some((Side::Right, 5))
        );
        // Still resting there: no.
        assert_eq!(arrived(Some(Side::Right), Some((Side::Right, 9))), None);
        // Slid round to another edge: that's new.
        assert_eq!(
            arrived(Some(Side::Right), Some((Side::Top, 1))),
            Some((Side::Top, 1))
        );
        assert_eq!(arrived(Some(Side::Left), None), None);
        // Back without crossing: put a little inside, off the edge.
        let one = [r(0.0, 0.0, 1920.0, 1080.0)];
        let (x, y) = off_the_edge(&one, 1919.0, 500.0).unwrap();
        assert_eq!((x, y), (1879.0, 500.0));
        assert_eq!(edge_at(&one, x, y, true), None);
        assert_eq!(off_the_edge(&one, 900.0, 500.0), None);
    }

    #[test]
    fn leaving_and_coming_back_meet_at_the_same_spot() {
        // Out at this computer's right edge, 30% down; in at the other's
        // left edge, 30% down; pushed back out there, in again here.
        let here = [r(0.0, 0.0, 2560.0, 1440.0)];
        let there = [r(0.0, 0.0, 1920.0, 1080.0)];
        let (side, pos) = edge_at(&here, 2559.0, 0.3 * 1439.0, false).unwrap();
        assert_eq!((side, pos), (Side::Right, 3_000));
        let (x, y) = entry_point(&there, side.opposite(), pos).unwrap();
        assert!((y - 0.3 * 1079.0).abs() < 0.6);
        // Moving along or away from the edge over there doesn't come back.
        assert_eq!(pushed_out(&there, 0.0, y, 0, 5, Side::Left), None);
        assert_eq!(pushed_out(&there, x, y, -3, 0, Side::Left), None);
        assert_eq!(pushed_out(&there, 0.0, y, 4, 0, Side::Left), None);
        // Pushed left at its left edge: back, at the same spot.
        let back = pushed_out(&there, 0.0, y, -4, 0, Side::Left).unwrap();
        assert!(back.abs_diff(3_000) <= 2, "{back}");
        let (hx, hy) = entry_point(&here, Side::Right, back).unwrap();
        assert_eq!(hx, 2559.0 - INSET_PX);
        assert!((hy - 0.3 * 1439.0).abs() < 1.0);
        // Another edge over there isn't the way back.
        assert_eq!(pushed_out(&there, 1919.0, y, 4, 0, Side::Left), None);
    }

    fn core(
        role: &str,
        node: Option<&str>,
        reason: Option<&str>,
        edge: Option<&str>,
        pos: Option<u16>,
    ) -> KvmState {
        KvmState {
            role: role.into(),
            node_id: node.map(str::to_string),
            name: None,
            edge: edge.map(str::to_string),
            pos,
            reason: reason.map(str::to_string),
        }
    }

    fn capturing() -> Phase {
        let (p, a) = step(
            &Phase::Idle,
            Input::EdgeHit {
                node: "pc",
                edge: Side::Right,
                pos: 10,
            },
        );
        assert_eq!(
            a,
            [Action::StartCore {
                node: "pc".into(),
                edge: Side::Right,
                pos: 10
            }]
        );
        let (p, a) = step(
            &p,
            Input::Core(&core("controlling", Some("pc"), None, Some("right"), None)),
        );
        assert_eq!(a, [Action::BeginCapture { node: "pc".into() }]);
        assert_eq!(p, Phase::Capturing { node: "pc".into() });
        p
    }

    #[test]
    fn the_capture_always_lets_go() {
        // The link drops (or the other end goes quiet): the core says idle.
        for reason in ["link_lost", "timeout", "off", "busy", "ended", "stopped"] {
            let (p, a) = step(
                &capturing(),
                Input::Core(&core("idle", None, Some(reason), None, None)),
            );
            assert_eq!(p, Phase::Idle);
            assert_eq!(a, [Action::EndCapture { back: None }], "{reason}");
        }
        // The hotkey: let go here and tell the core.
        let (p, a) = step(&capturing(), Input::Hotkey);
        assert_eq!(p, Phase::Idle);
        assert_eq!(a, [Action::EndCapture { back: None }, Action::StopCore]);
        // The hooks failed: the same.
        let (p, a) = step(&capturing(), Input::CaptureFailed);
        assert_eq!(p, Phase::Idle);
        assert_eq!(a, [Action::EndCapture { back: None }, Action::StopCore]);
        // The pointer came back: let go, and put it at the matching spot.
        let (p, a) = step(
            &capturing(),
            Input::Core(&core(
                "idle",
                None,
                Some("returned"),
                Some("right"),
                Some(4_200),
            )),
        );
        assert_eq!(p, Phase::Idle);
        assert_eq!(
            a,
            [Action::EndCapture {
                back: Some((Side::Right, 4_200))
            }]
        );
        // Refused before it started: nothing was captured.
        let (p, _) = step(
            &Phase::Idle,
            Input::EdgeHit {
                node: "pc",
                edge: Side::Right,
                pos: 10,
            },
        );
        let (p, a) = step(
            &p,
            Input::Core(&core("idle", None, Some("off"), None, None)),
        );
        assert_eq!((p, a), (Phase::Idle, vec![]));
        let (p, a) = step(&Phase::Starting { node: "pc".into() }, Input::StartFailed);
        assert_eq!((p, a), (Phase::Idle, vec![]));
    }

    #[test]
    fn being_controlled_plays_and_always_lets_go() {
        let (p, a) = step(
            &Phase::Idle,
            Input::Core(&core(
                "controlled",
                Some("mac"),
                None,
                Some("left"),
                Some(300),
            )),
        );
        assert_eq!(p, Phase::Controlled { node: "mac".into() });
        assert_eq!(
            a,
            [Action::EnterHere {
                edge: Side::Left,
                pos: 300
            }]
        );
        // The edge and the hotkey do nothing while controlled.
        let (p2, a) = step(
            &p,
            Input::EdgeHit {
                node: "x",
                edge: Side::Left,
                pos: 1,
            },
        );
        assert_eq!((&p2, a.len()), (&p, 0));
        assert_eq!(step(&p, Input::Hotkey).1, vec![]);
        // Whatever ends it, everything held here is let go.
        for reason in ["link_lost", "timeout", "stopped", "returned"] {
            let (q, a) = step(
                &p,
                Input::Core(&core("idle", None, Some(reason), None, None)),
            );
            assert_eq!(q, Phase::Idle);
            assert_eq!(a, [Action::ReleaseHere], "{reason}");
        }
        // Controlled by another one straight away: let go first.
        let (q, a) = step(
            &p,
            Input::Core(&core("controlled", Some("pc"), None, Some("top"), Some(1))),
        );
        assert_eq!(q, Phase::Controlled { node: "pc".into() });
        assert_eq!(a[0], Action::ReleaseHere);
        // Controlling while controlled (never expected): let go first too.
        let (q, a) = step(
            &p,
            Input::Core(&core("controlling", Some("pc"), None, None, None)),
        );
        assert_eq!(q, Phase::Capturing { node: "pc".into() });
        assert_eq!(
            a,
            [
                Action::ReleaseHere,
                Action::BeginCapture { node: "pc".into() }
            ]
        );
    }

    #[test]
    fn settings_default_to_on_with_ctrl_alt_esc() {
        let p = load_prefs(None);
        assert!(p.enabled);
        assert!(p.swap);
        assert_eq!(p.hotkey, Hotkey::CtrlAltEsc);
    }
}
