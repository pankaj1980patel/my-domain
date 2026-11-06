// This computer's sound played on your phone (and the earbuds on it), over
// the encrypted link (mdcore `features::audio_share`, the other way round
// from `phone_audio.rs`: here this computer is the source, the phone the
// sink).
//
// Two ways in, and the phone's owner always taps first:
// - "Play on <phone>" on the phone's page here (`computer_audio_offer`):
//   the phone shows a prompt or notification, and plays only after a tap
//   there (its listen arrives as `CoreEvent::AudioListen`);
// - "Play <computer>'s sound here" on the phone: its listen arrives
//   directly.
// Either way, every start posts "Your computer's sound is playing on
// <phone>" (even when quiet), and while it plays a Stop sits in the tray
// menu and in the app's sidebar.
//
// Capture, per OS:
// - macOS 13 or later: ScreenCaptureKit (an `SCStream` with audio on and
//   this app's own audio excluded, so a phone's audio playing here never
//   loops back). It needs the Screen Recording permission ("Screen & System
//   Audio Recording"); macOS asks, and nothing is captured without it.
// - Linux: the default output's monitor, through cpal's ALSA host: an input
//   named like a monitor if there is one, else the PulseAudio plugin
//   pointed at `@DEFAULT_MONITOR@` (PulseAudio, and PipeWire's
//   pipewire-pulse).
// - Windows: WASAPI loopback (cpal opens the default output as an input).
// On Linux and Windows the capture includes everything this computer
// plays, a phone's audio played here too: so the two don't run together.
//
// What's captured is converted to s16le stereo at the link's rate (48 kHz
// direct, 24 kHz through the relay) in 20 ms frames (`Encoder`) and sent to
// the phones that listen. It stops on Stop (here, the tray, the phone),
// when the phones stop renewing their listens (gone, or the link went), or
// when the capture ends.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mdcore::features::audio_share::RATE_DIRECT;
use tauri::{AppHandle, Emitter, Manager};

use crate::Eng;

/// Milliseconds of audio per frame sent.
pub const FRAME_MS: u32 = 20;
/// The phones' listens: after a start, this long for the first.
const GRACE: Duration = Duration::from_secs(20);
/// No phone listened for this long: stop (a listen not renewed runs out
/// after 25 s, so a lost phone ends here).
const IDLE: Duration = Duration::from_secs(5);
/// "Play on <phone>" waits this long for the tap there.
const ASK_FOR: Duration = Duration::from_secs(90);
/// Who listens: looked at this often, not every frame.
const CHECK: Duration = Duration::from_millis(500);
/// Frames waiting to go out at most (a second): more are dropped.
const QUEUE: usize = 50;

// --- the pure part: captured samples to frames ---

/// Sends frames on a steady clock, one every `FRAME_MS`, however the
/// capture hands them over: ScreenCaptureKit and WASAPI deliver audio in
/// bursts, and a burst on the wire is a burst in the phone's buffer
/// (late frames, then an underrun). Frame `n` goes at `start + n * FRAME_MS`.
/// Falling behind by more than `MAX_BEHIND` (the capture stalled) starts
/// the clock again from now, as does getting ahead by more than
/// `MAX_AHEAD` (the capture's clock runs fast: never let a backlog grow).
#[derive(Default)]
pub struct Pacer {
    start: Option<Instant>,
    sent: u32,
}

/// Behind the clock by more than this: start it again.
pub const MAX_BEHIND: Duration = Duration::from_millis(60);
/// Ahead of real time by more than this: start it again.
pub const MAX_AHEAD: Duration = Duration::from_millis(100);

impl Pacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// When the next frame should go, given it's `now`.
    pub fn due(&mut self, now: Instant) -> Instant {
        let start = *self.start.get_or_insert(now);
        let due = start + Duration::from_millis(u64::from(FRAME_MS) * u64::from(self.sent));
        if now > due + MAX_BEHIND || due > now + MAX_AHEAD {
            self.start = Some(now);
            self.sent = 0;
            return now;
        }
        due
    }

    /// The frame went.
    pub fn sent(&mut self) {
        self.sent = self.sent.saturating_add(1);
    }
}

/// Captured audio (f32, any channels, any rate) to the frames the phone
/// plays: s16le stereo at `out_rate`, `FRAME_MS` each. Resampled linearly.
pub struct Encoder {
    in_rate: u32,
    in_channels: usize,
    out_rate: u32,
    /// The last input frame of the previous chunk (left, right).
    prev: Option<[f32; 2]>,
    /// Where the next output sample falls, in input frames from `prev`.
    pos: f64,
    /// Output samples (interleaved) not yet a whole frame.
    pending: Vec<i16>,
}

impl Encoder {
    pub fn new(out_rate: u32) -> Self {
        Encoder {
            in_rate: 0,
            in_channels: 0,
            out_rate,
            prev: None,
            pos: 0.0,
            pending: Vec::new(),
        }
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// Bytes in one frame sent.
    pub fn frame_bytes(&self) -> usize {
        (self.out_rate * FRAME_MS / 1000) as usize * 2 * 2
    }

    /// The capture's format; a change starts over.
    pub fn set_input(&mut self, rate: u32, channels: usize) {
        if rate != self.in_rate || channels != self.in_channels {
            self.in_rate = rate;
            self.in_channels = channels;
            self.prev = None;
            self.pos = 0.0;
        }
    }

    /// The rate to send at (the link's); a change drops what's pending.
    pub fn set_out_rate(&mut self, rate: u32) {
        if rate != self.out_rate && rate > 0 {
            self.out_rate = rate;
            self.pending.clear();
            self.pos = 0.0;
        }
    }

    /// Add captured samples (interleaved, in the format `set_input` gave);
    /// returns the whole frames now ready.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<u8>> {
        if self.in_rate == 0 || self.in_channels == 0 || self.out_rate == 0 {
            return Vec::new();
        }
        let ch = self.in_channels;
        let stereo = |f: &[f32]| -> [f32; 2] {
            match f {
                [m] => [*m, *m],
                [l, r, ..] => [*l, *r],
                _ => [0.0, 0.0],
            }
        };
        let mut frames: Vec<[f32; 2]> = Vec::with_capacity(samples.len() / ch + 1);
        frames.extend(self.prev);
        frames.extend(samples.chunks_exact(ch).map(stereo));
        if frames.len() < 2 {
            self.prev = frames.last().copied();
            return Vec::new();
        }
        let step = f64::from(self.in_rate) / f64::from(self.out_rate);
        let to_i16 = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        while self.pos + 1.0 < frames.len() as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            let (a, b) = (frames[i], frames[i + 1]);
            self.pending.push(to_i16(a[0] + (b[0] - a[0]) * f));
            self.pending.push(to_i16(a[1] + (b[1] - a[1]) * f));
            self.pos += step;
        }
        self.pos -= (frames.len() - 1) as f64;
        self.prev = frames.last().copied();
        let per = self.frame_bytes() / 2;
        let mut out = Vec::new();
        while self.pending.len() >= per {
            let frame: Vec<u8> = self
                .pending
                .drain(..per)
                .flat_map(i16::to_le_bytes)
                .collect();
            out.push(frame);
        }
        out
    }
}

