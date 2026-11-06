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
// through the relay; a quarter of that as ADPCM, decoded by the core) in 20
// ms frames. Here the core's adaptive jitter buffer
// (`mdcore::features::audio_jitter`: its target grows on trouble and shrinks
// when steady, lower on a nearby link; it conceals a short gap with a faded
// repeat and follows the phone's clock with a tiny speed change) feeds a
// cpal output stream, resampled to the device's rate. The listen is renewed every 10 s while it plays, so the
// phone stops by itself if this computer goes away.
//
// It stops on Stop here or on the phone, when the link goes, when nothing
// has come for `STALL` (or the phone never started within `NO_ANSWER`).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mdcore::features::audio_share::RENEW_EVERY;
use mdcore::kv::FileKv;
use tauri::{AppHandle, Emitter, Manager};

use crate::Eng;

pub use mdcore::features::audio_jitter::Jitter;

/// Nothing for this long while playing: the phone went away.
const STALL: Duration = Duration::from_secs(6);
/// The phone never started (its owner didn't answer).
const NO_ANSWER: Duration = Duration::from_secs(90);

const KEY_OUTPUT: &str = "phone_audio_output";
const KEY_VOLUME: &str = "phone_audio_volume";

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
    /// The adaptive buffer's target now, ms.
    pub target_ms: u32,
    /// Gaps filled by concealment so far, ms.
    pub concealed_ms: u32,
    /// Lost frames recovered from the redundant copy in the next.
    pub recovered: u32,
    /// The speed nudge following the phone's clock, parts per million.
    pub drift_ppm: i32,
    /// On a nearby low-latency link now (the badge).
    pub low_latency: bool,
    /// Coming over Bluetooth now (the badge; the buffer keeps more).
    pub via_bluetooth: bool,
    /// Lost frames Opus made up (its packet-loss concealment).
    pub plc: u32,
    /// End-to-end latency now and before the last move, ms.
    pub latency_ms: Option<u32>,
    pub before_ms: Option<u32>,
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
    let (near, rx, via) = match app.try_state::<Eng>() {
        Some(e) => (
            e.nearby_status(&s.node),
            e.audio_rx_stats(&s.node),
            e.audio_via(&s.node),
        ),
        None => Default::default(),
    };
    let via_bluetooth = via == "bluetooth";
    let mut j = s.shared.jitter.lock().unwrap_or_else(|e| e.into_inner());
    // Over Bluetooth the buffer keeps more (records come 60 ms at a time).
    if via_bluetooth != j.bluetooth() {
        if via_bluetooth {
            j.set_bluetooth(true);
        } else {
            j.set_low_latency(near.low_latency);
        }
    }
    let stats = j.stats();
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
        target_ms: j.target_ms(),
        concealed_ms: stats.concealed_ms,
        recovered: rx.recovered,
        drift_ppm: stats.drift_ppm,
        low_latency: near.low_latency,
        via_bluetooth,
        plc: rx.concealed,
        latency_ms: near.latency_ms,
        before_ms: near.before_ms,
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
        jitter: Mutex::new(Jitter::new()),
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

/// The phone whose audio plays here (or was asked to), if any.
pub fn playing_from() -> Option<String> {
    SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.node.clone())
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

/// How the stream from a phone stands (`CoreEvent::NearbyMedia`): on a
/// nearby low-latency link the buffer may shrink lower.
pub fn on_nearby(app: &AppHandle, status: &mdcore::features::nearby::NearbyStatus) {
    let shared = {
        let s = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        match s.as_ref() {
            Some(s) if s.node == status.node_id => s.shared.clone(),
            _ => return,
        }
    };
    if let Ok(mut j) = shared.jitter.lock() {
        if j.low_latency() == status.low_latency {
            return;
        }
        j.set_low_latency(status.low_latency);
    }
    emit(app, None);
}
