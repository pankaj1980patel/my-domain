// Webhooks: outside tools on this computer (Home Assistant, Shortcuts, a
// script, a Stream Deck) ask my-domain to do what its own buttons do. Set up
// in Settings (Webhooks).
//
// What keeps it safe:
//  - off until switched on; then a small HTTP server listens on 127.0.0.1
//    only (port `DEFAULT_PORT`, changeable), never on the network;
//  - every request needs `Authorization: Bearer <token>`: 32 random bytes,
//    URL-safe base64, made when it's first switched on, kept with the app's
//    other secrets (OS secure store; the debug builds' file), compared in
//    constant time. A wrong or missing token gets a bare 401, and after
//    `MAX_FAILURES` of them in a minute every request gets 429 until the
//    minute is up, so the token can't be guessed;
//  - a web page in a browser can't use it: any `Origin` header is refused
//    (browsers add one to cross-site requests; `curl` and friends don't),
//    as is a `Host` other than `127.0.0.1:<port>` / `localhost:<port>`
//    (DNS rebinding), and POSTs must be JSON (no "simple" form posts);
//  - POST bodies of at most `MAX_BODY`; slow or oversized requests are cut
//    off;
//  - it does only what the app's buttons do: a device action, the same way
//    schedules and automations run one (`schedule::run_action`, so `run`
//    still needs Run command switched on and a command there), text to a
//    device's clipboard or as a notice, or an automation's action. No
//    shell commands of its own;
//  - a notification each time it runs something (at most
//    `NOTICES_PER_MINUTE`, so a loop doesn't flood the screen).
//
// The API (JSON in and out):
//   GET  /v1/devices     → [{name, node, connected, actions}]
//   POST /v1/action      {"device", "action", "args"?} → {"ok", "via"}
//   POST /v1/text        {"device", "text", "as"?: "clipboard"|"notice"}
//   POST /v1/automation  {"automation": id or label}
// A device is its node id, or its name (any case); "this computer" is this
// one. Failures are {"ok": false, "error": "..."} with a 400.
//
// Plain std threads and `httparse` (already in the tree, via tungstenite):
// a few requests a minute don't need an async server.

use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine as _;
use mdcore::kv::FileKv;
use mdcore::CoreError;
use serde::de::DeserializeOwned;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

/// Where it listens unless changed.
pub const DEFAULT_PORT: u16 = 47821;
/// The secret store's key for the token.
const TOKEN_KEY: &str = "webhook-token";
const ENABLED_KEY: &str = "enabled";
const PORT_KEY: &str = "port";
/// The biggest request body, in bytes.
const MAX_BODY: usize = 16 * 1024;
/// The biggest request line and headers, in bytes.
const MAX_HEAD: usize = 8 * 1024;
/// The most headers in a request.
const MAX_HEADERS: usize = 32;
/// Wrong or missing tokens allowed per `FAIL_WINDOW`.
const MAX_FAILURES: usize = 10;
const FAIL_WINDOW: Duration = Duration::from_secs(60);
/// Notifications per minute, at most.
const NOTICES_PER_MINUTE: usize = 6;
/// Requests handled at once; more are dropped.
const MAX_CONNECTIONS: usize = 8;
/// How long one request may take to arrive.
const READ_DEADLINE: Duration = Duration::from_secs(10);
/// How often the listener looks for a stop.
const POLL: Duration = Duration::from_millis(100);
/// What a device is called when it's this computer.
const THIS_COMPUTER: &str = "this computer";

// ---------------------------------------------------------------------------
// Checks (pure, tested)
// ---------------------------------------------------------------------------

/// A new token: 32 random bytes, URL-safe base64 (43 characters).
pub fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Equal, in time that depends only on the lengths.
fn same(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    std::hint::black_box(diff) == 0
}

/// Does the `Authorization` header carry `token`? An empty token (none
/// made yet) never matches.
pub fn token_ok(token: &str, authorization: Option<&str>) -> bool {
    let Some(given) = authorization.and_then(|h| {
        let (scheme, rest) = h.trim().split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| rest.trim())
    }) else {
        return false;
    };
    !token.is_empty() & same(token.as_bytes(), given.as_bytes())
}