/// Samples from buffers as captured: one buffer of interleaved channels,
/// or one buffer per channel (planar). Each is (channels in it, bytes);
/// samples are f32 (`float`) or 16-bit, native-endian. Returns them
/// interleaved, with the channel count. Used by the macOS capture (and tests).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn interleave(buffers: &[(usize, &[u8])], float: bool) -> Option<(Vec<f32>, usize)> {
    let size = if float { 4 } else { 2 };
    let read = |b: &[u8]| -> f32 {
        if float {
            f32::from_ne_bytes([b[0], b[1], b[2], b[3]])
        } else {
            f32::from(i16::from_ne_bytes([b[0], b[1]])) / 32768.0
        }
    };
    match buffers {
        [] => None,
        [(ch, data)] => {
            let ch = (*ch).max(1);
            let n = data.len() / size / ch * ch;
            Some((data.chunks_exact(size).take(n).map(read).collect(), ch))
        }
        _ => {
            // Planar: buffer k holds channel k (one channel each).
            let channels = buffers.len();
            let frames = buffers.iter().map(|(_, d)| d.len() / size).min()?;
            let mut out = Vec::with_capacity(frames * channels);
            for i in 0..frames {
                for (_, d) in buffers {
                    out.push(read(&d[i * size..(i + 1) * size]));
                }
            }
            Some((out, channels))
        }
    }
}

/// The tray's Stop line while it plays (`None` when it doesn't).
pub fn stop_label(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "Stop playing this computer\u{2019}s sound on {}",
        names.join(", ")
    ))
}

// --- the session ---

/// Where captured audio goes: the capture callback pushes, the sender
/// thread sends.
struct Feed {
    enc: Mutex<Encoder>,
    /// The rate to send at (the slowest link's).
    out_rate: AtomicU32,
    tx: SyncSender<(u32, Vec<u8>)>,
    /// The capture ended by itself (why).
    failed: Mutex<Option<String>>,
}

impl Feed {
    fn push(&self, samples: &[f32], rate: u32, channels: usize) {
        let Ok(mut e) = self.enc.lock() else {
            return;
        };
        e.set_input(rate, channels);
        e.set_out_rate(self.out_rate.load(Ordering::Relaxed));
        let rate = e.out_rate();
        for f in e.push(samples) {
            // A full queue (the link can't keep up): drop, never wait.
            let _ = self.tx.try_send((rate, f));
        }
    }

    fn fail(&self, why: String) {
        if let Ok(mut f) = self.failed.lock() {
            f.get_or_insert(why);
        }
    }
}

#[derive(Default)]
struct State {
    /// Phones it plays on: node -> the rate for its link.
    targets: BTreeMap<String, u32>,
    /// "Play on <phone>" sent, waiting for the tap there: node -> when.
    asked: BTreeMap<String, Instant>,
    /// The capture running, if any: set to stop it.
    stop: Option<Arc<AtomicBool>>,
    feed: Option<Arc<Feed>>,
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    targets: BTreeMap::new(),
    asked: BTreeMap::new(),
    stop: None,
    feed: None,
    error: None,
});
/// One start at a time (two phones' listens can come together).
static STARTING: Mutex<()> = Mutex::new(());

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where this computer's sound stands, for the phone's tile and the
/// sidebar.
#[derive(Clone, Debug, Default, serde::Serialize, specta::Type)]
pub struct ComputerAudioStatus {
    /// This computer can capture its sound (macOS 13 or later, Linux,
    /// Windows).
    pub supported: bool,
    /// The phones it plays on now.
    pub playing_to: Vec<String>,
    /// Phones asked to play it, waiting for the tap there.
    pub asked: Vec<String>,
    /// Why it stopped or couldn't start, if it did.
    pub error: Option<String>,
}

fn status() -> ComputerAudioStatus {
    let mut s = state();
    let now = Instant::now();
    s.asked.retain(|_, at| now.duration_since(*at) < ASK_FOR);
    ComputerAudioStatus {
        supported: supported(),
        playing_to: s.targets.keys().cloned().collect(),
        asked: s.asked.keys().cloned().collect(),
        error: s.error.clone(),
    }
}

fn emit(app: &AppHandle) {
    let _ = app.emit("computer-audio", status());
}

/// Whether this computer's sound is going to a phone now.
pub fn streaming() -> bool {
    !state().targets.is_empty()
}

/// The tray's Stop item while it plays: (id suffix, label).
pub fn tray_items(app: &AppHandle) -> Vec<(String, String)> {
    let nodes: Vec<String> = state().targets.keys().cloned().collect();
    let names: Vec<String> = nodes
        .iter()
        .map(|n| crate::notify::device_name(app, n))
        .collect();
    stop_label(&names)
        .map(|l| vec![("stop".to_string(), l)])
        .unwrap_or_default()
}

/// The tray's Stop was chosen.
pub fn tray_action(app: &AppHandle, _what: &str) {
    let app = app.clone();
    std::thread::spawn(move || stop_all(&app, true, None));
}

/// Whether this computer can capture its sound at all.
pub fn supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::available()
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

