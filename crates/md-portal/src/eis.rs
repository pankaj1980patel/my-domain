//! A receiver of the libei protocol ("ei"), the way InputCapture hands
//! over the captured mouse and keyboard: the portal's `ConnectToEIS`
//! gives a socket, the compositor sends this computer's input on it while
//! a capture is active. Only what a receiver needs is here (no libei
//! crate is in the build): the handshake, binding the seat, and the
//! pointer, button, scroll and keyboard events.
//!
//! Wire format: each message is a 16-byte header (object id u64, length
//! u32 including the header, opcode u32), then its arguments, each 4-byte
//! aligned: u32 / i32 / f32, u64 (object ids, new ids), strings as a u32
//! length including the NUL, the bytes and padding. File descriptors
//! (the keymap) travel beside the bytes and aren't needed here. Native
//! byte order (little-endian on the machines this runs on).

use std::collections::HashMap;

const HEADER: usize = 16;
/// The handshake object, there from the start.
pub const HANDSHAKE: u64 = 0;
/// `ei_handshake.context_type`: a receiver.
const RECEIVER: u32 = 1;
/// Interfaces announced, all at version 1.
const INTERFACES: [&str; 10] = [
    "ei_connection",
    "ei_callback",
    "ei_pingpong",
    "ei_seat",
    "ei_device",
    "ei_pointer",
    "ei_pointer_absolute",
    "ei_scroll",
    "ei_button",
    "ei_keyboard",
];
/// The capabilities bound.
const WANTED: [&str; 4] = ["ei_pointer", "ei_button", "ei_scroll", "ei_keyboard"];

/// One message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    pub object: u64,
    pub opcode: u32,
    pub body: Vec<u8>,
}

/// An argument to send.
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    U32(u32),
    U64(u64),
    Str(&'a str),
}

