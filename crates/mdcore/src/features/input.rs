//! Phone as touchpad and keyboard: the phone moves a computer's pointer,
//! clicks, scrolls, types text and presses a few keys. The core carries the
//! events; the computer's host app plays them back (desktop: `enigo`).
//!
//!   input { from, events: [InputEvent] }   phone → computer
//!
//! Each event is `{"kind": "move", "dx", "dy"}`, `{"kind": "click",
//! "button": "left" | "right" | "middle"}`, `{"kind": "scroll", "dx", "dy"}`,
//! `{"kind": "text", "text"}`, `{"kind": "key", "key": "enter" |
//! "backspace" | "left" | "right" | "up" | "down" | "esc"}` or
//! `{"kind": "gesture", "fingers": 3 | 4, "dir": "left" | "right" | "up" |
//! "down"}` (a multi-finger swipe; the computer picks its own shortcut) or
//! `{"kind": "point", "x", "y"}` (the pointer to a spot on the screen, in
//! ten-thousandths of its width and height).
//!
//! The other way round, a computer showing a phone's screen (see
//! `features::screen`) sends `{"kind": "tap", "x", "y"}` and `{"kind":
//! "swipe", "x", "y", "x2", "y2", "ms"}` (same spots as `point`) for the
//! phone to play with its Accessibility helper, while the phone shares its
//! screen with that computer and its owner allows control there. A device
//! drops event kinds it doesn't know (older versions: these), one by one,
//! never the whole message, and never shows them as chat.
//!
//! - Only over a live link, never queued: a click or a key pressed minutes
//!   late, when a link finally came up, would do something you didn't mean.
//!   With no link, the send fails and the connect ladder starts, so the next
//!   one goes.
//! - Pointer moves and scrolls are batched: `Engine::input_move` and
//!   `Engine::input_scroll` add up, and what's pending goes out every
//!   `BATCH_EVERY` as one message. A click, text or key sends what's
//!   pending first, so the order holds.
//! - The computer plays back events only from your own devices (the message
//!   decrypted with your key, from a device in its list) and only from a
//!   device allowed to control it (`features::perms`, "control": off until
//!   allowed for that device). It arrives as `CoreEvent::Input`. Otherwise
//!   it's refused: the sender hears why (`perm_refused`), the owner is
//!   asked. Taps and swipes for a phone (`tap`, `swipe`) aren't this: the
//!   phone plays them only while it shares its screen with that computer
//!   and its owner allows control there.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::json;

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::model::InputEvent;
use crate::platform::Platform;

use super::perms::Perm;
use super::{ControlHandler, ControlMsg};

/// How often pending pointer moves and scrolls go out (about 50 a second).
pub const BATCH_EVERY: Duration = Duration::from_millis(20);
/// Most events in one message; more are dropped.
pub const MAX_EVENTS: usize = 64;
/// Longest typed text in one event, in characters.
pub const MAX_TEXT: usize = 500;
/// `InputEvent::Point` coordinates go from 0 to this (the far edge).
pub const POINT_SCALE: u16 = 10_000;
/// Longest `InputEvent::Swipe`, in milliseconds (a long press included).
pub const MAX_SWIPE_MS: u16 = 3_000;
/// Largest move or scroll in one event, in pixels either way.
pub const MAX_STEP: i32 = 4000;
/// The batching thread stops after this long with nothing to send.
const IDLE_STOP: Duration = Duration::from_secs(2);
/// With no link, the connect ladder starts at most this often per device.
const CONNECT_EVERY: Duration = Duration::from_secs(5);

/// Moves and scrolls added up for one device, not sent yet.
#[derive(Default)]
struct Pending {
    move_x: i32,
    move_y: i32,
    scroll_x: i32,
    scroll_y: i32,
}

impl Pending {
    /// As events (a move, then a scroll; zero ones left out).
    fn events(&self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        if self.move_x != 0 || self.move_y != 0 {
            out.push(InputEvent::Move {
                dx: self.move_x.clamp(-MAX_STEP, MAX_STEP),
                dy: self.move_y.clamp(-MAX_STEP, MAX_STEP),
            });
        }
        if self.scroll_x != 0 || self.scroll_y != 0 {
            out.push(InputEvent::Scroll {
                dx: self.scroll_x.clamp(-MAX_STEP, MAX_STEP),
                dy: self.scroll_y.clamp(-MAX_STEP, MAX_STEP),
            });
        }
        out
    }
}

