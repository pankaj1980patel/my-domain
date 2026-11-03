// The phone as this computer's touchpad and keyboard (mdcore
// `features::input`): the core hands over pointer moves, clicks, scrolls,
// typed text and a few keys, and they're played back here with `enigo`.
//
// One thread owns the `enigo` handle and plays the events in the order they
// came. On macOS, nothing is delivered until the user allows my-domain under
// Privacy & Security > Accessibility; until then one notice says so (once per
// run) instead of failing silently. On Linux, `enigo` talks to the X server,
// so on Wayland it reaches only X (XWayland) windows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use mdcore::model::{InputEvent, InputKey, MouseButton};
use tauri::AppHandle;

/// Pixels of scrolling on the phone per wheel step here.
const PX_PER_STEP: i32 = 20;

/// Why input can't be played back here yet, for the phone to show: macOS
/// "accessibility"; Linux "display" with no X server to talk to.
pub fn ready() -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    if !crate::device::accessibility_trusted() {
        return Err("accessibility");
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() {
        return Err("display");
    }
    Ok(())
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
                eprintln!("[input] {e:?}: {err}");
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
        _ => "Your phone's touchpad couldn't control this computer.",
    };
    crate::notify::post_urgent(app, "my-domain can't use your phone as a touchpad", body);
}

#[cfg(test)]
mod tests {
    use super::steps;

    #[test]
    fn scroll_pixels_become_wheel_steps() {
        assert_eq!(steps(45), (2, 5));
        assert_eq!(steps(-45), (-2, -5));
        assert_eq!(steps(19), (0, 19));
    }
}
