//! The phone as a touchpad and keyboard, and Do Not Disturb following the
//! phone (see `features::input`, `features::dnd`).

use super::*;
use crate::model::{InputEvent, InputKey, MouseButton, SwipeDir};

impl<P: Platform> Engine<P> {
    // --- touchpad and keyboard: the phone's side ---

    /// Move a computer's pointer by `dx`, `dy` pixels. Moves add up and go
    /// out together every `features::input::BATCH_EVERY`, so call this for
    /// every touch move. Fails with `NoLink` (and starts connecting) when
    /// there's no link to the computer: input is never queued.
    pub fn input_move(&self, node_id: &str, dx: i32, dy: i32) -> crate::Result<()> {
        crate::features::input::add(&self.ctx, node_id, dx, dy, false)
    }

    /// Scroll on a computer by `dx`, `dy` pixels: positive `dy` scrolls
    /// down (towards the end of the page, as dragging two fingers up does on
    /// a touchpad), positive `dx` to the right. Batched like moves.
    pub fn input_scroll(&self, node_id: &str, dx: i32, dy: i32) -> crate::Result<()> {
        crate::features::input::add(&self.ctx, node_id, dx, dy, true)
    }

    /// Click `button` ("left", "right" or "middle") on a computer.
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

    /// Put a computer's pointer at a spot on its main screen: `x`, `y` in
    /// ten-thousandths of its width and height from the top left (see
    /// `features::input::POINT_SCALE`), e.g. where you tapped on a picture
    /// of the screen.
    pub fn input_point(&self, node_id: &str, x: u16, y: u16) -> crate::Result<()> {
        self.send_input(node_id, vec![InputEvent::Point { x, y }])
    }