/// The `Host` header names this server: `127.0.0.1:<port>` or
/// `localhost:<port>`. Anything else may be a rebound DNS name.
pub fn host_ok(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// No `Origin` header: not a web page. Browsers send one with every
/// cross-site request, and "null" from sandboxed frames and local files, so
/// any value at all is refused.
pub fn origin_ok(origin: Option<&str>) -> bool {
    origin.is_none()
}

/// The body as `T`: the content type must be JSON.
pub fn parse_body<T: DeserializeOwned>(content_type: Option<&str>, body: &[u8]) -> Result<T, Fail> {
    let json = content_type.is_some_and(|c| {
        c.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json {
        return Err(Fail::Status(415));
    }
    serde_json::from_slice(body)
        .map_err(|e| Fail::Bad(format!("that isn\u{2019}t the JSON expected: {e}")))
}

/// A device tools can use, as `/v1/devices` lists it.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    pub node: String,
    pub connected: bool,
    /// What `/v1/action` can ask it to do.
    pub actions: Vec<String>,
}

/// The device `query` names, in `devices`: "this computer" or `me` is this
/// one; else a node id, else a name (any case, as listed or as known to
/// the core, e.g. with ".local").
pub fn find_device(devices: &[DeviceInfo], me: &str, query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("say which device".into());
    }
    if query.eq_ignore_ascii_case(THIS_COMPUTER) || query == me {
        return Ok(me.to_string());
    }
    if let Some(d) = devices.iter().find(|d| d.node == query) {
        return Ok(d.node.clone());
    }
    let named: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| {
            d.name.eq_ignore_ascii_case(query)
                || crate::notify::nice_name(&d.name).eq_ignore_ascii_case(query)
        })
        .collect();
    match named.as_slice() {
        [d] => Ok(d.node.clone()),
        [] => Err(format!(
            "there\u{2019}s no device called \u{201c}{query}\u{201d}"
        )),
        _ => Err(format!(
            "more than one device is called \u{201c}{query}\u{201d}; use its node id"
        )),
    }
}

/// At most `max` hits in any `span`.
pub struct Window {
    max: usize,
    span: Duration,
    hits: VecDeque<Instant>,
}

impl Window {
    pub fn new(max: usize, span: Duration) -> Self {
        Window {
            max,
            span,
            hits: VecDeque::new(),
        }
    }

    /// Already `max` hits within the last `span`?
    pub fn full(&mut self, now: Instant) -> bool {
        while self
            .hits
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.span)
        {
            self.hits.pop_front();
        }
        self.hits.len() >= self.max
    }

    /// Count one, unless full. Was it counted?
    pub fn hit(&mut self, now: Instant) -> bool {
        if self.full(now) {
            return false;
        }
        self.hits.push_back(now);
        true
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// A request, with header names in lower case.
#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request ends without doing anything.
#[derive(Debug, PartialEq)]
pub enum Fail {
    /// Just this status, no body.
    Status(u16),
    /// A 400 with `{"ok": false, "error": …}`.
    Bad(String),
}

/// The request line and headers in `buf`, if they're all there; the
/// request and where the body starts.
fn parse_head(buf: &[u8]) -> Result<Option<(Request, usize)>, Fail> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    let n = match req.parse(buf) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(httparse::Error::TooManyHeaders) => return Err(Fail::Status(431)),
        Err(_) => return Err(Fail::Status(400)),
    };
    let headers = req
        .headers
        .iter()
        .map(|h| {
            let value = String::from_utf8_lossy(h.value).into_owned();
            (h.name.to_ascii_lowercase(), value)
        })
        .collect();
    let request = Request {
        method: req.method.unwrap_or_default().to_string(),
        path: req.path.unwrap_or_default().to_string(),
        headers,
        body: Vec::new(),
    };
    Ok(Some((request, n)))
}

/// Read one request: headers up to `MAX_HEAD`, a body up to `MAX_BODY`
/// (by `Content-Length`; chunked bodies aren't taken), before `deadline`.
pub fn read_request(stream: &mut impl Read, deadline: Instant) -> Result<Request, Fail> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let (mut req, start) = loop {
        if let Some(found) = parse_head(&buf)? {
            break found;
        }
        if buf.len() > MAX_HEAD {
            return Err(Fail::Status(431));
        }
        if Instant::now() > deadline {
            return Err(Fail::Status(408));
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Fail::Status(400)),
            Ok(k) => buf.extend_from_slice(&chunk[..k]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err(Fail::Status(408)),
        }
    };
    if req.header("transfer-encoding").is_some() {
        return Err(Fail::Status(411));
    }
    let len = match req.header("content-length") {
        None => 0,
        Some(v) => v.trim().parse::<usize>().map_err(|_| Fail::Status(400))?,
    };
    if len > MAX_BODY {
        return Err(Fail::Status(413));
    }
    let mut body = buf.split_off(start);
    while body.len() < len {
        if Instant::now() > deadline {
            return Err(Fail::Status(408));
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(Fail::Status(400)),
            Ok(k) => body.extend_from_slice(&chunk[..k]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err(Fail::Status(408)),
        }
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

/// What the server answers.
#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    /// JSON, or empty.
    pub body: String,
}

impl Response {
    fn json(status: u16, value: serde_json::Value) -> Self {
        Response {
            status,
            body: value.to_string(),
        }
    }

    fn ok(via: &str) -> Self {
        Self::json(200, serde_json::json!({ "ok": true, "via": via }))
    }

    fn from_fail(f: Fail) -> Self {
        match f {
            Fail::Status(status) => Response {
                status,
                body: String::new(),
            },
            Fail::Bad(error) => Self::json(400, serde_json::json!({ "ok": false, "error": error })),
        }
    }

    fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            408 => "Request Timeout",
            411 => "Length Required",
            413 => "Payload Too Large",
            415 => "Unsupported Media Type",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            _ => "Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n",
            self.status,
            self.body.len()
        );
        if !self.body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        if self.status == 401 {
            head.push_str("WWW-Authenticate: Bearer\r\n");
        }
        head.push_str("\r\n");
        out.write_all(head.as_bytes())?;
        out.write_all(self.body.as_bytes())?;
        out.flush()
    }
}