#[derive(Default)]
struct Batch {
    pending: HashMap<String, Pending>,
    /// The batching thread is running.
    flushing: bool,
}

/// The touchpad's state: pending moves, and when connecting last started.
#[derive(Default)]
pub struct Input {
    batch: Mutex<Batch>,
    connecting: Mutex<HashMap<String, Instant>>,
}

/// Taps and swipes for a phone's screen: the phone's own call (see the
/// module doc), not remote input.
fn for_a_phone(e: &InputEvent) -> bool {
    matches!(e, InputEvent::Tap { .. } | InputEvent::Swipe { .. })
}

/// Events made safe to play back: at most `MAX_EVENTS`, moves and scrolls
/// within `MAX_STEP`, text without control characters and at most
/// `MAX_TEXT` long, gestures with three or four fingers, spots on the
/// screen within it and swipes at most `MAX_SWIPE_MS` long; empty ones
/// dropped.
pub fn clean(events: Vec<InputEvent>) -> Vec<InputEvent> {
    events
        .into_iter()
        .take(MAX_EVENTS)
        .filter_map(|e| match e {
            InputEvent::Move { dx, dy } | InputEvent::Scroll { dx, dy } if dx == 0 && dy == 0 => {
                None
            }
            InputEvent::Move { dx, dy } => Some(InputEvent::Move {
                dx: dx.clamp(-MAX_STEP, MAX_STEP),
                dy: dy.clamp(-MAX_STEP, MAX_STEP),
            }),
            InputEvent::Scroll { dx, dy } => Some(InputEvent::Scroll {
                dx: dx.clamp(-MAX_STEP, MAX_STEP),
                dy: dy.clamp(-MAX_STEP, MAX_STEP),
            }),
            InputEvent::Text { text } => {
                let text: String = text
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_TEXT)
                    .collect();
                (!text.is_empty()).then_some(InputEvent::Text { text })
            }
            InputEvent::Gesture { fingers, .. } if !(3..=4).contains(&fingers) => None,
            InputEvent::Point { x, y } => Some(InputEvent::Point {
                x: x.min(POINT_SCALE),
                y: y.min(POINT_SCALE),
            }),
            InputEvent::Tap { x, y } => Some(InputEvent::Tap {
                x: x.min(POINT_SCALE),
                y: y.min(POINT_SCALE),
            }),
            InputEvent::Swipe { x, y, x2, y2, ms } => Some(InputEvent::Swipe {
                x: x.min(POINT_SCALE),
                y: y.min(POINT_SCALE),
                x2: x2.min(POINT_SCALE),
                y2: y2.min(POINT_SCALE),
                ms: ms.clamp(1, MAX_SWIPE_MS),
            }),
            other => Some(other),
        })
        .collect()
}

/// There's a route to `node`; if not, start connecting (at most every
/// `CONNECT_EVERY`) and say so.
fn need_link<P: Platform>(ctx: &NetCtx<P>, node: &str) -> crate::Result<()> {
    if !ctx.peers.lock().contains_key(node) {
        return Err(CoreError::PeerNotFound);
    }
    if ctx.ws_conns.lock().contains_key(node) {
        return Ok(());
    }
    let due = {
        let mut c = ctx.extras.input.connecting.lock();
        let due = c.get(node).is_none_or(|at| at.elapsed() >= CONNECT_EVERY);
        if due {
            c.insert(node.to_string(), Instant::now());
        }
        due
    };
    if due {
        crate::engine::start_ladder(ctx, node);
    }
    Err(CoreError::NoLink("not connected yet: connecting".into()))
}

/// Send `events` to `node` over its link now.
fn send_now<P: Platform>(ctx: &NetCtx<P>, node: &str, events: &[InputEvent]) -> crate::Result<()> {
    let (me, name) = {
        let id = ctx.identity.lock();
        (id.node_id.clone(), id.name.clone())
    };
    let msg = json!({ "type": "input", "from": me, "events": events }).to_string();
    crate::transport::send_payload(ctx, &name, node, &msg)
}

