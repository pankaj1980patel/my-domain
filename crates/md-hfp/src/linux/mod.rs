//! The hands-free unit on Linux: one worker thread owns the link (the
//! RFCOMM channel and its [`Engine`]) and the audio link; everything else
//! talks to it through a channel. Threads around it:
//!
//! - zbus's own executor thread runs our `org.bluez.Profile1` object and
//!   forwards BlueZ's calls as messages;
//! - an RFCOMM reader per link, an SCO listener while a link is up, an SCO
//!   pump per audio link; each only reads and forwards;
//! - a dispatcher calls the app's `on_event` in order, so the app may call
//!   back into [`Hfp`] from there without deadlocking the worker.

mod bluez;
mod sco;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use crate::engine::{Config, Engine, Event, Output};
use crate::{
    address_from_path, conflict_message, likely_owner, parse_address, Gateway, HfEvent, Owner,
    ScoPcm,
};

/// How long a command waits for the worker.
const REPLY_WITHIN: Duration = Duration::from_secs(5);
/// After asking BlueZ to connect: no channel by then means someone else
/// took it.
const CONNECT_WITHIN: Duration = Duration::from_secs(10);
/// After "audio on the phone": the phone's own audio link attempts are
/// turned away for this long.
const KEEP_ON_PHONE: Duration = Duration::from_secs(4);
/// CVSD's sample rate.
const CVSD_HZ: u32 = 8000;

const NOT_CONNECTED: &str =
    "Your phone isn\u{2019}t connected for calls. Connect it in Settings \u{2192} Calls on this computer.";

type Job = Box<dyn FnOnce(&mut Worker) + Send>;

/// What the worker hears.
pub(crate) enum Msg {
    NewConnection {
        device: String,
        fd: OwnedFd,
    },
    RequestDisconnection(String),
    Released,
    Rx {
        gen: u64,
        bytes: Vec<u8>,
    },
    RxClosed(u64),
    /// An audio link arrived (listener) or was opened (by us).
    Sco {
        fd: OwnedFd,
        remote: [u8; 6],
    },
    ScoClosed(u64),
    ConnectFailed {
        gen: u64,
        err: zbus::Error,
    },
    ConnectTimeout(u64),
    Do(Job),
}

/// Readable without asking the worker.
#[derive(Default)]
struct Shared {
    name: String,
    audio: Option<Arc<ScoPcm>>,
    problem: Option<String>,
    names: HashMap<String, String>,
}

/// The hands-free unit (see the crate docs). Cheap to share: every method
/// hands its work to the worker thread.
pub struct Hfp {
    tx: Sender<Msg>,
    shared: Arc<Mutex<Shared>>,
}

// The app keeps it in a static and calls it from any thread.
const _: fn() = || {
    fn shareable<T: Send + Sync>() {}
    shareable::<Hfp>();
};

impl Hfp {
    /// Connect to BlueZ, register the hands-free profile and start the
    /// threads. `on_event` runs on a thread of its own, in order.
    pub fn start(on_event: impl Fn(HfEvent) + Send + 'static) -> Result<Hfp, String> {
        let (tx, rx) = mpsc::channel();
        let conn = bluez::connect(tx.clone())
            .map_err(|e| format!("Bluetooth (BlueZ) isn\u{2019}t available: {e}"))?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let features = Config::cvsd_only().features as u16;
        let mut problem = match bluez::register(&conn, features) {
            Ok(bluez::Registered::Ok) => None,
            Ok(bluez::Registered::Taken) => Some(conflict_message(owner(&conn))),
            Err(e) => {
                return Err(format!(
                    "Bluetooth (BlueZ) refused the hands-free role: {e}"
                ))
            }
        };
        if problem.is_none() && bluez::ofono_running(&conn) {
            problem = Some(conflict_message(Owner::Ofono));
        }
        let adapter = bluez::adapter_address(&conn)
            .and_then(|a| parse_address(&a))
            .unwrap_or([0; 6]);
        if let Ok(mut s) = shared.lock() {
            s.problem.clone_from(&problem);
        }
        let (etx, erx) = mpsc::channel::<HfEvent>();
        std::thread::Builder::new()
            .name("hfp-events".into())
            .spawn(move || {
                for ev in erx {
                    on_event(ev);
                }
            })
            .map_err(|e| e.to_string())?;
        if problem.is_some() {
            let _ = etx.send(HfEvent::Problem(problem));
        }
        let mut worker = Worker {
            conn,
            tx: tx.clone(),
            events: etx,
            shared: shared.clone(),
            adapter,
            gen: 0,
            want: None,
            connecting: None,
            link: None,
            audio: None,
            listening: None,
            keep_on_phone_until: None,
        };
        std::thread::Builder::new()
            .name("hfp".into())
            .spawn(move || worker.run(rx))
            .map_err(|e| e.to_string())?;
        Ok(Hfp { tx, shared })
    }

