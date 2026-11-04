//! The hands-free unit on Windows: call control only.
//!
//! Why this shape (researched for Windows 10 and 11):
//!
//! - Windows' own hands-free role (what Phone Link's "Calls" uses) is
//!   behind `Windows.ApplicationModel.Calls.PhoneLineTransportDevice`,
//!   which needs the *restricted* capability `phoneLineTransportManagement`
//!   and so package identity (MSIX, or a sparse package registered in
//!   Developer Mode). Without identity `RequestAccessAsync` answers
//!   `DeniedBySystem`; even with it, `RegisterApp` has been reported to
//!   leave `IsRegistered` false for full-trust desktop apps. An unpackaged
//!   app like this one can't use it.
//! - What does work for any desktop program is an RFCOMM socket (Winsock,
//!   `AF_BTH`) to the phone's Hands-Free Audio Gateway service (0x111F):
//!   Windows finds the channel over SDP and the same AT [`Engine`] as on
//!   Linux runs over it. That's the whole service-level link: ringing,
//!   caller id, answer, decline, hang up, dial, redial, keypad tones,
//!   signal and battery.
//! - Call audio rides a SCO link, and Windows opens SCO only for
//!   kernel-mode Bluetooth profile drivers (`BRB_SCO_OPEN_CHANNEL`): no
//!   user-mode API carries it. So the audio stays on the phone (its
//!   earpiece, speaker, or a headset paired with it); [`Hfp::audio`] is
//!   always `None` and asking for the audio here says why.
//!
//! If Windows' own hands-free (Phone Link's Calls) already holds the
//! phone, the phone turns this link down; [`Hfp::problem`] then says how to
//! free it ([`crate::WINDOWS_CONFLICT`]).
//!
//! Unlike BlueZ there's no profile the phone can connect *to* here, so
//! this side always connects, and while a phone is wanted a dropped or
//! failed link is retried quietly every [`RETRY_EVERY`].
//!
//! Threads: one worker owns the link and its engine; a reader per link
//! forwards bytes; a dispatcher calls the app's `on_event` in order.

mod bt;

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::engine::{Config, Engine, Event, Output};
use crate::{address_to_u64, windows_connect_error, ConnectFailure, Gateway, HfEvent, ScoPcm};

/// How long a command waits for the worker.
const REPLY_WITHIN: Duration = Duration::from_secs(5);
/// While a phone is wanted and not connected, try again this often.
const RETRY_EVERY: Duration = Duration::from_secs(30);

const NOT_CONNECTED: &str =
    "Your phone isn\u{2019}t connected for calls. Connect it in Settings \u{2192} Calls on this computer.";

/// Asking for the call audio on this computer.
pub const NO_AUDIO: &str = "Call audio stays on your phone: Windows only lets Bluetooth \
drivers built into Windows carry phone-call audio, not apps. Answer, hang up, dial and the \
keypad still work from here.";

type Job = Box<dyn FnOnce(&mut Worker) + Send>;

enum Msg {
    Do(Job),
    Connected { gen: u64, sock: bt::Rfcomm },
    ConnectFailed { gen: u64, code: i32 },
    Retry(u64),
    Rx { gen: u64, bytes: Vec<u8> },
    RxClosed(u64),
}

/// Readable without asking the worker.
#[derive(Default)]
struct Shared {
    name: String,
    problem: Option<String>,
    names: HashMap<String, String>,
}

/// The hands-free unit (see the module docs). Cheap to share: every method
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

fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|p| p.into_inner())
}

