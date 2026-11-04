// A phone's media audio (music, videos, podcasts; never calls) played on
// this computer's speakers or headphones (mdcore `features::audio_share`),
// like a Bluetooth speaker but over the encrypted link.
//
// Two ways in: "Play this phone's audio here" on the phone's page (the
// phone asks its owner, then Android's own prompt), or "Play on <this
// computer>" on the phone (`CoreEvent::AudioOffer`): then this starts by
// itself and says so. One phone at a time.
//
// The phone sends raw PCM (s16le stereo, 48 kHz over a direct link, 24 kHz
// through the relay) in 20 ms frames. Here a small jitter buffer (`Jitter`:
// starts playing once `TARGET_MS` is buffered, drops late frames, trims
// back to the target when more than `MAX_MS` piles up, re-primes after an
// underrun and counts it) feeds a cpal output stream, resampled to the
// device's rate. The listen is renewed every 10 s while it plays, so the
// phone stops by itself if this computer goes away.
//
// It stops on Stop here or on the phone, when the link goes, when nothing
// has come for `STALL` (or the phone never started within `NO_ANSWER`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mdcore::features::audio_share::RENEW_EVERY;
use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager};

use crate::Eng;

/// Buffered before playing starts (and again after an underrun).
pub const TARGET_MS: u32 = 80;
/// More than this buffered (the phone's clock runs ahead, or a burst after
/// a stall): trimmed back to `TARGET_MS`.
pub const MAX_MS: u32 = 200;
/// Nothing for this long while playing: the phone went away.
const STALL: Duration = Duration::from_secs(6);
/// The phone never started (its owner didn't answer).
const NO_ANSWER: Duration = Duration::from_secs(90);

const KEY_OUTPUT: &str = "phone_audio_output";
const KEY_VOLUME: &str = "phone_audio_volume";

/// Audio waiting to be played: stereo frames at the phone's rate, as f32.
pub struct Jitter {
    rate: u32,
    channels: u8,
    /// Interleaved L, R.
    buf: VecDeque<f32>,
    next_seq: Option<u64>,
    /// Waiting for `TARGET_MS` before playing.
    priming: bool,
    /// Where the next output sample falls between the first two frames.
    pos: f64,
    target_ms: u32,
    max_ms: u32,
    /// Times it ran dry while playing.
    pub underruns: u32,
    /// Frames that came after a later one (dropped).
    pub late: u32,
    /// Times it was trimmed for holding too much.
    pub trimmed: u32,
}

impl Jitter {
    pub fn new(target_ms: u32, max_ms: u32) -> Self {
        Jitter {
            rate: 0,
            channels: 0,
            buf: VecDeque::new(),
            next_seq: None,
            priming: true,
            pos: 0.0,
            target_ms,
            max_ms: max_ms.max(target_ms),
            underruns: 0,
            late: 0,
            trimmed: 0,
        }
    }

    fn frames(&self) -> usize {
        self.buf.len() / 2
    }

    fn frames_for(&self, ms: u32) -> usize {
        (u64::from(self.rate) * u64::from(ms) / 1000) as usize
    }

    /// How much is waiting, in milliseconds.
    pub fn buffered_ms(&self) -> u32 {
        if self.rate == 0 {
            return 0;
        }
        u32::try_from(self.frames() as u64 * 1000 / u64::from(self.rate)).unwrap_or(u32::MAX)
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn playing(&self) -> bool {
        !self.priming
    }

    /// A frame from the phone: `pcm` is s16le, `channels` interleaved, at
    /// `rate` Hz. A new format starts over; a late frame is dropped.
    pub fn push(&mut self, seq: u64, rate: u32, channels: u8, pcm: &[u8]) {
        if rate == 0 || !(1..=2).contains(&channels) {
            return;
        }
        if rate != self.rate || channels != self.channels {
            self.rate = rate;
            self.channels = channels;
            self.buf.clear();
            self.next_seq = None;
            self.priming = true;
            self.pos = 0.0;
        }
        if self.next_seq.is_some_and(|n| seq < n) {
            self.late = self.late.saturating_add(1);
            return;
        }
        self.next_seq = Some(seq.saturating_add(1));
        let samples = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0);
        if channels == 1 {
            for s in samples {
                self.buf.extend([s, s]);
            }
        } else {
            self.buf.extend(samples);
            if self.buf.len() % 2 == 1 {
                self.buf.pop_back();
            }
        }
        if self.frames() > self.frames_for(self.max_ms) {
            let excess = self.frames() - self.frames_for(self.target_ms);
            self.buf.drain(..excess * 2);
            self.pos = 0.0;
            self.trimmed = self.trimmed.saturating_add(1);
        }
        if self.priming && self.frames() >= self.frames_for(self.target_ms).max(2) {
            self.priming = false;
        }
    }