    /// Run `f` on the worker and wait for its answer.
    fn ask<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Worker) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (rtx, rrx) = mpsc::channel();
        self.tx
            .send(Msg::Do(Box::new(move |w| {
                let _ = rtx.send(f(w));
            })))
            .map_err(|_| "Bluetooth calls stopped.".to_string())?;
        rrx.recv_timeout(REPLY_WITHIN)
            .map_err(|_| "Bluetooth didn\u{2019}t answer in time.".to_string())?
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Another program has the hands-free role, or similar (for Settings).
    pub fn problem(&self) -> Option<String> {
        self.shared().problem.clone()
    }

    /// Paired phones offering the Hands-Free Audio Gateway service.
    pub fn gateways(&self) -> Result<Vec<Gateway>, String> {
        self.ask(|w| {
            let list = bluez::gateways(&w.conn)?;
            let mut s = w.shared();
            for d in &list {
                s.names
                    .insert(d.gateway.address.clone(), d.gateway.name.clone());
            }
            Ok(list.into_iter().map(|d| d.gateway).collect())
        })
    }

    /// Connect to the paired phone at `address` (the link comes up later:
    /// [`Event::Ready`]).
    pub fn connect(&self, address: &str) -> Result<(), String> {
        let address = address.to_ascii_uppercase();
        self.ask(move |w| w.connect(address))
    }

    /// Drop the link (and stop wanting it).
    pub fn disconnect(&self) -> Result<(), String> {
        self.ask(|w| {
            w.want = None;
            w.connecting = None;
            if let Some(l) = &w.link {
                let (conn, path) = (w.conn.clone(), l.path.clone());
                std::thread::spawn(move || {
                    let _ = bluez::profile(&conn, &path, false);
                });
            }
            w.teardown();
            Ok(())
        })
    }

    /// Answer the ringing call; `audio_here` false keeps the audio on the
    /// phone.
    pub fn answer(&self, audio_here: bool) -> Result<(), String> {
        self.ask(move |w| {
            w.engine(|e| e.answer())?;
            if !audio_here {
                w.keep_on_phone();
            }
            Ok(())
        })
    }

    pub fn hang_up(&self) -> Result<(), String> {
        self.ask(|w| w.engine(|e| e.hang_up()))
    }

    pub fn dial(&self, number: &str) -> Result<(), String> {
        let n = number.to_string();
        self.ask(move |w| w.engine(|e| e.dial(&n)))
    }

    pub fn redial(&self) -> Result<(), String> {
        self.ask(|w| w.engine(|e| e.redial()))
    }

    pub fn dtmf(&self, c: char) -> Result<(), String> {
        self.ask(move |w| w.engine(|e| e.dtmf(c)))
    }

    /// Ask for the call list (callers' names, a call already under way).
    pub fn list_calls(&self) -> Result<(), String> {
        self.ask(|w| w.engine(|e| e.list_calls()))
    }

    /// Bring the call audio here: ask the phone (`AT+BCC`) when it
    /// negotiates codecs, else open the audio link ourselves.
    pub fn audio_here(&self) -> Result<(), String> {
        self.ask(|w| {
            w.keep_on_phone_until = None;
            if w.audio.is_some() {
                return Ok(());
            }
            let negotiates = w
                .link
                .as_ref()
                .is_some_and(|l| l.engine.codec_negotiation());
            if negotiates {
                w.engine(|e| e.audio_connect())
            } else {
                w.connect_audio()
            }
        })
    }

    /// Close the audio link: the phone's earpiece takes over; the link
    /// (and the controls) stays.
    pub fn audio_to_phone(&self) -> Result<(), String> {
        self.ask(|w| {
            if w.link.is_none() {
                return Err(NOT_CONNECTED.into());
            }
            w.keep_on_phone();
            Ok(())
        })
    }

    /// Open the audio link directly (when the phone didn't on request).
    pub fn connect_audio(&self) -> Result<(), String> {
        self.ask(|w| {
            w.keep_on_phone_until = None;
            w.connect_audio()
        })
    }

    /// The open audio link's samples.
    pub fn audio(&self) -> Option<Arc<ScoPcm>> {
        self.shared().audio.clone()
    }

    /// The connected phone's name (empty when none).
    pub fn device_name(&self) -> String {
        self.shared().name.clone()
    }

    /// A paired phone's name, when seen.
    pub fn cached_name(&self, address: &str) -> Option<String> {
        self.shared()
            .names
            .get(&address.to_ascii_uppercase())
            .cloned()
    }
}