impl Hfp {
    /// Start Winsock and the threads. `on_event` runs on a thread of its
    /// own, in order.
    pub fn start(on_event: impl Fn(HfEvent) + Send + 'static) -> Result<Hfp, String> {
        bt::startup()?;
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (etx, erx) = mpsc::channel::<HfEvent>();
        std::thread::Builder::new()
            .name("hfp-events".into())
            .spawn(move || {
                for ev in erx {
                    on_event(ev);
                }
            })
            .map_err(|e| e.to_string())?;
        let mut worker = Worker {
            tx: tx.clone(),
            events: etx,
            shared: shared.clone(),
            gen: 0,
            want: None,
            connecting: None,
            link: None,
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

    /// Something keeps calls from working (Windows' own hands-free has
    /// the phone), for Settings.
    pub fn problem(&self) -> Option<String> {
        lock(&self.shared).problem.clone()
    }

    /// Paired phones offering calls. Asks Windows directly (no radio
    /// traffic), on the caller's thread.
    pub fn gateways(&self) -> Result<Vec<Gateway>, String> {
        let list = bt::paired_gateways()?;
        let mut s = lock(&self.shared);
        for g in &list {
            s.names.insert(g.address.clone(), g.name.clone());
        }
        Ok(list)
    }

    /// Connect to the paired phone at `address` (the link comes up later:
    /// [`Event::Ready`]; a failure arrives as [`HfEvent::ConnectFailed`]).
    pub fn connect(&self, address: &str) -> Result<(), String> {
        let address = address.to_ascii_uppercase();
        if address_to_u64(&address).is_none() {
            return Err("That isn\u{2019}t a Bluetooth address.".into());
        }
        // The name for Settings, and a check it's still paired (not on the
        // worker: it's a system call that can take a moment).
        if let Ok(list) = self.gateways() {
            if !list.iter().any(|g| g.address == address) {
                return Err("That phone isn\u{2019}t paired with this computer any more (or doesn\u{2019}t offer calls). Pair it in Windows Settings \u{2192} Bluetooth & devices first.".into());
            }
        }
        self.ask(move |w| w.connect(address))
    }

    /// Drop the link (and stop wanting it).
    pub fn disconnect(&self) -> Result<(), String> {
        self.ask(|w| {
            w.want = None;
            w.connecting = None;
            w.next_gen();
            w.teardown();
            Ok(())
        })
    }

    /// Answer the ringing call. The audio stays on the phone whatever
    /// `audio_here` says (see the module docs).
    pub fn answer(&self, _audio_here: bool) -> Result<(), String> {
        self.ask(|w| w.engine(|e| e.answer()))
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

    /// The call audio can't come to this computer on Windows.
    pub fn audio_here(&self) -> Result<(), String> {
        Err(NO_AUDIO.into())
    }

    /// The audio is on the phone already.
    pub fn audio_to_phone(&self) -> Result<(), String> {
        self.ask(|w| {
            if w.link.is_none() {
                return Err(NOT_CONNECTED.into());
            }
            Ok(())
        })
    }

    pub fn connect_audio(&self) -> Result<(), String> {
        Err(NO_AUDIO.into())
    }

    /// Never any samples on Windows.
    pub fn audio(&self) -> Option<Arc<ScoPcm>> {
        None
    }

    /// The connected phone's name (empty when none).
    pub fn device_name(&self) -> String {
        lock(&self.shared).name.clone()
    }

    /// A paired phone's name, when seen.
    pub fn cached_name(&self, address: &str) -> Option<String> {
        lock(&self.shared)
            .names
            .get(&address.to_ascii_uppercase())
            .cloned()
    }
}

struct Link {
    gen: u64,
    address: String,
    sock: Arc<bt::Rfcomm>,
    engine: Engine,
}

struct Worker {
    tx: Sender<Msg>,
    events: Sender<HfEvent>,
    shared: Arc<Mutex<Shared>>,
    /// Bumped by every connect, link and disconnect: stale results and
    /// retries compare against it.
    gen: u64,
    /// The phone to be connected to.
    want: Option<String>,
    /// A connect attempt under way: its gen, and whether a failure is said
    /// out loud (asked for) or not (a background retry).
    connecting: Option<(u64, bool)>,
    link: Option<Link>,
}

impl Worker {
    fn run(&mut self, rx: Receiver<Msg>) {
        for msg in rx {
            self.handle(msg);
        }
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        lock(&self.shared)
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
            Msg::Connected { gen, sock } => {
                if self.connecting.is_some_and(|(g, _)| g == gen) && self.link.is_none() {
                    self.new_link(sock);
                } else {
                    // Stale (disconnected or replaced meanwhile): dropping
                    // it closes it.
                    sock.shutdown();
                }
            }
            Msg::ConnectFailed { gen, code } => {
                let Some((g, loud)) = self.connecting else {
                    return;
                };
                if g != gen || self.link.is_some() {
                    return;
                }
                self.connecting = None;
                let (msg, how) = windows_connect_error(code);
                eprintln!("[hfp] connect failed (Winsock {code})");
                if how == ConnectFailure::Taken {
                    self.set_problem(Some(msg.clone()));
                }
                if loud {
                    self.emit(HfEvent::ConnectFailed(msg));
                }
                if how == ConnectFailure::Away {
                    self.retry_later();
                }
            }
            Msg::Retry(gen) => {
                if gen == self.gen && self.link.is_none() && self.connecting.is_none() {
                    if let Some(address) = self.want.clone() {
                        self.start_connect(address, false);
                    }
                }
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
                    // The phone went (out of range, Bluetooth off): come
                    // back when it does.
                    self.next_gen();
                    self.retry_later();
                }
            }
        }
    }

    /// Try the wanted phone again after a while, unless something happens
    /// first.
    fn retry_later(&self) {
        if self.want.is_none() {
            return;
        }
        let (gen, tx) = (self.gen, self.tx.clone());
        let _ = std::thread::Builder::new()
            .name("hfp-retry".into())
            .spawn(move || {
                std::thread::sleep(RETRY_EVERY);
                let _ = tx.send(Msg::Retry(gen));
            });
    }

    fn connect(&mut self, address: String) -> Result<(), String> {
        let same = self.want.as_deref() == Some(address.as_str());
        self.want = Some(address.clone());
        if same && (self.link.is_some() || self.connecting.is_some()) {
            // Connected, or connecting already: a failure will be said.
            if let Some((_, loud)) = self.connecting.as_mut() {
                *loud = true;
            }
            return Ok(());
        }
        self.teardown();
        self.start_connect(address, true);
        Ok(())
    }

    fn start_connect(&mut self, address: String, loud: bool) {
        let Some(addr) = address_to_u64(&address) else {
            return;
        };
        let gen = self.next_gen();
        self.connecting = Some((gen, loud));
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("hfp-connect".into())
            .spawn(move || {
                let _ = match bt::Rfcomm::connect(addr) {
                    Ok(sock) => tx.send(Msg::Connected { gen, sock }),
                    Err(code) => tx.send(Msg::ConnectFailed { gen, code }),
                };
            });
        if spawned.is_err() {
            self.connecting = None;
        }
    }

    fn new_link(&mut self, sock: bt::Rfcomm) {
        let Some(address) = self.want.clone() else {
            return;
        };
        self.connecting = None;
        let gen = self.next_gen();
        let sock = Arc::new(sock);
        {
            let (reader, tx) = (sock.clone(), self.tx.clone());
            let _ = std::thread::Builder::new()
                .name("hfp-rfcomm".into())
                .spawn(move || read_loop(&reader, gen, &tx));
        }
        {
            let mut s = self.shared();
            let name = s
                .names
                .get(&address)
                .cloned()
                .unwrap_or_else(|| address.clone());
            s.name = name;
        }
        // It's ours: whatever kept it from us before is gone.
        self.set_problem(None);
        let mut engine = Engine::new(Config::cvsd_only());
        engine.start();
        self.link = Some(Link {
            gen,
            address,
            sock,
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
                    if let Err(e) = link.sock.send_all(s.as_bytes()) {
                        eprintln!("[hfp] write failed: {e}");
                        broken = true;
                        break;
                    }
                }
                Output::Event(e) => events.push(e),
            }
        }
        for e in events {
            if let Event::SetupFailed(why) = &e {
                eprintln!("[hfp] setup failed: {why}");
                self.emit(HfEvent::ConnectFailed(format!(
                    "Your phone didn\u{2019}t accept this computer as a hands-free ({why})."
                )));
                broken = true;
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

    /// Drop the link (says `Disconnected` if there was one).
    fn teardown(&mut self) {
        if let Some(l) = self.link.take() {
            eprintln!("[hfp] hands-free link to {} closed", l.address);
            l.sock.shutdown();
            self.shared().name.clear();
            self.emit(HfEvent::Disconnected);
        }
    }
}

/// RFCOMM bytes to the worker until the channel closes.
fn read_loop(sock: &bt::Rfcomm, gen: u64, tx: &Sender<Msg>) {
    let mut buf = [0u8; 1024];
    loop {
        match sock.recv(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let bytes = buf[..n].to_vec();
                if tx.send(Msg::Rx { gen, bytes }).is_err() {
                    return;
                }
            }
        }
    }
    let _ = tx.send(Msg::RxClosed(gen));
}
