//! Per-feature platform capability traits. A platform implements only the ones
//! it supports; the Engine/Services query for them and degrade gracefully when
//! absent (e.g. android has no clipboard today).
//!
//! The AV / input / screen traits are seams for the heavy roadmap services
//! (audio-video calls, remote control). They are defined now so those services
//! can be written against a stable interface; their bodies arrive later and are
//! gated behind the `av` / `remote-control` cargo features.

/// System clipboard access. Desktop: arboard. Android: not yet implemented.
pub trait Clipboard: Send + Sync {
    fn get(&self) -> Option<String>;
    fn set(&self, text: &str);
    /// Whether what's on the clipboard now must stay on this device: a
    /// password copied from a password manager (marked concealed by the OS
    /// convention), or a copy from an app the user excluded. Such a value is
    /// neither synced, kept in the history, nor handed to a device pulling
    /// the clipboard. Asked only when the value changed. Default: never.
    fn keep_private(&self) -> bool {
        false
    }
}

/// Post a local OS notification (the receive side of notification sharing).
pub trait Notifier: Send + Sync {
    fn post(&self, title: &str, body: &str);
}

/// Microphone capture + speaker playback (audio calls). Roadmap seam.
pub trait AudioIo: Send + Sync {}

/// Camera capture + frame display (video calls). Roadmap seam.
pub trait VideoIo: Send + Sync {}

/// Screen capture source (remote control / screen share). Roadmap seam.
pub trait ScreenSource: Send + Sync {}

/// Inject keyboard/mouse/touch events (remote control). Roadmap seam.
pub trait InputSink: Send + Sync {}