struct Link {
    gen: u64,
    address: String,
    path: String,
    writer: File,
    engine: Engine,
}

struct Audio {
    gen: u64,
    fd: Arc<OwnedFd>,
    stop: Arc<AtomicBool>,
}

pub(crate) struct Worker {
    conn: Connection,
    tx: Sender<Msg>,
    events: Sender<HfEvent>,
    shared: Arc<Mutex<Shared>>,
    adapter: [u8; 6],
    gen: u64,
    /// The phone to be connected to.
    want: Option<String>,
    /// A connect attempt under way.
    connecting: Option<u64>,
    link: Option<Link>,
    audio: Option<Audio>,
    /// The SCO listener's stop flag while it runs.
    listening: Option<Arc<AtomicBool>>,
    keep_on_phone_until: Option<Instant>,
}

impl Worker {
    fn run(&mut self, rx: Receiver<Msg>) {
        for msg in rx {
            self.handle(msg);
        }
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn emit(&self, e: HfEvent) {
        let _ = self.events.send(e);
    }

    fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    fn set_problem(&self, p: Option<String>) {
        let changed = {
            let mut s = self.shared();
            let changed = s.problem != p;
            s.problem.clone_from(&p);
            changed
        };
        if changed {
            self.emit(HfEvent::Problem(p));
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Do(job) => job(self),
            Msg::NewConnection { device, fd } => self.new_connection(device, fd),
            Msg::RequestDisconnection(device) => {
                if self.link.as_ref().is_some_and(|l| l.path == device) {
                    self.teardown();
                }
            }
            Msg::Released => {
                eprintln!("[hfp] BlueZ released the hands-free profile");
                self.teardown();
            }
            Msg::Rx { gen, bytes } => {
                if let Some(l) = self.link.as_mut().filter(|l| l.gen == gen) {
                    l.engine.receive(&bytes);
                    self.flush();
                }
            }
            Msg::RxClosed(gen) => {
                if self.link.as_ref().is_some_and(|l| l.gen == gen) {
                    self.teardown();
                }
            }
            Msg::Sco { fd, remote } => self.sco(fd, remote),
            Msg::ScoClosed(gen) => {
                if self.audio.as_ref().is_some_and(|a| a.gen == gen) {
                    self.audio = None;
                    self.shared().audio = None;
                    self.emit(HfEvent::Audio(None));
                }
            }
            Msg::ConnectFailed { gen, err } => {
                if self.connecting == Some(gen) && self.link.is_none() {
                    self.connecting = None;
                    eprintln!("[hfp] connect failed: {err}");
                    let msg = match bluez::connect_error(&err) {
                        Some(m) => m,
                        None => {
                            // Already connected, but not to us.
                            let m = self.conflict();
                            self.set_problem(Some(m.clone()));
                            m
                        }
                    };
                    self.emit(HfEvent::ConnectFailed(msg));
                }
            }
            Msg::ConnectTimeout(gen) => {
                if self.connecting == Some(gen) && self.link.is_none() {
                    // BlueZ connected the channel, but not to us.
                    self.connecting = None;
                    let msg = self.conflict();
                    self.set_problem(Some(msg.clone()));
                    self.emit(HfEvent::ConnectFailed(msg));
                }
            }
        }
    }

    /// Someone else has the hands-free role: who, and what to do.
    fn conflict(&self) -> String {
        conflict_message(owner(&self.conn))
    }

    fn connect(&mut self, address: String) -> Result<(), String> {
        if parse_address(&address).is_none() {
            return Err("That isn\u{2019}t a Bluetooth address.".into());
        }
        let again = self.want.as_deref() == Some(address.as_str());
        self.want = Some(address.clone());
        if self.link.as_ref().is_some_and(|l| l.address == address)
            || (again && self.connecting.is_some())
        {
            // Connected, or connecting already (BlueZ would say "in
            // progress").
            return Ok(());
        }
        self.teardown();
        let dev = bluez::gateways(&self.conn)?
            .into_iter()
            .find(|d| d.gateway.address.eq_ignore_ascii_case(&address))
            .ok_or("That phone isn\u{2019}t paired with this computer any more (or doesn\u{2019}t offer calls). Pair it in your Bluetooth settings first.")?;
        self.shared()
            .names
            .insert(address, dev.gateway.name.clone());
        let gen = self.next_gen();
        self.connecting = Some(gen);
        let (conn, tx, path) = (self.conn.clone(), self.tx.clone(), dev.path);
        std::thread::spawn(move || {
            if let Err(err) = bluez::profile(&conn, &path, true) {
                let _ = tx.send(Msg::ConnectFailed { gen, err });
            }
        });
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(CONNECT_WITHIN);
            let _ = tx.send(Msg::ConnectTimeout(gen));
        });
        Ok(())
    }

    fn new_connection(&mut self, device: String, fd: OwnedFd) {
        let Some(address) = address_from_path(&device) else {
            return;
        };
        if self.want.as_deref() != Some(address.as_str()) {
            // Not the phone picked in Settings (or calls are off): closing
            // the fd tells BlueZ no.
            eprintln!("[hfp] turned away a hands-free channel from {address}");
            return;
        }
        self.teardown();
        self.connecting = None;
        if let Err(e) = sco::set_blocking(fd.as_raw_fd()) {
            eprintln!("[hfp] can't use the channel: {e}");
            return;
        }
        let writer = File::from(fd);
        let reader = match writer.try_clone() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[hfp] can't read the channel: {e}");
                return;
            }
        };
        let gen = self.next_gen();
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("hfp-rfcomm".into())
            .spawn(move || read_loop(reader, gen, tx));
        let name = bluez::alias(&self.conn, &device).unwrap_or_else(|| address.clone());
        {
            let mut s = self.shared();
            s.name.clone_from(&name);
            s.names.insert(address.clone(), name);
        }
        // It's ours: whatever kept it from us before is gone.
        self.set_problem(None);
        let mut engine = Engine::new(Config::cvsd_only());
        engine.start();
        self.link = Some(Link {
            gen,
            address,
            path: device,
            writer,
            engine,
        });
        self.flush();
    }

    /// Write what the engine wants written; pass its events on.
    fn flush(&mut self) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let out = link.engine.drain();
        let mut broken = false;
        let mut events = Vec::new();
        for o in out {
            match o {
                Output::Send(s) => {
                    if let Err(e) = link.writer.write_all(s.as_bytes()) {
                        eprintln!("[hfp] write failed: {e}");
                        broken = true;
                        break;
                    }
                }
                Output::Event(e) => events.push(e),
            }
        }
        for e in events {
            match &e {
                Event::Ready { .. } => self.listen(),
                Event::SetupFailed(why) => {
                    eprintln!("[hfp] setup failed: {why}");
                    self.emit(HfEvent::ConnectFailed(format!(
                        "Your phone didn\u{2019}t accept this computer as a hands-free ({why})."
                    )));
                    broken = true;
                }
                _ => {}
            }
            self.emit(HfEvent::At(e));
        }
        if broken {
            self.teardown();
        }
    }

    /// Run an engine command and send it.
    fn engine(
        &mut self,
        f: impl FnOnce(&mut Engine) -> Result<(), crate::engine::Error>,
    ) -> Result<(), String> {
        let link = self.link.as_mut().ok_or(NOT_CONNECTED)?;
        f(&mut link.engine).map_err(|e| match e {
            crate::engine::Error::NotReady => NOT_CONNECTED.to_string(),
            other => other.to_string(),
        })?;
        self.flush();
        Ok(())
    }

    /// Drop the link and the audio (says `Disconnected` if there was one).
    fn teardown(&mut self) {
        self.close_audio();
        if let Some(stop) = self.listening.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(l) = self.link.take() {
            sco::shutdown(l.writer.as_raw_fd());
            self.shared().name.clear();
            self.emit(HfEvent::Disconnected);
        }
    }

    fn keep_on_phone(&mut self) {
        self.keep_on_phone_until = Some(Instant::now() + KEEP_ON_PHONE);
        self.close_audio();
    }

    fn close_audio(&mut self) {
        if let Some(a) = self.audio.take() {
            a.stop.store(true, Ordering::Relaxed);
            sco::shutdown(a.fd.as_raw_fd());
            self.shared().audio = None;
            self.emit(HfEvent::Audio(None));
        }
    }

    /// Listen for the phone's audio links while the link is up.
    fn listen(&mut self) {
        if self.listening.is_some() {
            return;
        }
        let listener = match sco::listen(self.adapter) {
            Ok(l) => l,
            Err(e) => {
                // Someone else listens (PipeWire's own hands-free or
                // headset roles): the phone's links go there, so audio
                // comes only from links we open (`connect_audio`).
                eprintln!("[hfp] can't listen for call audio ({e}); opening it from here only");
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.listening = Some(stop.clone());
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("hfp-sco-listen".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match sco::accept(&listener, Duration::from_millis(500)) {
                        Ok(Some((fd, remote))) => {
                            let _ = tx.send(Msg::Sco { fd, remote });
                        }
                        Ok(None) => {}
                        Err(e) => {
                            eprintln!("[hfp] call audio listener stopped: {e}");
                            break;
                        }
                    }
                }
            });
    }

    /// Open the audio link to the phone (in the background).
    fn connect_audio(&mut self) -> Result<(), String> {
        let link = self.link.as_ref().ok_or(NOT_CONNECTED)?;
        if self.audio.is_some() {
            return Ok(());
        }
        let remote = parse_address(&link.address).ok_or(NOT_CONNECTED)?;
        let (local, tx) = (self.adapter, self.tx.clone());
        std::thread::spawn(move || match sco::connect(local, remote) {
            Ok(fd) => {
                let _ = tx.send(Msg::Sco { fd, remote });
            }
            Err(e) => eprintln!("[hfp] couldn't open the call audio link: {e}"),
        });
        Ok(())
    }

    /// An audio link: ours to pump if it's the phone's and wanted here.
    fn sco(&mut self, fd: OwnedFd, remote: [u8; 6]) {
        let ours = self
            .link
            .as_ref()
            .and_then(|l| parse_address(&l.address))
            .is_some_and(|a| a == remote);
        let refused = self.keep_on_phone_until.is_some_and(|t| Instant::now() < t);
        if !ours || refused || self.audio.is_some() {
            // Dropping the fd closes the link.
            return;
        }
        let gen = self.next_gen();
        let pcm = Arc::new(ScoPcm::new(CVSD_HZ));
        let stop = Arc::new(AtomicBool::new(false));
        let fd = Arc::new(fd);
        {
            let (pcm, stop, fd, tx) = (pcm.clone(), stop.clone(), fd.clone(), self.tx.clone());
            let _ = std::thread::Builder::new()
                .name("hfp-sco".into())
                .spawn(move || {
                    pump(&fd, &pcm, &stop);
                    let _ = tx.send(Msg::ScoClosed(gen));
                });
        }
        self.audio = Some(Audio { gen, fd, stop });
        self.shared().audio = Some(pcm.clone());
        self.emit(HfEvent::Audio(Some(pcm)));
    }
}

