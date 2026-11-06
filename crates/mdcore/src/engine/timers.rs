//! Countdown timers shared between your devices (see `features::timers`).

use super::*;
use crate::features::timers::{self, Op};
use crate::model::Timer;

impl<P: Platform> Engine<P> {
    /// The timers to show: running ones by when they end (ended ones,
    /// still ringing, first), then paused ones. Timers started or changed
    /// on your other devices arrive as `TimersChanged`.
    pub fn timers(&self) -> Vec<Timer> {
        timers::all(&self.ctx)
    }

    /// Start a timer for `duration_ms` (a second to a week) that rings on
    /// `ring_on` (node ids, this device's included if it should) when it
    /// ends. Goes to your linked devices that share timers at once, the
    /// others at their next link. Makes no event here.
    pub fn start_timer(
        &self,
        label: &str,
        duration_ms: u64,
        ring_on: &[String],
    ) -> crate::Result<Timer> {
        timers::start_timer(&self.ctx, label, duration_ms, ring_on)
    }

    /// Pause a running timer.
    pub fn pause_timer(&self, id: &str) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Pause)
    }

    /// Resume a paused timer.
    pub fn resume_timer(&self, id: &str) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Resume)
    }

    /// Cancel a timer on every device.
    pub fn cancel_timer(&self, id: &str) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Cancel)
    }

    /// A timer ended: stop it ringing on every device.
    pub fn dismiss_timer(&self, id: &str) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Dismiss)
    }

    /// Add `ms` to a timer (one that ended starts again for that long).
    pub fn add_timer_time(&self, id: &str, ms: u64) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Add(ms))
    }

    /// Start a timer again from the top.
    pub fn restart_timer(&self, id: &str) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Restart)
    }

    /// Change which devices ring when a timer ends.
    pub fn set_timer_ring(&self, id: &str, ring_on: &[String]) -> crate::Result<Timer> {
        timers::change_timer(&self.ctx, id, &Op::Ring(ring_on.to_vec()))
    }

    /// Device `node_id` said it shares timers (its app is new enough).
    pub fn timers_supported(&self, node_id: &str) -> bool {
        timers::supports(&self.ctx, node_id)
    }
}
