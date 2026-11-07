//! The RemoteDesktop portal: pointer and keyboard events played here as if
//! from a real device, after the user allowed it once.

use zvariant::{OwnedObjectPath, Value};

use super::{Opts, Portal};
use crate::caps::{REMOTE_DESKTOP, SCREENCAST};
use crate::response::{self, Stream};

/// Keyboard (1) and pointer (2).
const DEVICES: u32 = 3;

pub struct RemoteDesktop {
    p: Portal,
    session: OwnedObjectPath,
    /// The screens shared with the session (for absolute moves), when the
    /// portal gave them.
    pub streams: Vec<Stream>,
}

impl RemoteDesktop {
    /// Ask to control this computer (and, with `screens`, to know its
    /// screens, for moving the pointer to a place). `restore`: the token
    /// from last time. Also the token to keep.
    pub fn start(
        restore: Option<&str>,
        screens: bool,
    ) -> Result<(RemoteDesktop, Option<String>), String> {
        let p = Portal::new()?;
        let version = p.version(REMOTE_DESKTOP).unwrap_or(1);
        let session = p.create_session(REMOTE_DESKTOP, Opts::new())?;
        let mut opts = Opts::new();
        opts.insert("types", Value::from(DEVICES));
        if version >= 2 {
            opts.insert("persist_mode", Value::from(2u32));
            if let Some(t) = restore.filter(|t| !t.is_empty()) {
                opts.insert("restore_token", Value::from(t.to_string()));
            }
        }
        p.on_session(REMOTE_DESKTOP, "SelectDevices", &session, opts)?;
        if screens && p.version(SCREENCAST).is_some() {
            let mut opts = Opts::new();
            opts.insert("types", Value::from(1u32));
            opts.insert("multiple", Value::from(true));
            // Without the screens, moves are relative only.
            if let Err(e) = p.on_session(SCREENCAST, "SelectSources", &session, opts) {
                eprintln!("[portal] no screens for the remote desktop: {e}");
            }
        }
        let started = p.start(REMOTE_DESKTOP, &session)?;
        let granted = response::get_u32(&started, "devices").unwrap_or(DEVICES);
        if granted & 2 == 0 {
            return Err("The pointer wasn\u{2019}t allowed.".into());
        }
        let token = response::restore_token(&started);
        let streams = response::streams(&started);
        Ok((
            RemoteDesktop {
                p,
                session,
                streams,
            },
            token,
        ))
    }

    fn notify<B>(&self, method: &str, args: B) -> bool
    where
        B: zbus::export::serde::Serialize + zvariant::DynamicType,
    {
        self.p.call(REMOTE_DESKTOP, method, &args)
    }

    pub fn motion(&self, dx: f64, dy: f64) -> bool {
        self.notify("NotifyPointerMotion", (&self.session, Opts::new(), dx, dy))
    }

    /// To (`x`, `y`) within stream `node`.
    pub fn motion_to(&self, node: u32, x: f64, y: f64) -> bool {
        self.notify(
            "NotifyPointerMotionAbsolute",
            (&self.session, Opts::new(), node, x, y),
        )
    }

    /// An evdev button (BTN_LEFT...).
    pub fn button(&self, code: u16, down: bool) -> bool {
        self.notify(
            "NotifyPointerButton",
            (&self.session, Opts::new(), i32::from(code), u32::from(down)),
        )
    }

    /// Wheel clicks: axis 0 vertical (positive down), 1 horizontal.
    pub fn wheel(&self, axis: u32, steps: i32) -> bool {
        self.notify(
            "NotifyPointerAxisDiscrete",
            (&self.session, Opts::new(), axis, steps),
        )
    }

    /// An evdev key.
    pub fn key(&self, code: u16, down: bool) -> bool {
        self.notify(
            "NotifyKeyboardKeycode",
            (&self.session, Opts::new(), i32::from(code), u32::from(down)),
        )
    }

    /// An X keysym (typed text).
    pub fn keysym(&self, sym: u32, down: bool) -> bool {
        self.notify(
            "NotifyKeyboardKeysym",
            (&self.session, Opts::new(), sym as i32, u32::from(down)),
        )
    }
}
