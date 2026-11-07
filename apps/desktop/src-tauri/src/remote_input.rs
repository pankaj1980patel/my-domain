// The phone as this computer's touchpad and keyboard (mdcore
// `features::input`): the core hands over pointer moves, clicks, scrolls,
// typed text and a few keys, and they're played back here with `enigo`.
//
// One thread owns the `enigo` handle and plays the events in the order they
// came. On macOS, nothing is delivered until the user allows my-domain under
// Privacy & Security > Accessibility; until then one notice says so (once per
// run) instead of failing silently. On Linux X11, `enigo` talks to the X
// server. On Wayland the events go through the RemoteDesktop portal (asked
// once, then remembered), else `/dev/uinput` when this user may write it
// (`md_portal`; never asks for root); with neither, `enigo` reaches only X
// (XWayland) windows.
//
// Three- and four-finger swipes become this computer's own shortcuts (see
// `shortcut`), the way a laptop's touchpad does it:
// - three fingers left/right: switch to the previous/next app;
// - four fingers left/right: switch desktops (Spaces on a Mac);
// - three or four fingers up: show all windows; down: show the desktop
//   (a Mac: the app's windows, App Exposé).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use mdcore::model::{InputEvent, InputKey, MouseButton, SwipeDir};
use tauri::AppHandle;

/// Pixels of scrolling on the phone per wheel step here.
const PX_PER_STEP: i32 = 20;

/// Why input can't be played back here yet, for the phone to show: macOS
/// "accessibility"; Linux "display" with no X server to talk to, or
/// "wayland_input" on Wayland with neither the portal nor uinput (nor
/// XWayland).
pub fn ready() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    if !crate::device::accessibility_trusted() {
        return Err("accessibility");
    }
    #[cfg(target_os = "linux")]
    {
        if wayland_input() {
            return Ok(());
        }
        if std::env::var_os("DISPLAY").is_none() {
            return Err(if md_portal::Session::wayland() {
                "wayland_input"
            } else {
                "display"
            });
        }
    }
    Ok(())
}

/// Wayland, with the RemoteDesktop portal or uinput to play input through.
#[cfg(target_os = "linux")]
pub(crate) fn wayland_input() -> bool {
    md_portal::Session::wayland() && md_portal::service::input_way().is_ok()
}

/// Play `events` back, in order, on the input thread.
pub fn play(app: &AppHandle, events: Vec<InputEvent>) {
    static TX: OnceLock<Mutex<Sender<Vec<InputEvent>>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Vec<InputEvent>>();
        let app = app.clone();
        std::thread::Builder::new()
            .name("remote-input".into())
            .spawn(move || {
                let mut player = Player::default();
                for events in rx {
                    player.play(&app, &events);
                }
            })
            .ok();
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(events);
    }
}

#[derive(Default)]
struct Player {
    enigo: Option<Enigo>,
    /// Scroll pixels not yet a whole wheel step.
    rest_x: i32,
    rest_y: i32,
}

impl Player {
    fn play(&mut self, app: &AppHandle, events: &[InputEvent]) {
        if let Err(why) = ready() {
            tell_once(app, why);
            return;
        }
        #[cfg(target_os = "linux")]
        if wayland_input() {
            self.play_wayland(app, events);
            return;
        }
        if self.enigo.is_none() {
            let settings = Settings {
                // We say so ourselves (once), rather than macOS's prompt
                // popping up on every event.
                open_prompt_to_get_permissions: false,
                ..Settings::default()
            };
            match Enigo::new(&settings) {
                Ok(e) => self.enigo = Some(e),
                Err(e) => {
                    eprintln!("[input] can't control this computer: {e}");
                    tell_once(app, "enigo");
                    return;
                }
            }
        }
        for e in events {
            if let Err(err) = self.one(e) {
                // The kind only: a text event may be a password.
                eprintln!("[input] {}: {err}", input_kind(e));
            }
        }
    }

