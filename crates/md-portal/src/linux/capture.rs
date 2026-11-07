//! The InputCapture portal: barriers on the screen edges where another
//! computer sits. When the pointer pushes through one, the compositor
//! stops moving it here and sends this computer's mouse and keyboard over
//! the EIS socket instead, until told to let go (`release`).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::blocking::MessageIterator;
use zbus::message::Type;
use zbus::MatchRule;
use zvariant::{OwnedObjectPath, Value};

use super::{Opts, Portal};
use crate::caps::{DEST, INPUT_CAPTURE, PATH};
use crate::eis;
use crate::geom::{self, Barrier, Edge, Rect};
use crate::keys;
use crate::response::{self, Activation, Dict};

/// Keyboard (1) and pointer (2).
const CAPABILITIES: u32 = 3;
/// Smooth-scroll pixels per wheel click (touchpads).
const PX_PER_STEP: f32 = 15.0;
/// Where the pointer is let go: this far inside the edge it left by.
const LET_GO_INSIDE: f64 = 3.0;

/// What was captured.
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
    /// Wheel clicks; `dy` positive scrolls down.
    Wheel {
        dx: i32,
        dy: i32,
    },
    /// An evdev key code.
    Key {
        code: u16,
        down: bool,
    },
}

type Sink = Arc<Mutex<Option<Sender<Captured>>>>;

/// What the portal said happened.
enum Signal {
    Activated(Activation),
    Deactivated,
    Disabled,
    ZonesChanged,
}

pub struct CaptureSession {
    p: Portal,
    session: OwnedObjectPath,
    zones: Vec<Rect>,
    barriers: Vec<Barrier>,
    sink: Sink,
    signals: Receiver<Signal>,
    /// The capture going on: its id, where the pointer was, the edge.
    active: Option<(u32, (f64, f64), Edge)>,
    /// The barriers need setting again (screens changed, or disabled).
    stale: bool,
    eis: UnixStream,
    stopped: Arc<AtomicBool>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("the input capture portal didn\u{2019}t answer: {e}")
}

impl CaptureSession {
    /// Ask to capture this computer's input (the portal may ask the user)
    /// and connect to its EIS socket. Nothing is captured until `arm`.
    pub fn start() -> Result<CaptureSession, String> {
        let p = Portal::new()?;
        let mut extra = Opts::new();
        extra.insert("capabilities", Value::from(CAPABILITIES));
        let session = p.create_session(INPUT_CAPTURE, extra)?;
        let signals = listen(&p)?;
        let reply = p
            .conn
            .call_method(
                Some(DEST),
                PATH,
                Some(INPUT_CAPTURE),
                "ConnectToEIS",
                &(&session, Opts::new()),
            )
            .map_err(err)?;
        let fd: zvariant::OwnedFd = reply.body().deserialize().map_err(err)?;
        let fd: OwnedFd = fd.into();
        let eis = UnixStream::from(fd);
        let sink: Sink = Arc::new(Mutex::new(None));
        let stopped = Arc::new(AtomicBool::new(false));
        let reader = eis.try_clone().map_err(err)?;
        let (s, stop) = (sink.clone(), stopped.clone());
        std::thread::Builder::new()
            .name("input-capture".into())
            .spawn(move || read_eis(reader, &s, &stop))
            .map_err(err)?;
        Ok(CaptureSession {
            p,
            session,
            zones: Vec::new(),
            barriers: Vec::new(),
            sink,
            signals,
            active: None,
            stale: true,
            eis,
            stopped,
        })
    }

    /// Put barriers on `edges` of the screens and start watching them.
    /// The barriers the compositor refused are left out.
    pub fn arm(&mut self, edges: &[Edge]) -> Result<(), String> {
        let d = self
            .p
            .on_session(INPUT_CAPTURE, "GetZones", &self.session, Opts::new())?;
        let (zones, zone_set) = response::zones(&d);
        let wanted = geom::barriers(&zones, edges);
        let list: Vec<HashMap<&str, Value<'_>>> = wanted
            .iter()
            .map(|b| {
                let mut m = HashMap::new();
                m.insert("barrier_id", Value::from(b.id));
                m.insert("position", Value::from((b.x1, b.y1, b.x2, b.y2)));
                m
            })
            .collect();
        let (opts, it) = self.p.ask("barriers", Opts::new())?;
        self.p
            .conn
            .call_method(
                Some(DEST),
                PATH,
                Some(INPUT_CAPTURE),
                "SetPointerBarriers",
                &(&self.session, opts, list, zone_set),
            )
            .map_err(err)?;
        let failed = response::failed_barriers(&self.p.wait(it)?);
        self.barriers = wanted
            .into_iter()
            .filter(|b| !failed.contains(&b.id))
            .collect();
        self.zones = zones;
        if !self
            .p
            .call(INPUT_CAPTURE, "Enable", &(&self.session, Opts::new()))
        {
            return Err(err("Enable"));
        }
        self.stale = false;
        Ok(())
    }

    /// Stop watching the edges (nothing placed around this computer).
    pub fn disarm(&mut self) {
        self.release();
        let _ = self
            .p
            .call(INPUT_CAPTURE, "Disable", &(&self.session, Opts::new()));
        self.stale = true;
    }

    /// The screens, as the portal sees them.
    pub fn zones(&self) -> &[Rect] {
        &self.zones
    }

    /// The barriers need setting again (`arm`).
    pub fn stale(&self) -> bool {
        self.stale
    }