    /// Fill a device buffer (`out_channels` interleaved at `out_rate`),
    /// resampling linearly, at `volume` (0 to 1). Silence while priming;
    /// running dry counts an underrun and primes again.
    pub fn fill(&mut self, out: &mut [f32], out_channels: usize, out_rate: u32, volume: f32) {
        let out_channels = out_channels.max(1);
        let step = if out_rate == 0 || self.rate == 0 {
            1.0
        } else {
            f64::from(self.rate) / f64::from(out_rate)
        };
        for frame in out.chunks_mut(out_channels) {
            frame.iter_mut().for_each(|o| *o = 0.0);
            if self.priming {
                continue;
            }
            let i = self.pos as usize;
            if i + 1 >= self.frames() {
                self.underruns = self.underruns.saturating_add(1);
                self.priming = true;
                continue;
            }
            let f = (self.pos - i as f64) as f32;
            let at = |k: usize| self.buf[k];
            let l = at(2 * i) + (at(2 * i + 2) - at(2 * i)) * f;
            let r = at(2 * i + 1) + (at(2 * i + 3) - at(2 * i + 1)) * f;
            if out_channels == 1 {
                frame[0] = (l + r) * 0.5 * volume;
            } else {
                frame[0] = l * volume;
                frame[1] = r * volume;
            }
            self.pos += step;
            let whole = self.pos as usize;
            if whole > 0 {
                self.buf.drain(..(whole * 2).min(self.buf.len()));
                self.pos -= whole as f64;
            }
        }
    }
}

/// What the player and the event handler share.
struct Shared {
    jitter: Mutex<Jitter>,
    /// 0 to 100.
    volume: AtomicU32,
    stop: AtomicBool,
    /// The output was changed: open it again.
    reopen: AtomicBool,
    last_frame: Mutex<Option<Instant>>,
    output: Mutex<Option<String>>,
    error: Mutex<Option<String>>,
}

struct Session {
    node: String,
    shared: Arc<Shared>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Where a phone's audio stands, for its tile.
#[derive(Clone, Debug, Default, serde::Serialize, specta::Type)]
pub struct PhoneAudioStatus {
    /// The phone playing here (or asked to), if any.
    pub node: Option<String>,
    /// "idle", "waiting" (asked; the phone hasn't started), "playing", or
    /// "buffering" (filling up again after running dry).
    pub state: String,
    pub buffered_ms: u32,
    pub underruns: u32,
    pub late: u32,
    pub trimmed: u32,
    /// The phone's rate, Hz (0 until it plays).
    pub rate: u32,
    /// The output it plays on (`None`: the system default).
    pub output: Option<String>,
    pub volume: u32,
    /// Why it stopped or can't play, if it did.
    pub error: Option<String>,
}

/// This computer's outputs, for the picker.
#[derive(Clone, Debug, Default, serde::Serialize, specta::Type)]
pub struct AudioOutputs {
    pub outputs: Vec<String>,
    pub default_output: Option<String>,
}

fn prefs(app: &AppHandle) -> Option<FileKv> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")))
}

fn saved_output(app: &AppHandle) -> Option<String> {
    prefs(app)
        .and_then(|kv| kv.get(KEY_OUTPUT))
        .filter(|o| !o.is_empty())
}

