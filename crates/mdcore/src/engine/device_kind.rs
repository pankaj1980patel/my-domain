//! What kind of device each of your devices is (see `features::device_kind`).

use std::collections::BTreeMap;

use super::*;
use crate::features::device_kind;
use crate::model::DeviceKind;

impl<P: Platform> Engine<P> {
    /// This device's own kind, as the app found it (OS, form, model,
    /// maker; the icon is worked out). Said in its status, `addr_update`
    /// and registry entry; linked devices hear a change at once.
    pub fn set_device_kind(&self, kind: DeviceKind) {
        device_kind::set_own(&self.ctx, kind);
    }

    /// This device's own kind as set (with its icon); `None` until set.
    pub fn own_device_kind(&self) -> Option<DeviceKind> {
        device_kind::own(&self.ctx)
    }

    /// What `node_id` is: what it said, a guess for an older app, else
    /// the `"unknown"` icon.
    pub fn device_kind(&self, node_id: &str) -> DeviceKind {
        device_kind::of(&self.ctx, node_id)
    }

    /// Every device's kind that's known, by node id.
    pub fn device_kinds(&self) -> BTreeMap<String, DeviceKind> {
        device_kind::all(&self.ctx)
    }
}
