//! What this Wayland session offers: the portals (and their versions) and
//! the helper programs, and what each feature can use because of them.

/// The portal service and object.
pub const DEST: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";

pub const SCREENSHOT: &str = "org.freedesktop.portal.Screenshot";
pub const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
pub const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
pub const INPUT_CAPTURE: &str = "org.freedesktop.portal.InputCapture";
pub const GLOBAL_SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";

/// Every interface asked about, in the order of `Portals`' fields.
pub const INTERFACES: [&str; 5] = [
    SCREENSHOT,
    SCREENCAST,
    REMOTE_DESKTOP,
    INPUT_CAPTURE,
    GLOBAL_SHORTCUTS,
];

/// The portals present, each with its `version` property (none: absent).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Portals {
    pub screenshot: Option<u32>,
    pub screencast: Option<u32>,
    pub remote_desktop: Option<u32>,
    pub input_capture: Option<u32>,
    pub global_shortcuts: Option<u32>,
}

impl Portals {
    /// From (interface, version) pairs, as read from the portal object.
    /// A version of 0 counts as absent (a backend that doesn't implement
    /// it answers so on some systems).
    pub fn from_versions<'a>(found: impl IntoIterator<Item = (&'a str, u32)>) -> Portals {
        let mut p = Portals::default();
        for (iface, v) in found {
            let v = (v > 0).then_some(v);
            match iface {
                SCREENSHOT => p.screenshot = v,
                SCREENCAST => p.screencast = v,
                REMOTE_DESKTOP => p.remote_desktop = v,
                INPUT_CAPTURE => p.input_capture = v,
                GLOBAL_SHORTCUTS => p.global_shortcuts = v,
                _ => {}
            }
        }
        p
    }

    /// The screen cast can be remembered (`persist_mode`, restore token):
    /// version 4 or later.
    pub fn screencast_remembers(&self) -> bool {
        self.screencast.is_some_and(|v| v >= 4)
    }

    /// The remote desktop can be remembered: version 2 or later.
    pub fn remote_remembers(&self) -> bool {
        self.remote_desktop.is_some_and(|v| v >= 2)
    }
}

/// Helper programs and devices found here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    /// PipeWire's socket is there (`$XDG_RUNTIME_DIR/pipewire-0`).
    pub pipewire: bool,
    /// `gst-launch-1.0` with the `pipewiresrc` and `jpegenc` elements.
    pub gst_pipewire: bool,
    /// `grim` (wlroots compositors: Sway, Hyprland...).
    pub grim: bool,
    /// `wl-copy` / `wl-paste`.
    pub wl_clipboard: bool,
    /// This user may write `/dev/uinput`.
    pub uinput: bool,
    /// Something shows tray icons (`org.kde.StatusNotifierWatcher`).
    pub tray: bool,
}

/// How this screen is shared with a phone on Wayland, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenWay {
    /// The ScreenCast portal, read by GStreamer: about two pictures a
    /// second, asked once.
    Stream,
    /// `grim` (wlroots compositors only).
    Grim,
    /// The Screenshot portal: one picture every couple of seconds.
    Stills,
}

/// How input from another device is played here on Wayland.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputWay {
    /// The RemoteDesktop portal.
    Portal,
    /// `/dev/uinput`.
    Uinput,
}

/// The way to share the screen, or what's missing (`"gst_pipewire"`: the
/// screen cast is there but nothing reads it; `"portal"`: no portal for
/// the screen at all).
pub fn screen_way(p: &Portals, t: &Tools) -> Result<ScreenWay, &'static str> {
    if p.screencast.is_some() && t.gst_pipewire && t.pipewire {
        Ok(ScreenWay::Stream)
    } else if t.grim {
        Ok(ScreenWay::Grim)
    } else if p.screenshot.is_some() {
        Ok(ScreenWay::Stills)
    } else if p.screencast.is_some() {
        Err("gst_pipewire")
    } else {
        Err("portal")
    }
}

/// The way to play input, or `"wayland_input"` (neither the portal nor
/// uinput).
pub fn input_way(p: &Portals, t: &Tools) -> Result<InputWay, &'static str> {
    if p.remote_desktop.is_some() {
        Ok(InputWay::Portal)
    } else if t.uinput {
        Ok(InputWay::Uinput)
    } else {
        Err("wayland_input")
    }
}

/// This computer can send its own mouse and keyboard to another one
/// (not only be controlled).
pub fn can_send_input(p: &Portals) -> bool {
    p.input_capture.is_some()
}

/// One health line: a portal, a program or a device, and how it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// "linux_screencast", "linux_gst_pipewire", ...
    pub id: &'static str,
    /// "granted" (there) or "denied" (missing).
    pub state: &'static str,
    /// The feature groups that need it (empty: everything).
    pub features: &'static [&'static str],
}

