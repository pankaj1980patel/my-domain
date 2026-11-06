//! Share mouse and keyboard between computers (see `features::kvm`).

use super::*;
use crate::features::kvm::{self, Side};
use crate::model::{KvmEvent, KvmNeighbor, KvmState};

fn side(s: &str) -> crate::Result<Side> {
    Side::parse(s).ok_or_else(|| CoreError::InvalidInput(format!("not a side: {s}")))
}

impl<P: Platform> Engine<P> {
    /// Whether this device can share its mouse and keyboard (a computer).
    pub fn kvm_available(&self) -> bool {
        kvm::available(&self.ctx)
    }

    /// The computers placed around this one.
    pub fn kvm_layout(&self) -> Vec<KvmNeighbor> {
        kvm::layout(&self.ctx)
    }

    /// Place `node_id` at `side` ("left", "right", "top", "bottom") of this
    /// screen, or take it off (`None`). That computer is told where this
    /// one sits (the opposite side), so both agree. Returns the layout.
    pub fn kvm_set_neighbor(
        &self,
        node_id: &str,
        side_name: Option<&str>,
    ) -> crate::Result<Vec<KvmNeighbor>> {
        let s = side_name.map(side).transpose()?;
        kvm::set_neighbor(&self.ctx, node_id, s)
    }

    /// Device `node_id` shares mouse and keyboard (its app is new enough).
    pub fn kvm_supported(&self, node_id: &str) -> bool {
        kvm::supports(&self.ctx, node_id)
    }

    /// Device `node_id`'s OS ("macos", "windows", "linux"), "" if it
    /// doesn't share mouse and keyboard.
    pub fn kvm_peer_os(&self, node_id: &str) -> String {
        kvm::peer_os(&self.ctx, node_id)
    }

    /// Whether your other computers may control this one.
    pub fn kvm_allowed(&self) -> bool {
        kvm::allowed(&self.ctx)
    }

    /// Off lets go at once if another computer controls this one.
    pub fn set_kvm_allowed(&self, on: bool) {
        kvm::set_allowed(&self.ctx, on)
    }

    /// Whether the clipboard follows the active screen.
    pub fn kvm_clip_follows(&self) -> bool {
        kvm::clip_follows(&self.ctx)
    }

    pub fn set_kvm_clip_follows(&self, on: bool) {
        kvm::set_clip_follows(&self.ctx, on)
    }

    /// Who this computer's mouse and keyboard are with now.
    pub fn kvm_state(&self) -> KvmState {
        kvm::current(&self.ctx)
    }

    /// The computer at `edge` of this screen, if one is placed there.
    pub fn kvm_neighbor_at(&self, edge: &str) -> Option<String> {
        Side::parse(edge).and_then(|s| kvm::neighbor_at(&self.ctx, s))
    }

    /// The pointer went out at `edge` of this screen, `pos`
    /// ten-thousandths along it: start controlling `node_id`.
    pub fn kvm_start(&self, node_id: &str, edge: &str, pos: u16) -> crate::Result<()> {
        kvm::start(&self.ctx, node_id, side(edge)?, pos)
    }

    /// Pointer motion while controlling (adds up; goes about 125 times a
    /// second).
    pub fn kvm_motion(&self, dx: i32, dy: i32) {
        kvm::motion(&self.ctx, dx, dy)
    }

    /// Buttons, keys and wheel turns while controlling, reliably and in
    /// order.
    pub fn kvm_send(&self, events: Vec<KvmEvent>) -> crate::Result<()> {
        kvm::send(&self.ctx, events)
    }

    /// Let go, either end (`why`: "stopped" for the hotkey or Stop).
    pub fn kvm_stop(&self, why: &str) {
        kvm::stop(&self.ctx, why)
    }

    /// Controlled: the pointer was pushed out across `edge` of this screen
    /// (`pos` along it). Gives control back if that's the edge it came in
    /// at; returns whether it did.
    pub fn kvm_leave(&self, edge: &str, pos: u16) -> bool {
        Side::parse(edge).is_some_and(|s| kvm::leave(&self.ctx, s, pos))
    }
}
