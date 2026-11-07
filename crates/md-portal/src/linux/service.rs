//! What the desktop app calls on Wayland: the portals and tools (looked at
//! again after a minute), the remembered restore tokens, and one screen
//! cast, one remote-input session and one input capture for the app's
//! life, opened when first needed. A refusal (the user said no) is
//! remembered for a while so the prompt doesn't come back on every event.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::capture::{CaptureSession, Captured};
use super::inject::Injector;
use super::screen::{self, ScreenStream};
use crate::caps::{self, InputWay, Portals, ScreenWay, Tools};
use crate::clip;
use crate::geom::{Edge, Rect};

/// How long the portals and tools found are trusted.
const CAPS_FOR: Duration = Duration::from_secs(60);
/// After a refusal, how long before asking again.
const SCREEN_PAUSE: Duration = Duration::from_secs(30);
const INPUT_PAUSE: Duration = Duration::from_secs(120);
const CAPTURE_PAUSE: Duration = Duration::from_secs(1800);
/// Pictures a second from the screen cast.
const FPS: u32 = 2;

static DIR: OnceLock<PathBuf> = OnceLock::new();

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Where the restore tokens are kept (the app's config folder).
pub fn configure(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn token_path(name: &str) -> Option<PathBuf> {
    DIR.get().map(|d| d.join(format!("portal-{name}.token")))
}

fn load_token(name: &str) -> Option<String> {
    let t = std::fs::read_to_string(token_path(name)?).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn save_token(name: &str, token: Option<&str>) {
    let (Some(path), Some(token)) = (token_path(name), token) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let written = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .and_then(|mut f| {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            f.write_all(token.as_bytes())
        });
    if let Err(e) = written {
        eprintln!("[portal] couldn't keep the {name} permission: {e}");
    }
}

/// Forget the remembered permissions (the next use asks again).
pub fn forget_tokens() {
    for name in ["screencast", "remote-desktop"] {
        if let Some(p) = token_path(name) {
            let _ = std::fs::remove_file(p);
        }
    }
}

static CAPS: Mutex<Option<(Instant, Portals, Tools)>> = Mutex::new(None);

/// The portals and tools here. The first look runs `gst-inspect-1.0` (the
/// app makes it at start, off the UI's thread); after a minute the last
/// answer is still given while a fresh look runs in the background.
pub fn caps() -> (Portals, Tools) {
    let mut g = lock(&CAPS);
    if let Some((at, p, t)) = *g {
        if at.elapsed() >= CAPS_FOR {
            // Once per expiry: the next callers see the new time.
            *g = Some((Instant::now(), p, t));
            let _ = std::thread::Builder::new()
                .name("portal-caps".into())
                .spawn(|| {
                    let (p, t) = (super::portals(), super::tools());
                    *lock(&CAPS) = Some((Instant::now(), p, t));
                });
        }
        return (p, t);
    }
    let (p, t) = (super::portals(), super::tools());
    *g = Some((Instant::now(), p, t));
    (p, t)
}

fn paused(until: &Mutex<Option<Instant>>) -> bool {
    lock(until).is_some_and(|t| Instant::now() < t)
}

fn pause(until: &Mutex<Option<Instant>>, d: Duration) {
    *lock(until) = Some(Instant::now() + d);
}

// --- the screen ---------------------------------------------------------------

static STREAM: Mutex<Option<ScreenStream>> = Mutex::new(None);
static SCREEN_REFUSED: Mutex<Option<Instant>> = Mutex::new(None);

pub fn screen_way() -> Result<ScreenWay, &'static str> {
    let (p, t) = caps();
    caps::screen_way(&p, &t)
}

/// A JPEG of the screen, `width` wide, by `way` (`Grim` is the app's own).
pub fn screen_jpeg(way: ScreenWay, width: u32, quality: u8) -> Option<Vec<u8>> {
    match way {
        ScreenWay::Stream => {
            let mut g = lock(&STREAM);
            if g.as_ref().is_some_and(ScreenStream::ended) {
                // Stopped from the desktop's own indicator: not again at once.
                *g = None;
                pause(&SCREEN_REFUSED, SCREEN_PAUSE);
            }
            if g.is_none() {
                if paused(&SCREEN_REFUSED) {
                    return None;
                }
                match ScreenStream::start(load_token("screencast").as_deref(), width, FPS, quality)
                {
                    Ok((s, token)) => {
                        save_token("screencast", token.as_deref());
                        *g = Some(s);
                    }
                    Err(e) => {
                        eprintln!("[portal] no screen cast: {e}");
                        pause(&SCREEN_REFUSED, SCREEN_PAUSE);
                        return None;
                    }
                }
            }
            g.as_ref()?.latest()
        }
        ScreenWay::Stills => still(width, quality),
        ScreenWay::Grim => None,
    }
}

/// Nobody watches any more: end the cast.
pub fn screen_stop() {
    *lock(&STREAM) = None;
}

fn still(width: u32, quality: u8) -> Option<Vec<u8>> {
    if paused(&SCREEN_REFUSED) {
        return None;
    }
    let png = match screen::screenshot() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[portal] no screenshot: {e}");
            pause(&SCREEN_REFUSED, SCREEN_PAUSE);
            return None;
        }
    };
    let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).ok()?;
    let img = if img.width() > width {
        img.resize(width, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

// --- input played here --------------------------------------------------------

static INPUT: Mutex<Option<Injector>> = Mutex::new(None);
static INPUT_REFUSED: Mutex<Option<Instant>> = Mutex::new(None);

pub fn input_way() -> Result<InputWay, &'static str> {
    let (p, t) = caps();
    caps::input_way(&p, &t)
}

/// Run `f` with the input session, opening it first if need be (the
/// portal may ask the user). The error is a sentence.
pub fn with_input<T>(f: impl FnOnce(&mut Injector) -> T) -> Result<T, String> {
    let mut g = lock(&INPUT);
    if g.is_none() {
        if paused(&INPUT_REFUSED) {
            return Err("Controlling this computer wasn\u{2019}t allowed.".into());
        }
        let way = input_way().map_err(String::from)?;
        match Injector::open(way, load_token("remote-desktop").as_deref()) {
            Ok((inj, token)) => {
                save_token("remote-desktop", token.as_deref());
                *g = Some(inj);
            }
            Err(e) => {
                pause(&INPUT_REFUSED, INPUT_PAUSE);
                return Err(e);
            }
        }
    }
    match g.as_mut() {
        Some(inj) => Ok(f(inj)),
        None => Err("no input session".into()),
    }
}

/// The session stopped working (ended from the desktop's indicator):
/// open a new one next time.
pub fn input_reset() {
    *lock(&INPUT) = None;
}

/// The screens and pointer as the open input session knows them (none
/// open: nothing, and nothing is asked).
pub fn input_screens() -> Vec<Rect> {
    lock(&INPUT)
        .as_ref()
        .map(Injector::screens)
        .unwrap_or_default()
}

pub fn input_cursor() -> Option<(f64, f64)> {
    lock(&INPUT).as_ref().and_then(Injector::cursor)
}

/// Put the pointer at (`x`, `y`), if an input session is open.
pub fn input_warp(x: f64, y: f64) -> bool {
    lock(&INPUT).as_mut().is_some_and(|i| i.warp(x, y))
}

// --- this computer's input sent elsewhere (InputCapture) -------------------------

struct Kvm {
    session: CaptureSession,
    edges: Vec<Edge>,
    attached: bool,
    since: Option<Instant>,
}

static KVM: Mutex<Option<Kvm>> = Mutex::new(None);
static KVM_REFUSED: Mutex<Option<Instant>> = Mutex::new(None);

/// This computer can send its mouse and keyboard (the InputCapture portal).
pub fn can_send() -> bool {
    caps().0.input_capture.is_some()
}

/// Watch `edges` (barriers set up or changed as needed) for up to `wait`:
/// the pointer pushed through one. Then the edge, where the pointer was,
/// and the screens as the portal sees them; the capture is on from here,
/// until `kvm_release`.
pub fn kvm_watch(edges: &[Edge], wait: Duration) -> Option<(Edge, (f64, f64), Vec<Rect>)> {
    let mut g = lock(&KVM);
    if g.is_none() {
        if paused(&KVM_REFUSED) {
            drop(g);
            std::thread::sleep(wait);
            return None;
        }
        match CaptureSession::start() {
            Ok(session) => {
                *g = Some(Kvm {
                    session,
                    edges: Vec::new(),
                    attached: false,
                    since: None,
                })
            }
            Err(e) => {
                eprintln!("[portal] no input capture: {e}");
                pause(&KVM_REFUSED, CAPTURE_PAUSE);
                return None;
            }
        }
    }
    let rearm = g
        .as_ref()
        .is_some_and(|k| k.session.stale() || k.edges != edges);
    if rearm {
        let armed = g.as_mut().map(|k| k.session.arm(edges));
        if let Some(Err(e)) = armed {
            eprintln!("[portal] couldn't watch the screen edges: {e}");
            *g = None;
            pause(&KVM_REFUSED, SCREEN_PAUSE);
            return None;
        }
        if let Some(k) = g.as_mut() {
            k.edges = edges.to_vec();
        }
    }
    let k = g.as_mut()?;
    let (edge, at) = k.session.next_crossing(wait)?;
    k.attached = false;
    k.since = Some(Instant::now());
    Some((edge, at, k.session.zones().to_vec()))
}

/// Send the captured input to `tx`; false when nothing is being captured.
pub fn kvm_attach(tx: Sender<Captured>) -> bool {
    let mut g = lock(&KVM);
    let Some(k) = g.as_mut() else {
        return false;
    };
    if !k.session.active() {
        return false;
    }
    k.session.attach(tx);
    k.attached = true;
    true
}

/// Give the pointer and keyboard back to this computer.
pub fn kvm_release() {
    if let Some(k) = lock(&KVM).as_mut() {
        k.session.release();
        k.attached = false;
        k.since = None;
    }
}

/// A capture began but nobody took it for `after` (starting failed):
/// give it back.
pub fn kvm_release_unclaimed(after: Duration) {
    if let Some(k) = lock(&KVM).as_mut() {
        if k.session.active() && !k.attached && k.since.is_some_and(|s| s.elapsed() > after) {
            k.session.release();
            k.since = None;
        }
    }
}

/// Stop watching the edges (sharing off, or nobody placed around).
pub fn kvm_stop() {
    if let Some(k) = lock(&KVM).as_mut() {
        if !k.edges.is_empty() {
            k.session.disarm();
            k.edges.clear();
        }
    }
}

// --- the clipboard ------------------------------------------------------------

/// The copy on the clipboard is a password manager's concealed one. None:
/// can't tell here (the compositor has no data-control protocol: GNOME).
pub fn clip_concealed() -> Option<bool> {
    use wl_clipboard_rs::paste::{
        get_contents, get_mime_types, ClipboardType, Error, MimeType, Seat,
    };
    let types: Vec<String> = match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
        Ok(t) => t.into_iter().collect(),
        Err(Error::NoSeats | Error::ClipboardEmpty | Error::NoMimeType) => return Some(false),
        Err(_) => return None,
    };
    if !types.iter().any(|t| t == clip::SECRET_HINT) {
        return Some(false);
    }
    let hint = get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(clip::SECRET_HINT),
    )
    .ok()
    .and_then(|(r, _)| {
        let mut v = Vec::new();
        r.take(64).read_to_end(&mut v).ok()?;
        Some(v)
    });
    Some(clip::concealed(&types, hint.as_deref()))
}

/// The clipboard's text through `wl-paste` (when the app's own clipboard
/// can't reach it).
pub fn wl_paste() -> Option<String> {
    let out = Command::new("wl-paste")
        .args(clip::paste_args())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Put `text` on the clipboard through `wl-copy`.
pub fn wl_copy(text: &str) -> bool {
    let Ok(mut child) = Command::new("wl-copy")
        .args(clip::copy_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
    // wl-copy goes on serving in the background; this one returns.
    child.wait().is_ok_and(|s| s.success()) && wrote
}