/// RFCOMM bytes to the worker until the channel closes.
fn read_loop(mut reader: File, gen: u64, tx: Sender<Msg>) {
    let mut buf = [0u8; 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx
                    .send(Msg::Rx {
                        gen,
                        bytes: buf[..n].to_vec(),
                    })
                    .is_err()
                {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = tx.send(Msg::RxClosed(gen));
}

/// Move one audio link's packets: each one in, one of the same size out.
fn pump(fd: &OwnedFd, pcm: &ScoPcm, stop: &AtomicBool) {
    if let Err(e) = sco::set_read_timeout(fd, Duration::from_millis(300)) {
        eprintln!("[hfp] call audio: {e}");
    }
    let mut buf = vec![0u8; sco::mtu(fd).max(48) * 4];
    while !stop.load(Ordering::Relaxed) {
        match sco::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                pcm.from_air(&buf[..n]);
                let out = pcm.to_air(n);
                if sco::write(fd, &out).is_err() {
                    break;
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }
    sco::shutdown(fd.as_raw_fd());
}

/// Who probably has the hands-free role.
fn owner(conn: &Connection) -> Owner {
    let mut programs = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for e in dir.flatten() {
            if e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
            {
                if let Ok(comm) = std::fs::read_to_string(e.path().join("comm")) {
                    programs.push(comm.trim().to_string());
                }
            }
        }
    }
    likely_owner(
        programs.iter().map(String::as_str),
        bluez::ofono_running(conn),
    )
}