const NOT_SUPPORTED: &str =
    "Playing this computer\u{2019}s sound on a phone needs macOS 13 or later, Linux or Windows.";
const LOOPS_BACK: &str = "Stop playing the phone\u{2019}s audio here first: this computer\u{2019}s sound would carry it back to the phone.";

/// What must hold before capturing (the OS supports it, macOS's
/// permission, no loop back).
fn can_start() -> Result<(), String> {
    if !supported() {
        return Err(NOT_SUPPORTED.into());
    }
    #[cfg(target_os = "macos")]
    mac::permission()?;
    // Elsewhere the capture hears what this app plays too.
    if !cfg!(target_os = "macos") && crate::phone_audio::playing_here() {
        return Err(LOOPS_BACK.into());
    }
    Ok(())
}

/// "Play on <phone>": offer this computer's sound to `node`; it plays once
/// its owner taps on the phone.
fn offer(app: &AppHandle, node: &str) -> Result<(), String> {
    can_start()?;
    let engine = app.try_state::<Eng>().ok_or("not started yet")?;
    engine.audio_offer(node, true).map_err(|e| e.to_string())?;
    {
        let mut s = state();
        s.error = None;
        if !s.targets.contains_key(node) {
            s.asked.insert(node.to_string(), Instant::now());
        }
    }
    emit(app);
    Ok(())
}

/// A phone started (`on`) or stopped listening to this computer's sound
/// (`CoreEvent::AudioListen`; a stop is also its "no" to an offer).
pub fn on_listen(app: &AppHandle, from: &str, on: bool) {
    if !on {
        let none_left = {
            let mut s = state();
            s.asked.remove(from);
            s.targets.remove(from);
            update_rate(&s);
            s.targets.is_empty()
        };
        if none_left {
            stop_capture();
        }
        crate::tray::refresh(app);
        emit(app);
        return;
    }
    let who = crate::notify::device_name(app, from);
    match start_to(app, from) {
        Ok(false) => {}
        Ok(true) => crate::notify::post_always(
            app,
            &format!("Your computer\u{2019}s sound is playing on {who}"),
            "Stop it from the my-domain menu, the app, or the phone.",
        ),
        Err(e) => {
            eprintln!("[computer-audio] can't play on {from}: {e}");
            // Tell the phone, so it stops waiting.
            if let Some(engine) = app.try_state::<Eng>() {
                let _ = engine.audio_offer(from, false);
            }
            {
                let mut s = state();
                s.asked.remove(from);
                s.error = Some(e.clone());
            }
            crate::notify::post_always(
                app,
                &format!("This computer\u{2019}s sound can\u{2019}t play on {who}"),
                &e,
            );
            emit(app);
        }
    }
}

/// The send rate: the slowest link's.
fn update_rate(s: &State) {
    if let Some(feed) = &s.feed {
        let rate = s.targets.values().copied().min().unwrap_or(RATE_DIRECT);
        feed.out_rate.store(rate, Ordering::Relaxed);
    }
}

/// Play on `node` (it listens): capture if not yet. `false`: it already
/// played there.
fn start_to(app: &AppHandle, node: &str) -> Result<bool, String> {
    let _one = STARTING.lock().unwrap_or_else(|e| e.into_inner());
    if state().targets.contains_key(node) {
        return Ok(false);
    }
    can_start()?;
    let engine = app.try_state::<Eng>().ok_or("not started yet")?;
    // Says "playing" (the phone moves from waiting to playing) and gives
    // the rate for its link.
    let rate = engine.audio_offer(node, true).map_err(|e| e.to_string())?;
    let running = state().stop.is_some();
    if !running {
        start_capture(app, rate)?;
    }
    {
        let mut s = state();
        s.asked.remove(node);
        s.error = None;
        s.targets.insert(node.to_string(), rate);
        update_rate(&s);
    }
    crate::tray::refresh(app);
    emit(app);
    Ok(true)
}