#[derive(serde::Deserialize)]
struct ActionBody {
    device: String,
    action: String,
    #[serde(default)]
    args: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct TextBody {
    device: String,
    text: String,
    #[serde(default, rename = "as")]
    mode: Option<String>,
}

#[derive(serde::Deserialize)]
struct AutomationBody {
    automation: String,
}

/// What the server acts on: the app, or a stand-in in tests.
pub trait Backend: Send + Sync + 'static {
    /// This computer's node id.
    fn me(&self) -> String;
    /// This computer first, then your other devices.
    fn devices(&self) -> Vec<DeviceInfo>;
    /// Run `action` on `node`; the transport, "queued" or "here".
    fn action(&self, node: &str, action: &str, args: &str) -> Result<String, String>;
    /// Text to `node`'s clipboard ("clipboard") or as a notice ("notify").
    fn text(&self, node: &str, text: &str, mode: &str) -> Result<String, String>;
    /// Run an automation's action; what it does, and how it went.
    fn automation(&self, which: &str) -> Result<(String, String), String>;
    /// Something ran: say so (or why it couldn't).
    fn ran(&self, title: &str, body: &str);
}

/// What one listening server shares with its requests.
struct Ctx {
    backend: Arc<dyn Backend>,
    token: Arc<Mutex<String>>,
    port: u16,
    failures: Mutex<Window>,
}

/// Answer one request: the checks, then the route.
fn handle(ctx: &Ctx, req: &Request) -> Response {
    match respond(ctx, req) {
        Ok(r) => r,
        Err(f) => Response::from_fail(f),
    }
}

fn respond(ctx: &Ctx, req: &Request) -> Result<Response, Fail> {
    if !host_ok(req.header("host"), ctx.port) || !origin_ok(req.header("origin")) {
        return Err(Fail::Status(403));
    }
    let now = Instant::now();
    let mut failures = ctx.failures.lock().map_err(|_| Fail::Status(429))?;
    if failures.full(now) {
        return Err(Fail::Status(429));
    }
    let token = ctx.token.lock().map(|t| t.clone()).unwrap_or_default();
    if !token_ok(&token, req.header("authorization")) {
        failures.hit(now);
        return Err(Fail::Status(401));
    }
    drop(failures);
    let route = req.path.split('?').next().unwrap_or_default();
    match (req.method.as_str(), route) {
        ("GET", "/v1/devices") => {
            let list = serde_json::to_value(ctx.backend.devices()).unwrap_or_default();
            Ok(Response::json(200, list))
        }
        ("POST", "/v1/action") => action(ctx, req),
        ("POST", "/v1/text") => text(ctx, req),
        ("POST", "/v1/automation") => automation(ctx, req),
        (_, "/v1/devices" | "/v1/action" | "/v1/text" | "/v1/automation") => Err(Fail::Status(405)),
        _ => Err(Fail::Status(404)),
    }
}

/// A device's name for a notification.
fn name_of(devices: &[DeviceInfo], me: &str, node: &str) -> String {
    if node == me {
        return THIS_COMPUTER.into();
    }
    devices
        .iter()
        .find(|d| d.node == node)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| node.to_string())
}

fn action(ctx: &Ctx, req: &Request) -> Result<Response, Fail> {
    let body: ActionBody = parse_body(req.header("content-type"), &req.body)?;
    let (me, devices) = (ctx.backend.me(), ctx.backend.devices());
    let node = find_device(&devices, &me, &body.device).map_err(Fail::Bad)?;
    let args = match &body.args {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(v) => v.to_string(),
    };
    let device = name_of(&devices, &me, &node);
    let result = ctx.backend.action(&node, &body.action, &args);
    tell(ctx, &format!("{} on {device}", body.action), &result);
    result.map(|via| Response::ok(&via)).map_err(Fail::Bad)
}

fn text(ctx: &Ctx, req: &Request) -> Result<Response, Fail> {
    let body: TextBody = parse_body(req.header("content-type"), &req.body)?;
    let mode = match body.mode.as_deref().unwrap_or("clipboard") {
        "clipboard" => "clipboard",
        "notice" | "notify" => "notify",
        _ => {
            return Err(Fail::Bad(
                "\u{201c}as\u{201d} is clipboard or notice".into(),
            ))
        }
    };
    let (me, devices) = (ctx.backend.me(), ctx.backend.devices());
    let node = find_device(&devices, &me, &body.device).map_err(Fail::Bad)?;
    let device = name_of(&devices, &me, &node);
    let result = ctx.backend.text(&node, &body.text, mode);
    let what = if mode == "clipboard" {
        format!("text to {device}\u{2019}s clipboard")
    } else {
        format!("a notice to {device}")
    };
    tell(ctx, &what, &result);
    result.map(|via| Response::ok(&via)).map_err(Fail::Bad)
}

