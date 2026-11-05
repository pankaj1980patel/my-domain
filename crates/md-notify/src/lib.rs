//! This computer's own app notifications, read as they're shown, so the
//! desktop app can pass them to a phone (the reverse of the phone's
//! notification mirror).
//!
//! - Linux: the session D-Bus. A connection becomes a *monitor*
//!   (`org.freedesktop.DBus.Monitoring.BecomeMonitor`, what `dbus-monitor`
//!   does; allowed for your own session) for just the notification traffic:
//!   `org.freedesktop.Notifications` `Notify` calls and their replies (the
//!   reply carries the id), `CloseNotification` and `NotificationClosed`,
//!   and GNOME's `org.gtk.Notifications` `AddNotification` /
//!   `RemoveNotification`. A dismiss from the phone closes a freedesktop
//!   notification with `CloseNotification`, only while it's still on screen.
//! - Windows: `UserNotificationListener` needs the User Notification
//!   Listener capability, declared in a packaged app's manifest; this app
//!   isn't packaged, so nothing here yet.
//! - macOS: there's no public way to read other apps' notifications.
//!
//! Only while asked to: nothing runs until [`Watcher::start`], and dropping
//! the watcher stops it. The parts that don't touch the bus are in
//! [`parse`], tested on every OS.

pub mod parse;

#[cfg(target_os = "linux")]
mod linux;

pub use parse::{Change, Seen};

/// This computer's notifications can be read here at all (Linux).
pub const SUPPORTED: bool = cfg!(target_os = "linux");

/// Reads notifications as they come and go, until dropped.
pub struct Watcher {
    #[cfg(target_os = "linux")]
    inner: linux::Watcher,
    #[cfg(not(target_os = "linux"))]
    _never: std::convert::Infallible,
}

impl Watcher {
    /// Start reading. `skip`: app names (or desktop file / GNOME app ids)
    /// never passed on, this app's own. `on_change` runs on the watcher's
    /// thread. The error is a sentence for the settings screen.
    pub fn start(
        skip: Vec<String>,
        on_change: impl FnMut(Change) + Send + 'static,
    ) -> Result<Watcher, String> {
        #[cfg(target_os = "linux")]
        {
            linux::Watcher::start(skip, on_change).map(|inner| Watcher { inner })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (skip, on_change);
            Err("reading this computer's notifications isn't possible here".into())
        }
    }

    /// Close the notification `key` on this computer, if it's still on
    /// screen and can be closed from here (freedesktop ones). Blocking (a
    /// D-Bus call). True when asked.
    pub fn dismiss(&self, key: &str) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.inner.dismiss(key)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = key;
            match self._never {}
        }
    }
}
