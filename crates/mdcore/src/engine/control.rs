//! The phone as a touchpad and keyboard, and Do Not Disturb following the
//! phone (see `features::input`, `features::dnd`).

use super::*;
use crate::model::{InputEvent, InputKey, MouseButton};

impl<P: Platform> Engine<P> {
    // --- touchpad and keyboard: the phone's side ---

    /// Move a computer's pointer by `dx`, `dy` pixels. Moves add up and go
    /// out together every `features::input::BATCH_EVERY`, so call this for
    /// every touch move. Fails with `NoLink` (and starts connecting) when
    /// there's no link to the computer: input is never queued.
    pub fn input_move(&self, node_id: &str, dx: i32, dy: i32) -> crate::Result<()> {
        crate::features::input::add(&self.ctx, node_id, dx, dy, false)
    }

    /// Scroll on a computer by `dx`, `dy` pixels (positive `dy` scrolls the
    /// content down, like dragging two fingers up). Batched like moves.
    pub fn input_scroll(&self, node_id: &str, dx: i32, dy: i32) -> crate::Result<()> {
        crate::features::input::add(&self.ctx, node_id, dx, dy, true)
    }

    /// Click `button` ("left" or "right") on a computer.
    pub fn input_click(&self, node_id: &str, button: &str) -> crate::Result<()> {
        let button = MouseButton::parse(button)
            .ok_or_else(|| CoreError::InvalidInput(format!("not a mouse button: {button}")))?;
        self.send_input(node_id, vec![InputEvent::Click { button }])
    }

    /// Type `text` on a computer (control characters are left out).
    pub fn input_text(&self, node_id: &str, text: &str) -> crate::Result<()> {
        self.send_input(
            node_id,
            vec![InputEvent::Text {
                text: text.to_string(),
            }],
        )
    }

    /// Press `key` on a computer: "enter", "backspace", "left", "right",
    /// "up", "down" or "esc".
    pub fn input_key(&self, node_id: &str, key: &str) -> crate::Result<()> {
        let key = InputKey::parse(key)
            .ok_or_else(|| CoreError::InvalidInput(format!("not a key: {key}")))?;
        self.send_input(node_id, vec![InputEvent::Key { key }])
    }

    /// Send input events to a computer now, after any pending moves and
    /// scrolls. It plays them back if it allows remote input (it lists
    /// "input" in its status actions then).
    pub fn send_input(&self, node_id: &str, events: Vec<InputEvent>) -> crate::Result<()> {
        crate::features::input::send(&self.ctx, node_id, events)
    }

    // --- touchpad and keyboard: the computer's side ---

    /// Whether this device plays back input from your other devices
    /// (`CoreEvent::Input`). On until turned off; kept across restarts.
    pub fn remote_input_allowed(&self) -> bool {
        crate::features::input::allowed(&self.ctx)
    }

    pub fn set_remote_input_allowed(&self, on: bool) {
        crate::features::input::set_allowed(&self.ctx, on);
    }

    // --- Do Not Disturb ---

    /// This device's Do Not Disturb state, as the host reads it from the
    /// OS. Your other devices are told when it changes (`CoreEvent::Dnd`).
    pub fn set_dnd(&self, on: bool) {
        crate::features::dnd::set(&self.ctx, on);
    }

    /// Your devices that are in Do Not Disturb now (node ids).
    pub fn dnd_active(&self) -> Vec<String> {
        crate::features::dnd::active(&self.ctx)
    }
}
