//! SCO (call audio) sockets: `AF_BLUETOOTH` / `BTPROTO_SCO`, which neither
//! std nor socket2 know, so straight libc. Linux lets any program with
//! Bluetooth access open these (macOS doesn't).
//!
//! CVSD: the kernel's default voice setting (`BT_VOICE_CVSD_16BIT`): 16-bit
//! LE mono PCM at 8 kHz in and out, the radio does the CVSD coding.

// Raw socket calls on fds this module owns (each `unsafe` says why).
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Duration;

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_SCO: libc::c_int = 2;
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_VOICE: libc::c_int = 11;
const BT_VOICE_CVSD_16BIT: u16 = 0x0060;
const SOL_SCO: libc::c_int = 17;
const SCO_OPTIONS: libc::c_int = 1;

/// `struct sockaddr_sco`.
#[repr(C)]
struct SockaddrSco {
    family: libc::sa_family_t,
    bdaddr: [u8; 6],
}

/// `struct bt_voice`.
#[repr(C)]
struct BtVoice {
    setting: u16,
}

/// `struct sco_options`.
#[repr(C)]
struct ScoOptions {
    mtu: u16,
}

fn check(r: libc::c_int) -> io::Result<libc::c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

fn addr(bdaddr: [u8; 6]) -> SockaddrSco {
    SockaddrSco {
        family: AF_BLUETOOTH as libc::sa_family_t,
        bdaddr,
    }
}

fn socket() -> io::Result<OwnedFd> {
    // SAFETY: plain socket(2); the fd is ours from here on.
    let fd = check(unsafe {
        libc::socket(
            AF_BLUETOOTH,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            BTPROTO_SCO,
        )
    })?;
    // SAFETY: a fresh fd nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn bind(fd: &OwnedFd, local: [u8; 6]) -> io::Result<()> {
    let a = addr(local);
    // SAFETY: a valid sockaddr_sco and its size.
    check(unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&a as *const SockaddrSco).cast(),
            std::mem::size_of::<SockaddrSco>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

fn set_cvsd(fd: &OwnedFd) -> io::Result<()> {
    let v = BtVoice {
        setting: BT_VOICE_CVSD_16BIT,
    };
    // SAFETY: setsockopt with a bt_voice and its size.
    check(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            SOL_BLUETOOTH,
            BT_VOICE,
            (&v as *const BtVoice).cast(),
            std::mem::size_of::<BtVoice>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

/// Listen for the phone opening the audio link, on the adapter `local`
/// (its own address rather than "any": a program listening on "any", like
/// PipeWire, then still gets other devices' links). `EADDRINUSE` when
/// another program listens on the same address.
pub fn listen(local: [u8; 6]) -> io::Result<OwnedFd> {
    let fd = socket()?;
    bind(&fd, local)?;
    set_cvsd(&fd)?;
    // SAFETY: listen(2) on our socket.
    check(unsafe { libc::listen(fd.as_raw_fd(), 1) })?;
    Ok(fd)
}

/// Wait up to `timeout` for a link on `listener`: the new socket and the
/// remote's address (`bdaddr_t`), or `None` on timeout.
pub fn accept(listener: &OwnedFd, timeout: Duration) -> io::Result<Option<(OwnedFd, [u8; 6])>> {
    let mut p = libc::pollfd {
        fd: listener.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
    // SAFETY: one valid pollfd.
    let n = unsafe { libc::poll(&mut p, 1, ms) };
    if n < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::Interrupted {
            Ok(None)
        } else {
            Err(e)
        };
    }
    if n == 0 {
        return Ok(None);
    }
    let mut a = addr([0; 6]);
    let mut len = std::mem::size_of::<SockaddrSco>() as libc::socklen_t;
    // SAFETY: accept4 into a sockaddr_sco-sized buffer.
    let fd = check(unsafe {
        libc::accept4(
            listener.as_raw_fd(),
            (&mut a as *mut SockaddrSco).cast(),
            &mut len,
            libc::SOCK_CLOEXEC,
        )
    })?;
    // SAFETY: a fresh fd nothing else owns.
    Ok(Some((unsafe { OwnedFd::from_raw_fd(fd) }, a.bdaddr)))
}

/// Open the audio link to `remote` from adapter `local` (blocks until the
/// phone accepts or refuses).
pub fn connect(local: [u8; 6], remote: [u8; 6]) -> io::Result<OwnedFd> {
    let fd = socket()?;
    bind(&fd, local)?;
    set_cvsd(&fd)?;
    let a = addr(remote);
    // SAFETY: a valid sockaddr_sco and its size.
    check(unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&a as *const SockaddrSco).cast(),
            std::mem::size_of::<SockaddrSco>() as libc::socklen_t,
        )
    })?;
    Ok(fd)
}

/// The link's packet size (48 bytes when it can't be read).
pub fn mtu(fd: &OwnedFd) -> usize {
    let mut o = ScoOptions { mtu: 0 };
    let mut len = std::mem::size_of::<ScoOptions>() as libc::socklen_t;
    // SAFETY: getsockopt into a sco_options and its size.
    let r = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            SOL_SCO,
            SCO_OPTIONS,
            (&mut o as *mut ScoOptions).cast(),
            &mut len,
        )
    };
    if r < 0 || o.mtu == 0 {
        48
    } else {
        o.mtu as usize
    }
}

/// Reads give up after `t` (`WouldBlock`), so a pump can look at its stop flag.
pub fn set_read_timeout(fd: &OwnedFd, t: Duration) -> io::Result<()> {
    let tv = libc::timeval {
        tv_sec: t.as_secs() as libc::time_t,
        tv_usec: t.subsec_micros() as libc::suseconds_t,
    };
    // SAFETY: setsockopt with a timeval and its size.
    check(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&tv as *const libc::timeval).cast(),
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

pub fn read(fd: &OwnedFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: read(2) into a buffer of that length.
    let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn write(fd: &OwnedFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: write(2) from a buffer of that length.
    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

/// Make reads and writes on this socket wait (BlueZ hands over its RFCOMM
/// sockets non-blocking; our reader thread wants to block).
pub fn set_blocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl(2) flag get/set on an fd the caller keeps open.
    let flags = check(unsafe { libc::fcntl(fd, libc::F_GETFL) })?;
    if flags & libc::O_NONBLOCK != 0 {
        // SAFETY: as above.
        check(unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) })?;
    }
    Ok(())
}

/// Wake whatever blocks on this socket and end the link (the fd itself is
/// closed by its owner).
pub fn shutdown(fd: RawFd) {
    // SAFETY: shutdown(2) on a socket fd still open (its owner keeps it).
    unsafe {
        libc::shutdown(fd, libc::SHUT_RDWR);
    }
}