    /// Wait up to `wait` for the pointer to push through a barrier: its
    /// edge and where the pointer was.
    pub fn next_crossing(&mut self, wait: Duration) -> Option<(Edge, (f64, f64))> {
        let signal = self.signals.recv_timeout(wait).ok()?;
        match signal {
            Signal::Activated(a) => {
                let edge = a
                    .barrier
                    .and_then(|id| self.barriers.iter().find(|b| b.id == id))
                    .map(|b| b.edge)?;
                let at = a.cursor.unwrap_or_default();
                self.active = Some((a.id.unwrap_or(0), at, edge));
                Some((edge, at))
            }
            Signal::Deactivated => {
                self.active = None;
                None
            }
            Signal::Disabled | Signal::ZonesChanged => {
                self.active = None;
                self.stale = true;
                None
            }
        }
    }

    /// A capture is going on (the pointer went through a barrier).
    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// Where the captured input goes from now on.
    pub fn attach(&self, tx: Sender<Captured>) {
        if let Ok(mut s) = self.sink.lock() {
            *s = Some(tx);
        }
    }

    /// Give the pointer back here, just inside the edge it left by.
    pub fn release(&mut self) {
        if let Ok(mut s) = self.sink.lock() {
            *s = None;
        }
        let Some((id, at, edge)) = self.active.take() else {
            return;
        };
        let mut opts = Opts::new();
        opts.insert("activation_id", Value::from(id));
        opts.insert(
            "cursor_position",
            Value::from(geom::inside(edge, at, LET_GO_INSIDE)),
        );
        let _ = self
            .p
            .call(INPUT_CAPTURE, "Release", &(&self.session, opts));
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.release();
        self.stopped.store(true, Ordering::Release);
        let _ = self.eis.shutdown(std::net::Shutdown::Both);
    }
}

/// The portal's InputCapture signals for this session, on a channel.
fn listen(p: &Portal) -> Result<Receiver<Signal>, String> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(INPUT_CAPTURE)
        .map_err(err)?
        .build();
    let it = MessageIterator::for_match_rule(rule, &p.conn, Some(64)).map_err(err)?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("input-capture-signals".into())
        .spawn(move || {
            for m in it {
                let Ok(m) = m else { break };
                let header = m.header();
                let Some(member) = header.member().map(|m| m.as_str().to_string()) else {
                    continue;
                };
                let signal = match member.as_str() {
                    "Activated" => match m.body().deserialize::<(OwnedObjectPath, Dict)>() {
                        Ok((_, d)) => Signal::Activated(response::activation(&d)),
                        Err(_) => continue,
                    },
                    "Deactivated" => Signal::Deactivated,
                    "Disabled" => Signal::Disabled,
                    "ZonesChanged" => Signal::ZonesChanged,
                    _ => continue,
                };
                if tx.send(signal).is_err() {
                    break;
                }
            }
        })
        .map_err(err)?;
    Ok(rx)
}

/// The EIS socket: handshake, then the captured events to the sink.
fn read_eis(mut sock: UnixStream, sink: &Sink, stopped: &AtomicBool) {
    let mut r = eis::Receiver::new("my-domain");
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let mut motion = eis::Motion::default();
    let (mut wheel_rest, mut smooth) = ((0i32, 0i32), (0f32, 0f32));
    while !stopped.load(Ordering::Acquire) {
        let n = match sock.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        let mut out = Vec::new();
        loop {
            let m = match eis::take(&mut buf) {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(eis::Garbled) => return,
            };
            let Some(ev) = r.on(&m, &mut out) else {
                continue;
            };
            let got = match ev {
                eis::Event::Motion { dx, dy } => {
                    let (dx, dy) = motion.add(dx, dy);
                    (dx != 0 || dy != 0).then_some(Captured::Motion { dx, dy })
                }
                eis::Event::Button { button, down } => {
                    keys::button_number(button).map(|button| Captured::Button { button, down })
                }
                eis::Event::ScrollDiscrete { dx, dy } => {
                    let x = wheel_rest.0 + dx;
                    let y = wheel_rest.1 + dy;
                    wheel_rest = (x % 120, y % 120);
                    let (sx, sy) = (x / 120, y / 120);
                    (sx != 0 || sy != 0).then_some(Captured::Wheel { dx: sx, dy: sy })
                }
                eis::Event::Scroll { dx, dy } => {
                    let x = smooth.0 + dx;
                    let y = smooth.1 + dy;
                    let (sx, sy) = ((x / PX_PER_STEP) as i32, (y / PX_PER_STEP) as i32);
                    smooth = (x - sx as f32 * PX_PER_STEP, y - sy as f32 * PX_PER_STEP);
                    (sx != 0 || sy != 0).then_some(Captured::Wheel { dx: sx, dy: sy })
                }
                eis::Event::Key { key, down } => u16::try_from(key)
                    .ok()
                    .map(|code| Captured::Key { code, down }),
                eis::Event::Emulating(_) => None,
                eis::Event::Disconnected(why) => {
                    eprintln!("[portal] the input capture ended: {why}");
                    return;
                }
            };
            if let Some(got) = got {
                if let Some(tx) = sink.lock().ok().and_then(|s| s.clone()) {
                    let _ = tx.send(got);
                }
            }
        }
        if !out.is_empty() && sock.write_all(&out).is_err() {
            break;
        }
    }
}
