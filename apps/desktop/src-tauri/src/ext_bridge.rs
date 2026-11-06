// The browser extension's way in (apps/browser-extension).
//
// Browsers can't talk to an app directly; they start a "native messaging
// host" and talk to it over its stdin/stdout: each message is a 4-byte
// length (native byte order) and that many bytes of JSON. Our host is this
// same executable started by the browser (`host_requested`: the browser
// passes `chrome-extension://<id>/`, Firefox the manifest's path and the
// add-on id; `--native-messaging` by hand). It runs no window: it reads a
// message, checks it (`Request::parse`), hands it to the RUNNING app over a
// local channel and passes the answer back, until the browser closes stdin.
//
// The local channel (`ipc`), same framing:
//  - macOS and Linux: a Unix domain socket, `<config dir>/ext-bridge/
//    bridge.sock`, in a folder only this user can open (0700) and itself
//    0600, so only this user's processes can connect;
//  - Windows: 127.0.0.1 on a random port with a random token, both in
//    `<config dir>\ext-bridge.json` (in the user's own profile), the same
//    way `--send` finds the running app (`send_to.rs`). The token travels
//    in every message and is compared in constant time.
//
// What it can do is what the app's buttons do, to your own devices only
// (the core's device list): list them, send text (clipboard) or a link (a
// message), open a link there, ask one to download a link (remote
// download), save a link to Read later, and send a file the extension
// fetched (an image), in chunks under the 1 MB message cap, saved to a
// temporary folder and sent as a normal transfer. Everything is checked
// twice: by the host before it goes to the app, and by the app.
//
// Settings > Browser extension writes the hosts' manifests (`hosts`) that
// tell each browser where this executable is and which extension ids may
// start it: files in each browser's NativeMessagingHosts folder on macOS
// and Linux, a file plus an HKCU registry key on Windows.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The host's name, as the extension asks for it (`connectNative`).
pub const HOST_NAME: &str = "com.landiscover.mydomain";
/// The unpacked extension's id in Chrome, Edge and Brave: fixed by the
/// public `key` in apps/browser-extension/manifest.json.
pub const DEV_CHROME_ID: &str = "jonpnfgijnakjfneooofcaeobabdecpj";
/// The add-on id in Firefox (`browser_specific_settings.gecko.id`).
pub const FIREFOX_ID: &str = "my-domain@landiscover.app";
/// The biggest message either way (Chrome's cap for a host's messages).
pub const MAX_MESSAGE: usize = 1 << 20;
/// The biggest piece of a file in one message (base64 makes it 4/3).
pub const MAX_CHUNK: usize = 512 * 1024;
/// The biggest file the extension may send.
pub const MAX_UPLOAD: u64 = 25 * 1024 * 1024;
/// The longest text sent to a clipboard.
pub const MAX_TEXT: usize = 64 * 1024;
/// The longest link.
pub const MAX_URL: usize = 4096;
/// The longest page title kept.
pub const MAX_TITLE: usize = 300;
/// The longest file name kept.
const MAX_NAME: usize = 120;
/// Files sent at once.
const MAX_UPLOADS: usize = 4;
/// A file not finished within this long is dropped.
const UPLOAD_IDLE: Duration = Duration::from_secs(5 * 60);
/// Sent files are kept this long (a transfer may resume), then cleared.
const KEEP_SENT: Duration = Duration::from_secs(24 * 60 * 60);
/// The local channel's message version.
const IPC_VERSION: u32 = 1;
/// How long a request may take.
const IPC_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Framing: a 4-byte length in native byte order, then the JSON
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum FrameError {
    /// Bigger than `MAX_MESSAGE` (the length said so; nothing more read).
    TooBig(usize),
    /// The stream ended inside a message.
    Truncated,
    Io(std::io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TooBig(n) => write!(f, "message of {n} bytes is over the 1 MB limit"),
            FrameError::Truncated => write!(f, "the message was cut short"),
            FrameError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// One message, or `None` when the stream ends cleanly between messages.
pub fn read_frame(r: &mut impl Read) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_ne_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(FrameError::TooBig(len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).map_err(|e| match e.kind() {
        ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(e),
    })?;
    Ok(Some(buf))
}

/// Write one message (refused when over `MAX_MESSAGE`).
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> Result<(), FrameError> {
    if payload.len() > MAX_MESSAGE {
        return Err(FrameError::TooBig(payload.len()));
    }
    let len = (payload.len() as u32).to_ne_bytes();
    w.write_all(&len).map_err(FrameError::Io)?;
    w.write_all(payload).map_err(FrameError::Io)?;
    w.flush().map_err(FrameError::Io)
}

// ---------------------------------------------------------------------------
// Requests, checked
// ---------------------------------------------------------------------------

/// What the extension may ask.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// Is the app there? `{ok, app, version}`.
    Hello {},
    /// Your devices: `{ok, devices: [...]}`.
    ListDevices {},
    /// Text onto a device's clipboard.
    SendText { device: String, text: String },
    /// A link (a page's or a link's) as a message.
    SendUrl {
        device: String,
        url: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// Open a link in the device's browser.
    OpenUrl { device: String, url: String },
    /// Ask the device to download a link itself.
    DownloadOn { device: String, url: String },
    /// Save a link to Read later (all your devices).
    ReadLaterAdd {
        url: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// A piece of a file: `offset` 0 starts it, `done` sends it.
    SendFile {
        device: String,
        upload: String,
        name: String,
        offset: u64,
        data: String,
        #[serde(default)]
        done: bool,
    },
}

/// A request that is not acceptable, with why.
#[derive(Debug, Clone, PartialEq)]
pub struct Invalid(pub String);

fn invalid<T>(why: impl Into<String>) -> Result<T, Invalid> {
    Err(Invalid(why.into()))
}

/// A device id: what the core uses (letters, digits, `-_.:`), not too long.
pub fn device_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// A web link: http or https with a host, no spaces or control characters,
/// at most `MAX_URL`. Trimmed.
pub fn web_url(url: &str) -> Result<String, Invalid> {
    let url = url.trim();
    if url.is_empty() {
        return invalid("no link");
    }
    if url.len() > MAX_URL {
        return invalid("that link is too long");
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return invalid("that isn't a link");
    }
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return invalid("only http and https links");
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    if host.is_empty() || host.starts_with(':') {
        return invalid("that link has no site");
    }
    Ok(url.to_string())
}

/// A page title: control characters out, spaces tidied, cut to
/// `MAX_TITLE`; blank is none.
pub fn clean_title(title: Option<&str>) -> Option<String> {
    let t: String = title?
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.chars().take(MAX_TITLE).collect();
    (!t.is_empty()).then_some(t)
}

/// A file name to save under: no folders, no control or reserved
/// characters, no leading dots, at most `MAX_NAME`; "image" if nothing is
/// left.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    let cut: String = cleaned.chars().take(MAX_NAME).collect();
    let cut = cut.trim_end_matches(['.', ' ']).to_string();
    if cut.is_empty() {
        "image".to_string()
    } else if mdcore::features::link_fetch::windows_reserved(&cut) {
        // `CON.png` would open a device on Windows, not make a file.
        format!("_{cut}")
    } else {
        cut
    }
}

/// An upload id: 8 to 64 of `A-Za-z0-9_-`.
pub fn upload_ok(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Request {
    /// Parse and check a message; the request as it should be used
    /// (links trimmed, titles and names cleaned).
    pub fn parse(bytes: &[u8]) -> Result<Request, Invalid> {
        let req: Request =
            serde_json::from_slice(bytes).map_err(|e| Invalid(format!("not a request: {e}")))?;
        req.checked()
    }

    pub fn checked(self) -> Result<Request, Invalid> {
        let device = |d: &str| {
            if device_ok(d) {
                Ok(())
            } else {
                invalid("unknown device")
            }
        };
        Ok(match self {
            Request::Hello {} | Request::ListDevices {} => self,
            Request::SendText { device: d, text } => {
                device(&d)?;
                if text.trim().is_empty() {
                    return invalid("nothing to send");
                }
                if text.len() > MAX_TEXT {
                    return invalid("that text is too long");
                }
                if text.contains('\0') {
                    return invalid("that isn't text");
                }
                Request::SendText { device: d, text }
            }
            Request::SendUrl {
                device: d,
                url,
                title,
            } => {
                device(&d)?;
                Request::SendUrl {
                    device: d,
                    url: web_url(&url)?,
                    title: clean_title(title.as_deref()),
                }
            }
            Request::OpenUrl { device: d, url } => {
                device(&d)?;
                Request::OpenUrl {
                    device: d,
                    url: web_url(&url)?,
                }
            }
            Request::DownloadOn { device: d, url } => {
                device(&d)?;
                Request::DownloadOn {
                    device: d,
                    url: web_url(&url)?,
                }
            }
            Request::ReadLaterAdd { url, title } => Request::ReadLaterAdd {
                url: web_url(&url)?,
                title: clean_title(title.as_deref()),
            },
            Request::SendFile {
                device: d,
                upload,
                name,
                offset,
                data,
                done,
            } => {
                device(&d)?;
                if !upload_ok(&upload) {
                    return invalid("bad upload id");
                }
                if offset > MAX_UPLOAD {
                    return invalid("that file is too big");
                }
                // base64 of at most MAX_CHUNK bytes.
                if data.len() > MAX_CHUNK.div_ceil(3) * 4 {
                    return invalid("that piece is too big");
                }
                Request::SendFile {
                    device: d,
                    upload,
                    name: safe_name(&name),
                    offset,
                    data,
                    done,
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Answers
// ---------------------------------------------------------------------------

/// Error codes the extension shows in its own words.
pub mod code {
    pub const APP_NOT_RUNNING: &str = "app_not_running";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const UNKNOWN_DEVICE: &str = "unknown_device";
    pub const NO_REMOTE_DOWNLOAD: &str = "no_remote_download";
    pub const TOO_BIG: &str = "too_big";
    pub const FAILED: &str = "failed";
}

pub fn fail(code: &str, detail: impl Into<String>) -> Value {
    json!({ "ok": false, "error": code, "detail": detail.into() })
}

fn done(mut extra: Value) -> Value {
    if let Some(o) = extra.as_object_mut() {
        o.insert("ok".into(), Value::Bool(true));
    }
    extra
}

/// One of your devices, as the extension shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtDevice {
    pub node: String,
    pub name: String,
    pub online: bool,
    /// One of the device icons (`mdcore::features::device_kind::ICONS`):
    /// the one you picked for it, else the automatic one.
    pub icon: String,
    pub os: String,
    pub form: String,
    pub model: String,
    /// Takes "Download on" (remote download).
    pub can_download: bool,
}

/// What the app does for the extension (the app's own buttons; a fake in
/// tests).
pub trait Backend {
    /// Your other devices (never this computer).
    fn devices(&self) -> Vec<ExtDevice>;
    fn send_text(&self, node: &str, text: &str) -> Result<String, String>;
    fn send_message(&self, node: &str, text: &str) -> Result<String, String>;
    fn open_url(&self, node: &str, url: &str) -> Result<String, String>;
    fn download_on(&self, node: &str, url: &str) -> Result<String, String>;
    fn read_later(&self, url: &str, title: Option<&str>) -> Result<String, String>;
    fn send_file(&self, node: &str, path: &Path) -> Result<String, String>;
}

/// Answer one checked request.
pub fn handle(backend: &dyn Backend, uploads: &Uploads, req: Request) -> Value {
    let own = |node: &str| backend.devices().into_iter().find(|d| d.node == node);
    let answer = |r: Result<String, String>| match r {
        Ok(via) => done(json!({ "via": via })),
        Err(e) => fail(code::FAILED, e),
    };
    match req {
        Request::Hello {} => done(json!({
            "app": "my-domain",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        Request::ListDevices {} => done(json!({ "devices": backend.devices() })),
        Request::ReadLaterAdd { url, title } => answer(backend.read_later(&url, title.as_deref())),
        Request::SendText { device, text } => match own(&device) {
            None => fail(code::UNKNOWN_DEVICE, "not one of your devices"),
            Some(_) => answer(backend.send_text(&device, &text)),
        },
        Request::SendUrl { device, url, title } => match own(&device) {
            None => fail(code::UNKNOWN_DEVICE, "not one of your devices"),
            Some(_) => {
                let text = match title {
                    Some(t) if t != url => format!("{t}\n{url}"),
                    _ => url,
                };
                answer(backend.send_message(&device, &text))
            }
        },
        Request::OpenUrl { device, url } => match own(&device) {
            None => fail(code::UNKNOWN_DEVICE, "not one of your devices"),
            Some(_) => answer(backend.open_url(&device, &url)),
        },
        Request::DownloadOn { device, url } => match own(&device) {
            None => fail(code::UNKNOWN_DEVICE, "not one of your devices"),
            Some(d) if !d.can_download => fail(
                code::NO_REMOTE_DOWNLOAD,
                "that device doesn't download links",
            ),
            Some(_) => answer(backend.download_on(&device, &url)),
        },
        Request::SendFile {
            device,
            upload,
            name,
            offset,
            data,
            done: last,
        } => {
            if own(&device).is_none() {
                uploads.drop_upload(&upload);
                return fail(code::UNKNOWN_DEVICE, "not one of your devices");
            }
            let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data.as_bytes())
            else {
                uploads.drop_upload(&upload);
                return fail(code::BAD_REQUEST, "that piece isn't base64");
            };
            match uploads.add(&upload, &device, &name, offset, &bytes) {
                Err((c, why)) => fail(c, why),
                Ok(received) if !last => done(json!({ "received": received })),
                Ok(_) => match uploads.finish(&upload) {
                    Err((c, why)) => fail(c, why),
                    Ok((node, path)) => answer(backend.send_file(&node, &path)),
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Files arriving in pieces
// ---------------------------------------------------------------------------

struct Upload {
    node: String,
    path: PathBuf,
    len: u64,
    touched: Instant,
}

/// Files the extension is sending, piece by piece, under `root`
/// (`<root>/<upload id>/<name>`).
pub struct Uploads {
    root: PathBuf,
    open: Mutex<HashMap<String, Upload>>,
}

type UploadError = (&'static str, String);

impl Uploads {
    pub fn new(root: PathBuf) -> Self {
        Uploads {
            root,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Add a piece; the bytes received so far.
    pub fn add(
        &self,
        id: &str,
        node: &str,
        name: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, UploadError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| (code::FAILED, "busy".into()))?;
        let now = Instant::now();
        // Forget what was left half-sent.
        let stale: Vec<String> = open
            .iter()
            .filter(|(_, u)| now.duration_since(u.touched) > UPLOAD_IDLE)
            .map(|(k, _)| k.clone())
            .collect();
        for k in stale {
            if let Some(u) = open.remove(&k) {
                remove_upload_dir(&u.path);
            }
        }
        if offset == 0 {
            if let Some(old) = open.remove(id) {
                remove_upload_dir(&old.path);
            }
            if open.len() >= MAX_UPLOADS {
                return Err((code::FAILED, "too many files at once".into()));
            }
            self.clear_old();
            let dir = self.root.join(id);
            create_private_dir(&dir).map_err(|e| (code::FAILED, e.to_string()))?;
            let path = dir.join(safe_name(name));
            std::fs::File::create(&path).map_err(|e| (code::FAILED, e.to_string()))?;
            open.insert(
                id.to_string(),
                Upload {
                    node: node.to_string(),
                    path,
                    len: 0,
                    touched: now,
                },
            );
        }
        let Some(up) = open.get_mut(id) else {
            return Err((code::BAD_REQUEST, "that file wasn't started".into()));
        };
        if up.node != node || up.len != offset {
            let up = open.remove(id).expect("just found");
            remove_upload_dir(&up.path);
            return Err((code::BAD_REQUEST, "pieces out of order".into()));
        }
        if up.len + bytes.len() as u64 > MAX_UPLOAD {
            let up = open.remove(id).expect("just found");
            remove_upload_dir(&up.path);
            return Err((code::TOO_BIG, "that file is too big".into()));
        }
        let written = std::fs::OpenOptions::new()
            .append(true)
            .open(&up.path)
            .and_then(|mut f| f.write_all(bytes));
        if let Err(e) = written {
            let up = open.remove(id).expect("just found");
            remove_upload_dir(&up.path);
            return Err((code::FAILED, e.to_string()));
        }
        up.len += bytes.len() as u64;
        up.touched = now;
        Ok(up.len)
    }

    /// The file is all there: its device and path.
    pub fn finish(&self, id: &str) -> Result<(String, PathBuf), UploadError> {
        let mut open = self
            .open
            .lock()
            .map_err(|_| (code::FAILED, "busy".into()))?;
        let up = open
            .remove(id)
            .ok_or((code::BAD_REQUEST, "that file wasn't started".to_string()))?;
        if up.len == 0 {
            remove_upload_dir(&up.path);
            return Err((code::BAD_REQUEST, "that file is empty".into()));
        }
        Ok((up.node, up.path))
    }

    pub fn drop_upload(&self, id: &str) {
        if let Ok(mut open) = self.open.lock() {
            if let Some(up) = open.remove(id) {
                remove_upload_dir(&up.path);
            }
        }
    }

    /// Files sent more than a day ago (their transfers are long done).
    fn clear_old(&self) {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return;
        };
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > KEEP_SENT);
            if old && upload_ok(&e.file_name().to_string_lossy()) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

fn remove_upload_dir(file: &Path) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A folder only this user can open.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The local channel between the host and the app
// ---------------------------------------------------------------------------

/// What the host sends the app: the request (already checked) and, on
/// Windows, the token.
#[derive(Serialize, Deserialize)]
struct Envelope {
    v: u32,
    #[serde(default)]
    token: String,
    req: Value,
}

/// Constant-time comparison.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Read one envelope from the host, answer it.
fn serve_one(
    stream: &mut (impl Read + Write),
    token: &str,
    backend: &dyn Backend,
    uploads: &Uploads,
) {
    let answer = match read_frame(stream) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Envelope>(&bytes) {
            Ok(env) if env.v != IPC_VERSION => fail(code::BAD_REQUEST, "unknown version"),
            Ok(env) if !same(env.token.as_bytes(), token.as_bytes()) => {
                // Nothing more for a caller without the token.
                return;
            }
            Ok(env) => {
                let raw = serde_json::to_vec(&env.req).unwrap_or_default();
                match Request::parse(&raw) {
                    Ok(req) => handle(backend, uploads, req),
                    Err(Invalid(why)) => fail(code::BAD_REQUEST, why),
                }
            }
            Err(_) => fail(code::BAD_REQUEST, "not a request"),
        },
        Ok(None) => return,
        Err(FrameError::TooBig(_)) => fail(code::TOO_BIG, "message over 1 MB"),
        Err(_) => return,
    };
    let _ = write_frame(stream, &serde_json::to_vec(&answer).unwrap_or_default());
}

/// The config dir's `ext-bridge` names (one per lab instance, `TEST=...`).
fn channel_name(base: &str, ext: &str) -> String {
    match crate::test_suffix() {
        Some(s) => format!("{base}-{s}{ext}"),
        None => format!("{base}{ext}"),
    }
}

#[cfg(unix)]
mod ipc {
    use super::*;
    use std::os::unix::net::{UnixListener, UnixStream};

    /// `<config dir>/ext-bridge/bridge.sock`.
    pub fn socket_path() -> Option<PathBuf> {
        Some(
            crate::send_to::config_dir()?
                .join("ext-bridge")
                .join(channel_name("bridge", ".sock")),
        )
    }

    /// Send one envelope to the running app, its answer.
    pub fn call(req: &Value) -> Result<Vec<u8>, String> {
        let path = socket_path().ok_or("no config folder")?;
        let mut s = UnixStream::connect(&path).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(IPC_TIMEOUT)).ok();
        s.set_write_timeout(Some(IPC_TIMEOUT)).ok();
        let env = Envelope {
            v: IPC_VERSION,
            token: String::new(),
            req: req.clone(),
        };
        write_frame(&mut s, &serde_json::to_vec(&env).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        read_frame(&mut s)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no answer".to_string())
    }

    /// Listen (in the app). Err when it can't.
    pub fn listen(
        serve: impl Fn(&mut UnixStream, &str) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let path = socket_path().ok_or("no config folder")?;
        let dir = path.parent().ok_or("no folder")?;
        create_private_dir(dir).map_err(|e| e.to_string())?;
        // Another copy of the app already answering: leave it be.
        if UnixStream::connect(&path).is_ok() {
            return Err("another my-domain is already listening".into());
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        let serve = std::sync::Arc::new(serve);
        let busy = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::sync::atomic::Ordering;
                if busy.fetch_add(1, Ordering::SeqCst) >= 4 {
                    busy.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let (serve, busy) = (serve.clone(), busy.clone());
                std::thread::spawn(move || {
                    stream.set_read_timeout(Some(IPC_TIMEOUT)).ok();
                    stream.set_write_timeout(Some(IPC_TIMEOUT)).ok();
                    serve(&mut stream, "");
                    busy.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Ok(())
    }
}

#[cfg(windows)]
mod ipc {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

    #[derive(Serialize, Deserialize)]
    struct Listening {
        port: u16,
        token: String,
    }

    /// `<config dir>\ext-bridge.json`.
    fn info_file() -> Option<PathBuf> {
        Some(crate::send_to::config_dir()?.join(channel_name("ext-bridge", ".json")))
    }

    pub fn call(req: &Value) -> Result<Vec<u8>, String> {
        let file = info_file().ok_or("no config folder")?;
        let info: Listening = std::fs::read_to_string(file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .ok_or("not running")?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
        let mut s =
            TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(IPC_TIMEOUT)).ok();
        s.set_write_timeout(Some(IPC_TIMEOUT)).ok();
        let env = Envelope {
            v: IPC_VERSION,
            token: info.token,
            req: req.clone(),
        };
        write_frame(&mut s, &serde_json::to_vec(&env).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        read_frame(&mut s)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no answer".to_string())
    }

    pub fn listen(
        serve: impl Fn(&mut TcpStream, &str) + Send + Sync + 'static,
    ) -> Result<(), String> {
        let file = info_file().ok_or("no config folder")?;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let info = Listening {
            port,
            token: token.clone(),
        };
        file.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, serde_json::to_vec(&info).unwrap_or_default()))
            .map_err(|e| e.to_string())?;
        let serve = std::sync::Arc::new(serve);
        let busy = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::sync::atomic::Ordering;
                if busy.fetch_add(1, Ordering::SeqCst) >= 4 {
                    busy.fetch_sub(1, Ordering::SeqCst);
                    continue;
                }
                let (serve, busy, token) = (serve.clone(), busy.clone(), token.clone());
                std::thread::spawn(move || {
                    stream.set_read_timeout(Some(IPC_TIMEOUT)).ok();
                    stream.set_write_timeout(Some(IPC_TIMEOUT)).ok();
                    serve(&mut stream, &token);
                    busy.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The host (this executable, started by a browser)
// ---------------------------------------------------------------------------

/// Was this executable started by a browser as the extension's host?
/// Chrome, Edge and Brave pass `chrome-extension://<id>/` (and on Windows
/// `--parent-window=…`); Firefox passes the manifest's path and the add-on
/// id; `--native-messaging` by hand.
pub fn host_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let args: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|a| a.into().to_string_lossy().into_owned())
        .collect();
    let Some(first) = args.first() else {
        return false;
    };
    first == "--native-messaging"
        || first.starts_with("chrome-extension://")
        || (Path::new(first)
            .file_name()
            .is_some_and(|n| n.to_string_lossy() == format!("{HOST_NAME}.json"))
            && args.get(1).is_some_and(|id| id == FIREFOX_ID))
}

/// Answer one message from the browser: check it, pass it to the app.
pub fn host_answer(bytes: &[u8], call: impl Fn(&Value) -> Result<Vec<u8>, String>) -> Vec<u8> {
    let answer = match serde_json::from_slice::<Value>(bytes) {
        Err(_) => fail(code::BAD_REQUEST, "not JSON"),
        Ok(value) => match Request::parse(bytes) {
            Err(Invalid(why)) => fail(code::BAD_REQUEST, why),
            Ok(_) => match call(&value) {
                Ok(reply) => return reply,
                Err(e) => fail(
                    code::APP_NOT_RUNNING,
                    format!("my-domain isn't running ({e})"),
                ),
            },
        },
    };
    serde_json::to_vec(&answer).unwrap_or_default()
}

/// The host's loop over stdin/stdout; the exit code.
pub fn run_host() -> i32 {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    loop {
        let reply = match read_frame(&mut stdin) {
            Ok(None) => return 0,
            Ok(Some(bytes)) => host_answer(&bytes, ipc::call),
            Err(e) => {
                // The stream can't be followed any more: say why and stop.
                let msg = fail(code::TOO_BIG, e.to_string());
                let _ = write_frame(&mut stdout, &serde_json::to_vec(&msg).unwrap_or_default());
                return 1;
            }
        };
        if write_frame(&mut stdout, &reply).is_err() {
            return 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Host manifests: where each browser looks
// ---------------------------------------------------------------------------

pub mod hosts {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Os {
        Mac,
        Linux,
        Windows,
    }

    impl Os {
        pub fn this() -> Os {
            if cfg!(target_os = "macos") {
                Os::Mac
            } else if cfg!(windows) {
                Os::Windows
            } else {
                Os::Linux
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Browser {
        Chrome,
        Chromium,
        Edge,
        Brave,
        Firefox,
    }

    pub const ALL: [Browser; 5] = [
        Browser::Chrome,
        Browser::Edge,
        Browser::Brave,
        Browser::Chromium,
        Browser::Firefox,
    ];

    impl Browser {
        pub fn id(self) -> &'static str {
            match self {
                Browser::Chrome => "chrome",
                Browser::Chromium => "chromium",
                Browser::Edge => "edge",
                Browser::Brave => "brave",
                Browser::Firefox => "firefox",
            }
        }

        pub fn name(self) -> &'static str {
            match self {
                Browser::Chrome => "Google Chrome",
                Browser::Chromium => "Chromium",
                Browser::Edge => "Microsoft Edge",
                Browser::Brave => "Brave",
                Browser::Firefox => "Firefox",
            }
        }

        pub fn is_firefox(self) -> bool {
            self == Browser::Firefox
        }
    }

    /// The browser's own folder (macOS and Linux): there means installed
    /// (or at least run once).
    pub fn browser_dir(os: Os, b: Browser, home: &Path) -> Option<PathBuf> {
        let rel = match (os, b) {
            (Os::Mac, Browser::Chrome) => "Library/Application Support/Google/Chrome",
            (Os::Mac, Browser::Chromium) => "Library/Application Support/Chromium",
            (Os::Mac, Browser::Edge) => "Library/Application Support/Microsoft Edge",
            (Os::Mac, Browser::Brave) => "Library/Application Support/BraveSoftware/Brave-Browser",
            (Os::Mac, Browser::Firefox) => "Library/Application Support/Mozilla",
            (Os::Linux, Browser::Chrome) => ".config/google-chrome",
            (Os::Linux, Browser::Chromium) => ".config/chromium",
            (Os::Linux, Browser::Edge) => ".config/microsoft-edge",
            (Os::Linux, Browser::Brave) => ".config/BraveSoftware/Brave-Browser",
            (Os::Linux, Browser::Firefox) => ".mozilla",
            (Os::Windows, _) => return None,
        };
        Some(home.join(rel))
    }

    /// Where the manifest goes on macOS and Linux.
    pub fn manifest_path(os: Os, b: Browser, home: &Path) -> Option<PathBuf> {
        let folder = match (os, b) {
            (Os::Linux, Browser::Firefox) => "native-messaging-hosts",
            _ => "NativeMessagingHosts",
        };
        Some(
            browser_dir(os, b, home)?
                .join(folder)
                .join(format!("{HOST_NAME}.json")),
        )
    }

    /// The HKCU key naming the manifest on Windows.
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    pub fn registry_key(b: Browser) -> String {
        let vendor = match b {
            Browser::Chrome => r"Google\Chrome",
            Browser::Chromium => "Chromium",
            Browser::Edge => r"Microsoft\Edge",
            Browser::Brave => r"BraveSoftware\Brave-Browser",
            Browser::Firefox => "Mozilla",
        };
        format!(r"HKCU\Software\{vendor}\NativeMessagingHosts\{HOST_NAME}")
    }

    /// Where the manifest itself goes on Windows (one for the Chrome
    /// family, one for Firefox), under the app's config dir.
    pub fn windows_manifest(config: &Path, b: Browser) -> PathBuf {
        let file = if b.is_firefox() {
            "firefox"
        } else {
            "chromium"
        };
        config
            .join("native-messaging")
            .join(format!("{file}-{HOST_NAME}.json"))
    }

    /// A Chrome extension id: 32 letters a to p.
    pub fn chrome_id_ok(id: &str) -> bool {
        id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
    }

    /// The Chrome ids to allow: the unpacked one, then the ones given
    /// (trimmed, checked, no repeats).
    pub fn chrome_ids(extra: &[String]) -> Result<Vec<String>, String> {
        let mut ids = vec![DEV_CHROME_ID.to_string()];
        for id in extra {
            let id = id.trim().to_ascii_lowercase();
            if id.is_empty() {
                continue;
            }
            if !chrome_id_ok(&id) {
                return Err(format!("\"{id}\" isn't an extension id (32 letters a-p)"));
            }
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// The manifest's JSON.
    pub fn manifest_json(b: Browser, exe: &Path, chrome_ids: &[String]) -> String {
        let mut m = json!({
            "name": HOST_NAME,
            "description": "my-domain: send pages, links and pictures to your devices",
            "path": exe.to_string_lossy(),
            "type": "stdio",
        });
        if b.is_firefox() {
            m["allowed_extensions"] = json!([FIREFOX_ID]);
        } else {
            let origins: Vec<String> = chrome_ids
                .iter()
                .map(|id| format!("chrome-extension://{id}/"))
                .collect();
            m["allowed_origins"] = json!(origins);
        }
        serde_json::to_string_pretty(&m).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// The app's side
// ---------------------------------------------------------------------------

use tauri::{AppHandle, Manager, State};

use crate::Eng;
use mdcore::kv::FileKv;
use mdcore::CoreError;

const IDS_KEY: &str = "chrome_ids";

/// Settings > Browser extension, and the files sent from the browser.
pub struct ExtBridge {
    kv: Option<FileKv>,
    uploads: Uploads,
    listening: Mutex<Result<(), String>>,
}

impl ExtBridge {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("browser-extension.json")));
        ExtBridge {
            kv,
            uploads: Uploads::new(std::env::temp_dir().join(channel_name("my-domain-browser", ""))),
            listening: Mutex::new(Err("not started".into())),
        }
    }

    fn extra_ids(&self) -> Vec<String> {
        self.kv
            .as_ref()
            .and_then(|kv| kv.get(IDS_KEY))
            .map(|s| {
                s.split(',')
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

struct AppBackend {
    app: AppHandle,
}

impl AppBackend {
    fn engine(&self) -> Result<State<'_, Eng>, String> {
        self.app
            .try_state::<Eng>()
            .ok_or_else(|| "not started yet".to_string())
    }
}

impl Backend for AppBackend {
    fn devices(&self) -> Vec<ExtDevice> {
        let Ok(engine) = self.engine() else {
            return Vec::new();
        };
        let me = engine.identity().node_id;
        let live = engine.connected_peers();
        let kinds = engine.device_kinds();
        let mut out: Vec<ExtDevice> = Vec::new();
        for p in engine.get_peers() {
            if p.node_id == me || out.iter().any(|d| d.node == p.node_id) {
                continue;
            }
            let kind = kinds.get(&p.node_id).cloned().unwrap_or_default();
            let icon = crate::device_labels::icon(&self.app, &p.node_id).unwrap_or_else(|| {
                if kind.icon.is_empty() {
                    "unknown".to_string()
                } else {
                    kind.icon.clone()
                }
            });
            out.push(ExtDevice {
                name: crate::device_labels::display_name(&self.app, &p.node_id, &p.name),
                online: live.contains(&p.node_id),
                icon,
                os: kind.os,
                form: kind.form,
                model: kind.model,
                can_download: engine.remote_download_supported(&p.node_id),
                node: p.node_id,
            });
        }
        out
    }

    fn send_text(&self, node: &str, text: &str) -> Result<String, String> {
        let engine = self.engine()?;
        let via = engine
            .send_text(node, text, "clipboard")
            .map_err(|e| e.to_string())?;
        crate::clip_history::on_sent(&self.app, node, text);
        Ok(via)
    }

    fn send_message(&self, node: &str, text: &str) -> Result<String, String> {
        let engine = self.engine()?;
        let id = uuid::Uuid::new_v4().to_string();
        crate::chat_history::on_sending(&self.app, node, &id, text);
        let result = engine.send_with_id(node, text, &id);
        crate::chat_history::on_send_result(&self.app, &id, &result);
        result.map_err(|e| e.to_string())
    }

    fn open_url(&self, node: &str, url: &str) -> Result<String, String> {
        self.engine()?
            .open_url(node, url)
            .map_err(|e| e.to_string())
    }

    fn download_on(&self, node: &str, url: &str) -> Result<String, String> {
        let item = self
            .engine()?
            .request_remote_download(node, url)
            .map_err(|e| e.to_string())?;
        Ok(item.id)
    }

    fn read_later(&self, url: &str, title: Option<&str>) -> Result<String, String> {
        let item = self
            .engine()?
            .save_read_later(url, title)
            .map_err(|e| e.to_string())?;
        Ok(item.id)
    }

    fn send_file(&self, node: &str, path: &Path) -> Result<String, String> {
        let path = path.to_string_lossy();
        let id = self
            .engine()?
            .send_file(node, &path)
            .map_err(|e| e.to_string())?;
        crate::transfers::on_sending(&self.app, &id, node, &path);
        Ok(id)
    }
}

/// Start answering the host (always on: only this user's processes, and
/// only browsers given a manifest, can reach it).
pub fn spawn(app: AppHandle) {
    let handle = app.clone();
    let result = ipc::listen(move |stream, token| {
        let Some(state) = handle.try_state::<ExtBridge>() else {
            return;
        };
        let backend = AppBackend {
            app: handle.clone(),
        };
        serve_one(stream, token, &backend, &state.uploads);
    });
    if let Err(e) = &result {
        eprintln!("[browser] not listening for the extension: {e}");
    }
    if let Some(state) = app.try_state::<ExtBridge>() {
        if let Ok(mut l) = state.listening.lock() {
            *l = result;
        }
    }
}

/// One browser's line in Settings.
#[derive(Serialize, specta::Type, Clone, Debug)]
pub struct BrowserHost {
    pub id: String,
    pub name: String,
    /// The browser seems to be on this computer (always true on Windows,
    /// where it isn't looked for).
    pub found: bool,
    /// Its manifest is in place.
    pub installed: bool,
}

/// Settings > Browser extension.
#[derive(Serialize, specta::Type, Clone, Debug)]
pub struct BrowserExtension {
    pub browsers: Vec<BrowserHost>,
    /// Published extension ids allowed besides the unpacked one.
    pub chrome_ids: Vec<String>,
    pub dev_chrome_id: String,
    pub firefox_id: String,
    /// The app answers the extension.
    pub listening: bool,
    pub error: Option<String>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The executable browsers start (an AppImage's own path on Linux).
fn host_exe() -> Result<PathBuf, String> {
    if cfg!(target_os = "linux") {
        if let Some(app_image) = std::env::var_os("APPIMAGE") {
            return Ok(PathBuf::from(app_image));
        }
    }
    std::env::current_exe().map_err(|e| format!("can't find my-domain's executable: {e}"))
}

#[cfg(windows)]
fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn status(state: &ExtBridge) -> BrowserExtension {
    use hosts::*;
    let os = Os::this();
    let browsers = ALL
        .iter()
        .map(|&b| {
            let (found, installed) = match os {
                #[cfg(windows)]
                Os::Windows => (true, reg(&["query", &registry_key(b), "/ve"])),
                #[cfg(not(windows))]
                Os::Windows => (false, false),
                _ => match home() {
                    Some(h) => (
                        browser_dir(os, b, &h).is_some_and(|d| d.is_dir()),
                        manifest_path(os, b, &h).is_some_and(|p| p.is_file()),
                    ),
                    None => (false, false),
                },
            };
            BrowserHost {
                id: b.id().into(),
                name: b.name().into(),
                found,
                installed,
            }
        })
        .collect();
    let listening = state.listening.lock().map(|l| l.clone()).unwrap_or(Ok(()));
    BrowserExtension {
        browsers,
        chrome_ids: state.extra_ids(),
        dev_chrome_id: DEV_CHROME_ID.into(),
        firefox_id: FIREFOX_ID.into(),
        listening: listening.is_ok(),
        error: listening.err(),
    }
}

/// Write one browser's manifest; false when that browser isn't here.
fn install_one(b: hosts::Browser, exe: &Path, ids: &[String]) -> Result<bool, String> {
    use hosts::*;
    let body = manifest_json(b, exe, ids);
    match Os::this() {
        Os::Windows => {
            let config = crate::send_to::config_dir().ok_or("no config folder")?;
            let file = windows_manifest(&config, b);
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&file, body).map_err(|e| e.to_string())?;
            #[cfg(windows)]
            {
                let path = file.to_string_lossy().into_owned();
                let key = registry_key(b);
                if !reg(&["add", &key, "/ve", "/t", "REG_SZ", "/d", &path, "/f"]) {
                    return Err(format!("couldn't add {key}"));
                }
            }
            Ok(true)
        }
        os => {
            let home = home().ok_or("no home folder")?;
            if !browser_dir(os, b, &home).is_some_and(|d| d.is_dir()) {
                return Ok(false);
            }
            let file = manifest_path(os, b, &home).ok_or("no folder")?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&file, body).map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

fn remove_all() -> Result<(), String> {
    use hosts::*;
    match Os::this() {
        Os::Windows => {
            #[cfg(windows)]
            for b in ALL {
                let key = registry_key(b);
                if reg(&["query", &key, "/ve"]) && !reg(&["delete", &key, "/f"]) {
                    return Err(format!("couldn't remove {key}"));
                }
            }
            if let Some(config) = crate::send_to::config_dir() {
                for b in ALL {
                    let _ = std::fs::remove_file(windows_manifest(&config, b));
                }
            }
            Ok(())
        }
        os => {
            let home = home().ok_or("no home folder")?;
            for b in ALL {
                if let Some(p) = manifest_path(os, b, &home).filter(|p| p.is_file()) {
                    std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
                }
            }
            Ok(())
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn browser_extension_status(state: State<ExtBridge>) -> BrowserExtension {
    status(&state)
}

/// Let the browsers on this computer start my-domain for the extension:
/// the unpacked extension's id plus `chrome_ids` (published ones).
#[tauri::command]
#[specta::specta]
pub fn browser_extension_install(
    chrome_ids: Vec<String>,
    state: State<ExtBridge>,
) -> Result<BrowserExtension, CoreError> {
    let ids = hosts::chrome_ids(&chrome_ids).map_err(CoreError::InvalidInput)?;
    let exe = host_exe().map_err(CoreError::InvalidInput)?;
    let mut any = false;
    for b in hosts::ALL {
        any |= install_one(b, &exe, &ids).map_err(CoreError::InvalidInput)?;
    }
    if !any {
        return Err(CoreError::InvalidInput(
            "no Chrome, Edge, Brave, Chromium or Firefox found on this computer".into(),
        ));
    }
    if let Some(kv) = &state.kv {
        let _ = kv.set(IDS_KEY, &ids[1..].join(","));
    }
    Ok(status(&state))
}

/// Take the manifests away again (every browser).
#[tauri::command]
#[specta::specta]
pub fn browser_extension_remove(state: State<ExtBridge>) -> Result<BrowserExtension, CoreError> {
    remove_all().map_err(CoreError::InvalidInput)?;
    Ok(status(&state))
}

#[cfg(test)]
mod tests {
    use super::hosts::*;
    use super::*;
    use std::io::Cursor;

    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, payload).unwrap();
        out
    }

    #[test]
    fn frames_are_a_native_length_then_the_bytes() {
        let out = framed(br#"{"type":"hello"}"#);
        assert_eq!(&out[..4], &16u32.to_ne_bytes());
        assert_eq!(&out[4..], br#"{"type":"hello"}"#);
        let mut two = framed(b"{}");
        two.extend(framed(b"[1]"));
        let mut r = Cursor::new(two);
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"{}");
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"[1]");
        assert!(read_frame(&mut r).unwrap().is_none(), "clean end");
    }

    #[test]
    fn frames_over_a_megabyte_are_refused_without_reading_them() {
        let mut big = ((MAX_MESSAGE + 1) as u32).to_ne_bytes().to_vec();
        big.extend(b"xx");
        assert!(matches!(
            read_frame(&mut Cursor::new(big)),
            Err(FrameError::TooBig(n)) if n == MAX_MESSAGE + 1
        ));
        let exactly = vec![b' '; MAX_MESSAGE];
        let out = framed(&exactly);
        assert_eq!(
            read_frame(&mut Cursor::new(out)).unwrap().unwrap().len(),
            MAX_MESSAGE
        );
        let mut sink = Vec::new();
        assert!(matches!(
            write_frame(&mut sink, &vec![0u8; MAX_MESSAGE + 1]),
            Err(FrameError::TooBig(_))
        ));
        assert!(sink.is_empty());
    }

    #[test]
    fn cut_short_frames_are_errors() {
        assert!(matches!(
            read_frame(&mut Cursor::new(vec![5, 0])),
            Err(FrameError::Truncated)
        ));
        let mut short = 10u32.to_ne_bytes().to_vec();
        short.extend(b"abc");
        assert!(matches!(
            read_frame(&mut Cursor::new(short)),
            Err(FrameError::Truncated)
        ));
    }

    fn parse(s: &str) -> Result<Request, Invalid> {
        Request::parse(s.as_bytes())
    }

    #[test]
    fn requests_are_checked() {
        assert_eq!(parse(r#"{"type":"hello"}"#), Ok(Request::Hello {}));
        assert_eq!(
            parse(r#"{"type":"list_devices"}"#),
            Ok(Request::ListDevices {})
        );
        assert!(parse(r#"{"type":"rm_rf"}"#).is_err());
        assert!(
            parse(r#"{"type":"hello","extra":1}"#).is_err(),
            "unknown fields"
        );
        assert!(parse("not json").is_err());
        assert_eq!(
            parse(r#"{"type":"open_url","device":"n1","url":"  https://example.com/a?b#c "}"#),
            Ok(Request::OpenUrl {
                device: "n1".into(),
                url: "https://example.com/a?b#c".into()
            })
        );
        for bad in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https:///path",
            "http://exa mple.com",
            "https://example.com/\u{7}",
            "ftp://example.com",
        ] {
            let req = json!({"type": "open_url", "device": "n1", "url": bad}).to_string();
            assert!(parse(&req).is_err(), "{bad} refused");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL));
        assert!(web_url(&long).is_err());
        assert!(web_url("HTTPS://Example.com").is_ok());
        for bad_dev in ["", "../x", "a b", &"x".repeat(129), "n1;rm"] {
            let req = json!({"type":"send_text","device":bad_dev,"text":"hi"}).to_string();
            assert!(parse(&req).is_err(), "device {bad_dev:?} refused");
        }
        assert!(parse(r#"{"type":"send_text","device":"n1","text":"   "}"#).is_err());
        let huge = json!({"type":"send_text","device":"n1","text":"x".repeat(MAX_TEXT + 1)});
        assert!(parse(&huge.to_string()).is_err());
        assert_eq!(
            parse(r#"{"type":"read_later_add","url":"https://a.io","title":" A\n  page\u0000 "}"#),
            Ok(Request::ReadLaterAdd {
                url: "https://a.io".into(),
                title: Some("A page".into())
            })
        );
        assert_eq!(
            parse(r#"{"type":"send_url","device":"n1","url":"https://a.io","title":"   "}"#),
            Ok(Request::SendUrl {
                device: "n1".into(),
                url: "https://a.io".into(),
                title: None
            })
        );
    }

    #[test]
    fn file_pieces_are_checked() {
        let ok = json!({"type":"send_file","device":"n1","upload":"abcdefgh12","name":"../../etc/x.png","offset":0,"data":"aGk="});
        match parse(&ok.to_string()).unwrap() {
            Request::SendFile { name, done, .. } => {
                assert_eq!(name, "x.png");
                assert!(!done);
            }
            other => panic!("{other:?}"),
        }
        let short_id = json!({"type":"send_file","device":"n1","upload":"abc","name":"x","offset":0,"data":""});
        assert!(parse(&short_id.to_string()).is_err());
        let big = json!({"type":"send_file","device":"n1","upload":"abcdefgh12","name":"x","offset":0,"data":"A".repeat(MAX_CHUNK / 3 * 4 + 8)});
        assert!(parse(&big.to_string()).is_err());
        let far = json!({"type":"send_file","device":"n1","upload":"abcdefgh12","name":"x","offset":MAX_UPLOAD + 1,"data":""});
        assert!(parse(&far.to_string()).is_err());
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("photo.jpg"), "photo.jpg");
        assert_eq!(safe_name("C:\\Users\\x\\evil.exe"), "evil.exe");
        assert_eq!(safe_name("/a/b/../.hidden"), "hidden");
        assert_eq!(safe_name("..."), "image");
        assert_eq!(safe_name(""), "image");
        assert_eq!(safe_name("a<b>:c?.png"), "a_b__c_.png");
        assert_eq!(safe_name("tab\there.png"), "tabhere.png");
        assert_eq!(safe_name("https://x/CON.png"), "_CON.png");
        assert_eq!(safe_name("nul.jpg"), "_nul.jpg");
        assert_eq!(safe_name("com1"), "_com1");
        assert_eq!(safe_name("console.png"), "console.png");
        assert_eq!(safe_name(&"n".repeat(500)).chars().count(), 120);
    }

    #[test]
    fn the_host_is_recognised_from_its_arguments() {
        let yes = |a: &[&str]| host_requested(a.iter().map(|s| s.to_string()));
        assert!(yes(&[
            "my-domain",
            "chrome-extension://jonpnfgijnakjfneooofcaeobabdecpj/"
        ]));
        assert!(yes(&[
            "my-domain.exe",
            "chrome-extension://abc/",
            "--parent-window=0"
        ]));
        assert!(yes(&[
            "my-domain",
            "/home/u/.mozilla/native-messaging-hosts/com.landiscover.mydomain.json",
            FIREFOX_ID
        ]));
        assert!(yes(&["my-domain", "--native-messaging"]));
        assert!(!yes(&["my-domain"]));
        assert!(!yes(&["my-domain", "--send", "chrome-extension://x/"]));
        assert!(!yes(&["my-domain", "/tmp/other.json", FIREFOX_ID]));
    }

    #[test]
    fn manifests_go_where_each_browser_looks() {
        let home = Path::new("/home/u");
        let p = |os, b| manifest_path(os, b, home).unwrap();
        let file = format!("{HOST_NAME}.json");
        assert_eq!(
            p(Os::Mac, Browser::Chrome),
            home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
                .join(&file)
        );
        assert_eq!(
            p(Os::Mac, Browser::Edge),
            home.join("Library/Application Support/Microsoft Edge/NativeMessagingHosts")
                .join(&file)
        );
        assert_eq!(
            p(Os::Mac, Browser::Brave),
            home.join(
                "Library/Application Support/BraveSoftware/Brave-Browser/NativeMessagingHosts"
            )
            .join(&file)
        );
        assert_eq!(
            p(Os::Mac, Browser::Firefox),
            home.join("Library/Application Support/Mozilla/NativeMessagingHosts")
                .join(&file)
        );
        assert_eq!(
            p(Os::Linux, Browser::Chrome),
            home.join(".config/google-chrome/NativeMessagingHosts")
                .join(&file)
        );
        assert_eq!(
            p(Os::Linux, Browser::Chromium),
            home.join(".config/chromium/NativeMessagingHosts")
                .join(&file)
        );
        assert_eq!(
            p(Os::Linux, Browser::Firefox),
            home.join(".mozilla/native-messaging-hosts").join(&file)
        );
        assert!(manifest_path(Os::Windows, Browser::Chrome, home).is_none());
        assert_eq!(
            registry_key(Browser::Chrome),
            format!(r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{HOST_NAME}")
        );
        assert_eq!(
            registry_key(Browser::Edge),
            format!(r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\{HOST_NAME}")
        );
        assert_eq!(
            registry_key(Browser::Firefox),
            format!(r"HKCU\Software\Mozilla\NativeMessagingHosts\{HOST_NAME}")
        );
        assert_ne!(
            windows_manifest(Path::new("C:/cfg"), Browser::Firefox),
            windows_manifest(Path::new("C:/cfg"), Browser::Edge)
        );
    }

    #[test]
    fn manifests_allow_only_our_extensions() {
        let ids =
            chrome_ids(&[" ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP ".into(), String::new()]).unwrap();
        assert_eq!(
            ids,
            vec![
                DEV_CHROME_ID.to_string(),
                "abcdefghijklmnopabcdefghijklmnop".into()
            ]
        );
        assert!(chrome_ids(&["not-an-id".into()]).is_err());
        assert!(chrome_ids(&["zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz".into()]).is_err());
        let exe = Path::new("/Applications/my-domain.app/Contents/MacOS/my-domain");
        let chrome: Value =
            serde_json::from_str(&manifest_json(Browser::Chrome, exe, &ids)).unwrap();
        assert_eq!(chrome["name"], HOST_NAME);
        assert_eq!(chrome["type"], "stdio");
        assert_eq!(chrome["path"], exe.to_string_lossy().as_ref());
        assert_eq!(
            chrome["allowed_origins"],
            json!([
                format!("chrome-extension://{DEV_CHROME_ID}/"),
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop/"
            ])
        );
        assert!(chrome.get("allowed_extensions").is_none());
        let ff: Value = serde_json::from_str(&manifest_json(Browser::Firefox, exe, &ids)).unwrap();
        assert_eq!(ff["allowed_extensions"], json!([FIREFOX_ID]));
        assert!(ff.get("allowed_origins").is_none());
    }

    #[test]
    fn the_extension_manifest_matches() {
        let m: Value =
            serde_json::from_str(include_str!("../../../browser-extension/manifest.json")).unwrap();
        assert_eq!(m["browser_specific_settings"]["gecko"]["id"], FIREFOX_ID);
        assert_eq!(m["manifest_version"], 3);
        let bg = include_str!("../../../browser-extension/lib/messages.js");
        assert!(bg.contains(HOST_NAME), "the extension asks for our host");
        assert!(bg.contains(&MAX_CHUNK.to_string()), "same piece size");
        // The key fixes the unpacked id (DEV_CHROME_ID); it must be base64.
        assert!(base64::engine::general_purpose::STANDARD
            .decode(m["key"].as_str().unwrap())
            .is_ok());
    }

    struct Fake {
        log: Mutex<Vec<String>>,
    }

    impl Backend for Fake {
        fn devices(&self) -> Vec<ExtDevice> {
            vec![ExtDevice {
                node: "phone1".into(),
                name: "Pixel".into(),
                online: true,
                icon: "android_phone".into(),
                os: "android".into(),
                form: "phone".into(),
                model: "Pixel 8".into(),
                can_download: false,
            }]
        }
        fn send_text(&self, node: &str, text: &str) -> Result<String, String> {
            self.log.lock().unwrap().push(format!("text {node} {text}"));
            Ok("direct".into())
        }
        fn send_message(&self, node: &str, text: &str) -> Result<String, String> {
            self.log.lock().unwrap().push(format!("msg {node} {text}"));
            Ok("direct".into())
        }
        fn open_url(&self, node: &str, url: &str) -> Result<String, String> {
            self.log.lock().unwrap().push(format!("open {node} {url}"));
            Ok("direct".into())
        }
        fn download_on(&self, node: &str, url: &str) -> Result<String, String> {
            self.log.lock().unwrap().push(format!("dl {node} {url}"));
            Ok("id".into())
        }
        fn read_later(&self, url: &str, title: Option<&str>) -> Result<String, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("later {url} {title:?}"));
            Ok("id".into())
        }
        fn send_file(&self, node: &str, path: &Path) -> Result<String, String> {
            let body = std::fs::read(path).map_err(|e| e.to_string())?;
            self.log.lock().unwrap().push(format!(
                "file {node} {} {}",
                path.file_name().unwrap().to_string_lossy(),
                String::from_utf8_lossy(&body)
            ));
            Ok("t1".into())
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("md-ext-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn only_your_own_devices_get_anything() {
        let fake = Fake {
            log: Mutex::new(Vec::new()),
        };
        let up = Uploads::new(scratch("own"));
        let run = |s: &str| handle(&fake, &up, parse(s).unwrap());
        let r = run(r#"{"type":"open_url","device":"stranger","url":"https://a.io"}"#);
        assert_eq!(r["error"], code::UNKNOWN_DEVICE);
        let r = run(r#"{"type":"open_url","device":"phone1","url":"https://a.io"}"#);
        assert_eq!(r["ok"], true);
        let r = run(r#"{"type":"download_on","device":"phone1","url":"https://a.io/f.zip"}"#);
        assert_eq!(r["error"], code::NO_REMOTE_DOWNLOAD);
        let r = run(r#"{"type":"send_url","device":"phone1","url":"https://a.io","title":"A"}"#);
        assert_eq!(r["ok"], true);
        let r = run(r#"{"type":"list_devices"}"#);
        assert_eq!(r["devices"][0]["icon"], "android_phone");
        assert_eq!(r["devices"][0]["can_download"], false);
        assert_eq!(
            *fake.log.lock().unwrap(),
            vec!["open phone1 https://a.io", "msg phone1 A\nhttps://a.io"]
        );
    }

    #[test]
    fn files_arrive_in_pieces_and_go_as_one() {
        let fake = Fake {
            log: Mutex::new(Vec::new()),
        };
        let root = scratch("pieces");
        let up = Uploads::new(root.clone());
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        let piece = |offset: u64, data: &str, done: bool| {
            let req = json!({"type":"send_file","device":"phone1","upload":"up-0001-abc","name":"cat.png",
                "offset": offset, "data": b64(data), "done": done});
            handle(&fake, &up, parse(&req.to_string()).unwrap())
        };
        assert_eq!(piece(0, "hello ", false)["received"], 6);
        // A piece out of order drops the file.
        assert_eq!(piece(3, "x", false)["error"], code::BAD_REQUEST);
        assert_eq!(piece(6, "world", true)["error"], code::BAD_REQUEST);
        assert_eq!(piece(0, "hello ", false)["received"], 6);
        let r = piece(6, "world", true);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(
            fake.log.lock().unwrap().last().unwrap(),
            "file phone1 cat.png hello world"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn files_over_the_cap_are_refused() {
        let root = scratch("cap");
        let up = Uploads::new(root.clone());
        let chunk = vec![7u8; MAX_CHUNK];
        let mut offset = 0;
        let mut last = Ok(0);
        while offset <= MAX_UPLOAD {
            last = up.add("cap-upload-1", "phone1", "big.bin", offset, &chunk);
            if last.is_err() {
                break;
            }
            offset += MAX_CHUNK as u64;
        }
        assert_eq!(last.unwrap_err().0, code::TOO_BIG);
        assert!(up.finish("cap-upload-1").is_err(), "dropped");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_host_checks_before_the_app_sees_it() {
        let called = Mutex::new(0);
        let call = |_: &Value| {
            *called.lock().unwrap() += 1;
            Ok(br#"{"ok":true}"#.to_vec())
        };
        let r: Value = serde_json::from_slice(&host_answer(
            br#"{"type":"open_url","device":"n","url":"file:///"}"#,
            call,
        ))
        .unwrap();
        assert_eq!(r["error"], code::BAD_REQUEST);
        assert_eq!(*called.lock().unwrap(), 0);
        let r = host_answer(br#"{"type":"hello"}"#, call);
        assert_eq!(r, br#"{"ok":true}"#);
        let r: Value = serde_json::from_slice(&host_answer(br#"{"type":"hello"}"#, |_| {
            Err("refused".into())
        }))
        .unwrap();
        assert_eq!(r["error"], code::APP_NOT_RUNNING);
    }

    #[test]
    fn the_app_side_wants_the_token_and_answers_in_frames() {
        let fake = Fake {
            log: Mutex::new(Vec::new()),
        };
        let up = Uploads::new(scratch("ipc"));
        let env = |token: &str| {
            framed(
                &serde_json::to_vec(&json!({"v": 1, "token": token, "req": {"type": "hello"}}))
                    .unwrap(),
            )
        };
        struct Duplex(Cursor<Vec<u8>>, Vec<u8>);
        impl Read for Duplex {
            fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(b)
            }
        }
        impl Write for Duplex {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.1.write(b)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut s = Duplex(Cursor::new(env("secret")), Vec::new());
        serve_one(&mut s, "secret", &fake, &up);
        let reply = read_frame(&mut Cursor::new(s.1)).unwrap().unwrap();
        let v: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["app"], "my-domain");
        let mut s = Duplex(Cursor::new(env("wrong")), Vec::new());
        serve_one(&mut s, "secret", &fake, &up);
        assert!(s.1.is_empty(), "no answer without the token");
    }
}
