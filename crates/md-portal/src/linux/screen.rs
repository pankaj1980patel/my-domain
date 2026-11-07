//! This screen on Wayland: a ScreenCast stream read by GStreamer, or a
//! Screenshot portal still.

use std::io::Read;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zvariant::Value;

use super::{Opts, Portal};
use crate::caps::{DEST, PATH, SCREENCAST, SCREENSHOT};
use crate::{gst, response};

/// Where GStreamer finds the PipeWire remote in its own process.
const CHILD_FD: i32 = 3;

/// A screen cast while someone watches: the newest picture, as a JPEG.
pub struct ScreenStream {
    /// Keeps the portal session (dropping it ends the cast).
    _portal: Portal,
    child: Child,
    latest: Arc<Mutex<Option<Vec<u8>>>>,
    ended: Arc<AtomicBool>,
}

impl ScreenStream {
    /// Ask for the screen (the user picks one, unless `restore` picks the
    /// one picked before) and start GStreamer on it: `width` pixels wide,
    /// `fps` pictures a second, JPEG `quality`. Also the token to pass as
    /// `restore` next time (none: the portal can't remember).
    pub fn start(
        restore: Option<&str>,
        width: u32,
        fps: u32,
        quality: u8,
    ) -> Result<(ScreenStream, Option<String>), String> {
        let p = Portal::new()?;
        let version = p.version(SCREENCAST).unwrap_or(1);
        let session = p.create_session(SCREENCAST, Opts::new())?;
        let mut opts = Opts::new();
        // Monitors, one of them.
        opts.insert("types", Value::from(1u32));
        opts.insert("multiple", Value::from(false));
        // The pointer drawn into the picture, when the portal can.
        let cursors = super::property(&p.conn, SCREENCAST, "AvailableCursorModes").unwrap_or(0);
        if version >= 2 && cursors & 2 != 0 {
            opts.insert("cursor_mode", Value::from(2u32));
        }
        if version >= 4 {
            // Remembered until revoked.
            opts.insert("persist_mode", Value::from(2u32));
            if let Some(t) = restore.filter(|t| !t.is_empty()) {
                opts.insert("restore_token", Value::from(t.to_string()));
            }
        }
        p.on_session(SCREENCAST, "SelectSources", &session, opts)?;
        let started = p.start(SCREENCAST, &session)?;
        let node = response::streams(&started)
            .first()
            .map(|s| s.node)
            .ok_or("No screen was chosen.")?;
        let token = response::restore_token(&started);
        let reply = p
            .conn
            .call_method(
                Some(DEST),
                PATH,
                Some(SCREENCAST),
                "OpenPipeWireRemote",
                &(&session, Opts::new()),
            )
            .map_err(|e| format!("couldn\u{2019}t open the screen\u{2019}s stream: {e}"))?;
        let fd: zvariant::OwnedFd = reply
            .body()
            .deserialize()
            .map_err(|e| format!("couldn\u{2019}t open the screen\u{2019}s stream: {e}"))?;
        let fd: OwnedFd = fd.into();
        let mut child = gstreamer(fd, node, width, fps, quality)?;
        let mut out = child.stdout.take().ok_or("GStreamer gave no output")?;
        let latest = Arc::new(Mutex::new(None));
        let ended = Arc::new(AtomicBool::new(false));
        let (l, e) = (latest.clone(), ended.clone());
        std::thread::Builder::new()
            .name("screen-cast".into())
            .spawn(move || {
                let mut frames = gst::Frames::default();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match out.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            frames.push(&buf[..n]);
                            while let Some(f) = frames.next_frame() {
                                if let Ok(mut slot) = l.lock() {
                                    *slot = Some(f);
                                }
                            }
                        }
                    }
                }
                e.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok((
            ScreenStream {
                _portal: p,
                child,
                latest,
                ended,
            },
            token,
        ))
    }

    /// The newest picture so far.
    pub fn latest(&self) -> Option<Vec<u8>> {
        self.latest.lock().ok().and_then(|l| l.clone())
    }

    /// GStreamer stopped (the cast ended, or the user stopped it).
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }
}

impl Drop for ScreenStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `gst-launch-1.0` reading node `node` through the portal's PipeWire
/// remote `fd`, handed to it as fd 3. Its arguments are a vector: no shell.
#[allow(unsafe_code)]
fn gstreamer(fd: OwnedFd, node: u32, width: u32, fps: u32, quality: u8) -> Result<Child, String> {
    let raw = fd.as_raw_fd();
    let mut cmd = Command::new(gst::LAUNCH);
    cmd.args(gst::pipeline(CHILD_FD, node, width, fps, quality))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: runs in the child between fork and exec, calling only
    // async-signal-safe functions (dup2, fcntl) on a descriptor this
    // process owns until the spawn returns.
    unsafe {
        cmd.pre_exec(move || {
            if raw == CHILD_FD {
                // Already there: just keep it open across exec.
                if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::dup2(raw, CHILD_FD) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("couldn\u{2019}t start {}: {e}", gst::LAUNCH))?;
    // The child has its own copy now.
    drop(fd);
    Ok(child)
}

/// One picture of the screen through the Screenshot portal (no prompt
/// once allowed; GNOME asks the first time): the PNG's bytes. The file
/// the portal wrote is removed.
pub fn screenshot() -> Result<Vec<u8>, String> {
    let p = Portal::new()?;
    let mut opts = Opts::new();
    opts.insert("interactive", Value::from(false));
    let (opts, it) = p.ask("shot", opts)?;
    p.conn
        .call_method(
            Some(DEST),
            PATH,
            Some(SCREENSHOT),
            "Screenshot",
            &("", opts),
        )
        .map_err(|e| format!("couldn\u{2019}t take a screenshot: {e}"))?;
    let d = p.wait(it)?;
    let path = response::screenshot_path(&d).ok_or("The screenshot wasn\u{2019}t saved.")?;
    let png = std::fs::read(&path).map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&path);
    png
}