    /// Wayland: through the portal (or uinput).
    #[cfg(target_os = "linux")]
    fn play_wayland(&mut self, app: &AppHandle, events: &[InputEvent]) {
        let mut rest = (self.rest_x, self.rest_y);
        let done = md_portal::service::with_input(|inj| {
            for e in events {
                // The kind only: a text event may be a password.
                wayland::one(inj, e, &mut rest)
                    .map_err(|err| format!("{}: {err}", input_kind(e)))?;
            }
            Ok::<(), String>(())
        });
        (self.rest_x, self.rest_y) = rest;
        match done {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                // The session ended (stopped from the desktop's indicator):
                // a new one next time.
                eprintln!("[input] {e}");
                md_portal::service::input_reset();
            }
            Err(e) => {
                eprintln!("[input] can't control this computer: {e}");
                tell_once(app, "wayland_denied");
            }
        }
    }

    fn one(&mut self, e: &InputEvent) -> Result<(), enigo::InputError> {
        let Some(enigo) = self.enigo.as_mut() else {
            return Ok(());
        };
        match e {
            InputEvent::Move { dx, dy } => enigo.move_mouse(*dx, *dy, Coordinate::Rel),
            InputEvent::Click { button } => {
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                };
                enigo.button(b, Direction::Click)
            }
            InputEvent::Scroll { dx, dy } => {
                let (sx, rx) = steps(self.rest_x + dx);
                let (sy, ry) = steps(self.rest_y + dy);
                (self.rest_x, self.rest_y) = (rx, ry);
                if sy != 0 {
                    enigo.scroll(sy, Axis::Vertical)?;
                }
                if sx != 0 {
                    enigo.scroll(sx, Axis::Horizontal)?;
                }
                Ok(())
            }
            InputEvent::Text { text } => enigo.text(text),
            InputEvent::Key { key } => {
                let k = match key {
                    InputKey::Enter => Key::Return,
                    InputKey::Backspace => Key::Backspace,
                    InputKey::Left => Key::LeftArrow,
                    InputKey::Right => Key::RightArrow,
                    InputKey::Up => Key::UpArrow,
                    InputKey::Down => Key::DownArrow,
                    InputKey::Esc => Key::Escape,
                };
                enigo.key(k, Direction::Click)
            }
            InputEvent::Point { x, y } => {
                let (w, h) = enigo.main_display()?;
                let scale = i32::from(mdcore::features::input::POINT_SCALE);
                let at = |v: u16, size: i32| (i32::from(v) * (size - 1)) / scale;
                enigo.move_mouse(at(*x, w), at(*y, h), Coordinate::Abs)
            }
            InputEvent::Gesture { fingers, dir } => {
                let (mods, key) = shortcut(*fingers, *dir);
                press_combo(enigo, mods, key)
            }
            // Touches for a phone's screen: nothing to do on a computer.
            InputEvent::Tap { .. } | InputEvent::Swipe { .. } => Ok(()),
        }
    }
}

/// Paste into the app in front: Cmd+V on a Mac, Ctrl+V elsewhere ("Insert
/// where I am", `insert_capture`). Only with the permission input already
/// needs (`ready`); says why not otherwise.
pub fn paste() -> Result<(), &'static str> {
    ready().map_err(|why| match why {
        "accessibility" => {
            "Allow my-domain under Privacy & Security \u{203a} Accessibility to paste for you."
        }
        _ => "There\u{2019}s no screen here to paste into.",
    })?;
    #[cfg(target_os = "linux")]
    if wayland_input() {
        use md_portal::keys::code;
        return match md_portal::service::with_input(|inj| {
            inj.combo(&[code::LEFTCTRL], wayland::KEY_V)
        }) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("Couldn\u{2019}t press the keys."),
            Err(_) => Err("my-domain wasn\u{2019}t allowed to type here."),
        };
    }
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|_| "Couldn\u{2019}t reach the keyboard.")?;
    let held: &[Key] = if cfg!(target_os = "macos") {
        &[Key::Meta]
    } else {
        &[Key::Control]
    };
    press_combo(&mut enigo, held, Key::Unicode('v')).map_err(|_| "Couldn\u{2019}t press the keys.")
}