fn saved_volume(app: &AppHandle) -> u32 {
    prefs(app)
        .and_then(|kv| kv.get(KEY_VOLUME))
        .and_then(|v| v.parse().ok())
        .map_or(100, |v: u32| v.min(100))
}

fn status_of(s: Option<&Session>, app: &AppHandle) -> PhoneAudioStatus {
    let Some(s) = s else {
        return PhoneAudioStatus {
            state: "idle".into(),
            output: saved_output(app),
            volume: saved_volume(app),
            ..Default::default()
        };
    };
    let j = s.shared.jitter.lock().unwrap_or_else(|e| e.into_inner());
    let got = s
        .shared
        .last_frame
        .lock()
        .map(|l| l.is_some())
        .unwrap_or(false);
    PhoneAudioStatus {
        node: Some(s.node.clone()),
        state: if !got {
            "waiting"
        } else if j.playing() {
            "playing"
        } else {
            "buffering"
        }
        .into(),
        buffered_ms: j.buffered_ms(),
        underruns: j.underruns,
        late: j.late,
        trimmed: j.trimmed,
        rate: j.rate(),
        output: s.shared.output.lock().ok().and_then(|o| o.clone()),
        volume: s.shared.volume.load(Ordering::Relaxed),
        error: s.shared.error.lock().ok().and_then(|e| e.clone()),
    }
}

fn emit(app: &AppHandle, error: Option<String>) {
    let mut st = {
        let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        status_of(s.as_ref(), app)
    };
    if error.is_some() {
        st.error = error;
    }
    let _ = app.emit("phone-audio", st);
}

/// Play phone `node`'s audio here: listen to it (the phone asks its owner
/// unless it offered). One phone at a time; the same one again is a no-op
/// (`false`: it was already playing).
fn start(app: &AppHandle, node: &str) -> Result<bool, String> {
    {
        let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        if s.as_ref().is_some_and(|s| s.node == node) {
            return Ok(false);
        }
    }
    // Off macOS, this computer's sound going to a phone (`computer_audio`)
    // would carry the phone's audio straight back to it.
    if !cfg!(target_os = "macos") && crate::computer_audio::streaming() {
        return Err("Stop playing this computer\u{2019}s sound on the phone first: it would carry the phone\u{2019}s audio back to it.".into());
    }
    stop(app, true, None);
    let engine = app.try_state::<Eng>().ok_or("not started yet")?;
    engine.audio_listen(node, true).map_err(|e| e.to_string())?;
    let shared = Arc::new(Shared {
        jitter: Mutex::new(Jitter::new(TARGET_MS, MAX_MS)),
        volume: AtomicU32::new(saved_volume(app)),
        stop: AtomicBool::new(false),
        reopen: AtomicBool::new(false),
        last_frame: Mutex::new(None),
        output: Mutex::new(saved_output(app)),
        error: Mutex::new(None),
    });
    *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = Some(Session {
        node: node.to_string(),
        shared: shared.clone(),
    });
    let (app2, node2) = (app.clone(), node.to_string());
    std::thread::Builder::new()
        .name("phone-audio".into())
        .spawn(move || run(&app2, &node2, &shared))
        .map_err(|e| e.to_string())?;
    emit(app, None);
    Ok(true)
}

/// Stop playing (telling the phone, if `tell`); `why` shows on the tile.
fn stop(app: &AppHandle, tell: bool, why: Option<String>) {
    let Some(s) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    s.shared.stop.store(true, Ordering::Relaxed);
    if tell {
        if let Some(engine) = app.try_state::<Eng>() {
            let _ = engine.audio_listen(&s.node, false);
        }
    }
    emit(app, why);
}

/// Whether a phone's audio plays here now (or was asked to).
pub fn playing_here() -> bool {
    SESSION.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Stop if `node` is the phone playing (its link went, it stopped).
fn stop_if(app: &AppHandle, node: &str, tell: bool, why: Option<String>) {
    let ours = SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|s| s.node == node);
    if ours {
        stop(app, tell, why);
    }
}

