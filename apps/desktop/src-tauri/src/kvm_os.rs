// The OS side of "Share mouse and keyboard" (`kvm.rs`), kept thin: the
// screens and the pointer, moving the pointer, and capturing this
// computer's own mouse and keyboard while it controls another one.
//
//  - macOS: an event tap (CGEventTap, active, at the session level) that
//    swallows every mouse and keyboard event while it runs, the pointer
//    held where it is (CGAssociateMouseAndMouseCursorPosition) and hidden;
//    motion comes from the events' deltas. Needs Accessibility; without it
//    the tap can't be made and this says so (never asks macOS to grant it);
//  - Windows: low-level hooks (WH_MOUSE_LL, WH_KEYBOARD_LL) on their own
//    thread with a message loop; every event is swallowed, motion is how
//    far the pointer would have moved from where it's parked;
//  - Linux X11: grabs of the pointer (with an invisible cursor) and the
//    keyboard on the root window; motion is read from where the pointer
//    went and it's warped back;
//  - Linux Wayland (`md_portal`): an app can't read or move the pointer
//    there. Being controlled goes through the RemoteDesktop portal (its
//    screens place the pointer; where it is is followed from the moves
//    made), else /dev/uinput. Sending needs the InputCapture portal
//    (GNOME 45+, KDE Plasma 6): `kvm.rs` watches its edge barriers, and a
//    capture here hands over what the portal's EIS socket delivers;
//    without it this computer is "receive only".
//
// Dropping the `Capture` always lets go (and shows the pointer again).

#[cfg(not(target_os = "macos"))]
use tauri::AppHandle;

/// A screen, in the desktop's coordinates (pixels or, on a Mac, points;
/// the same units as `cursor` and `warp`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What the capture caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Captured {
    Motion {
        dx: i32,
        dy: i32,
    },
    /// 1 left, 2 right, 3 middle, 4 back, 5 forward.
    Button {
        button: u8,
        down: bool,
    },
    /// Steps; `dy` positive scrolls down.
    Wheel {
        dx: i32,
        dy: i32,
    },
    /// A HID usage (`kvm_keys`).
    Key {
        hid: u16,
        down: bool,
    },
}