/// Hold `mods`, press `key`, let go of `mods` (in reverse), even if a
/// press fails half way.
fn press_combo(enigo: &mut Enigo, mods: &[Key], key: Key) -> Result<(), enigo::InputError> {
    let mut held = Vec::new();
    let mut result = Ok(());
    for m in mods {
        match enigo.key(*m, Direction::Press) {
            Ok(()) => held.push(*m),
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    if result.is_ok() {
        result = enigo.key(key, Direction::Click);
    }
    for m in held.iter().rev() {
        let _ = enigo.key(*m, Direction::Release);
    }
    result
}

/// The shortcut for a swipe on this computer: modifiers to hold, then the
/// key. Fingers moving left show what's to the right (as on a touchpad).
fn shortcut(fingers: u8, dir: SwipeDir) -> (&'static [Key], Key) {
    use SwipeDir::*;
    #[cfg(target_os = "macos")]
    {
        match (fingers, dir) {
            (3, Right) => (&[Key::Meta], Key::Tab),
            (3, Left) => (&[Key::Meta, Key::Shift], Key::Tab),
            (_, Left) => (&[Key::Control], Key::RightArrow),
            (_, Right) => (&[Key::Control], Key::LeftArrow),
            (_, Up) => (&[Key::Control], Key::UpArrow),
            (_, Down) => (&[Key::Control], Key::DownArrow),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match (fingers, dir) {
            (3, Right) => (&[Key::Alt], Key::Tab),
            (3, Left) => (&[Key::Alt, Key::Shift], Key::Tab),
            (_, Left) => (&[Key::Control, Key::Meta], Key::RightArrow),
            (_, Right) => (&[Key::Control, Key::Meta], Key::LeftArrow),
            (_, Up) => (&[Key::Meta], Key::Tab),
            (_, Down) => (&[Key::Meta], Key::Unicode('d')),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // GNOME and KDE both know these.
        match (fingers, dir) {
            (3, Right) => (&[Key::Alt], Key::Tab),
            (3, Left) => (&[Key::Alt, Key::Shift], Key::Tab),
            (_, Left) => (&[Key::Control, Key::Alt], Key::RightArrow),
            (_, Right) => (&[Key::Control, Key::Alt], Key::LeftArrow),
            (_, Up) => (&[], Key::Meta),
            (_, Down) => (&[Key::Meta], Key::Unicode('d')),
        }
    }
}

/// Whole wheel steps in `px` pixels, and the pixels left over.
fn steps(px: i32) -> (i32, i32) {
    (px / PX_PER_STEP, px % PX_PER_STEP)
}

/// Tell the user, once per run, why the phone's touchpad does nothing.
fn tell_once(app: &AppHandle, why: &str) {
    static TOLD: AtomicBool = AtomicBool::new(false);
    if TOLD.swap(true, Ordering::Relaxed) {
        return;
    }
    let body = match why {
        "accessibility" => {
            "To use your phone as a touchpad, allow my-domain in System Settings > \
             Privacy & Security > Accessibility, then try again."
        }
        "display" => "Your phone's touchpad needs an X11 session (or XWayland) on this computer.",
        "wayland_input" => {
            "On Wayland, your phone's touchpad needs your desktop's remote desktop portal \
             (xdg-desktop-portal-gnome or -kde) or access to /dev/uinput. See Help \u{203a} \
             Troubleshooting."
        }
        "wayland_denied" => {
            "my-domain wasn't allowed to control this computer. Use your phone's touchpad again \
             in a couple of minutes to be asked again."
        }
        _ => "Your phone's touchpad couldn't control this computer.",
    };
    crate::notify::post_always(app, "my-domain can't use your phone as a touchpad", body);
}

/// Wayland: one event through the portal's (or uinput's) session.
#[cfg(target_os = "linux")]
mod wayland {
    use md_portal::keys::code;
    use md_portal::Injector;
    use mdcore::model::{InputEvent, InputKey, MouseButton, SwipeDir};

    /// evdev KEY_D and KEY_V.
    const KEY_D: u16 = 32;
    pub const KEY_V: u16 = 47;

    pub fn one(inj: &mut Injector, e: &InputEvent, rest: &mut (i32, i32)) -> Result<(), String> {
        match e {
            InputEvent::Move { dx, dy } => inj.motion(*dx, *dy),
            InputEvent::Click { button } => {
                let b = match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 3,
                };
                inj.button(b, true)?;
                inj.button(b, false)
            }
            InputEvent::Scroll { dx, dy } => {
                let (sx, rx) = super::steps(rest.0 + dx);
                let (sy, ry) = super::steps(rest.1 + dy);
                *rest = (rx, ry);
                if sx != 0 || sy != 0 {
                    inj.wheel(sx, sy)?;
                }
                Ok(())
            }
            InputEvent::Text { text } => inj.text(text),
            InputEvent::Key { key } => {
                let k = match key {
                    InputKey::Enter => code::ENTER,
                    InputKey::Backspace => code::BACKSPACE,
                    InputKey::Left => code::LEFT,
                    InputKey::Right => code::RIGHT,
                    InputKey::Up => code::UP,
                    InputKey::Down => code::DOWN,
                    InputKey::Esc => code::ESC,
                };
                inj.combo(&[], k)
            }
            InputEvent::Point { x, y } => {
                // The first screen the portal shared (none: uinput, which
                // can't place the pointer).
                let Some(s) = inj.screens().first().copied() else {
                    return Ok(());
                };
                let scale = f64::from(mdcore::features::input::POINT_SCALE);
                let px = s.x + f64::from(*x) * (s.w - 1.0) / scale;
                let py = s.y + f64::from(*y) * (s.h - 1.0) / scale;
                inj.warp(px, py);
                Ok(())
            }
            InputEvent::Gesture { fingers, dir } => {
                let (mods, key) = shortcut(*fingers, *dir);
                inj.combo(mods, key)
            }
            InputEvent::Tap { .. } | InputEvent::Swipe { .. } => Ok(()),
        }
    }

    /// The same shortcuts as `super::shortcut` on Linux, as evdev keys.
    fn shortcut(fingers: u8, dir: SwipeDir) -> (&'static [u16], u16) {
        use SwipeDir::*;
        match (fingers, dir) {
            (3, Right) => (&[code::LEFTALT], code::TAB),
            (3, Left) => (&[code::LEFTALT, code::LEFTSHIFT], code::TAB),
            (_, Left) => (&[code::LEFTCTRL, code::LEFTALT], code::RIGHT),
            (_, Right) => (&[code::LEFTCTRL, code::LEFTALT], code::LEFT),
            (_, Up) => (&[], code::LEFTMETA),
            (_, Down) => (&[code::LEFTMETA], KEY_D),
        }
    }
}

/// What kind of event `e` is, for the log (never what it carries).
fn input_kind(e: &InputEvent) -> &'static str {
    match e {
        InputEvent::Move { .. } => "move",
        InputEvent::Click { .. } => "click",
        InputEvent::Scroll { .. } => "scroll",
        InputEvent::Text { .. } => "text",
        InputEvent::Key { .. } => "key",
        InputEvent::Gesture { .. } => "gesture",
        InputEvent::Point { .. } => "point",
        InputEvent::Tap { .. } => "tap",
        InputEvent::Swipe { .. } => "swipe",
    }
}

#[cfg(test)]
mod tests {
    use super::{shortcut, steps};
    use enigo::Key;
    use mdcore::model::SwipeDir;

    #[test]
    fn swipes_become_shortcuts() {
        // Three fingers sideways switch apps (Tab), four switch desktops
        // (arrows), whatever the computer.
        assert_eq!(shortcut(3, SwipeDir::Right).1, Key::Tab);
        assert!(shortcut(3, SwipeDir::Left).0.contains(&Key::Shift));
        assert_eq!(shortcut(4, SwipeDir::Left).1, Key::RightArrow);
        assert_eq!(shortcut(4, SwipeDir::Right).1, Key::LeftArrow);
    }

    #[test]
    fn the_log_names_the_kind_never_the_typed_text() {
        let e = mdcore::model::InputEvent::Text {
            text: "hunter2".into(),
        };
        assert_eq!(super::input_kind(&e), "text");
    }

    #[test]
    fn scroll_pixels_become_wheel_steps() {
        assert_eq!(steps(45), (2, 5));
        assert_eq!(steps(-45), (-2, -5));
        assert_eq!(steps(19), (0, 19));
    }
}