/// The player: the output stream, the renewals, the stall watch.
fn run(app: &AppHandle, node: &str, shared: &Arc<Shared>) {
    let started = Instant::now();
    let mut renewed = Instant::now();
    let mut stream = None;
    let mut reopen = true;
    let mut was_playing = false;
    while !shared.stop.load(Ordering::Relaxed) {
        if reopen || shared.reopen.swap(false, Ordering::Relaxed) {
            reopen = false;
            drop(stream.take());
            let want = shared.output.lock().ok().and_then(|o| o.clone());
            match open(want.as_deref(), shared.clone()) {
                Ok(s) => {
                    stream = Some(s);
                    if let Ok(mut e) = shared.error.lock() {
                        *e = None;
                    }
                }
                Err(why) => {
                    eprintln!("[phone-audio] can't play: {why}");
                    stop_if(app, node, true, Some(why));
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(200));
        let now = Instant::now();
        if now.duration_since(renewed) >= RENEW_EVERY {
            renewed = now;
            if let Some(engine) = app.try_state::<Eng>() {
                let _ = engine.audio_listen(node, true);
            }
        }
        let last = shared.last_frame.lock().ok().and_then(|l| *l);
        match last {
            Some(at) if now.duration_since(at) > STALL => {
                stop_if(
                    app,
                    node,
                    true,
                    Some("The phone stopped sending its audio.".into()),
                );
                break;
            }
            None if now.duration_since(started) > NO_ANSWER => {
                stop_if(
                    app,
                    node,
                    true,
                    Some("The phone didn't start playing.".into()),
                );
                break;
            }
            Some(_) if !was_playing => {
                was_playing = true;
                emit(app, None);
            }
            _ => {}
        }
    }
    drop(stream);
}

/// Open output `want` (`None` or gone: the default) and play `shared`'s
/// buffer on it.
fn open(want: Option<&str>, shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let name = |d: &cpal::Device| d.name().unwrap_or_default();
    let device = want
        .and_then(|w| {
            host.output_devices()
                .ok()
                .and_then(|mut it| it.find(|d| name(d) == w))
        })
        .or_else(|| host.default_output_device())
        .ok_or("This computer has no speaker or headphones to play on.")?;
    let cfg = device
        .default_output_config()
        .map_err(|e| format!("The output can't play: {e}"))?;
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::I16 => build::<i16>(&device, &cfg, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, &cfg, shared),
        _ => build::<f32>(&device, &cfg, shared),
    }?;
    stream
        .play()
        .map_err(|e| format!("The output can't play: {e}"))?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(cfg.channels());
    let rate = cfg.sample_rate().0;
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            &cfg.config(),
            move |data: &mut [T], _| {
                scratch.clear();
                scratch.resize(data.len(), 0.0);
                let volume = shared.volume.load(Ordering::Relaxed) as f32 / 100.0;
                // Never wait on the audio thread: a busy buffer is a
                // moment of silence.
                if let Ok(mut j) = shared.jitter.try_lock() {
                    j.fill(&mut scratch, channels, rate, volume);
                }
                for (o, s) in data.iter_mut().zip(&scratch) {
                    *o = T::from_sample(*s);
                }
            },
            |e| eprintln!("[phone-audio] output: {e}"),
            None,
        )
        .map_err(|e| format!("The output can't play: {e}"))
}

// --- core events ---

