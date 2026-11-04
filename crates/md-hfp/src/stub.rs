//! Neither Linux nor Windows: the same API as the Linux [`Hfp`], refusing
//! everything, so code written against it type-checks on any machine.

use std::sync::Arc;

use crate::{Gateway, HfEvent, ScoPcm};

const UNSUPPORTED: &str = "Bluetooth calls need Linux or Windows.";

/// See the Linux version (`linux/mod.rs`); this one never starts.
pub struct Hfp {
    _never: (),
}

impl Hfp {
    pub fn start(on_event: impl Fn(HfEvent) + Send + 'static) -> Result<Hfp, String> {
        drop(on_event);
        Err(UNSUPPORTED.into())
    }

    pub fn problem(&self) -> Option<String> {
        None
    }

    pub fn gateways(&self) -> Result<Vec<Gateway>, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn connect(&self, _address: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn answer(&self, _audio_here: bool) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn hang_up(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn dial(&self, _number: &str) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn redial(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn dtmf(&self, _c: char) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn list_calls(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn audio_here(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn audio_to_phone(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn connect_audio(&self) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub fn audio(&self) -> Option<Arc<ScoPcm>> {
        None
    }

    pub fn device_name(&self) -> String {
        String::new()
    }

    pub fn cached_name(&self, _address: &str) -> Option<String> {
        None
    }
}