/// The screens from Tauri (Windows and Linux: physical pixels, as the
/// pointer is there).
#[cfg(not(target_os = "macos"))]
fn tauri_screens(app: &AppHandle) -> Vec<Rect> {
    app.available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| Rect {
                    x: f64::from(m.position().x),
                    y: f64::from(m.position().y),
                    w: f64::from(m.size().width),
                    h: f64::from(m.size().height),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub use os::{cursor, receive_only, screens, start_capture, unsupported, warp, Capture};

#[cfg(target_os = "macos")]
mod os {
    use super::{Captured, Rect};
    use crate::kvm_keys;
    use core_foundation::base::TCFType;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::display::CGDisplay;
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventType, CallbackResult, EventField,
    };
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    use core_graphics::geometry::CGPoint;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use tauri::AppHandle;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapEnable(tap: *mut std::ffi::c_void, enable: bool);
    }

    /// The tap's port, to turn it back on when macOS turns it off.
    static TAP: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    pub fn unsupported() -> Option<&'static str> {
        None
    }

    pub fn receive_only() -> bool {
        false
    }

    pub fn screens(_app: &AppHandle) -> Vec<Rect> {
        CGDisplay::active_displays()
            .unwrap_or_default()
            .into_iter()
            .map(|id| {
                let b = CGDisplay::new(id).bounds();
                Rect {
                    x: b.origin.x,
                    y: b.origin.y,
                    w: b.size.width,
                    h: b.size.height,
                }
            })
            .collect()
    }

    pub fn cursor() -> Option<(f64, f64)> {
        let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
        let p = CGEvent::new(src).ok()?.location();
        Some((p.x, p.y))
    }

    pub fn warp(x: f64, y: f64) {
        let _ = CGDisplay::warp_mouse_cursor_position(CGPoint::new(x, y));
        // A warp holds the pointer still for a moment; this lets it go.
        let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
    }

    pub struct Capture {
        run_loop: CFRunLoop,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            self.run_loop.stop();
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            TAP.store(std::ptr::null_mut(), Ordering::Release);
            let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(true);
            let _ = CGDisplay::main().show_cursor();
        }
    }

    /// The modifier keys' codes, toggled by "flags changed".
    fn modifier(code: u16) -> bool {
        matches!(code, 0x36..=0x3E)
    }

    fn caught(ty: CGEventType, e: &CGEvent, held: &Mutex<Vec<u16>>) -> Vec<Captured> {
        let int = |f| e.get_integer_value_field(f);
        let clamp = |v: i64| i32::try_from(v).unwrap_or(0);
        match ty {
            CGEventType::MouseMoved
            | CGEventType::LeftMouseDragged
            | CGEventType::RightMouseDragged
            | CGEventType::OtherMouseDragged => {
                let (dx, dy) = (
                    clamp(int(EventField::MOUSE_EVENT_DELTA_X)),
                    clamp(int(EventField::MOUSE_EVENT_DELTA_Y)),
                );
                if dx == 0 && dy == 0 {
                    vec![]
                } else {
                    vec![Captured::Motion { dx, dy }]
                }
            }
            CGEventType::LeftMouseDown => vec![Captured::Button {
                button: 1,
                down: true,
            }],
            CGEventType::LeftMouseUp => vec![Captured::Button {
                button: 1,
                down: false,
            }],
            CGEventType::RightMouseDown => vec![Captured::Button {
                button: 2,
                down: true,
            }],
            CGEventType::RightMouseUp => vec![Captured::Button {
                button: 2,
                down: false,
            }],
            CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => {
                let down = matches!(ty, CGEventType::OtherMouseDown);
                // 2 middle, 3 back, 4 forward (0-based).
                let button = match int(EventField::MOUSE_EVENT_BUTTON_NUMBER) {
                    2 => 3,
                    3 => 4,
                    4 => 5,
                    _ => return vec![],
                };
                vec![Captured::Button { button, down }]
            }
            CGEventType::ScrollWheel => {
                let dy = -clamp(int(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1));
                let dx = -clamp(int(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2));
                if dx == 0 && dy == 0 {
                    vec![]
                } else {
                    vec![Captured::Wheel { dx, dy }]
                }
            }
            CGEventType::KeyDown | CGEventType::KeyUp => {
                let code =
                    u16::try_from(int(EventField::KEYBOARD_EVENT_KEYCODE)).unwrap_or(u16::MAX);
                match kvm_keys::from_mac(code) {
                    Some(hid) => vec![Captured::Key {
                        hid,
                        down: matches!(ty, CGEventType::KeyDown),
                    }],
                    None => vec![],
                }
            }
            CGEventType::FlagsChanged => {
                let code =
                    u16::try_from(int(EventField::KEYBOARD_EVENT_KEYCODE)).unwrap_or(u16::MAX);
                let Some(hid) = kvm_keys::from_mac(code) else {
                    return vec![];
                };
                if code == 0x39 {
                    // Caps Lock: one press each time.
                    return vec![
                        Captured::Key { hid, down: true },
                        Captured::Key { hid, down: false },
                    ];
                }
                if !modifier(code) {
                    return vec![];
                }
                let Ok(mut h) = held.lock() else {
                    return vec![];
                };
                // Released if it was held, else pressed (the flags can't
                // tell left from right when both are down).
                let down = match h.iter().position(|k| *k == hid) {
                    Some(i) => {
                        h.remove(i);
                        false
                    }
                    None => {
                        h.push(hid);
                        true
                    }
                };
                vec![Captured::Key { hid, down }]
            }
            _ => vec![],
        }
    }

    pub fn start_capture(tx: Sender<Captured>) -> Result<Capture, &'static str> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<CFRunLoop, &'static str>>();
        let thread = std::thread::Builder::new()
            .name("kvm-tap".into())
            .spawn(move || {
                let held = Mutex::new(Vec::<u16>::new());
                let kinds = vec![
                    CGEventType::MouseMoved,
                    CGEventType::LeftMouseDragged,
                    CGEventType::RightMouseDragged,
                    CGEventType::OtherMouseDragged,
                    CGEventType::LeftMouseDown,
                    CGEventType::LeftMouseUp,
                    CGEventType::RightMouseDown,
                    CGEventType::RightMouseUp,
                    CGEventType::OtherMouseDown,
                    CGEventType::OtherMouseUp,
                    CGEventType::ScrollWheel,
                    CGEventType::KeyDown,
                    CGEventType::KeyUp,
                    CGEventType::FlagsChanged,
                ];
                let tap = CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::Default,
                    kinds,
                    move |_proxy, ty, e| {
                        if matches!(
                            ty,
                            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                        ) {
                            let port = TAP.load(Ordering::Acquire);
                            if !port.is_null() {
                                // SAFETY: the port lives as long as the tap,
                                // which outlives every call of this closure.
                                unsafe { CGEventTapEnable(port, true) };
                            }
                            return CallbackResult::Keep;
                        }
                        for c in caught(ty, e, &held) {
                            let _ = tx.send(c);
                        }
                        // Swallowed: nothing reaches this computer.
                        CallbackResult::Drop
                    },
                );
                let Ok(tap) = tap else {
                    let _ = ready_tx.send(Err("accessibility"));
                    return;
                };
                let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                    let _ = ready_tx.send(Err("tap"));
                    return;
                };
                TAP.store(
                    tap.mach_port().as_concrete_TypeRef() as *mut std::ffi::c_void,
                    Ordering::Release,
                );
                let rl = CFRunLoop::get_current();
                // SAFETY: a constant CoreFoundation string.
                rl.add_source(&source, unsafe { kCFRunLoopCommonModes });
                tap.enable();
                let _ = ready_tx.send(Ok(rl));
                CFRunLoop::run_current();
                drop(tap);
            })
            .map_err(|_| "thread")?;
        match ready_rx.recv() {
            Ok(Ok(run_loop)) => {
                let _ = CGDisplay::associate_mouse_and_mouse_cursor_position(false);
                let _ = CGDisplay::main().hide_cursor();
                Ok(Capture {
                    run_loop,
                    thread: Some(thread),
                })
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("tap"),
        }
    }
}

