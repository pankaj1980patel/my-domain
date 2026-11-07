//! `/dev/uinput`: a virtual mouse and keyboard the kernel treats as real
//! ones, every compositor included. Only when this user may already write
//! the device (a udev rule; see the help page): nothing here asks for root.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

const DEVICE: &str = "/dev/uinput";

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
pub const REL_X: u16 = 0;
pub const REL_Y: u16 = 1;
pub const REL_HWHEEL: u16 = 6;
pub const REL_WHEEL: u16 = 8;

// ioctl numbers ('U'): _IOW(type, nr, size) and _IO(type, nr).
const fn iow(nr: u64, size: u64) -> u64 {
    (1 << 30) | (size << 16) | (0x55 << 8) | nr
}
const UI_SET_EVBIT: u64 = iow(100, 4);
const UI_SET_KEYBIT: u64 = iow(101, 4);
const UI_SET_RELBIT: u64 = iow(102, 4);
const UI_DEV_SETUP: u64 = iow(3, std::mem::size_of::<libc::uinput_setup>() as u64);
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;

/// This user may write `/dev/uinput` (no root, no prompt).
pub fn writable() -> bool {
    OpenOptions::new().write(true).open(DEVICE).is_ok()
}

/// The virtual mouse and keyboard; removed when dropped.
pub struct Device {
    file: File,
}

fn ioctl(file: &File, req: u64, arg: libc::c_ulong) -> std::io::Result<()> {
    // SAFETY: the uinput ioctls used here take an int (passed by value) or
    // nothing; the setup one is called below with a pointer to a live
    // `uinput_setup`.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), req as _, arg) };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Device {
    pub fn new(name: &str) -> std::io::Result<Device> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(DEVICE)?;
        for ev in [EV_SYN, EV_KEY, EV_REL] {
            ioctl(&file, UI_SET_EVBIT, ev.into())?;
        }
        for rel in [REL_X, REL_Y, REL_WHEEL, REL_HWHEEL] {
            ioctl(&file, UI_SET_RELBIT, rel.into())?;
        }
        // Every keyboard key, and the mouse buttons.
        for key in (1u16..=248).chain(0x110..=0x114) {
            ioctl(&file, UI_SET_KEYBIT, key.into())?;
        }
        // SAFETY: all-zero is a valid uinput_setup (plain integers).
        let mut setup: libc::uinput_setup = unsafe { std::mem::zeroed() };
        setup.id.bustype = 0x06; // BUS_VIRTUAL
        setup.id.vendor = 0x1d6b;
        setup.id.product = 0x0104;
        setup.id.version = 1;
        for (dst, src) in setup.name.iter_mut().zip(name.bytes().take(79)) {
            *dst = src as libc::c_char;
        }
        // SAFETY: a pointer to the live, initialised setup for this call.
        let r = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                UI_DEV_SETUP as _,
                &setup as *const libc::uinput_setup,
            )
        };
        if r < 0 {
            return Err(std::io::Error::last_os_error());
        }
        ioctl(&file, UI_DEV_CREATE, 0)?;
        Ok(Device { file })
    }

    fn event(&mut self, ty: u16, code: u16, value: i32) -> std::io::Result<()> {
        // SAFETY: all-zero is a valid input_event (time 0: the kernel
        // stamps it).
        let mut ev: libc::input_event = unsafe { std::mem::zeroed() };
        ev.type_ = ty;
        ev.code = code;
        ev.value = value;
        // SAFETY: the bytes of a plain-old-data struct, for its lifetime.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&ev as *const libc::input_event).cast::<u8>(),
                std::mem::size_of::<libc::input_event>(),
            )
        };
        self.file.write_all(bytes)
    }

    fn sync(&mut self) -> std::io::Result<()> {
        self.event(EV_SYN, SYN_REPORT, 0)
    }

    /// Relative motion, or a wheel (`REL_WHEEL` positive scrolls up).
    pub fn rel(&mut self, axis: u16, value: i32) -> std::io::Result<()> {
        self.event(EV_REL, axis, value)?;
        self.sync()
    }

    pub fn motion(&mut self, dx: i32, dy: i32) -> std::io::Result<()> {
        if dx != 0 {
            self.event(EV_REL, REL_X, dx)?;
        }
        if dy != 0 {
            self.event(EV_REL, REL_Y, dy)?;
        }
        self.sync()
    }

    /// A key or button (evdev code).
    pub fn key(&mut self, code: u16, down: bool) -> std::io::Result<()> {
        self.event(EV_KEY, code, i32::from(down))?;
        self.sync()
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = ioctl(&self.file, UI_DEV_DESTROY, 0);
    }
}