/// A frame from a phone (`CoreEvent::AudioFrame`): into the buffer if it's
/// the phone playing here.
pub fn on_frame(from: &str, seq: u64, rate: u32, channels: u8, pcm: &str) {
    let shared = {
        let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        match s.as_ref() {
            Some(s) if s.node == from => s.shared.clone(),
            _ => return,
        }
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(pcm) else {
        return;
    };
    if let Ok(mut j) = shared.jitter.lock() {
        j.push(seq, rate, channels, &bytes);
    }
    if let Ok(mut l) = shared.last_frame.lock() {
        *l = Some(Instant::now());
    };
}

/// A phone started (`on`) playing its audio here from the phone, or
/// stopped (or its owner said no).
pub fn on_offer(app: &AppHandle, from: &str, on: bool) {
    if !on {
        stop_if(app, from, false, None);
        return;
    }
    match start(app, from) {
        Ok(false) => {}
        Ok(true) => {
            let who = crate::notify::device_name(app, from);
            crate::notify::post(
                app,
                &format!("{who} is playing its audio here"),
                "Stop it on its page here or on the phone.",
            );
        }
        Err(e) => eprintln!("[phone-audio] can't play {from}: {e}"),
    }
}

/// A device's link went: if it was playing here, that's the end.
pub fn on_disconnected(app: &AppHandle, node: &str) {
    stop_if(
        app,
        node,
        false,
        Some("The connection to the phone was lost.".into()),
    );
}

// --- commands ---

/// Where a phone's audio stands here.
#[tauri::command]
#[specta::specta]
pub fn phone_audio_status(app: AppHandle) -> PhoneAudioStatus {
    let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    status_of(s.as_ref(), &app)
}

/// This computer's speakers and headphones.
#[tauri::command]
#[specta::specta]
pub async fn phone_audio_outputs() -> AudioOutputs {
    let host = cpal::default_host();
    let mut outputs: Vec<String> = host
        .output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    outputs.dedup();
    AudioOutputs {
        outputs,
        default_output: host.default_output_device().and_then(|d| d.name().ok()),
    }
}

/// Ask phone `node_id` to play its audio here (its owner says yes on the
/// phone, then Android asks).
#[tauri::command]
#[specta::specta]
pub fn phone_audio_start(node_id: String, app: AppHandle) -> Result<(), String> {
    start(&app, &node_id).map(|_| ())
}

/// Stop playing the phone's audio here.
#[tauri::command]
#[specta::specta]
pub fn phone_audio_stop(app: AppHandle) {
    stop(&app, true, None);
}

/// Volume, 0 to 100 (kept).
#[tauri::command]
#[specta::specta]
pub fn phone_audio_set_volume(volume: u32, app: AppHandle) {
    let v = volume.min(100);
    if let Some(s) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        s.shared.volume.store(v, Ordering::Relaxed);
    }
    if let Some(kv) = prefs(&app) {
        let _ = kv.set(KEY_VOLUME, &v.to_string());
    }
}