/// Send `events` now, after any moves and scrolls still pending for `node`.
pub fn send<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    events: Vec<InputEvent>,
) -> crate::Result<()> {
    let events = clean(events);
    if events.is_empty() {
        return Err(CoreError::InvalidInput("nothing to send".into()));
    }
    need_link(ctx, node)?;
    let pending = ctx.extras.input.batch.lock().pending.remove(node);
    let mut all = pending.map(|p| p.events()).unwrap_or_default();
    all.extend(events);
    send_now(ctx, node, &all)
}

/// Add a pointer move (`scroll` false) or a scroll to what's pending for
/// `node`; it goes out within `BATCH_EVERY`.
pub fn add<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    dx: i32,
    dy: i32,
    scroll: bool,
) -> crate::Result<()> {
    need_link(ctx, node)?;
    if dx == 0 && dy == 0 {
        return Ok(());
    }
    let start = {
        let mut b = ctx.extras.input.batch.lock();
        let p = b.pending.entry(node.to_string()).or_default();
        if scroll {
            p.scroll_x = p.scroll_x.saturating_add(dx);
            p.scroll_y = p.scroll_y.saturating_add(dy);
        } else {
            p.move_x = p.move_x.saturating_add(dx);
            p.move_y = p.move_y.saturating_add(dy);
        }
        !std::mem::replace(&mut b.flushing, true)
    };
    if start {
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("input-batch".into())
            .spawn(move || flush_loop(&ctx))
            .ok();
    }
    Ok(())
}

/// Every `BATCH_EVERY`, send what's pending; stop after `IDLE_STOP` with
/// nothing to send (the next move starts it again).
fn flush_loop<P: Platform>(ctx: &NetCtx<P>) {
    let mut last = Instant::now();
    loop {
        std::thread::sleep(BATCH_EVERY);
        let due: Vec<(String, Pending)> = {
            let mut b = ctx.extras.input.batch.lock();
            if b.pending.is_empty() {
                if last.elapsed() >= IDLE_STOP {
                    b.flushing = false;
                    return;
                }
                continue;
            }
            b.pending.drain().collect()
        };
        last = Instant::now();
        for (node, p) in due {
            // A link that died in between: these moves are stale anyway.
            let _ = send_now(ctx, &node, &p.events());
        }
    }
}

pub struct InputHandler;