/// Start capturing (waits until it runs, or says why it can't).
fn start_capture(app: &AppHandle, rate: u32) -> Result<(), String> {
    let (tx, rx) = sync_channel(QUEUE);
    let feed = Arc::new(Feed {
        enc: Mutex::new(Encoder::new(rate)),
        out_rate: AtomicU32::new(rate),
        tx,
        failed: Mutex::new(None),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (app2, feed2, stop2) = (app.clone(), feed.clone(), stop.clone());
    std::thread::Builder::new()
        .name("computer-audio".into())
        .spawn(move || run(&app2, feed2, rx, &stop2, &started_tx))
        .map_err(|e| e.to_string())?;
    match started_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(Ok(())) => {
            let mut s = state();
            s.stop = Some(stop);
            s.feed = Some(feed);
            Ok(())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            Err("The sound capture didn\u{2019}t start.".into())
        }
    }
}

fn stop_capture() {
    let mut s = state();
    if let Some(stop) = s.stop.take() {
        stop.store(true, Ordering::Relaxed);
    }
    s.feed = None;
}

/// Stop playing on every phone (telling them, if `tell`); `why` shows on
/// the tile.
fn stop_all(app: &AppHandle, tell: bool, why: Option<String>) {
    let nodes: Vec<String> = {
        let mut s = state();
        let mut nodes: Vec<String> = s.targets.keys().cloned().collect();
        nodes.extend(s.asked.keys().cloned());
        s.targets.clear();
        s.asked.clear();
        if why.is_some() {
            s.error = why;
        }
        nodes
    };
    stop_capture();
    if tell {
        if let Some(engine) = app.try_state::<Eng>() {
            for n in &nodes {
                let _ = engine.audio_offer(n, false);
            }
        }
    }
    crate::tray::refresh(app);
    emit(app);
}

/// Stop playing on `node` only (the last one: stop capturing).
fn stop_one(app: &AppHandle, node: &str) {
    let none_left = {
        let mut s = state();
        s.targets.remove(node);
        s.asked.remove(node);
        update_rate(&s);
        s.targets.is_empty()
    };
    if none_left {
        stop_capture();
    }
    if let Some(engine) = app.try_state::<Eng>() {
        let _ = engine.audio_offer(node, false);
    }
    crate::tray::refresh(app);
    emit(app);
}

/// A device's link went: it no longer plays there.
pub fn on_disconnected(app: &AppHandle, node: &str) {
    let ours = {
        let s = state();
        s.targets.contains_key(node) || s.asked.contains_key(node)
    };
    if !ours {
        return;
    }
    let none_left = {
        let mut s = state();
        s.targets.remove(node);
        s.asked.remove(node);
        update_rate(&s);
        s.targets.is_empty()
    };
    if none_left {
        stop_capture();
    }
    crate::tray::refresh(app);
    emit(app);
}

/// The capture's thread: opens the capture (kept on this thread: neither
/// a cpal stream nor ScreenCaptureKit's objects move between threads),
/// sends the frames to the phones that listen, and stops when told or
/// when nobody listens any more.
fn run(
    app: &AppHandle,
    feed: Arc<Feed>,
    frames: Receiver<(u32, Vec<u8>)>,
    stop: &AtomicBool,
    started: &std::sync::mpsc::Sender<Result<(), String>>,
) {
    let capture = match open_capture(feed.clone()) {
        Ok(c) => c,
        Err(e) => {
            let _ = started.send(Err(e));
            return;
        }
    };
    let _ = started.send(Ok(()));
    let began = Instant::now();
    let mut last_heard = Instant::now();
    let mut checked: Option<Instant> = None;
    let mut listening: Vec<String> = Vec::new();
    let mut why: Option<String> = None;
    let mut pacer = Pacer::new();
    while !stop.load(Ordering::Relaxed) {
        let got = frames.recv_timeout(Duration::from_millis(100));
        let now = Instant::now();
        if checked.is_none_or(|c| now.duration_since(c) >= CHECK) {
            checked = Some(now);
            let Some(engine) = app.try_state::<Eng>() else {
                break;
            };
            let targets: Vec<String> = state().targets.keys().cloned().collect();
            listening = engine
                .audio_listeners()
                .into_iter()
                .filter(|n| targets.contains(n))
                .collect();
            if !listening.is_empty() {
                last_heard = now;
            } else if now.duration_since(began) > GRACE && now.duration_since(last_heard) > IDLE {
                why = Some("The phone stopped listening.".into());
                break;
            }
            if let Some(f) = feed.failed.lock().ok().and_then(|f| f.clone()) {
                why = Some(f);
                break;
            }
        }
        let Ok((rate, pcm)) = got else {
            continue;
        };
        if listening.is_empty() {
            continue;
        }
        // One frame every 20 ms, not in the capture's bursts.
        let due = pacer.due(Instant::now());
        let wait = due.saturating_duration_since(Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        if let Some(engine) = app.try_state::<Eng>() {
            let _ = engine.audio_frame_to(&pcm, rate, 2, &listening);
        }
        pacer.sent();
    }
    drop(capture);
    // Ended by itself (not by Stop): tell the phones and the window.
    if !stop.load(Ordering::Relaxed) {
        let ours = state().feed.as_ref().is_some_and(|f| Arc::ptr_eq(f, &feed));
        if ours {
            eprintln!("[computer-audio] stopped: {why:?}");
            stop_all(app, true, why);
        }
    }
}

/// A capture running; dropping it stops it (held only for that).
#[allow(dead_code)]
enum Capture {
    #[cfg(target_os = "macos")]
    Mac(mac::Capture),
    Cpal(cpal::Stream),
}

fn open_capture(feed: Arc<Feed>) -> Result<Capture, String> {
    #[cfg(target_os = "macos")]
    {
        mac::open(feed).map(Capture::Mac)
    }
    #[cfg(not(target_os = "macos"))]
    {
        loopback::open(feed).map(Capture::Cpal)
    }
}

// --- Linux and Windows: cpal (built everywhere, used there) ---

#[cfg_attr(target_os = "macos", allow(dead_code))]
mod loopback {
    use std::sync::Arc;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::Feed;

    /// Only Windows captures from an output device.
    #[cfg_attr(not(windows), allow(dead_code))]
    const NO_OUTPUT: &str = "This computer has no sound output to capture.";

    /// Capture what this computer plays and feed it on.
    pub fn open(feed: Arc<Feed>) -> Result<cpal::Stream, String> {
        let (device, cfg) = device()?;
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::I16 => build::<i16>(&device, &cfg, feed),
            cpal::SampleFormat::U16 => build::<u16>(&device, &cfg, feed),
            cpal::SampleFormat::I32 => build::<i32>(&device, &cfg, feed),
            _ => build::<f32>(&device, &cfg, feed),
        }?;
        stream
            .play()
            .map_err(|e| format!("The sound capture can\u{2019}t start: {e}"))?;
        Ok(stream)
    }

    fn build<T>(
        device: &cpal::Device,
        cfg: &cpal::SupportedStreamConfig,
        feed: Arc<Feed>,
    ) -> Result<cpal::Stream, String>
    where
        T: cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let channels = usize::from(cfg.channels());
        let rate = cfg.sample_rate().0;
        let mut scratch: Vec<f32> = Vec::new();
        let fail = feed.clone();
        device
            .build_input_stream::<T, _, _>(
                &cfg.config(),
                move |data: &[T], _| {
                    scratch.clear();
                    scratch.extend(data.iter().map(|s| s.to_sample::<f32>()));
                    feed.push(&scratch, rate, channels);
                },
                move |e| {
                    eprintln!("[computer-audio] capture: {e}");
                    if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                        fail.fail("The sound output went away.".into());
                    }
                },
                None,
            )
            .map_err(|e| format!("The sound capture can\u{2019}t start: {e}"))
    }

    /// Windows: the default output, opened as an input (cpal's WASAPI
    /// host then records in loopback mode).
    #[cfg(target_os = "windows")]
    fn device() -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(NO_OUTPUT)?;
        let cfg = device
            .default_output_config()
            .map_err(|e| format!("The sound output can\u{2019}t be captured: {e}"))?;
        Ok((device, cfg))
    }

    /// Linux (and anything else cpal runs on): the default output's
    /// monitor. An input that is a monitor if the host lists one; else
    /// the PulseAudio plugin's input pointed at `@DEFAULT_MONITOR@` (the
    /// monitor of the default output, on PulseAudio and on PipeWire's
    /// pipewire-pulse). The plugin reads `PULSE_SOURCE` when it opens, so
    /// it's set only around that, and put back.
    #[cfg(not(target_os = "windows"))]
    fn device() -> Result<(cpal::Device, cpal::SupportedStreamConfig), String> {
        let host = cpal::default_host();
        let name = |d: &cpal::Device| d.name().unwrap_or_default();
        if let Some(d) = host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| name(d).to_lowercase().contains("monitor")))
        {
            if let Ok(cfg) = d.default_input_config() {
                return Ok((d, cfg));
            }
        }
        let was = std::env::var_os("PULSE_SOURCE");
        std::env::set_var("PULSE_SOURCE", "@DEFAULT_MONITOR@");
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| name(d) == "pulse"))
            .and_then(|d| d.default_input_config().ok().map(|c| (d, c)));
        match was {
            Some(v) => std::env::set_var("PULSE_SOURCE", v),
            None => std::env::remove_var("PULSE_SOURCE"),
        }
        found.ok_or_else(|| {
            "No way to capture this computer\u{2019}s sound: it needs PulseAudio or PipeWire (with pipewire-pulse) and the ALSA PulseAudio plugin.".into()
        })
    }
}