#[cfg(target_os = "windows")]
mod os {
    use super::{tauri_screens, Captured, Rect};
    use crate::kvm_keys;
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;
    use tauri::AppHandle;
    use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, GetMessageW, PostThreadMessageW, SetCursorPos,
        SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
        LLKHF_UP, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_QUIT,
    };

    /// Where the captured events go, and where the pointer is parked.
    static SINK: Mutex<Option<(Sender<Captured>, i32, i32)>> = Mutex::new(None);

    pub fn unsupported() -> Option<&'static str> {
        None
    }

    pub fn receive_only() -> bool {
        false
    }

    pub fn screens(app: &AppHandle) -> Vec<Rect> {
        tauri_screens(app)
    }

    pub fn cursor() -> Option<(f64, f64)> {
        let mut p = POINT::default();
        // SAFETY: a valid out pointer.
        unsafe { GetCursorPos(&mut p) }.ok()?;
        Some((f64::from(p.x), f64::from(p.y)))
    }

    pub fn warp(x: f64, y: f64) {
        // SAFETY: plain call.
        let _ = unsafe { SetCursorPos(x.round() as i32, y.round() as i32) };
    }

    fn send(c: Captured) {
        if let Ok(g) = SINK.lock() {
            if let Some((tx, _, _)) = g.as_ref() {
                let _ = tx.send(c);
            }
        }
    }

    fn park() -> (i32, i32) {
        SINK.lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| (s.1, s.2)))
            .unwrap_or((0, 0))
    }

    fn steps(delta: i16) -> i32 {
        let s = i32::from(delta) / 120;
        if s == 0 {
            i32::from(delta.signum())
        } else {
            s
        }
    }

    fn mouse(msg: u32, info: &MSLLHOOKSTRUCT) -> Option<Captured> {
        let hi = (info.mouseData >> 16) as u16;
        Some(match msg {
            0x0200 => {
                let (px, py) = park();
                let (dx, dy) = (info.pt.x - px, info.pt.y - py);
                if dx == 0 && dy == 0 {
                    return None;
                }
                Captured::Motion { dx, dy }
            }
            0x0201 => Captured::Button {
                button: 1,
                down: true,
            },
            0x0202 => Captured::Button {
                button: 1,
                down: false,
            },
            0x0204 => Captured::Button {
                button: 2,
                down: true,
            },
            0x0205 => Captured::Button {
                button: 2,
                down: false,
            },
            0x0207 => Captured::Button {
                button: 3,
                down: true,
            },
            0x0208 => Captured::Button {
                button: 3,
                down: false,
            },
            0x020B | 0x020C => Captured::Button {
                button: if hi == 1 { 4 } else { 5 },
                down: msg == 0x020B,
            },
            0x020A => Captured::Wheel {
                dx: 0,
                dy: -steps(hi as i16),
            },
            0x020E => Captured::Wheel {
                dx: steps(hi as i16),
                dy: 0,
            },
            _ => return None,
        })
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: for WH_MOUSE_LL, lparam points at an MSLLHOOKSTRUCT.
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if info.flags & LLMHF_INJECTED == 0 {
                if let Some(c) = mouse(wparam.0 as u32, info) {
                    send(c);
                }
                return LRESULT(1);
            }
        }
        // SAFETY: passing the hook's own arguments on.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: for WH_KEYBOARD_LL, lparam points at a KBDLLHOOKSTRUCT.
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if !info.flags.contains(LLKHF_INJECTED) {
                let down = !info.flags.contains(LLKHF_UP);
                let ext = info.flags.contains(LLKHF_EXTENDED);
                if let Some(hid) = kvm_keys::from_win(info.vkCode as u16, ext) {
                    send(Captured::Key { hid, down });
                }
                return LRESULT(1);
            }
        }
        // SAFETY: passing the hook's own arguments on.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub struct Capture {
        thread_id: u32,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            // SAFETY: plain call; the thread's message loop ends on WM_QUIT.
            let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            if let Ok(mut g) = SINK.lock() {
                *g = None;
            }
        }
    }

    pub fn start_capture(tx: Sender<Captured>) -> Result<Capture, &'static str> {
        let (px, py) = cursor().map_or((0, 0), |(x, y)| (x as i32, y as i32));
        if let Ok(mut g) = SINK.lock() {
            *g = Some((tx, px, py));
        }
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, &'static str>>();
        let thread = std::thread::Builder::new()
            .name("kvm-hooks".into())
            .spawn(move || {
                // SAFETY: hooks set and removed on this thread, which runs
                // the message loop they need.
                unsafe {
                    let tid = GetCurrentThreadId();
                    let m = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
                    let k = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0);
                    match (m, k) {
                        (Ok(m), Ok(k)) => {
                            let _ = ready_tx.send(Ok(tid));
                            let mut msg = MSG::default();
                            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                            let _ = UnhookWindowsHookEx(m);
                            let _ = UnhookWindowsHookEx(k);
                        }
                        (m, k) => {
                            if let Ok(m) = m {
                                let _ = UnhookWindowsHookEx(m);
                            }
                            if let Ok(k) = k {
                                let _ = UnhookWindowsHookEx(k);
                            }
                            let _ = ready_tx.send(Err("hooks"));
                        }
                    }
                }
            })
            .map_err(|_| "thread")?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Capture {
                thread_id,
                thread: Some(thread),
            }),
            other => {
                let _ = thread.join();
                if let Ok(mut g) = SINK.lock() {
                    *g = None;
                }
                Err(match other {
                    Ok(Err(e)) => e,
                    _ => "hooks",
                })
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod os {
    use super::{tauri_screens, Captured, Rect};
    use crate::kvm_keys;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;
    use tauri::AppHandle;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, EventMask, GrabMode, GrabStatus};
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    fn wayland() -> bool {
        md_portal::Session::wayland()
    }

    pub fn unsupported() -> Option<&'static str> {
        if wayland() {
            // Being controlled needs the remote desktop portal or uinput;
            // sending, the input capture portal (`receive_only`).
            return md_portal::service::input_way().err().map(|_| "wayland");
        }
        if std::env::var_os("DISPLAY").is_none() {
            return Some("display");
        }
        None
    }

    /// Wayland without the InputCapture portal: other computers can
    /// control this one, but it can't send its own mouse and keyboard.
    pub fn receive_only() -> bool {
        wayland() && !md_portal::service::can_send()
    }

    /// A connection for looking at and moving the pointer.
    fn shared() -> Option<&'static Mutex<(RustConnection, u32)>> {
        static CONN: OnceLock<Option<Mutex<(RustConnection, u32)>>> = OnceLock::new();
        CONN.get_or_init(|| {
            let (c, n) = x11rb::connect(None).ok()?;
            let root = c.setup().roots.get(n)?.root;
            Some(Mutex::new((c, root)))
        })
        .as_ref()
    }

    pub fn screens(app: &AppHandle) -> Vec<Rect> {
        if wayland() {
            // The portal's screens, in the units its moves use.
            let s = md_portal::service::input_screens();
            if !s.is_empty() {
                return s
                    .into_iter()
                    .map(|r| Rect {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                    })
                    .collect();
            }
        }
        tauri_screens(app)
    }

    pub fn cursor() -> Option<(f64, f64)> {
        if wayland() {
            // Where the moves made here put it (no app may read it there).
            return md_portal::service::input_cursor();
        }
        let g = shared()?.lock().ok()?;
        let (c, root) = &*g;
        let r = c.query_pointer(*root).ok()?.reply().ok()?;
        Some((f64::from(r.root_x), f64::from(r.root_y)))
    }

    pub fn warp(x: f64, y: f64) {
        if wayland() {
            // Only through an input session already open: never asks.
            md_portal::service::input_warp(x, y);
            return;
        }
        let Some(m) = shared() else {
            return;
        };
        if let Ok(g) = m.lock() {
            let (c, root) = &*g;
            let _ = c.warp_pointer(
                x11rb::NONE,
                *root,
                0,
                0,
                0,
                0,
                x.round() as i16,
                y.round() as i16,
            );
            let _ = c.flush();
        }
    }

    pub struct Capture {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        /// An InputCapture portal capture (Wayland).
        portal: bool,
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if self.portal {
                // Gives the pointer back, and ends the forwarding thread.
                md_portal::service::kvm_release();
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    /// Wayland: what the portal captured since the pointer went through a
    /// barrier (`kvm.rs` saw it) goes to `tx`.
    fn portal_capture(tx: Sender<Captured>) -> Result<Capture, &'static str> {
        use md_portal::Captured as P;
        let (ptx, prx) = mpsc::channel::<P>();
        let thread = std::thread::Builder::new()
            .name("kvm-portal".into())
            .spawn(move || {
                for c in prx {
                    let got = match c {
                        P::Motion { dx, dy } => Some(Captured::Motion { dx, dy }),
                        P::Button { button, down } => Some(Captured::Button { button, down }),
                        P::Wheel { dx, dy } => Some(Captured::Wheel { dx, dy }),
                        P::Key { code, down } => {
                            kvm_keys::from_evdev(code).map(|hid| Captured::Key { hid, down })
                        }
                    };
                    if let Some(got) = got {
                        if tx.send(got).is_err() {
                            break;
                        }
                    }
                }
            })
            .map_err(|_| "thread")?;
        let capture = Capture {
            stop: Arc::new(AtomicBool::new(false)),
            thread: Some(thread),
            portal: true,
        };
        if md_portal::service::kvm_attach(ptx) {
            Ok(capture)
        } else {
            // Nothing captured (the pointer didn't go through a barrier).
            Err("wayland")
        }
    }

    struct Grab {
        root: u32,
        cursor: u32,
        pixmap: u32,
        park: (i16, i16),
    }

    fn grab(c: &RustConnection, n: usize) -> Result<Grab, String> {
        let root = c.setup().roots.get(n).ok_or("no screen")?.root;
        // An invisible cursor while captured.
        let pixmap = c.generate_id().map_err(|e| e.to_string())?;
        c.create_pixmap(1, pixmap, root, 1, 1)
            .map_err(|e| e.to_string())?;
        let cursor = c.generate_id().map_err(|e| e.to_string())?;
        c.create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)
            .map_err(|e| e.to_string())?;
        let mask = EventMask::POINTER_MOTION | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE;
        let p = c
            .grab_pointer(
                false,
                root,
                mask,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                x11rb::CURRENT_TIME,
            )
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        if p.status != GrabStatus::SUCCESS {
            return Err("pointer grab refused".into());
        }
        let k = c
            .grab_keyboard(
                false,
                root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        if k.status != GrabStatus::SUCCESS {
            let _ = c.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = c.flush();
            return Err("keyboard grab refused".into());
        }
        let q = c
            .query_pointer(root)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        Ok(Grab {
            root,
            cursor,
            pixmap,
            park: (q.root_x, q.root_y),
        })
    }

    fn button(detail: u8, down: bool) -> Option<Captured> {
        Some(match (detail, down) {
            (1, _) => Captured::Button { button: 1, down },
            (2, _) => Captured::Button { button: 3, down },
            (3, _) => Captured::Button { button: 2, down },
            (8, _) => Captured::Button { button: 4, down },
            (9, _) => Captured::Button { button: 5, down },
            (4, true) => Captured::Wheel { dx: 0, dy: -1 },
            (5, true) => Captured::Wheel { dx: 0, dy: 1 },
            (6, true) => Captured::Wheel { dx: -1, dy: 0 },
            (7, true) => Captured::Wheel { dx: 1, dy: 0 },
            _ => return None,
        })
    }

    pub fn start_capture(tx: Sender<Captured>) -> Result<Capture, &'static str> {
        if wayland() {
            return portal_capture(tx);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), &'static str>>();
        let thread = std::thread::Builder::new()
            .name("kvm-grab".into())
            .spawn(move || {
                let Ok((c, n)) = x11rb::connect(None) else {
                    let _ = ready_tx.send(Err("display"));
                    return;
                };
                let g = match grab(&c, n) {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("[kvm] couldn't grab the pointer and keyboard: {e}");
                        let _ = ready_tx.send(Err("grab"));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let (px, py) = g.park;
                while !stopping.load(Ordering::Acquire) {
                    let mut any = false;
                    while let Ok(Some(ev)) = c.poll_for_event() {
                        any = true;
                        let got = match ev {
                            Event::MotionNotify(e) => {
                                let (dx, dy) = (
                                    i32::from(e.root_x) - i32::from(px),
                                    i32::from(e.root_y) - i32::from(py),
                                );
                                if dx == 0 && dy == 0 {
                                    None
                                } else {
                                    let _ = c.warp_pointer(x11rb::NONE, g.root, 0, 0, 0, 0, px, py);
                                    let _ = c.flush();
                                    Some(Captured::Motion { dx, dy })
                                }
                            }
                            Event::ButtonPress(e) => button(e.detail, true),
                            Event::ButtonRelease(e) => button(e.detail, false),
                            Event::KeyPress(e) => {
                                kvm_keys::from_evdev(u16::from(e.detail).saturating_sub(8))
                                    .map(|hid| Captured::Key { hid, down: true })
                            }
                            Event::KeyRelease(e) => {
                                kvm_keys::from_evdev(u16::from(e.detail).saturating_sub(8))
                                    .map(|hid| Captured::Key { hid, down: false })
                            }
                            _ => None,
                        };
                        if let Some(got) = got {
                            let _ = tx.send(got);
                        }
                    }
                    if !any {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
                let _ = c.ungrab_keyboard(x11rb::CURRENT_TIME);
                let _ = c.ungrab_pointer(x11rb::CURRENT_TIME);
                let _ = c.free_cursor(g.cursor);
                let _ = c.free_pixmap(g.pixmap);
                let _ = c.flush();
            })
            .map_err(|_| "thread")?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Capture {
                stop,
                thread: Some(thread),
                portal: false,
            }),
            other => {
                let _ = thread.join();
                Err(match other {
                    Ok(Err(e)) => e,
                    _ => "grab",
                })
            }
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod os {
    use super::{Captured, Rect};
    use std::sync::mpsc::Sender;
    use tauri::AppHandle;

    pub fn unsupported() -> Option<&'static str> {
        Some("unsupported")
    }
    pub fn receive_only() -> bool {
        false
    }
    pub fn screens(_app: &AppHandle) -> Vec<Rect> {
        Vec::new()
    }
    pub fn cursor() -> Option<(f64, f64)> {
        None
    }
    pub fn warp(_x: f64, _y: f64) {}
    pub struct Capture;
    pub fn start_capture(_tx: Sender<Captured>) -> Result<Capture, &'static str> {
        Err("unsupported")
    }
}

#[cfg(test)]
mod tests {
    use super::Rect;

    #[test]
    fn a_screen_holds_its_own_pixels_only() {
        let r = Rect {
            x: -1920.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        assert!(r.contains(-1920.0, 0.0));
        assert!(r.contains(-1.0, 1079.0));
        assert!(!r.contains(0.0, 10.0));
        assert!(!r.contains(-5.0, 1080.0));
    }
}