impl<P: Platform> ControlHandler<P> for InputHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["input"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Your own devices only: decrypted with your key (or it wouldn't be
        // here), and in this device's list.
        if from.is_empty() || !ctx.peers.lock().contains_key(&from) {
            return;
        }
        let events: Vec<InputEvent> = match msg.json("events") {
            serde_json::Value::Array(list) => list
                .into_iter()
                .take(MAX_EVENTS)
                .filter_map(|e| serde_json::from_value(e).ok())
                .collect(),
            _ => return,
        };
        let mut events = clean(events);
        if !super::perms::allowed(ctx, Perm::Control, &from) {
            let before = events.len();
            events.retain(for_a_phone);
            if events.len() < before {
                super::perms::refuse(ctx, Perm::Control, &from, true);
            }
        }
        if !events.is_empty() {
            ctx.sink.emit(CoreEvent::Input { from, events });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InputKey, MouseButton, SwipeDir};

    #[test]
    fn events_are_made_safe() {
        let long = "x".repeat(MAX_TEXT + 10);
        let got = clean(vec![
            InputEvent::Move { dx: 0, dy: 0 },
            InputEvent::Move { dx: 99_999, dy: -5 },
            InputEvent::Text {
                text: "hi\u{1b}[2J\nthere".into(),
            },
            InputEvent::Text {
                text: "\n\t".into(),
            },
            InputEvent::Text { text: long },
            InputEvent::Key {
                key: InputKey::Enter,
            },
        ]);
        assert_eq!(got.len(), 4);
        let swipes = clean(vec![
            InputEvent::Gesture {
                fingers: 2,
                dir: SwipeDir::Up,
            },
            InputEvent::Gesture {
                fingers: 4,
                dir: SwipeDir::Left,
            },
            InputEvent::Gesture {
                fingers: 9,
                dir: SwipeDir::Down,
            },
        ]);
        assert_eq!(
            swipes,
            [InputEvent::Gesture {
                fingers: 4,
                dir: SwipeDir::Left
            }]
        );
        assert_eq!(
            got[0],
            InputEvent::Move {
                dx: MAX_STEP,
                dy: -5
            }
        );
        assert_eq!(
            got[1],
            InputEvent::Text {
                text: "hi[2Jthere".into()
            }
        );
        assert!(matches!(&got[2], InputEvent::Text { text } if text.len() == MAX_TEXT));
        let many = vec![
            InputEvent::Click {
                button: MouseButton::Left
            };
            MAX_EVENTS * 2
        ];
        assert_eq!(clean(many).len(), MAX_EVENTS);
    }

    #[test]
    fn the_wire_form_is_tagged_by_kind() {
        let e = InputEvent::Click {
            button: MouseButton::Right,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap_or_default(),
            json!({"kind": "click", "button": "right"})
        );
        let back: InputEvent =
            serde_json::from_value(json!({"kind": "key", "key": "backspace"})).unwrap_or(e);
        assert_eq!(
            back,
            InputEvent::Key {
                key: InputKey::Backspace
            }
        );
        assert!(
            serde_json::from_value::<InputEvent>(json!({"kind": "key", "key": "f13"})).is_err()
        );
        assert_eq!(MouseButton::parse("left"), Some(MouseButton::Left));
        assert_eq!(MouseButton::parse("middle"), Some(MouseButton::Middle));
        assert_eq!(
            clean(vec![InputEvent::Point { x: 12_000, y: 5 }]),
            [InputEvent::Point {
                x: POINT_SCALE,
                y: 5
            }]
        );
        assert_eq!(
            serde_json::to_value(InputEvent::Gesture {
                fingers: 3,
                dir: SwipeDir::Up
            })
            .unwrap_or_default(),
            json!({"kind": "gesture", "fingers": 3, "dir": "up"})
        );
        assert_eq!(InputKey::parse("esc"), Some(InputKey::Esc));
        assert_eq!(InputKey::parse("Esc"), None);
    }

    #[test]
    fn taps_and_swipes_stay_on_the_screen() {
        assert_eq!(
            clean(vec![
                InputEvent::Tap { x: 65_000, y: 20 },
                InputEvent::Swipe {
                    x: 1,
                    y: 2,
                    x2: 40_000,
                    y2: 9_999,
                    ms: 0,
                },
                InputEvent::Swipe {
                    x: 5,
                    y: 5,
                    x2: 5,
                    y2: 5,
                    ms: 60_000,
                },
            ]),
            [
                InputEvent::Tap {
                    x: POINT_SCALE,
                    y: 20
                },
                InputEvent::Swipe {
                    x: 1,
                    y: 2,
                    x2: POINT_SCALE,
                    y2: 9_999,
                    ms: 1
                },
                InputEvent::Swipe {
                    x: 5,
                    y: 5,
                    x2: 5,
                    y2: 5,
                    ms: MAX_SWIPE_MS
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(InputEvent::Swipe {
                x: 1,
                y: 2,
                x2: 3,
                y2: 4,
                ms: 250
            })
            .unwrap_or_default(),
            json!({"kind": "swipe", "x": 1, "y": 2, "x2": 3, "y2": 4, "ms": 250})
        );
        assert_eq!(
            serde_json::from_value::<InputEvent>(json!({"kind": "tap", "x": 7, "y": 8})).ok(),
            Some(InputEvent::Tap { x: 7, y: 8 })
        );
    }

    #[test]
    fn pending_moves_become_one_move_and_one_scroll() {
        let p = Pending {
            move_x: 30,
            move_y: -4,
            scroll_x: 0,
            scroll_y: 120,
        };
        assert_eq!(
            p.events(),
            [
                InputEvent::Move { dx: 30, dy: -4 },
                InputEvent::Scroll { dx: 0, dy: 120 }
            ]
        );
        assert!(Pending::default().events().is_empty());
    }
}