// --- macOS: ScreenCaptureKit ---

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::NSString;

    use super::Feed;

    const PERMISSION: &str = "Allow my-domain to record this computer\u{2019}s sound: System Settings > Privacy & Security > Screen & System Audio Recording (Screen Recording), turn my-domain on, then try again.";

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "CoreMedia", kind = "framework")]
    extern "C" {
        fn CMSampleBufferIsValid(sbuf: *mut c_void) -> u8;
        fn CMSampleBufferGetFormatDescription(sbuf: *mut c_void) -> *mut c_void;
        fn CMAudioFormatDescriptionGetStreamBasicDescription(desc: *mut c_void) -> *const Asbd;
        fn CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sbuf: *mut c_void,
            size_needed: *mut usize,
            list: *mut AudioBufferList,
            list_size: usize,
            structure_allocator: *const c_void,
            block_allocator: *const c_void,
            flags: u32,
            block_out: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut AnyObject;
        fn dispatch_release(object: *mut AnyObject);
    }

    /// AudioStreamBasicDescription.
    #[repr(C)]
    struct Asbd {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    #[repr(C)]
    struct AudioBuffer {
        channels: u32,
        size: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct AudioBufferList {
        count: u32,
        buffers: [AudioBuffer; 1],
    }

    const FLAG_FLOAT: u32 = 1;
    const FLAG_NON_INTERLEAVED: u32 = 1 << 5;
    /// SCStreamOutputType: the screen, and its audio.
    const OUTPUT_SCREEN: isize = 0;
    const OUTPUT_AUDIO: isize = 1;

    /// Where the callback sends audio (one capture at a time).
    static FEED: Mutex<Option<Arc<Feed>>> = Mutex::new(None);

    /// ScreenCaptureKit is there (loaded now) and captures audio (macOS 13
    /// or later).
    pub fn available() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| {
            let path = c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit";
            // SAFETY: a system framework's path; RTLD_LAZY (1). A failure
            // returns null and leaves the classes missing.
            unsafe { dlopen(path.as_ptr(), 1) };
            AnyClass::get(c"SCStreamConfiguration")
                .is_some_and(|c| c.instance_method(sel!(setCapturesAudio:)).is_some())
                && AnyClass::get(c"SCStream").is_some()
                && AnyClass::get(c"SCShareableContent").is_some()
        })
    }

    /// The Screen Recording permission: macOS asks (the first time) when
    /// it's not given; never worked around.
    pub fn permission() -> Result<(), String> {
        // SAFETY: no arguments; reads the permission.
        if unsafe { CGPreflightScreenCaptureAccess() } {
            return Ok(());
        }
        // SAFETY: no arguments; shows macOS's own prompt (once).
        if unsafe { CGRequestScreenCaptureAccess() } {
            return Ok(());
        }
        Err(PERMISSION.into())
    }

    /// Our SCStreamOutput / SCStreamDelegate.
    fn handler_class() -> Option<&'static AnyClass> {
        static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
        *CLASS.get_or_init(|| {
            let mut b = ClassBuilder::new(c"MDSystemAudioOutput", AnyClass::get(c"NSObject")?)?;
            for p in [c"SCStreamOutput", c"SCStreamDelegate"] {
                if let Some(p) = AnyProtocol::get(p) {
                    b.add_protocol(p);
                }
            }
            // SAFETY: the signatures ScreenCaptureKit calls:
            // -stream:didOutputSampleBuffer:ofType: (id, CMSampleBufferRef,
            // NSInteger) and -stream:didStopWithError: (id, NSError *).
            unsafe {
                b.add_method(
                    sel!(stream:didOutputSampleBuffer:ofType:),
                    did_output as extern "C-unwind" fn(_, _, _, _, _),
                );
                b.add_method(
                    sel!(stream:didStopWithError:),
                    did_stop as extern "C-unwind" fn(_, _, _, _),
                );
            }
            Some(b.register())
        })
    }

    extern "C-unwind" fn did_output(
        _this: &AnyObject,
        _cmd: Sel,
        _stream: *mut AnyObject,
        sbuf: *mut c_void,
        kind: isize,
    ) {
        if kind != OUTPUT_AUDIO || sbuf.is_null() {
            return;
        }
        let feed = FEED.lock().ok().and_then(|f| f.clone());
        let Some(feed) = feed else {
            return;
        };
        // SAFETY: a valid sample buffer for this call's length.
        if let Some((samples, rate, channels)) = unsafe { samples_of(sbuf) } {
            feed.push(&samples, rate, channels);
        }
    }

    extern "C-unwind" fn did_stop(
        _this: &AnyObject,
        _cmd: Sel,
        _stream: *mut AnyObject,
        error: *mut AnyObject,
    ) {
        let why = describe(error).unwrap_or_else(|| "The sound capture stopped.".into());
        if let Some(feed) = FEED.lock().ok().and_then(|f| f.clone()) {
            feed.fail(why);
        }
    }

    /// An NSError's description.
    fn describe(error: *mut AnyObject) -> Option<String> {
        // SAFETY: `error` is null or an NSError.
        let error = unsafe { error.as_ref() }?;
        // SAFETY: -[NSError localizedDescription] returns an NSString.
        let d: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
        d.map(|d| d.to_string())
    }

    /// The samples in an audio sample buffer: (interleaved f32, rate,
    /// channels).
    unsafe fn samples_of(sbuf: *mut c_void) -> Option<(Vec<f32>, u32, usize)> {
        if CMSampleBufferIsValid(sbuf) == 0 {
            return None;
        }
        let desc = CMSampleBufferGetFormatDescription(sbuf);
        if desc.is_null() {
            return None;
        }
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(desc).as_ref()?;
        let float = asbd.format_flags & FLAG_FLOAT != 0;
        if !(float && asbd.bits_per_channel == 32 || !float && asbd.bits_per_channel == 16) {
            return None;
        }
        let mut needed = 0usize;
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sbuf,
            &mut needed,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
        );
        let needed = needed.max(std::mem::size_of::<AudioBufferList>());
        let mut store = vec![0u64; needed.div_ceil(8)];
        let list = store.as_mut_ptr().cast::<AudioBufferList>();
        let mut block: *mut c_void = std::ptr::null_mut();
        let status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sbuf,
            std::ptr::null_mut(),
            list,
            needed,
            std::ptr::null(),
            std::ptr::null(),
            1, // kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment
            &mut block,
        );
        if status != 0 {
            return None;
        }
        let count = (*list).count as usize;
        let first = std::ptr::addr_of!((*list).buffers).cast::<AudioBuffer>();
        let bufs = std::slice::from_raw_parts(first, count);
        let planar = asbd.format_flags & FLAG_NON_INTERLEAVED != 0;
        let parts: Vec<(usize, &[u8])> = bufs
            .iter()
            .filter(|b| !b.data.is_null())
            .map(|b| {
                let ch = if planar { 1 } else { b.channels as usize };
                (
                    ch,
                    std::slice::from_raw_parts(b.data.cast::<u8>(), b.size as usize),
                )
            })
            .collect();
        let out = if planar || parts.len() == 1 {
            super::interleave(&parts, float)
        } else {
            None
        };
        if !block.is_null() {
            CFRelease(block);
        }
        let (samples, channels) = out?;
        Some((samples, asbd.sample_rate as u32, channels))
    }

    /// A ScreenCaptureKit stream capturing this computer's sound.
    pub struct Capture {
        stream: Retained<AnyObject>,
        _handler: Retained<AnyObject>,
        queue: *mut AnyObject,
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            if let Ok(mut f) = FEED.lock() {
                *f = None;
            }
            let done = RcBlock::new(|_error: *mut AnyObject| {});
            // SAFETY: -[SCStream stopCaptureWithCompletionHandler:].
            let _: () =
                unsafe { msg_send![&*self.stream, stopCaptureWithCompletionHandler: &*done] };
            // The stream keeps the queue while it needs it.
            // SAFETY: the queue created in `open`, released once.
            unsafe { dispatch_release(self.queue) };
        }
    }

    /// Audio on, this app's own off (a phone's audio playing here never
    /// goes back), and the smallest picture (ScreenCaptureKit always
    /// captures one; it's dropped).
    pub fn stream_config() -> Result<Retained<AnyObject>, String> {
        let cls = AnyClass::get(c"SCStreamConfiguration")
            .ok_or("ScreenCaptureKit isn\u{2019}t available.")?;
        // SAFETY: SCStreamConfiguration's setters (BOOL, NSInteger, size_t).
        Ok(unsafe {
            let c: Retained<AnyObject> = msg_send![cls, new];
            let _: () = msg_send![&*c, setCapturesAudio: true];
            let _: () = msg_send![&*c, setExcludesCurrentProcessAudio: true];
            let _: () = msg_send![&*c, setSampleRate: 48_000isize];
            let _: () = msg_send![&*c, setChannelCount: 2isize];
            let _: () = msg_send![&*c, setWidth: 2usize];
            let _: () = msg_send![&*c, setHeight: 2usize];
            c
        })
    }

    /// Start capturing this computer's sound (its own app's excluded) into
    /// `feed`.
    pub fn open(feed: Arc<Feed>) -> Result<Capture, String> {
        if !available() {
            return Err(super::NOT_SUPPORTED.into());
        }
        permission()?;
        let cls = |name: &std::ffi::CStr| {
            AnyClass::get(name)
                .ok_or_else(|| "ScreenCaptureKit isn\u{2019}t available.".to_string())
        };
        let handler_cls = handler_class().ok_or("The sound capture can\u{2019}t be set up.")?;

        // What can be captured (also where macOS says no without the
        // permission).
        let (tx, rx) = channel::<Result<SendObj, String>>();
        let got = RcBlock::new(move |content: *mut AnyObject, error: *mut AnyObject| {
            // SAFETY: `content` is null or an SCShareableContent.
            let r = match unsafe { Retained::retain(content) } {
                Some(c) => Ok(SendObj(c)),
                None => Err(describe(error).unwrap_or_else(|| PERMISSION.into())),
            };
            let _ = tx.send(r);
        });
        // SAFETY: +[SCShareableContent getShareableContentWithCompletionHandler:].
        let _: () = unsafe {
            msg_send![cls(c"SCShareableContent")?, getShareableContentWithCompletionHandler: &*got]
        };
        let content = rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "macOS didn\u{2019}t answer.".to_string())??
            .0;

        // SAFETY: -[SCShareableContent displays] is an NSArray of SCDisplay.
        let displays: Option<Retained<AnyObject>> = unsafe { msg_send![&*content, displays] };
        let display: Option<Retained<AnyObject>> =
            displays.and_then(|d| unsafe { msg_send![&*d, firstObject] });
        let display = display.ok_or("No display to capture the sound with.")?;
        // SAFETY: +[NSArray array].
        let none: Retained<AnyObject> = unsafe { msg_send![cls(c"NSArray")?, array] };
        // SAFETY: -[SCContentFilter initWithDisplay:excludingWindows:].
        let filter: Option<Retained<AnyObject>> = unsafe {
            let a: Allocated<AnyObject> = msg_send![cls(c"SCContentFilter")?, alloc];
            msg_send![a, initWithDisplay: &*display, excludingWindows: &*none]
        };
        let filter = filter.ok_or("The sound capture can\u{2019}t be set up.")?;

        let config = stream_config()?;

        // SAFETY: +new on our NSObject subclass.
        let handler: Retained<AnyObject> = unsafe { msg_send![handler_cls, new] };
        // SAFETY: -[SCStream initWithFilter:configuration:delegate:].
        let stream: Option<Retained<AnyObject>> = unsafe {
            let a: Allocated<AnyObject> = msg_send![cls(c"SCStream")?, alloc];
            msg_send![a, initWithFilter: &*filter, configuration: &*config, delegate: &*handler]
        };
        let stream = stream.ok_or("The sound capture can\u{2019}t be set up.")?;

        // SAFETY: a serial queue (null attributes) with a static label.
        let queue = unsafe {
            dispatch_queue_create(c"my-domain.computer-audio".as_ptr(), std::ptr::null_mut())
        };
        if queue.is_null() {
            return Err("The sound capture can\u{2019}t be set up.".into());
        }
        for kind in [OUTPUT_AUDIO, OUTPUT_SCREEN] {
            let mut error: *mut AnyObject = std::ptr::null_mut();
            // SAFETY: -[SCStream addStreamOutput:type:sampleHandlerQueue:error:].
            let ok: bool = unsafe {
                msg_send![&*stream, addStreamOutput: &*handler, type: kind, sampleHandlerQueue: queue, error: &mut error]
            };
            if !ok {
                // SAFETY: the queue just created, not used by the stream.
                unsafe { dispatch_release(queue) };
                return Err(describe(error)
                    .unwrap_or_else(|| "The sound capture can\u{2019}t be set up.".into()));
            }
        }
        if let Ok(mut f) = FEED.lock() {
            *f = Some(feed);
        }
        let capture = Capture {
            stream,
            _handler: handler,
            queue,
        };

        let (tx, rx) = channel::<Option<String>>();
        let started = RcBlock::new(move |error: *mut AnyObject| {
            let _ = tx.send(if error.is_null() {
                None
            } else {
                Some(describe(error).unwrap_or_else(|| PERMISSION.into()))
            });
        });
        // SAFETY: -[SCStream startCaptureWithCompletionHandler:].
        let _: () =
            unsafe { msg_send![&*capture.stream, startCaptureWithCompletionHandler: &*started] };
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(None) => Ok(capture),
            Ok(Some(e)) => Err(e),
            Err(_) => Err("macOS didn\u{2019}t start the sound capture.".into()),
        }
    }

    /// An object handed from ScreenCaptureKit's queue to the waiting
    /// thread, once.
    struct SendObj(Retained<AnyObject>);
    // SAFETY: handed over once, then used only by the receiving thread
    // (SCShareableContent is immutable).
    unsafe impl Send for SendObj {}
}