/// A message as bytes.
pub fn encode(object: u64, opcode: u32, args: &[Arg<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for a in args {
        match a {
            Arg::U32(v) => body.extend_from_slice(&v.to_ne_bytes()),
            Arg::U64(v) => body.extend_from_slice(&v.to_ne_bytes()),
            Arg::Str(s) => {
                let len = s.len() as u32 + 1;
                body.extend_from_slice(&len.to_ne_bytes());
                body.extend_from_slice(s.as_bytes());
                body.push(0);
                while body.len() % 4 != 0 {
                    body.push(0);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.extend_from_slice(&object.to_ne_bytes());
    out.extend_from_slice(&((HEADER + body.len()) as u32).to_ne_bytes());
    out.extend_from_slice(&opcode.to_ne_bytes());
    out.extend(body);
    out
}

/// The stream makes no sense (a length shorter than the header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Garbled;

/// The next whole message off the front of `buf`.
pub fn take(buf: &mut Vec<u8>) -> Result<Option<Msg>, Garbled> {
    if buf.len() < HEADER {
        return Ok(None);
    }
    let u32_at = |i: usize| u32::from_ne_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
    let len = u32_at(8) as usize;
    if len < HEADER {
        return Err(Garbled);
    }
    if buf.len() < len {
        return Ok(None);
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&buf[..8]);
    let msg = Msg {
        object: u64::from_ne_bytes(id),
        opcode: u32_at(12),
        body: buf[HEADER..len].to_vec(),
    };
    buf.drain(..len);
    Ok(Some(msg))
}

/// Reads a message's arguments in order; `None` once it runs short.
struct Args<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Args<'a> {
    fn new(b: &'a [u8]) -> Self {
        Args { b, at: 0 }
    }
    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.b.get(self.at..self.at + N)?;
        self.at += N;
        let mut out = [0u8; N];
        out.copy_from_slice(s);
        Some(out)
    }
    fn u32(&mut self) -> Option<u32> {
        self.bytes::<4>().map(u32::from_ne_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.bytes::<4>().map(i32::from_ne_bytes)
    }
    fn f32(&mut self) -> Option<f32> {
        self.bytes::<4>().map(f32::from_ne_bytes)
    }
    fn u64(&mut self) -> Option<u64> {
        self.bytes::<8>().map(u64::from_ne_bytes)
    }
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        if len == 0 {
            return Some(String::new());
        }
        let raw = self.b.get(self.at..self.at + len)?;
        self.at += len.div_ceil(4) * 4;
        let text = raw.strip_suffix(&[0]).unwrap_or(raw);
        Some(String::from_utf8_lossy(text).into_owned())
    }
}

/// What came in.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Relative pointer motion, in logical pixels.
    Motion { dx: f32, dy: f32 },
    /// An evdev button (BTN_LEFT...).
    Button { button: u32, down: bool },
    /// Smooth scrolling, in logical pixels.
    Scroll { dx: f32, dy: f32 },
    /// Wheel clicks, 120 per click.
    ScrollDiscrete { dx: i32, dy: i32 },
    /// An evdev key code.
    Key { key: u32, down: bool },
    /// A device started or stopped sending (a capture began or ended).
    Emulating(bool),
    /// The compositor ended the connection.
    Disconnected(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Handshake,
    Connection,
    Callback,
    Pingpong,
    Seat,
    Device,
    Pointer,
    Button,
    Scroll,
    Keyboard,
    Other,
}

fn kind_of(interface: &str) -> Kind {
    match interface {
        "ei_pointer" => Kind::Pointer,
        "ei_button" => Kind::Button,
        "ei_scroll" => Kind::Scroll,
        "ei_keyboard" => Kind::Keyboard,
        "ei_device" => Kind::Device,
        "ei_seat" => Kind::Seat,
        "ei_callback" => Kind::Callback,
        "ei_pingpong" => Kind::Pingpong,
        _ => Kind::Other,
    }
}

/// The receiving end: feed it each message, send what it writes back.
pub struct Receiver {
    name: String,
    objects: HashMap<u64, Kind>,
    /// Each seat's capabilities: interface name -> mask bit.
    caps: HashMap<u64, Vec<(String, u64)>>,
}

impl Receiver {
    pub fn new(name: &str) -> Receiver {
        let mut objects = HashMap::new();
        objects.insert(HANDSHAKE, Kind::Handshake);
        Receiver {
            name: name.to_string(),
            objects,
            caps: HashMap::new(),
        }
    }

    /// Read one message; replies are added to `out`.
    pub fn on(&mut self, m: &Msg, out: &mut Vec<u8>) -> Option<Event> {
        let kind = *self.objects.get(&m.object)?;
        let mut a = Args::new(&m.body);
        match (kind, m.opcode) {
            // handshake_version: answer with ours, who we are, what we
            // speak, and finish.
            (Kind::Handshake, 0) => {
                // Version 1 is all there is so far, and all we speak.
                out.extend(encode(HANDSHAKE, 0, &[Arg::U32(1)]));
                out.extend(encode(HANDSHAKE, 3, &[Arg::Str(&self.name)]));
                out.extend(encode(HANDSHAKE, 2, &[Arg::U32(RECEIVER)]));
                for i in INTERFACES {
                    out.extend(encode(HANDSHAKE, 4, &[Arg::Str(i), Arg::U32(1)]));
                }
                out.extend(encode(HANDSHAKE, 1, &[]));
                None
            }
            // connection(serial, new id, version)
            (Kind::Handshake, 2) => {
                let _serial = a.u32()?;
                let id = a.u64()?;
                self.objects.insert(id, Kind::Connection);
                self.objects.remove(&HANDSHAKE);
                None
            }
            (Kind::Connection, 0) => {
                let _serial = a.u32();
                let _reason = a.u32();
                Some(Event::Disconnected(a.string().unwrap_or_default()))
            }
            // seat(new id, version)
            (Kind::Connection, 1) => {
                let id = a.u64()?;
                self.objects.insert(id, Kind::Seat);
                None
            }
            // ping(new id, version): pingpong.done(0) at once.
            (Kind::Connection, 3) => {
                let id = a.u64()?;
                out.extend(encode(id, 0, &[Arg::U64(0)]));
                None
            }
            (Kind::Seat, 2) => {
                let mask = a.u64()?;
                let iface = a.string()?;
                self.caps.entry(m.object).or_default().push((iface, mask));
                None
            }
            // done: bind what we want of what it has.
            (Kind::Seat, 3) => {
                let mask = self
                    .caps
                    .get(&m.object)
                    .map(|c| {
                        c.iter()
                            .filter(|(i, _)| WANTED.contains(&i.as_str()))
                            .fold(0u64, |acc, (_, bit)| acc | bit)
                    })
                    .unwrap_or(0);
                out.extend(encode(m.object, 1, &[Arg::U64(mask)]));
                None
            }
            (Kind::Seat, 4) => {
                let id = a.u64()?;
                self.objects.insert(id, Kind::Device);
                None
            }
            // interface(new id, name, version)
            (Kind::Device, 5) => {
                let id = a.u64()?;
                let name = a.string()?;
                self.objects.insert(id, kind_of(&name));
                None
            }
            (Kind::Device, 9) => Some(Event::Emulating(true)),
            (Kind::Device, 10) => Some(Event::Emulating(false)),
            (Kind::Pointer, 1) => Some(Event::Motion {
                dx: a.f32()?,
                dy: a.f32()?,
            }),
            (Kind::Scroll, 1) => Some(Event::Scroll {
                dx: a.f32()?,
                dy: a.f32()?,
            }),
            (Kind::Scroll, 2) => Some(Event::ScrollDiscrete {
                dx: a.i32()?,
                dy: a.i32()?,
            }),
            (Kind::Button, 1) => Some(Event::Button {
                button: a.u32()?,
                down: a.u32()? != 0,
            }),
            (Kind::Keyboard, 2) => Some(Event::Key {
                key: a.u32()?,
                down: a.u32()? != 0,
            }),
            // destroyed
            (
                Kind::Seat
                | Kind::Device
                | Kind::Pointer
                | Kind::Button
                | Kind::Scroll
                | Kind::Keyboard
                | Kind::Other,
                0,
            ) => {
                self.objects.remove(&m.object);
                self.caps.remove(&m.object);
                None
            }
            // A callback's done: the object's gone.
            (Kind::Callback, 0) => {
                self.objects.remove(&m.object);
                None
            }
            _ => None,
        }
    }
}

/// Fractions of a pixel carried over, so slow motion isn't lost.
#[derive(Default)]
pub struct Motion {
    rest: (f32, f32),
}

impl Motion {
    pub fn add(&mut self, dx: f32, dy: f32) -> (i32, i32) {
        let x = self.rest.0 + dx;
        let y = self.rest.1 + dy;
        let (ix, iy) = (x.trunc() as i32, y.trunc() as i32);
        self.rest = (x - ix as f32, y - iy as f32);
        (ix, iy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f32) -> Arg<'static> {
        Arg::U32(v.to_bits())
    }

    fn run(r: &mut Receiver, msgs: &[Vec<u8>]) -> (Vec<Event>, Vec<Msg>) {
        let mut buf: Vec<u8> = msgs.concat();
        let mut events = Vec::new();
        let mut out = Vec::new();
        while let Ok(Some(m)) = take(&mut buf) {
            if let Some(e) = r.on(&m, &mut out) {
                events.push(e);
            }
        }
        let mut sent = Vec::new();
        while let Ok(Some(m)) = take(&mut out) {
            sent.push(m);
        }
        (events, sent)
    }

    fn string_arg(m: &Msg) -> String {
        Args::new(&m.body).string().unwrap()
    }

    #[test]
    fn messages_round_trip_whatever_the_cut() {
        let a = encode(7, 3, &[Arg::Str("ei_pointer"), Arg::U32(1)]);
        assert_eq!(a.len() % 4, 0);
        let mut buf = a.clone();
        buf.extend(encode(1, 0, &[]));
        let mut part = buf[..10].to_vec();
        assert_eq!(take(&mut part), Ok(None));
        let m = take(&mut buf).unwrap().unwrap();
        assert_eq!((m.object, m.opcode), (7, 3));
        let mut args = Args::new(&m.body);
        assert_eq!(args.string().as_deref(), Some("ei_pointer"));
        assert_eq!(args.u32(), Some(1));
        assert_eq!(take(&mut buf).unwrap().unwrap().object, 1);
        let mut bad = encode(1, 0, &[]);
        bad[8] = 4;
        assert_eq!(take(&mut bad), Err(Garbled));
    }

    #[test]
    fn the_handshake_seat_and_input_come_through() {
        let mut r = Receiver::new("my-domain");
        let conn = 0xff00_0000_0000_0001u64;
        let seat = 0xff00_0000_0000_0002u64;
        let dev = 0xff00_0000_0000_0003u64;
        let (ptr, btn, scr, kbd) = (dev + 1, dev + 2, dev + 3, dev + 4);
        // The server's version: we answer with the whole handshake.
        let (ev, sent) = run(&mut r, &[encode(HANDSHAKE, 0, &[Arg::U32(1)])]);
        assert!(ev.is_empty());
        assert_eq!(sent[0].opcode, 0);
        assert_eq!(
            (sent[1].opcode, string_arg(&sent[1])),
            (3, "my-domain".into())
        );
        assert_eq!(sent[2].opcode, 2);
        assert_eq!(Args::new(&sent[2].body).u32(), Some(RECEIVER));
        assert!(sent
            .iter()
            .any(|m| m.opcode == 4 && string_arg(m) == "ei_keyboard"));
        assert_eq!(sent.last().map(|m| m.opcode), Some(1));

        let (_, sent) = run(
            &mut r,
            &[
                encode(HANDSHAKE, 2, &[Arg::U32(1), Arg::U64(conn), Arg::U32(1)]),
                encode(conn, 1, &[Arg::U64(seat), Arg::U32(1)]),
                encode(seat, 2, &[Arg::U64(1 << 0), Arg::Str("ei_pointer")]),
                encode(
                    seat,
                    2,
                    &[Arg::U64(1 << 1), Arg::Str("ei_pointer_absolute")],
                ),
                encode(seat, 2, &[Arg::U64(1 << 2), Arg::Str("ei_scroll")]),
                encode(seat, 2, &[Arg::U64(1 << 3), Arg::Str("ei_button")]),
                encode(seat, 2, &[Arg::U64(1 << 4), Arg::Str("ei_keyboard")]),
                encode(seat, 3, &[]),
            ],
        );
        // Bound: pointer, scroll, button, keyboard (not absolute).
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].object, sent[0].opcode), (seat, 1));
        assert_eq!(Args::new(&sent[0].body).u64(), Some(0b11101));

        let (ev, sent) = run(
            &mut r,
            &[
                encode(seat, 4, &[Arg::U64(dev), Arg::U32(1)]),
                encode(
                    dev,
                    5,
                    &[Arg::U64(ptr), Arg::Str("ei_pointer"), Arg::U32(1)],
                ),
                encode(dev, 5, &[Arg::U64(btn), Arg::Str("ei_button"), Arg::U32(1)]),
                encode(dev, 5, &[Arg::U64(scr), Arg::Str("ei_scroll"), Arg::U32(1)]),
                encode(
                    dev,
                    5,
                    &[Arg::U64(kbd), Arg::Str("ei_keyboard"), Arg::U32(1)],
                ),
                encode(dev, 9, &[Arg::U32(5), Arg::U32(1)]),
                encode(ptr, 1, &[f(1.5), f(-2.0)]),
                encode(btn, 1, &[Arg::U32(0x110), Arg::U32(1)]),
                encode(scr, 2, &[Arg::U32(0), Arg::U32(120)]),
                encode(kbd, 2, &[Arg::U32(30), Arg::U32(0)]),
                encode(conn, 3, &[Arg::U64(77), Arg::U32(1)]),
                encode(dev, 10, &[Arg::U32(6)]),
            ],
        );
        assert_eq!(
            ev,
            vec![
                Event::Emulating(true),
                Event::Motion { dx: 1.5, dy: -2.0 },
                Event::Button {
                    button: 0x110,
                    down: true
                },
                Event::ScrollDiscrete { dx: 0, dy: 120 },
                Event::Key {
                    key: 30,
                    down: false
                },
                Event::Emulating(false),
            ]
        );
        // The ping is answered on its pingpong object.
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].object, sent[0].opcode), (77, 0));

        // A destroyed device's interfaces go quiet only when they're
        // destroyed themselves; unknown objects are ignored.
        let (ev, _) = run(
            &mut r,
            &[
                encode(ptr, 0, &[Arg::U32(9)]),
                encode(ptr, 1, &[f(1.0), f(1.0)]),
                encode(12345, 1, &[f(1.0), f(1.0)]),
                encode(conn, 0, &[Arg::U32(1), Arg::U32(2), Arg::Str("bye")]),
            ],
        );
        assert_eq!(ev, vec![Event::Disconnected("bye".into())]);
    }

    #[test]
    fn slow_motion_adds_up() {
        let mut m = Motion::default();
        assert_eq!(m.add(0.4, -0.4), (0, 0));
        assert_eq!(m.add(0.4, -0.4), (0, 0));
        assert_eq!(m.add(0.4, -0.4), (1, -1));
        assert_eq!(m.add(3.0, 0.0), (3, 0));
    }
}