fn automation(ctx: &Ctx, req: &Request) -> Result<Response, Fail> {
    let body: AutomationBody = parse_body(req.header("content-type"), &req.body)?;
    match ctx.backend.automation(&body.automation) {
        Ok((what, via)) => {
            tell(ctx, &what, &Ok(via.clone()));
            Ok(Response::ok(&via))
        }
        Err(e) => {
            tell(ctx, &body.automation, &Err(e.clone()));
            Err(Fail::Bad(e))
        }
    }
}

/// The notification for something a webhook ran.
fn tell(ctx: &Ctx, what: &str, result: &Result<String, String>) {
    match result {
        Ok(via) if via == "queued" => ctx.backend.ran(
            &format!("Webhook: {what}"),
            "That device isn\u{2019}t reachable right now; it\u{2019}ll happen once it is.",
        ),
        Ok(_) => ctx.backend.ran(
            &format!("Webhook: {what}"),
            "Done, asked by a tool on this computer.",
        ),
        Err(e) => ctx
            .backend
            .ran(&format!("Webhook couldn\u{2019}t do {what}"), e),
    }
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

/// A listening server; stops when dropped.
pub struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Listen on 127.0.0.1:`port` (0: any free port). Fails if it's taken.
    pub fn start(
        port: u16,
        backend: Arc<dyn Backend>,
        token: Arc<Mutex<String>>,
    ) -> std::io::Result<Server> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let ctx = Arc::new(Ctx {
            backend,
            token,
            port,
            failures: Mutex::new(Window::new(MAX_FAILURES, FAIL_WINDOW)),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::Builder::new()
            .name("webhooks".into())
            .spawn(move || serve(listener, ctx, stopping))?;
        Ok(Server {
            port,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Accept until told to stop (the listener closes with this thread, which
/// frees the port).
fn serve(listener: TcpListener, ctx: Arc<Ctx>, stop: Arc<AtomicBool>) {
    let busy = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if busy.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue;
                }
                busy.fetch_add(1, Ordering::Relaxed);
                let (ctx, held) = (ctx.clone(), busy.clone());
                let spawned = std::thread::Builder::new()
                    .name("webhook".into())
                    .spawn(move || {
                        connection(stream, &ctx);
                        held.fetch_sub(1, Ordering::Relaxed);
                    });
                if spawned.is_err() {
                    busy.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(e) => {
                eprintln!("[webhooks] accept: {e}");
                std::thread::sleep(POLL);
            }
        }
    }
}

/// One connection: one request, one answer.
fn connection(mut stream: TcpStream, ctx: &Ctx) {
    // The listener doesn't block; on some systems its connections inherit
    // that.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let response = match read_request(&mut stream, Instant::now() + READ_DEADLINE) {
        Ok(req) => handle(ctx, &req),
        Err(f) => Response::from_fail(f),
    };
    let _ = response.write(&mut stream);
}

// ---------------------------------------------------------------------------
// The app's side
// ---------------------------------------------------------------------------

/// The server's view of the app.
struct AppBackend {
    app: AppHandle,
    notices: Mutex<Window>,
}

impl AppBackend {
    fn engine(&self) -> Result<State<'_, Eng>, String> {
        self.app
            .try_state::<Eng>()
            .ok_or_else(|| "not started yet".to_string())
    }
}

impl Backend for AppBackend {
    fn me(&self) -> String {
        self.engine()
            .map(|e| e.identity().node_id)
            .unwrap_or_default()
    }

    fn devices(&self) -> Vec<DeviceInfo> {
        let Ok(engine) = self.engine() else {
            return Vec::new();
        };
        let me = engine.identity().node_id;
        let mut out = vec![DeviceInfo {
            name: "This computer".into(),
            node: me.clone(),
            connected: true,
            actions: crate::device::local_actions(&self.app),
        }];
        let live = engine.connected_peers();
        let known = self
            .app
            .try_state::<Webhooks>()
            .and_then(|w| w.actions.lock().ok().map(|a| a.clone()))
            .unwrap_or_default();
        for p in engine.get_peers() {
            if out.iter().any(|d| d.node == p.node_id) {
                continue;
            }
            let mut actions = vec!["ring".to_string()];
            actions.extend(known.get(&p.node_id).cloned().unwrap_or_default());
            // A computer that may be woken: this one sends the wake-up.
            if engine.can_wake(&p.node_id) && !actions.iter().any(|a| a == "wake") {
                actions.push("wake".to_string());
            }
            out.push(DeviceInfo {
                name: crate::notify::nice_name(&p.name),
                connected: live.contains(&p.node_id),
                node: p.node_id,
                actions,
            });
        }
        out
    }

    fn action(&self, node: &str, action: &str, args: &str) -> Result<String, String> {
        crate::schedule::check_action(node, action, args).map_err(|e| e.to_string())?;
        crate::schedule::run_action(&self.app, node, action, args).map_err(|e| e.to_string())
    }

    fn text(&self, node: &str, text: &str, mode: &str) -> Result<String, String> {
        let engine = self.engine()?;
        if node != engine.identity().node_id {
            let how = engine
                .send_text(node, text, mode)
                .map_err(|e| e.to_string())?;
            if mode == "clipboard" {
                crate::clip_history::on_sent(&self.app, node, text);
            }
            return Ok(how);
        }
        if text.trim().is_empty() {
            return Err("nothing to send".into());
        }
        if mode == "clipboard" {
            engine.copy_to_clipboard(text).map_err(|e| e.to_string())?;
        } else {
            crate::notify::post(&self.app, "From a webhook", text);
        }
        Ok("here".into())
    }

    fn automation(&self, which: &str) -> Result<(String, String), String> {
        crate::automations::run_now(&self.app, which)
    }

    fn ran(&self, title: &str, body: &str) {
        let counted = self.notices.lock().is_ok_and(|mut n| n.hit(Instant::now()));
        if counted {
            crate::notify::post(&self.app, title, body);
        }
    }
}

/// What Settings shows.
#[derive(serde::Serialize, specta::Type, Clone, Debug)]
pub struct WebhookSettings {
    pub enabled: bool,
    pub port: u16,
    /// The bearer token ("" until it's first switched on).
    pub token: String,
    /// Listening now.
    pub running: bool,
    /// Why it isn't listening, e.g. the port is taken.
    pub error: Option<String>,
}

struct Inner {
    enabled: bool,
    port: u16,
    server: Option<Server>,
    error: Option<String>,
}

/// The webhook settings (`<app config dir>/webhooks.json`), the token and
/// the server.
pub struct Webhooks {
    kv: Option<FileKv>,
    inner: Mutex<Inner>,
    /// Shared with the server, so a new token counts at once.
    token: Arc<Mutex<String>>,
    /// node id -> the actions its latest status listed.
    actions: Mutex<HashMap<String, Vec<String>>>,
}

impl Webhooks {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("webhooks.json")));
        let get = |key: &str| kv.as_ref().and_then(|kv| kv.get(key));
        let enabled = get(ENABLED_KEY).as_deref() == Some("on");
        let port = get(PORT_KEY)
            .and_then(|p| p.parse().ok())
            .filter(|p| port_ok(*p))
            .unwrap_or(DEFAULT_PORT);
        Webhooks {
            kv,
            inner: Mutex::new(Inner {
                enabled,
                port,
                server: None,
                error: None,
            }),
            token: Arc::new(Mutex::new(String::new())),
            actions: Mutex::new(HashMap::new()),
        }
    }

    fn save(&self, key: &str, value: &str) {
        if let Some(kv) = &self.kv {
            if let Err(e) = kv.set(key, value) {
                eprintln!("[webhooks] couldn't save the setting: {e}");
            }
        }
    }

    fn token(&self) -> String {
        self.token.lock().map(|t| t.clone()).unwrap_or_default()
    }

    fn set_token(&self, app: &AppHandle, token: String) {
        crate::set_app_secret(app, TOKEN_KEY, Some(&token));
        if let Ok(mut t) = self.token.lock() {
            *t = token;
        }
    }

    /// The saved token, or a new one if there's none.
    fn ensure_token(&self, app: &AppHandle) -> Result<(), String> {
        if !self.token().is_empty() {
            return Ok(());
        }
        match crate::app_secret(app, TOKEN_KEY).filter(|t| !t.is_empty()) {
            Some(saved) => {
                if let Ok(mut t) = self.token.lock() {
                    *t = saved;
                }
            }
            None => self.set_token(app, new_token()?),
        }
        Ok(())
    }

    fn settings(&self) -> WebhookSettings {
        let token = self.token();
        let Ok(i) = self.inner.lock() else {
            return WebhookSettings {
                enabled: false,
                port: DEFAULT_PORT,
                token,
                running: false,
                error: None,
            };
        };
        WebhookSettings {
            enabled: i.enabled,
            port: i.port,
            token,
            running: i.server.is_some(),
            error: i.error.clone(),
        }
    }

    /// Listen (again) if switched on, else stop.
    fn apply(&self, app: &AppHandle, i: &mut Inner) {
        // The old server lets go of its port first.
        i.server = None;
        i.error = None;
        if !i.enabled {
            return;
        }
        if let Err(e) = self.ensure_token(app) {
            i.error = Some(e);
            return;
        }
        let backend = Arc::new(AppBackend {
            app: app.clone(),
            notices: Mutex::new(Window::new(NOTICES_PER_MINUTE, Duration::from_secs(60))),
        });
        match Server::start(i.port, backend, self.token.clone()) {
            Ok(server) => {
                eprintln!("[webhooks] listening on 127.0.0.1:{}", server.port());
                i.server = Some(server);
            }
            Err(e) if e.kind() == ErrorKind::AddrInUse => {
                i.error = Some(format!(
                    "Port {} is already in use by another app. Choose another port.",
                    i.port
                ));
            }
            Err(e) => i.error = Some(format!("Couldn\u{2019}t listen on port {}: {e}", i.port)),
        }
        if let Some(e) = &i.error {
            eprintln!("[webhooks] {e}");
        }
    }

    fn set_enabled(&self, app: &AppHandle, on: bool) {
        if let Ok(mut i) = self.inner.lock() {
            i.enabled = on;
            self.save(ENABLED_KEY, if on { "on" } else { "off" });
            self.apply(app, &mut i);
        }
    }

    fn set_port(&self, app: &AppHandle, port: u16) -> Result<(), CoreError> {
        if !port_ok(port) {
            return Err(CoreError::InvalidInput(
                "the port is a number from 1024 to 65535".into(),
            ));
        }
        let mut i = self
            .inner
            .lock()
            .map_err(|_| CoreError::Other("webhooks unavailable".into()))?;
        i.port = port;
        self.save(PORT_KEY, &port.to_string());
        self.apply(app, &mut i);
        Ok(())
    }
}

