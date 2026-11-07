//! Wayland on Linux, through the XDG desktop portals
//! (`org.freedesktop.portal.Desktop` on the session D-Bus).
//!
//! On Wayland an app can't read the screen, move the pointer, type, watch
//! the pointer reach a screen edge or grab keys for itself; the compositor
//! offers each through a portal, behind a prompt the user answers once:
//!
//! - screen: `ScreenCast` (a PipeWire stream, read by a `gst-launch-1.0
//!   pipewiresrc` child process that hands back JPEGs: no PipeWire crate),
//!   else `Screenshot` stills; a restore token skips the prompt next time;
//! - input from a phone or another computer: `RemoteDesktop`
//!   (NotifyPointerMotion / Button / Axis / KeyboardKeycode), with its
//!   screens for absolute moves; else `/dev/uinput` when this user may
//!   write it (never asks for root);
//! - sending this computer's mouse and keyboard to another one:
//!   `InputCapture` (xdg-desktop-portal 1.19+, GNOME 45+ / KDE Plasma 6):
//!   pointer barriers on the screen edges, the captured input read from
//!   the EIS socket the portal hands over (`eis`, a small receiver of the
//!   libei protocol: no libei crate); without it, "receive only";
//! - keyboard shortcuts: `GlobalShortcuts`.
//!
//! The parts that don't touch the bus, the socket or a process (results,
//! capabilities, command lines, the EIS wire format, key names, geometry)
//! are plain functions, tested on every OS; the Linux side stays thin.

pub mod caps;
pub mod clip;
pub mod eis;
pub mod geom;
pub mod gst;
pub mod keys;
#[cfg(any(target_os = "linux", test))]
pub mod response;
pub mod session;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub mod uinput;

pub use caps::{InputWay, Portals, ScreenWay, Tools};
pub use geom::{Edge, Rect};
pub use session::Session;

#[cfg(target_os = "linux")]
pub use linux::{
    capture::{CaptureSession, Captured},
    inject::Injector,
    portals,
    screen::{screenshot, ScreenStream},
    service,
    shortcuts::{Shortcuts, Wanted},
    tools, CANCELLED,
};