fn line(id: &'static str, there: bool, features: &'static [&'static str]) -> Line {
    Line {
        id,
        state: if there { "granted" } else { "denied" },
        features,
    }
}

/// The health check's Wayland lines. Only what matters: the uinput line
/// only when the remote desktop portal is missing, `grim` only when the
/// screen cast can't be read.
pub fn health_lines(p: &Portals, t: &Tools) -> Vec<Line> {
    const MEDIA: &[&str] = &["media"];
    const CLIP: &[&str] = &["clipboard"];
    let mut out = vec![
        line("linux_screencast", p.screencast.is_some(), MEDIA),
        line("linux_pipewire", t.pipewire, MEDIA),
        line("linux_gst_pipewire", t.gst_pipewire, MEDIA),
        line("linux_remote_desktop", p.remote_desktop.is_some(), MEDIA),
    ];
    if p.remote_desktop.is_none() {
        out.push(line("linux_uinput", t.uinput, MEDIA));
    }
    out.push(line(
        "linux_input_capture",
        p.input_capture.is_some(),
        MEDIA,
    ));
    out.push(line(
        "linux_global_shortcuts",
        p.global_shortcuts.is_some(),
        &[],
    ));
    out.push(line("linux_wl_clipboard", t.wl_clipboard, CLIP));
    out.push(line("linux_tray", t.tray, &[]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gnome46() -> Portals {
        Portals::from_versions([
            (SCREENSHOT, 2),
            (SCREENCAST, 5),
            (REMOTE_DESKTOP, 2),
            (INPUT_CAPTURE, 1),
            ("org.freedesktop.portal.Settings", 2),
        ])
    }

    #[test]
    fn versions_become_portals_and_zero_is_absent() {
        let p = gnome46();
        assert_eq!(p.screencast, Some(5));
        assert_eq!(p.input_capture, Some(1));
        assert_eq!(p.global_shortcuts, None);
        assert!(p.screencast_remembers() && p.remote_remembers());
        let old = Portals::from_versions([(SCREENCAST, 3), (REMOTE_DESKTOP, 1), (SCREENSHOT, 0)]);
        assert!(!old.screencast_remembers() && !old.remote_remembers());
        assert_eq!(old.screenshot, None);
    }

    #[test]
    fn the_screen_uses_the_best_way_there_is() {
        let all = Tools {
            pipewire: true,
            gst_pipewire: true,
            ..Tools::default()
        };
        assert_eq!(screen_way(&gnome46(), &all), Ok(ScreenWay::Stream));
        // No GStreamer plugin: Sway with grim, else stills, else say what's
        // missing.
        let grim = Tools {
            grim: true,
            ..Tools::default()
        };
        assert_eq!(screen_way(&gnome46(), &grim), Ok(ScreenWay::Grim));
        assert_eq!(
            screen_way(&gnome46(), &Tools::default()),
            Ok(ScreenWay::Stills)
        );
        let cast_only = Portals::from_versions([(SCREENCAST, 4)]);
        assert_eq!(
            screen_way(&cast_only, &Tools::default()),
            Err("gst_pipewire")
        );
        assert_eq!(
            screen_way(&Portals::default(), &Tools::default()),
            Err("portal")
        );
    }

    #[test]
    fn input_prefers_the_portal_then_uinput() {
        let t = Tools {
            uinput: true,
            ..Tools::default()
        };
        assert_eq!(input_way(&gnome46(), &t), Ok(InputWay::Portal));
        assert_eq!(input_way(&Portals::default(), &t), Ok(InputWay::Uinput));
        assert_eq!(
            input_way(&Portals::default(), &Tools::default()),
            Err("wayland_input")
        );
        assert!(can_send_input(&gnome46()));
        assert!(!can_send_input(&Portals::from_versions([(
            REMOTE_DESKTOP,
            2
        )])));
    }

    #[test]
    fn health_shows_each_portal_and_tool() {
        let lines = health_lines(&gnome46(), &Tools::default());
        let get = |id: &str| lines.iter().find(|l| l.id == id).map(|l| l.state);
        assert_eq!(get("linux_screencast"), Some("granted"));
        assert_eq!(get("linux_gst_pipewire"), Some("denied"));
        assert_eq!(get("linux_input_capture"), Some("granted"));
        assert_eq!(get("linux_global_shortcuts"), Some("denied"));
        // The portal is there: uinput doesn't matter.
        assert_eq!(get("linux_uinput"), None);
        let none = health_lines(&Portals::default(), &Tools::default());
        assert!(none.iter().any(|l| l.id == "linux_uinput"));
        assert_eq!(
            none.iter()
                .find(|l| l.id == "linux_wl_clipboard")
                .map(|l| l.features),
            Some(&["clipboard"][..])
        );
    }
}