/// Play on `output` (`None`: the system default); kept, and switched now
/// if playing.
#[tauri::command]
#[specta::specta]
pub fn phone_audio_set_output(output: Option<String>, app: AppHandle) {
    let output = output.filter(|o| !o.is_empty());
    if let Some(kv) = prefs(&app) {
        let _ = kv.set(KEY_OUTPUT, output.as_deref().unwrap_or(""));
    }
    if let Some(s) = SESSION.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if let Ok(mut o) = s.shared.output.lock() {
            *o = output;
        }
        s.shared.reopen.store(true, Ordering::Relaxed);
    }
    emit(&app, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ms` of a stereo frame at `rate`, every sample `v`.
    fn frame(rate: u32, ms: u32, v: i16) -> Vec<u8> {
        let n = (rate * ms / 1000) as usize * 2;
        (0..n).flat_map(|_| v.to_le_bytes()).collect()
    }

    fn pull(j: &mut Jitter, frames: usize, rate: u32) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        j.fill(&mut out, 2, rate, 1.0);
        out
    }

    #[test]
    fn it_waits_for_the_target_then_plays() {
        let mut j = Jitter::new(80, 200);
        for seq in 0..3 {
            j.push(seq, 48_000, 2, &frame(48_000, 20, 16_384));
        }
        assert!(!j.playing(), "60 ms isn't enough yet");
        assert!(pull(&mut j, 100, 48_000).iter().all(|s| *s == 0.0));
        j.push(3, 48_000, 2, &frame(48_000, 20, 16_384));
        assert!(j.playing());
        assert_eq!(j.buffered_ms(), 80);
        let out = pull(&mut j, 960, 48_000);
        assert!(out.iter().all(|s| (*s - 0.5).abs() < 1e-3));
        assert_eq!(j.buffered_ms(), 60);
        assert_eq!(j.underruns, 0);
    }

    #[test]
    fn running_dry_counts_an_underrun_and_primes_again() {
        let mut j = Jitter::new(40, 200);
        j.push(0, 48_000, 2, &frame(48_000, 40, 1000));
        assert!(j.playing());
        pull(&mut j, 48_000 / 10, 48_000);
        assert_eq!(j.underruns, 1);
        assert!(!j.playing());
        // Still dry: no new underrun counted while priming.
        pull(&mut j, 480, 48_000);
        assert_eq!(j.underruns, 1);
        j.push(1, 48_000, 2, &frame(48_000, 40, 1000));
        assert!(j.playing());
    }

    #[test]
    fn late_frames_are_dropped() {
        let mut j = Jitter::new(20, 200);
        j.push(5, 48_000, 2, &frame(48_000, 20, 1));
        j.push(4, 48_000, 2, &frame(48_000, 20, 1));
        j.push(5, 48_000, 2, &frame(48_000, 20, 1));
        assert_eq!(j.late, 2);
        assert_eq!(j.buffered_ms(), 20);
        // A gap (a lost frame) is not late.
        j.push(7, 48_000, 2, &frame(48_000, 20, 1));
        assert_eq!(j.late, 2);
        assert_eq!(j.buffered_ms(), 40);
    }

    #[test]
    fn too_much_is_trimmed_back_to_the_target() {
        let mut j = Jitter::new(80, 200);
        for seq in 0..11 {
            j.push(seq, 48_000, 2, &frame(48_000, 20, 1));
        }
        assert_eq!(j.trimmed, 1);
        assert_eq!(j.buffered_ms(), 80);
    }

    #[test]
    fn a_new_format_starts_over() {
        let mut j = Jitter::new(40, 200);
        j.push(0, 48_000, 2, &frame(48_000, 60, 1));
        assert!(j.playing());
        // The phone moved to the relay: 24 kHz, and its counter may restart.
        j.push(0, 24_000, 2, &frame(24_000, 20, 1));
        assert_eq!(j.rate(), 24_000);
        assert!(!j.playing());
        assert_eq!(j.buffered_ms(), 20);
        assert_eq!(j.late, 0);
    }

    #[test]
    fn it_resamples_to_the_device_rate() {
        // 24 kHz in, 48 kHz out: twice the frames, a ramp stays a ramp.
        let mut j = Jitter::new(20, 200);
        let pcm: Vec<u8> = (0..960i16)
            .flat_map(|i| {
                let v = i * 30;
                [v.to_le_bytes(), v.to_le_bytes()].concat()
            })
            .collect();
        j.push(0, 24_000, 2, &pcm);
        let out = pull(&mut j, 1000, 48_000);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        assert!(left.windows(2).all(|w| w[1] >= w[0]), "not monotonic");
        // Halfway between samples 10 and 11 at output frame 21.
        let want = (10.5 * 30.0) / 32768.0;
        assert!((left[21] - want).abs() < 1e-4, "{} vs {want}", left[21]);
        // 960 frames in last about 1918 out: 1000 out leave about half.
        assert!(
            (j.buffered_ms() as i64 - 19).abs() <= 1,
            "{}",
            j.buffered_ms()
        );
    }

    #[test]
    fn mono_in_and_out_and_volume() {
        let mut j = Jitter::new(20, 200);
        let pcm: Vec<u8> = (0..960).flat_map(|_| 8192i16.to_le_bytes()).collect();
        j.push(0, 48_000, 1, &pcm);
        let mut out = vec![1.0; 100];
        j.fill(&mut out, 1, 48_000, 0.5);
        assert!(out.iter().all(|s| (*s - 0.125).abs() < 1e-4));
        // Six output channels: the first two get it, the rest are silent.
        let mut out = vec![1.0; 60];
        j.fill(&mut out, 6, 48_000, 1.0);
        for f in out.chunks(6) {
            assert!((f[0] - 0.25).abs() < 1e-4 && (f[1] - 0.25).abs() < 1e-4);
            assert!(f[2..].iter().all(|s| *s == 0.0));
        }
    }
}
