// The phone's screen on this computer (mdcore `features::screen`, the
// other way round from `screen_share`): the phone sends JPEG snapshots of
// its screen while this computer watches it, after Android's own
// "start recording or casting?" prompt on the phone, every time.
//
// Two ways in: "Ask to see its screen" on the phone's page here (the phone
// asks its owner), or "Share this screen" on the phone, which offers it
// (`CoreEvent::ScreenOffer`): then this window comes forward on the phone's
// page and starts watching. The snapshots are shown by the UI and never
// kept on disk.
//
// With "Control" on here, and "Allow control" on at the phone (its sharing
// notification), presses on the picture become taps and swipes the phone
// plays with its Accessibility helper (`touch`).

use mdcore::error::CoreError;
use mdcore::features::input::{MAX_SWIPE_MS, POINT_SCALE};
use mdcore::model::InputEvent;
use tauri::{AppHandle, Emitter, State};

use crate::Eng;

/// How far a press may wander (in ten-thousandths of the screen) and still
/// be a tap or a long press.
const SLOP: u16 = 150;
/// A press held this long without moving is a long press.
const LONG_PRESS_MS: u32 = 500;
/// The shortest swipe sent (a flick faster than this is still a swipe).
const MIN_SWIPE_MS: u32 = 40;

/// A phone started (`on`) sharing its screen with this computer: say so,
/// bring the window forward on its page (which starts watching). When it
/// stops, the page says so itself.
pub fn on_offer(app: &AppHandle, from: &str, on: bool) {
    if !on {
        return;
    }
    let who = crate::notify::device_name(app, from);
    crate::notify::post_always(
        app,
        &format!("{who} is sharing its screen"),
        "It shows on its page here. Stop it here or on the phone.",
    );
    crate::tray::show_window(app);
    let _ = app.emit("open-device", serde_json::json!({ "node_id": from }));
}

/// What a press on the phone's picture means, from where it went down
/// (`x`, `y`) to where it came up (`x2`, `y2`), in ten-thousandths of the
/// picture, held `ms` milliseconds: a tap where it barely moved and was
/// short, a long press where it barely moved and was held, else a swipe.
pub fn touch(x: u16, y: u16, x2: u16, y2: u16, ms: u32) -> InputEvent {
    let (x, y) = (x.min(POINT_SCALE), y.min(POINT_SCALE));
    let (x2, y2) = (x2.min(POINT_SCALE), y2.min(POINT_SCALE));
    let still = x.abs_diff(x2) <= SLOP && y.abs_diff(y2) <= SLOP;
    let max = u32::from(MAX_SWIPE_MS);
    let ms_of = |v: u32| u16::try_from(v.min(max)).unwrap_or(MAX_SWIPE_MS);
    match (still, ms >= LONG_PRESS_MS) {
        (true, false) => InputEvent::Tap { x, y },
        (true, true) => InputEvent::Swipe {
            x,
            y,
            x2: x,
            y2: y,
            ms: ms_of(ms),
        },
        (false, _) => InputEvent::Swipe {
            x,
            y,
            x2,
            y2,
            ms: ms_of(ms.max(MIN_SWIPE_MS)),
        },
    }
}

/// A press on a phone's picture (see `touch`), sent for the phone to play.
/// It does only while it shares its screen with this computer and its
/// owner allows control from here (its status says `screen_control`).
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub fn phone_touch(
    node_id: String,
    x: u16,
    y: u16,
    x2: u16,
    y2: u16,
    ms: u32,
    engine: State<Eng>,
) -> Result<(), CoreError> {
    engine.send_input(&node_id, vec![touch(x, y, x2, y2, ms)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_still_press_is_a_tap() {
        assert_eq!(
            touch(5000, 2000, 5100, 1900, 120),
            InputEvent::Tap { x: 5000, y: 2000 }
        );
        // Off the picture's edge: on its edge.
        assert_eq!(
            touch(12_000, 0, 12_000, 0, 10),
            InputEvent::Tap {
                x: POINT_SCALE,
                y: 0
            }
        );
    }

    #[test]
    fn a_held_still_press_is_a_long_press() {
        assert_eq!(
            touch(100, 100, 120, 90, 800),
            InputEvent::Swipe {
                x: 100,
                y: 100,
                x2: 100,
                y2: 100,
                ms: 800
            }
        );
        assert_eq!(
            touch(100, 100, 100, 100, 60_000),
            InputEvent::Swipe {
                x: 100,
                y: 100,
                x2: 100,
                y2: 100,
                ms: MAX_SWIPE_MS
            }
        );
    }

    #[test]
    fn a_press_that_moved_is_a_swipe() {
        assert_eq!(
            touch(5000, 8000, 5000, 2000, 300),
            InputEvent::Swipe {
                x: 5000,
                y: 8000,
                x2: 5000,
                y2: 2000,
                ms: 300
            }
        );
        // A flick faster than the shortest swipe still swipes.
        assert_eq!(
            touch(1000, 5000, 9000, 5000, 5),
            InputEvent::Swipe {
                x: 1000,
                y: 5000,
                x2: 9000,
                y2: 5000,
                ms: 40
            }
        );
    }
}