// --- commands ---

/// Where this computer's sound stands.
#[tauri::command]
#[specta::specta]
pub fn computer_audio_status() -> ComputerAudioStatus {
    status()
}

/// Offer this computer's sound to phone `node_id`: it plays once its owner
/// taps on the phone.
#[tauri::command]
#[specta::specta]
pub async fn computer_audio_offer(node_id: String, app: AppHandle) -> Result<(), String> {
    offer(&app, &node_id)
}

/// Stop playing this computer's sound on `node_id` (`None`: everywhere).
#[tauri::command]
#[specta::specta]
pub async fn computer_audio_stop(node_id: Option<String>, app: AppHandle) {
    match node_id {
        Some(n) => stop_one(&app, &n),
        None => stop_all(&app, true, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples_of(frame: &[u8]) -> Vec<i16> {
        frame
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect()
    }

    #[test]
    fn frames_go_on_a_steady_clock() {
        let t0 = Instant::now();
        let ms = |n: u64| Duration::from_millis(n);
        let mut p = Pacer::new();
        // A burst of five frames at once: spread 20 ms apart.
        let dues: Vec<Instant> = (0..5)
            .map(|_| {
                let d = p.due(t0);
                p.sent();
                d
            })
            .collect();
        assert_eq!(dues, (0..5).map(|i| t0 + ms(20 * i)).collect::<Vec<_>>());
        // On time afterwards: still on the clock.
        assert_eq!(p.due(t0 + ms(95)), t0 + ms(100));
        p.sent();
        // The capture stalled: the clock starts again, no catch-up burst.
        let late = t0 + ms(400);
        assert_eq!(p.due(late), late);
        p.sent();
        assert_eq!(p.due(late), late + ms(20));
        // Frames far faster than real time: never a growing backlog.
        let mut p = Pacer::new();
        for _ in 0..6 {
            p.due(t0);
            p.sent();
        }
        assert_eq!(p.due(t0), t0, "120 ms ahead: started again");
    }

    #[test]
    fn frames_are_20_ms_of_stereo() {
        let mut e = Encoder::new(48_000);
        e.set_input(48_000, 2);
        // 30 ms in: one frame out, the rest waits.
        let out = e.push(&vec![0.5; 1440 * 2]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 960 * 2 * 2);
        assert!(samples_of(&out[0]).iter().all(|s| (*s - 16384).abs() <= 1));
        // 10 ms more makes the second (one input frame is held back, to
        // interpolate from).
        assert!(e.push(&vec![0.5; 480 * 2]).is_empty());
        assert_eq!(e.push(&[0.5, 0.5]).len(), 1);
        let mut e = Encoder::new(24_000);
        e.set_input(24_000, 2);
        assert_eq!(e.push(&vec![0.0; 481 * 2])[0].len(), 480 * 2 * 2);
        assert_eq!(e.frame_bytes(), 1920);
    }

    #[test]
    fn it_resamples_and_a_ramp_stays_a_ramp() {
        // 48 kHz in, 24 kHz out (the relay's rate): half the frames.
        let mut e = Encoder::new(24_000);
        e.set_input(48_000, 2);
        let ramp: Vec<f32> = (0..9600)
            .flat_map(|i| {
                let v = i as f32 / 20_000.0;
                [v, -v]
            })
            .collect();
        // Fed in uneven pieces, as a capture does.
        let mut out = Vec::new();
        for piece in ramp.chunks(2 * 333) {
            out.extend(e.push(piece));
        }
        let all: Vec<i16> = out.iter().flat_map(|f| samples_of(f)).collect();
        assert_eq!(all.len() / 2, 4800, "100 ms at 24 kHz");
        let left: Vec<i16> = all.iter().step_by(2).copied().collect();
        assert!(left.windows(2).all(|w| w[1] > w[0]), "not monotonic");
        // Output frame k is input frame 2k.
        let want = |k: usize| ((2 * k) as f32 / 20_000.0 * 32767.0).round() as i16;
        for k in [0, 1, 777, 4799] {
            assert!(
                (left[k] - want(k)).abs() <= 1,
                "{k}: {} vs {}",
                left[k],
                want(k)
            );
        }
        assert!(
            all.iter().skip(1).step_by(2).all(|s| *s <= 0),
            "the right channel"
        );

        // 44.1 kHz in, 48 kHz out: about 48/44.1 times the frames.
        let mut e = Encoder::new(48_000);
        e.set_input(44_100, 2);
        let n: usize = (0..10).map(|_| e.push(&vec![0.1; 4410 * 2]).len()).sum();
        assert!((49..=50).contains(&n), "{n} frames for a second");
    }

    #[test]
    fn mono_surround_and_loud_input() {
        // Mono: both sides.
        let mut e = Encoder::new(48_000);
        e.set_input(48_000, 1);
        let out = e.push(&vec![0.25; 961]);
        let s = samples_of(&out[0]);
        assert!(s.iter().all(|v| (*v - 8192).abs() <= 1));
        // Six channels: the front pair.
        let mut e = Encoder::new(48_000);
        e.set_input(48_000, 6);
        let six: Vec<f32> = (0..961)
            .flat_map(|_| [0.5, -0.5, 1.0, 1.0, 1.0, 1.0])
            .collect();
        let s = samples_of(&e.push(&six)[0]);
        assert!(s
            .chunks(2)
            .all(|p| (p[0] - 16384).abs() <= 1 && (p[1] + 16384).abs() <= 1));
        // Too loud: clipped, never wrapped.
        let mut e = Encoder::new(48_000);
        e.set_input(48_000, 2);
        let s = samples_of(&e.push(&vec![3.0; 962 * 2])[0]);
        assert!(s.iter().all(|v| *v == 32767));
    }

    #[test]
    fn a_new_rate_starts_over() {
        let mut e = Encoder::new(48_000);
        e.set_input(48_000, 2);
        assert!(e.push(&vec![0.0; 500 * 2]).is_empty());
        // The link moved to the relay: what was pending at 48 kHz goes.
        e.set_out_rate(24_000);
        let out = e.push(&vec![0.0; 960 * 2]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 1920);
        // Nothing for a broken format.
        let mut e = Encoder::new(48_000);
        assert!(e.push(&[0.0; 100]).is_empty());
    }

    #[test]
    fn captured_buffers_interleave() {
        let f = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_ne_bytes()).collect() };
        // Planar (ScreenCaptureKit's usual): one buffer per channel.
        let (l, r) = (f(&[0.1, 0.2, 0.3]), f(&[-0.1, -0.2, -0.3]));
        let (s, ch) = interleave(&[(1, &l), (1, &r)], true).unwrap();
        assert_eq!(ch, 2);
        assert_eq!(s, [0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        // Interleaved in one buffer.
        let both = f(&[0.5, -0.5, 0.25, -0.25]);
        let (s, ch) = interleave(&[(2, &both)], true).unwrap();
        assert_eq!((s.len(), ch), (4, 2));
        // 16-bit.
        let pcm: Vec<u8> = [16384i16, -16384]
            .iter()
            .flat_map(|x| x.to_ne_bytes())
            .collect();
        let (s, ch) = interleave(&[(2, &pcm)], false).unwrap();
        assert_eq!((s, ch), (vec![0.5, -0.5], 2));
        assert!(interleave(&[], true).is_none());
    }

    /// ScreenCaptureKit loads, our output class registers, and the stream
    /// settings take (debug builds check each message's types, so a wrong
    /// one fails here). Capturing itself needs the permission: not tested.
    #[cfg(target_os = "macos")]
    #[test]
    fn screen_capture_kit_is_set_up() {
        use objc2::runtime::AnyClass;
        use objc2::sel;
        assert!(mac::available(), "macOS 13 or later");
        let cfg = mac::stream_config().unwrap();
        let rate: isize = unsafe { objc2::msg_send![&*cfg, sampleRate] };
        assert_eq!(rate, 48_000);
        let audio: bool = unsafe { objc2::msg_send![&*cfg, capturesAudio] };
        assert!(audio);
        let stream = AnyClass::get(c"SCStream").unwrap();
        let enc = |s| {
            stream
                .instance_method(s)
                .map(|m| {
                    (0..m.arguments_count())
                        .map(|i| m.argument_type(i).unwrap().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default()
        };
        // The messages `mac::open` sends with a stream, which can't be sent
        // here without the permission: their argument types as sent there
        // (an object, NSInteger, a queue, NSError **; blocks).
        assert_eq!(
            enc(sel!(addStreamOutput:type:sampleHandlerQueue:error:)),
            "@,:,@,q,@,^@"
        );
        assert_eq!(
            enc(sel!(initWithFilter:configuration:delegate:)),
            "@,:,@,@,@"
        );
        assert_eq!(enc(sel!(startCaptureWithCompletionHandler:)), "@,:,@?");
        assert_eq!(enc(sel!(stopCaptureWithCompletionHandler:)), "@,:,@?");
    }

    #[test]
    fn the_tray_says_where_it_plays() {
        assert_eq!(stop_label(&[]), None);
        assert_eq!(
            stop_label(&["Pixel".into()]).as_deref(),
            Some("Stop playing this computer\u{2019}s sound on Pixel")
        );
    }
}