/// Ports below 1024 need special rights.
fn port_ok(port: u16) -> bool {
    port >= 1024
}

/// A device's status arrived: remember the actions it lists, for
/// `/v1/devices`.
pub fn on_status(app: &AppHandle, from: &str, status: &serde_json::Value) {
    let Some(state) = app.try_state::<Webhooks>() else {
        return;
    };
    let Some(list) = status.get("actions").and_then(|a| a.as_array()) else {
        return;
    };
    let actions = list
        .iter()
        .filter_map(|a| a.as_str())
        .filter(|a| *a != "ring")
        .map(String::from)
        .collect();
    if let Ok(mut known) = state.actions.lock() {
        known.insert(from.to_string(), actions);
    };
}

/// Start listening at launch if it was left on.
pub fn spawn(app: AppHandle) {
    let Some(state) = app.try_state::<Webhooks>() else {
        return;
    };
    if let Ok(mut i) = state.inner.lock() {
        if i.enabled {
            state.apply(&app, &mut i);
        }
    };
}

/// The webhook settings: on or off, the port, the token, and whether it's
/// listening.
#[tauri::command]
#[specta::specta]
pub fn webhook_settings(app: AppHandle, state: State<Webhooks>) -> WebhookSettings {
    // Show the token once it's on even if it hasn't been read yet.
    if state.settings().enabled {
        let _ = state.ensure_token(&app);
    }
    state.settings()
}