    /// A three- or four-finger swipe on a computer: `dir` is "left",
    /// "right", "up" or "down", the way the fingers moved. The computer
    /// turns it into its own shortcut (see the desktop's `remote_input`).
    pub fn input_gesture(&self, node_id: &str, fingers: u8, dir: &str) -> crate::Result<()> {
        let dir = SwipeDir::parse(dir)
            .ok_or_else(|| CoreError::InvalidInput(format!("not a direction: {dir}")))?;
        if !(3..=4).contains(&fingers) {
            return Err(CoreError::InvalidInput(format!(
                "gestures use 3 or 4 fingers, not {fingers}"
            )));
        }
        self.send_input(node_id, vec![InputEvent::Gesture { fingers, dir }])
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

    // --- waking a sleeping computer ---

    /// Whether `node` said it may be woken (Wake-on-LAN; see
    /// `features::wake`). Kept across restarts.
    pub fn can_wake(&self, node_id: &str) -> bool {
        !crate::features::wake::known(&self.ctx, node_id).is_empty()
    }

    /// Wake `node` from sleep: broadcasts its magic packet from here, and
    /// asks each other linked device of yours on its network (that says it
    /// can) to send it too. Works on a computer set to wake for network
    /// access. Fails when it never said it may be woken, or nothing could
    /// be sent.
    pub fn wake_device(&self, node_id: &str) -> crate::Result<crate::model::WakeReport> {
        crate::features::wake::wake(&self.ctx, node_id)
    }

    // --- screen sharing ---

    /// Start (`on`) or stop watching `node`'s screen: it sends snapshots
    /// (`CoreEvent::ScreenFrame`) while it allows it. A watch ends by itself
    /// after `features::screen::WATCH_FOR`, so renew it every
    /// `features::screen::RENEW_EVERY` while the screen shows. Over a live
    /// link only (fails with `NoLink` and starts connecting otherwise).
    pub fn screen_watch(&self, node_id: &str, on: bool) -> crate::Result<()> {
        crate::features::screen::watch(&self.ctx, node_id, on)
    }

    /// Whether your other devices may watch this screen. On until turned
    /// off; kept across restarts. Turning it off ends every watch.
    pub fn screen_allowed(&self) -> bool {
        crate::features::screen::allowed(&self.ctx)
    }

    pub fn set_screen_allowed(&self, on: bool) {
        crate::features::screen::set_allowed(&self.ctx, on);
    }

    /// The devices watching this screen now: while there are any, take
    /// snapshots and send them with `screen_frame`.
    pub fn screen_watchers(&self) -> Vec<String> {
        crate::features::screen::watchers(&self.ctx)
    }

    /// Send a snapshot of this screen (a JPEG, at most
    /// `features::screen::MAX_FRAME_BYTES`) to everyone watching; returns
    /// how many it went to.
    pub fn screen_frame(&self, jpeg: &[u8]) -> crate::Result<usize> {
        crate::features::screen::send_frame(&self.ctx, jpeg)
    }

    /// `screen_frame`, to those of `nodes` watching only: a phone shares
    /// its screen with the computers its owner chose.
    pub fn screen_frame_to(&self, jpeg: &[u8], nodes: &[String]) -> crate::Result<usize> {
        crate::features::screen::send_frame_to(&self.ctx, jpeg, Some(nodes))
    }

    /// Tell computer `node` this device shares its screen with it (`on`;
    /// it starts watching, `CoreEvent::ScreenOffer` there), or stopped.
    /// Only to a computer whose status says `"screen_view": true` (older
    /// ones would show it as a chat message). Over a live link only.
    pub fn screen_offer(&self, node_id: &str, on: bool) -> crate::Result<()> {
        crate::features::screen::offer(&self.ctx, node_id, on)
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

    // --- one device's audio on another (features::audio_share) ---
    // A phone's audio on a computer, or a computer's on a phone: the same
    // calls both ways round. The sink plays, the source sends.

    /// This device plays another's audio (`sink`) and/or sends its own
    /// (`source`). Said in its status (`audio_sink`, `audio_source`); off
    /// until set.
    pub fn set_audio_roles(&self, sink: bool, source: bool) {
        crate::features::audio_share::set_roles(&self.ctx, sink, source);
    }

    /// What `node` last said in its status: (it plays another device's
    /// audio, it sends its own). Kept across restarts.
    pub fn audio_caps(&self, node_id: &str) -> (bool, bool) {
        crate::features::audio_share::caps_of(&self.ctx, node_id)
    }

    /// Sink: ask source `node` to play its audio here (`on`; renew every
    /// `features::audio_share::RENEW_EVERY` while playing, a listen ends
    /// by itself after `LISTEN_FOR`), or to stop (also the answer "no" to
    /// its offer). Its frames arrive as `CoreEvent::AudioFrame` while this
    /// listens. Only to a device whose status says `audio_source`; over a
    /// live link only.
    pub fn audio_listen(&self, node_id: &str, on: bool) -> crate::Result<()> {
        crate::features::audio_share::listen(&self.ctx, node_id, on)
    }

    /// Source: the sinks listening to this device's audio now.
    pub fn audio_listeners(&self) -> Vec<String> {
        crate::features::audio_share::listeners(&self.ctx)
    }

    /// Source: tell sink `node` this device plays its audio to it (`on`;
    /// `CoreEvent::AudioOffer` there, and it listens once its owner agrees),
    /// or stopped (also the answer when its owner said no). Returns the
    /// rate to capture at for the link (48 kHz direct, 24 kHz relayed; 0
    /// when stopping); fails over a Bluetooth-only link. Only to a device
    /// whose status says `audio_sink`.
    pub fn audio_offer(&self, node_id: &str, on: bool) -> crate::Result<u32> {
        crate::features::audio_share::offer(&self.ctx, node_id, on)
    }

    /// Source: send captured audio (s16le PCM, `channels` interleaved, at
    /// `rate` Hz, at most `features::audio_share::MAX_FRAME_BYTES`) to
    /// those of `nodes` listening; returns how many it went to.
    pub fn audio_frame_to(
        &self,
        pcm: &[u8],
        rate: u32,
        channels: u8,
        nodes: &[String],
    ) -> crate::Result<usize> {
        crate::features::audio_share::send_frame_to(&self.ctx, pcm, rate, channels, nodes)
    }
}
