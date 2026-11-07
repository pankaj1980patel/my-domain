//! Which kind of graphical session this is, from the environment.

/// The graphical session this app runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
    /// No display at all (a console, ssh).
    None,
}

impl Session {
    /// From `XDG_SESSION_TYPE`, `WAYLAND_DISPLAY` and `DISPLAY`. The
    /// session type wins; without it, a Wayland socket means Wayland
    /// (XWayland sets `DISPLAY` too), else `DISPLAY` means X11.
    pub fn detect(
        session_type: Option<&str>,
        wayland_display: Option<&str>,
        display: Option<&str>,
    ) -> Session {
        let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
        match session_type
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("wayland") => return Session::Wayland,
            Some("x11") => return Session::X11,
            _ => {}
        }
        if set(wayland_display) {
            Session::Wayland
        } else if set(display) {
            Session::X11
        } else {
            Session::None
        }
    }

    /// This process's session.
    pub fn current() -> Session {
        let var = |k: &str| std::env::var(k).ok();
        Session::detect(
            var("XDG_SESSION_TYPE").as_deref(),
            var("WAYLAND_DISPLAY").as_deref(),
            var("DISPLAY").as_deref(),
        )
    }

    /// Running under Wayland (Linux only).
    pub fn wayland() -> bool {
        cfg!(target_os = "linux") && Session::current() == Session::Wayland
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    #[test]
    fn the_session_type_wins_then_the_sockets() {
        let d = Session::detect;
        assert_eq!(d(Some("wayland"), None, Some(":0")), Session::Wayland);
        assert_eq!(d(Some("Wayland"), None, None), Session::Wayland);
        assert_eq!(d(Some("x11"), Some("wayland-0"), Some(":0")), Session::X11);
        // tty / unset: what the sockets say; XWayland's DISPLAY doesn't
        // make it X11.
        assert_eq!(
            d(Some("tty"), Some("wayland-0"), Some(":0")),
            Session::Wayland
        );
        assert_eq!(d(None, None, Some(":1")), Session::X11);
        assert_eq!(d(None, Some(" "), Some("")), Session::None);
        assert_eq!(d(None, None, None), Session::None);
    }
}