/// Turn webhooks on (making a token the first time) or off.
#[tauri::command]
#[specta::specta]
pub fn set_webhooks_enabled(on: bool, app: AppHandle, state: State<Webhooks>) -> WebhookSettings {
    state.set_enabled(&app, on);
    state.settings()
}

/// Listen on another port (1024–65535).
#[tauri::command]
#[specta::specta]
pub fn set_webhook_port(
    port: u16,
    app: AppHandle,
    state: State<Webhooks>,
) -> Result<WebhookSettings, CoreError> {
    state.set_port(&app, port)?;
    Ok(state.settings())
}

/// Replace the token: tools using the old one stop working.
#[tauri::command]
#[specta::specta]
pub fn new_webhook_token(
    app: AppHandle,
    state: State<Webhooks>,
) -> Result<WebhookSettings, CoreError> {
    let token = new_token().map_err(CoreError::Other)?;
    state.set_token(&app, token);
    Ok(state.settings())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_and_url_safe() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 43);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, new_token().unwrap());
    }

    #[test]
    fn checks_the_token() {
        let t = "abc123";
        assert!(token_ok(t, Some("Bearer abc123")));
        assert!(token_ok(t, Some("bearer  abc123 ")));
        assert!(!token_ok(t, Some("Bearer abc124")));
        assert!(!token_ok(t, Some("Bearer abc1234")));
        assert!(!token_ok(t, Some("Bearer abc12")));
        assert!(!token_ok(t, Some("Basic abc123")));
        assert!(!token_ok(t, Some("abc123")));
        assert!(!token_ok(t, None));
        // No token made yet: nothing gets in.
        assert!(!token_ok("", Some("Bearer ")));
        assert!(!token_ok("", Some("Bearer x")));
    }

    #[test]
    fn compares_whole_strings() {
        assert!(same(b"", b""));
        assert!(same(b"same", b"same"));
        assert!(!same(b"same", b"sane"));
        assert!(!same(b"same", b"same\0"));
        assert!(!same(b"", b"\0"));
    }

    #[test]
    fn checks_host_and_origin() {
        assert!(host_ok(Some("127.0.0.1:47821"), 47821));
        assert!(host_ok(Some("LOCALHOST:47821"), 47821));
        assert!(!host_ok(Some("127.0.0.1:47822"), 47821));
        assert!(!host_ok(Some("127.0.0.1"), 47821));
        assert!(!host_ok(Some("evil.example:47821"), 47821));
        assert!(!host_ok(Some("192.168.0.2:47821"), 47821));
        assert!(!host_ok(None, 47821));
        assert!(origin_ok(None));
        assert!(!origin_ok(Some("https://evil.example")));
        assert!(!origin_ok(Some("null")));
        assert!(!origin_ok(Some("")));
    }

    #[test]
    fn parses_json_bodies_only() {
        let json = Some("application/json; charset=utf-8");
        let b: ActionBody = parse_body(json, br#"{"device":"Pixel","action":"ring"}"#).unwrap();
        assert_eq!((b.device.as_str(), b.action.as_str()), ("Pixel", "ring"));
        assert!(b.args.is_none());
        let b: TextBody = parse_body(json, br#"{"device":"x","text":"hi","as":"notice"}"#).unwrap();
        assert_eq!(b.mode.as_deref(), Some("notice"));
        // A form post, or no type at all.
        assert_eq!(
            parse_body::<ActionBody>(Some("text/plain"), b"{}").err(),
            Some(Fail::Status(415))
        );
        assert_eq!(
            parse_body::<ActionBody>(None, b"{}").err(),
            Some(Fail::Status(415))
        );
        assert!(matches!(
            parse_body::<ActionBody>(json, br#"{"device":"x"}"#),
            Err(Fail::Bad(_))
        ));
        assert!(matches!(
            parse_body::<ActionBody>(json, b"not json"),
            Err(Fail::Bad(_))
        ));
    }

    fn dev(name: &str, node: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            node: node.into(),
            connected: true,
            actions: vec!["ring".into()],
        }
    }

    #[test]
    fn finds_devices() {
        let list = [
            dev("This computer", "me"),
            dev("Pixel 8", "p8"),
            dev("Pankajs-MacBook-Pro.local", "mac"),
            dev("Tablet", "t1"),
            dev("tablet", "t2"),
        ];
        assert_eq!(find_device(&list, "me", "this computer").unwrap(), "me");
        assert_eq!(find_device(&list, "me", "This Computer").unwrap(), "me");
        assert_eq!(find_device(&list, "me", "me").unwrap(), "me");
        assert_eq!(find_device(&list, "me", "p8").unwrap(), "p8");
        assert_eq!(find_device(&list, "me", "pixel 8").unwrap(), "p8");
        assert_eq!(find_device(&list, "me", " PIXEL 8 ").unwrap(), "p8");
        // As the core knows it, or tidied as the app shows it.
        assert_eq!(
            find_device(&list, "me", "Pankajs-MacBook-Pro.local").unwrap(),
            "mac"
        );
        assert_eq!(
            find_device(&list, "me", "pankajs macbook pro").unwrap(),
            "mac"
        );
        // Two of that name: the node id decides.
        assert!(find_device(&list, "me", "tablet")
            .unwrap_err()
            .contains("more than one"));
        assert_eq!(find_device(&list, "me", "t2").unwrap(), "t2");
        assert!(find_device(&list, "me", "toaster")
            .unwrap_err()
            .contains("no device"));
        assert!(find_device(&list, "me", " ").is_err());
    }

    #[test]
    fn window_counts_a_minute() {
        let t0 = Instant::now();
        let mut w = Window::new(2, Duration::from_secs(60));
        assert!(w.hit(t0));
        assert!(w.hit(t0 + Duration::from_secs(1)));
        assert!(w.full(t0 + Duration::from_secs(2)));
        assert!(!w.hit(t0 + Duration::from_secs(59)));
        // The first one has aged out.
        assert!(w.hit(t0 + Duration::from_secs(60)));
        assert!(w.full(t0 + Duration::from_secs(60)));
        assert!(!w.full(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn reads_requests() {
        let raw = b"POST /v1/text HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Type: application/json\r\nContent-Length: 5\r\n\r\nhello and more";
        let req = read_request(&mut &raw[..], Instant::now() + READ_DEADLINE).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/v1/text");
        assert_eq!(req.header("host"), Some("127.0.0.1:1"));
        assert_eq!(req.body, b"hello");
        // Too big, chunked, cut short, garbage.
        let big = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        let deadline = Instant::now() + READ_DEADLINE;
        assert_eq!(
            read_request(&mut big.as_bytes(), deadline).err(),
            Some(Fail::Status(413))
        );
        let chunked = b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(
            read_request(&mut &chunked[..], deadline).err(),
            Some(Fail::Status(411))
        );
        let short = b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc";
        assert_eq!(
            read_request(&mut &short[..], deadline).err(),
            Some(Fail::Status(400))
        );
        assert_eq!(
            read_request(&mut &b"\x00\x01 nonsense\r\n\r\n"[..], deadline).err(),
            Some(Fail::Status(400))
        );
        let endless = vec![b'a'; MAX_HEAD + 10];
        let mut head = b"GET / HTTP/1.1\r\nX: ".to_vec();
        head.extend(endless);
        assert_eq!(
            read_request(&mut &head[..], deadline).err(),
            Some(Fail::Status(431))
        );
    }

    /// A stand-in for the app: two devices, and a log of what it was asked.
    struct Stub {
        log: Mutex<Vec<String>>,
    }

    impl Backend for Stub {
        fn me(&self) -> String {
            "me".into()
        }
        fn devices(&self) -> Vec<DeviceInfo> {
            vec![dev("This computer", "me"), dev("Pixel 8", "p8")]
        }
        fn action(&self, node: &str, action: &str, args: &str) -> Result<String, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("{node} {action} {args}"));
            Ok(if node == "me" { "here" } else { "quic" }.into())
        }
        fn text(&self, node: &str, text: &str, mode: &str) -> Result<String, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("{node} {mode} {text}"));
            Ok("queued".into())
        }
        fn automation(&self, which: &str) -> Result<(String, String), String> {
            Err(format!("no automation {which}"))
        }
        fn ran(&self, title: &str, _body: &str) {
            self.log.lock().unwrap().push(format!("told: {title}"));
        }
    }

    /// Send `raw` to the server; the status and body it answers.
    fn call(port: u16, raw: &str) -> (u16, String) {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let status = out[9..12].parse().unwrap();
        let body = out.split_once("\r\n\r\n").unwrap().1.to_string();
        (status, body)
    }

    fn get(port: u16, extra: &str) -> String {
        format!("GET /v1/devices HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}\r\n")
    }

    fn post(port: u16, path: &str, token: &str, body: &str) -> String {
        format!(
            "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn serves_on_loopback_with_the_token() {
        let stub = Arc::new(Stub {
            log: Mutex::new(Vec::new()),
        });
        let token = Arc::new(Mutex::new("s3cret-token".to_string()));
        let server = Server::start(0, stub.clone(), token.clone()).unwrap();
        let port = server.port();
        let auth = "Authorization: Bearer s3cret-token\r\n";

        // No token, a wrong one: 401 and nothing else.
        let (status, body) = call(port, &get(port, ""));
        assert_eq!((status, body.as_str()), (401, ""));
        let (status, _) = call(port, &get(port, "Authorization: Bearer nope\r\n"));
        assert_eq!(status, 401);
        // A web page (any Origin), or a rebound name: 403, token or not.
        let origin = format!("{auth}Origin: https://evil.example\r\n");
        assert_eq!(call(port, &get(port, &origin)).0, 403);
        let rebound =
            format!("GET /v1/devices HTTP/1.1\r\nHost: evil.example:{port}\r\n{auth}\r\n");
        assert_eq!(call(port, &rebound).0, 403);
        // With it: the devices.
        let (status, body) = call(port, &get(port, auth));
        assert_eq!(status, 200);
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[1]["name"], "Pixel 8");
        assert_eq!(list[1]["connected"], true);
        assert_eq!(list[1]["actions"][0], "ring");

        // An action, by name; text, to this computer.
        let (status, body) = call(
            port,
            &post(
                port,
                "/v1/action",
                "s3cret-token",
                r#"{"device":"pixel 8","action":"flashlight","args":{"on":true}}"#,
            ),
        );
        assert_eq!(
            (status, body.as_str()),
            (200, r#"{"ok":true,"via":"quic"}"#)
        );
        let (status, body) = call(
            port,
            &post(
                port,
                "/v1/text",
                "s3cret-token",
                r#"{"device":"this computer","text":"hi","as":"notice"}"#,
            ),
        );
        assert_eq!(
            (status, body.as_str()),
            (200, r#"{"ok":true,"via":"queued"}"#)
        );
        let (status, body) = call(
            port,
            &post(
                port,
                "/v1/action",
                "s3cret-token",
                r#"{"device":"toaster","action":"ring"}"#,
            ),
        );
        assert_eq!(status, 400);
        assert!(body.contains(r#""ok":false"#));
        assert_eq!(
            call(port, &post(port, "/v1/nothing", "s3cret-token", "{}")).0,
            404
        );
        {
            let log = stub.log.lock().unwrap();
            assert!(log.contains(&r#"p8 flashlight {"on":true}"#.to_string()));
            assert!(log.contains(&"me notify hi".to_string()));
            assert!(log
                .iter()
                .any(|l| l == "told: Webhook: flashlight on Pixel 8"));
        }

        // A new token: the old one stops working at once.
        *token.lock().unwrap() = "another".into();
        assert_eq!(call(port, &get(port, auth)).0, 401);

        // Guessing: after ten misses in a minute, even the right one waits.
        for _ in 0..MAX_FAILURES {
            call(port, &get(port, "Authorization: Bearer guess\r\n"));
        }
        let right = "Authorization: Bearer another\r\n";
        assert_eq!(call(port, &get(port, right)).0, 429);

        // Stopping frees the port.
        drop(server);
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
        let again = Server::start(port, stub, token).unwrap();
        assert_eq!(again.port(), port);
    }

    #[test]
    fn a_taken_port_is_an_error() {
        let held = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let stub = Arc::new(Stub {
            log: Mutex::new(Vec::new()),
        });
        let err = Server::start(port, stub, Arc::new(Mutex::new("t".into())))
            .err()
            .unwrap();
        assert_eq!(err.kind(), ErrorKind::AddrInUse);
    }
}
