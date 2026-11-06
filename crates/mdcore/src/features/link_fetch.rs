//! Downloading a web link for "Download on <device>"
//! (`features::remote_download`): which links may be fetched, what the
//! file is called, and the fetch itself (HTTP(S) only, with resume).
//!
//! # Which links
//!
//! `check_url`: only `http` and `https`, with a host, no user name or
//! password in it (public links only: nothing logs in, no cookies or
//! headers are sent). International names are taken as their ASCII
//! (punycode) form. A link to this device or the local network (a
//! private, loopback, link-local or shared address, `localhost`, a name
//! without a dot, `.local`, `.lan`, `.internal`, `.home.arpa`) is refused
//! unless LAN links are allowed; an unspecified, multicast or broadcast
//! address never goes.
//!
//! The check is made again on every redirect, and once more on the
//! addresses a name resolves to (`Guard`, the HTTP client's resolver), so
//! a public name that points at a private address is refused too.
//!
//! # The name
//!
//! From `Content-Disposition` (`filename*` before `filename`), else the
//! last part of the (final) link's path, else its host; an extension is
//! added from the content type when there's none; then made safe
//! (`safe_name`): no folders, no control or direction-changing characters,
//! no names Windows keeps for itself, at most `MAX_NAME` bytes.
//!
//! # Resume
//!
//! The file goes into a partial file. A dropped connection is retried (up
//! to `FetchOpts::retries` times, waiting longer each time) from where it
//! stopped, with `Range: bytes=<have>-` (and `If-Range` with the first
//! answer's `ETag` or `Last-Modified`, so a file that changed starts over
//! instead of being stitched together). A server that answers 200 instead
//! of 206 sends the whole file again: the partial file starts from zero.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use url::{Host, Url};

/// The longest link taken, in bytes.
pub const MAX_URL: usize = 8 * 1024;
/// Redirects followed at most.
pub const MAX_REDIRECTS: u32 = 10;
/// The longest file name, in bytes.
pub const MAX_NAME: usize = 150;
/// What a resolver error says when every address was a local one.
const BLOCKED: &str = "points to this device or the local network";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = "my-domain (download on your devices)";

/// Why a link can't be fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkProblem {
    /// Not a web (`http`/`https`) link with a host.
    NotWeb,
    /// It has a user name or password in it.
    Login,
    /// To this device or the local network, and LAN links aren't allowed.
    Local,
    /// To an address no download goes to (unspecified, multicast,
    /// broadcast).
    Never,
}

impl LinkProblem {
    /// For the user.
    pub fn reason(&self) -> &'static str {
        match self {
            LinkProblem::NotWeb => "only web links (http or https) can be downloaded",
            LinkProblem::Login => "links with a user name or password can't be downloaded",
            LinkProblem::Local => {
                "the link points to the local network (allow local network links in Settings)"
            }
            LinkProblem::Never => "the link points to an address that can't be downloaded",
        }
    }
}

/// An address on the internet: not this device, the local network, or a
/// range kept for something else. IPv4 inside IPv6 (mapped, NAT64, 6to4)
/// is judged by its IPv4.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = embedded_v4(&v6) {
                return is_public_v4(v4);
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] & 0xffc0) == 0xfec0 // site-local (old)
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x2001 && s[1] == 0) // Teredo (carries an IPv4)
                || (s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 1) // local NAT64
                || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)) // discard
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    !(v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // shared (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking
        || o[0] >= 240)
}

/// The IPv4 address an IPv6 one carries: mapped (`::ffff:a.b.c.d`),
/// NAT64 (`64:ff9b::a.b.c.d`) or 6to4 (`2002:aabb:ccdd::`).
fn embedded_v4(v6: &Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let s = v6.segments();
    let o = v6.octets();
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        return Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    if s[0] == 0x2002 {
        return Some(Ipv4Addr::new(o[2], o[3], o[4], o[5]));
    }
    // IPv4-compatible (`::a.b.c.d`, deprecated); `::` and `::1` are not.
    if s[..6].iter().all(|x| *x == 0) && !v6.is_unspecified() && !v6.is_loopback() {
        return Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    None
}

/// No download ever goes here, LAN links allowed or not.
fn never_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = embedded_v4(&v6) {
                return never_ip(IpAddr::V4(v4));
            }
            v6.is_unspecified() || v6.is_multicast()
        }
    }
}

/// Whether a download may go to `ip`.
pub fn ip_allowed(ip: IpAddr, allow_lan: bool) -> bool {
    !never_ip(ip) && (allow_lan || is_public_ip(ip))
}

/// A name that only means something on the local network.
fn local_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    !h.contains('.')
        || h == "localhost"
        || [
            ".localhost",
            ".local",
            ".lan",
            ".internal",
            ".home.arpa",
            ".intranet",
        ]
        .iter()
        .any(|s| h.ends_with(s))
}

/// `raw` as a link a download may go to (see the module's notes).
pub fn check_url(raw: &str, allow_lan: bool) -> Result<Url, LinkProblem> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL || raw.chars().any(char::is_control) {
        return Err(LinkProblem::NotWeb);
    }
    let url = Url::parse(raw).map_err(|_| LinkProblem::NotWeb)?;
    check_parsed(&url, allow_lan)?;
    Ok(url)
}

fn check_parsed(url: &Url, allow_lan: bool) -> Result<(), LinkProblem> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(LinkProblem::NotWeb);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(LinkProblem::Login);
    }
    let ip = match url.host() {
        None => return Err(LinkProblem::NotWeb),
        Some(Host::Domain(d)) => {
            if d.is_empty() {
                return Err(LinkProblem::NotWeb);
            }
            if !allow_lan && local_name(d) {
                return Err(LinkProblem::Local);
            }
            return Ok(());
        }
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
    };
    if never_ip(ip) {
        return Err(LinkProblem::Never);
    }
    if !allow_lan && !is_public_ip(ip) {
        return Err(LinkProblem::Local);
    }
    Ok(())
}

/// The host to show for `url` (no `www.`), "" when there's none.
pub fn host_of(url: &str) -> String {
    let Ok(u) = Url::parse(url) else {
        return String::new();
    };
    let h = match u.host() {
        Some(Host::Domain(d)) => idna::domain_to_unicode(d).0,
        Some(h) => h.to_string(),
        None => return String::new(),
    };
    h.strip_prefix("www.").unwrap_or(&h).to_string()
}

// --- names -------------------------------------------------------------------

fn pct_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// The parameters of a header value (`attachment; filename="a b.zip"`):
/// lower-case names, values unquoted.
fn params(v: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut parts = Vec::new();
    let (mut cur, mut quoted, mut escaped) = (String::new(), false, false);
    for c in v.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => {
                cur.push(c);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ';' if !quoted => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    for p in parts {
        let Some((k, val)) = p.split_once('=') else {
            continue;
        };
        let k = k.trim().to_ascii_lowercase();
        let val = val.trim();
        let val = match val.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
            Some(inner) => {
                let mut s = String::new();
                let mut esc = false;
                for c in inner.chars() {
                    if esc {
                        s.push(c);
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else {
                        s.push(c);
                    }
                }
                s
            }
            None => val.to_string(),
        };
        out.push((k, val));
    }
    out
}

/// The file name a `Content-Disposition` value gives: `filename*`
/// (RFC 5987: `UTF-8''r%C3%A9sum%C3%A9.pdf`, ISO-8859-1 too) before
/// `filename`. Not made safe yet (`safe_name`).
pub fn disposition_name(v: &str) -> Option<String> {
    let ps = params(v);
    let star = ps
        .iter()
        .find(|(k, _)| k == "filename*")
        .and_then(|(_, val)| {
            let mut it = val.splitn(3, '\'');
            let charset = it.next()?.trim().to_ascii_lowercase();
            let _lang = it.next()?;
            let bytes = pct_decode(it.next()?);
            match charset.as_str() {
                "utf-8" | "utf8" => String::from_utf8(bytes).ok(),
                "iso-8859-1" | "latin1" | "us-ascii" => {
                    Some(bytes.iter().map(|b| char::from(*b)).collect())
                }
                _ => None,
            }
        });
    star.or_else(|| {
        ps.into_iter()
            .find(|(k, _)| k == "filename")
            .map(|(_, val)| val)
    })
    .filter(|n| !n.trim().is_empty())
}

/// The last part of `url`'s path, decoded ("" when the path ends in `/`).
pub fn url_name(url: &str) -> String {
    let Ok(u) = Url::parse(url) else {
        return String::new();
    };
    let last = u
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("");
    let bytes = pct_decode(last);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A usual extension for a content type, for a name without one.
fn ext_for(content_type: &str) -> Option<&'static str> {
    let t = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    Some(match t.as_str() {
        "text/html" => "html",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "application/json" => "json",
        "application/gzip" | "application/x-gzip" => "gz",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "audio/mpeg" => "mp3",
        "video/mp4" => "mp4",
        "application/vnd.android.package-archive" => "apk",
        _ => return None,
    })
}

/// Characters that change the direction text shows in (they can make
/// `report` + U+202E + `fdp.exe` look like `reportexe.pdf`), or show nothing.
fn hidden_char(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// A downloaded file's name as it's saved: a plain name (no folders, no
/// hidden file), without control, direction-changing or reserved
/// characters, not a name Windows keeps for devices, at most `MAX_NAME`
/// bytes (the extension kept). "download" when nothing is left.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !hidden_char(*c)).collect();
    let mut n = super::files::safe_name(&capped(&clean));
    if n == "file" && clean.trim().trim_matches('.').is_empty() {
        n = "download".to_string();
    }
    if windows_reserved(&n) {
        n = format!("_{n}");
    }
    capped(&n)
}

/// Whether Windows takes `name` for a device (`CON`, `NUL`, `COM1`, ...,
/// with any extension): such a file can't be made there.
pub fn windows_reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// `n` cut to `MAX_NAME` bytes, its (short) extension kept.
fn capped(n: &str) -> String {
    if n.len() <= MAX_NAME {
        return n.to_string();
    }
    let (stem, ext) = match n.rfind('.') {
        Some(i) if i > 0 && n.len() - i <= 16 => (&n[..i], &n[i..]),
        _ => (n, ""),
    };
    let mut keep = MAX_NAME - ext.len();
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{ext}", stem[..keep].trim_end())
}

/// The name to save a download as: from `disposition`, else `url`'s path,
/// else its host; with an extension from `content_type` if it has none.
pub fn pick_name(url: &str, disposition: Option<&str>, content_type: Option<&str>) -> String {
    let from_header = disposition.and_then(disposition_name);
    let raw = from_header
        .clone()
        .or_else(|| Some(url_name(url)).filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| host_of(url));
    let mut name = safe_name(&raw);
    let has_ext = name.rfind('.').is_some_and(|i| i > 0 && i < name.len() - 1);
    if !has_ext && from_header.is_none() {
        if let Some(ext) = content_type.and_then(ext_for) {
            name = safe_name(&format!("{name}.{ext}"));
        }
    }
    name
}

// --- fetching -----------------------------------------------------------------

/// The HTTP client's resolver: only addresses a download may go to.
struct Guard {
    allow_lan: bool,
}

impl ureq::Resolver for Guard {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let all: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        let ok: Vec<SocketAddr> = all
            .into_iter()
            .filter(|a| ip_allowed(a.ip(), self.allow_lan))
            .collect();
        if ok.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                BLOCKED,
            ));
        }
        Ok(ok)
    }
}

/// How a fetch goes.
pub struct FetchOpts<'a> {
    pub allow_lan: bool,
    /// The biggest file taken, in bytes (0: no limit).
    pub max_bytes: u64,
    /// Tries after the first when the connection drops.
    pub retries: u32,
    /// The first wait before a retry (doubles each time).
    pub retry_wait: Duration,
    /// Set to stop.
    pub cancel: &'a AtomicBool,
}

/// Why a fetch stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Cancelled,
    /// The link (or a redirect) isn't one a download may go to.
    Link(LinkProblem),
    /// The server answered with this status.
    Http(u16),
    /// Bigger than the limit (bytes, as far as known).
    TooBig(u64),
    TooManyRedirects,
    /// The connection failed or dropped.
    Net(String),
    /// Writing the file failed.
    Disk(String),
}

impl FetchError {
    /// Worth trying again (from where it stopped).
    pub fn retryable(&self) -> bool {
        match self {
            FetchError::Net(_) => true,
            FetchError::Http(code) => *code >= 500 || *code == 408 || *code == 429,
            _ => false,
        }
    }

    /// For the user.
    pub fn reason(&self) -> String {
        match self {
            FetchError::Cancelled => "cancelled".into(),
            FetchError::Link(p) => p.reason().into(),
            FetchError::Http(401 | 403) => {
                "the site refused it (links that need a login can't be downloaded)".into()
            }
            FetchError::Http(404 | 410) => "the site says the file isn't there".into(),
            FetchError::Http(code) => format!("the site answered with error {code}"),
            FetchError::TooBig(size) if *size > 0 => format!(
                "it's bigger than the limit set for downloads ({} MB)",
                size.div_ceil(1024 * 1024)
            ),
            FetchError::TooBig(_) => "it's bigger than the limit set for downloads".into(),
            FetchError::TooManyRedirects => "the link redirects too many times".into(),
            FetchError::Net(why) => format!("couldn't reach the site ({why})"),
            FetchError::Disk(why) => format!("couldn't save the file ({why})"),
        }
    }
}

/// What a HEAD request (or the first answer) says about the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub final_url: String,
    pub name: String,
    /// Bytes, if the server said.
    pub size: Option<u64>,
}

/// A finished fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub name: String,
    pub size: u64,
    pub final_url: String,
}

/// A download slower than `MIN_SPEED` bytes a second over a whole
/// `SPEED_WINDOW` is given up (it can be tried again): a server that sends
/// a byte just inside the read timeout would otherwise hold the one
/// download slot for ever.
pub const MIN_SPEED: u64 = 1024;
pub const SPEED_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Watches a download's speed (`MIN_SPEED` over `SPEED_WINDOW`).
pub struct Stall {
    since: std::time::Instant,
    bytes: u64,
}

impl Stall {
    pub fn new(now: std::time::Instant) -> Self {
        Stall {
            since: now,
            bytes: 0,
        }
    }

    /// Count `n` more bytes at `now`; true if the last window was too slow.
    pub fn too_slow(&mut self, n: u64, now: std::time::Instant) -> bool {
        self.bytes = self.bytes.saturating_add(n);
        let took = now.saturating_duration_since(self.since);
        if took < SPEED_WINDOW {
            return false;
        }
        let slow = self.bytes < MIN_SPEED.saturating_mul(took.as_secs());
        *self = Stall::new(now);
        slow
    }
}

fn agent(allow_lan: bool) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .user_agent(USER_AGENT)
        .resolver(Guard { allow_lan })
        .build()
}

fn net_error(e: &ureq::Transport) -> FetchError {
    let text = e.to_string();
    if text.contains(BLOCKED) {
        return FetchError::Link(LinkProblem::Local);
    }
    FetchError::Net(match e.kind() {
        ureq::ErrorKind::Dns => "the site's name wasn't found".into(),
        ureq::ErrorKind::ConnectionFailed => "the connection failed".into(),
        _ => e
            .message()
            .map(str::to_string)
            .unwrap_or_else(|| e.kind().to_string()),
    })
}

/// `method` to `url` with `headers`, following redirects (each one
/// checked). Any status comes back as a response.
fn open(
    agent: &ureq::Agent,
    method: &str,
    url: &Url,
    headers: &[(&str, String)],
    opts: &FetchOpts,
) -> Result<(ureq::Response, Url), FetchError> {
    let mut url = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        check_parsed(&url, opts.allow_lan).map_err(FetchError::Link)?;
        if opts.cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let mut req = agent.request_url(method, &url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(net_error(&t)),
        };
        let status = resp.status();
        if (300..400).contains(&status) && status != 304 {
            let Some(loc) = resp.header("location") else {
                return Ok((resp, url));
            };
            url = url
                .join(loc.trim())
                .map_err(|_| FetchError::Link(LinkProblem::NotWeb))?;
            continue;
        }
        return Ok((resp, url));
    }
    Err(FetchError::TooManyRedirects)
}

fn header_u64(resp: &ureq::Response, name: &str) -> Option<u64> {
    resp.header(name).and_then(|v| v.trim().parse().ok())
}

/// `bytes <start>-<end>/<total>` -> (start, total if known).
pub fn content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = rest.split_once('/')?;
    let (start, _end) = range.trim().split_once('-')?;
    let total = total.trim();
    Some((
        start.trim().parse().ok()?,
        if total == "*" {
            None
        } else {
            Some(total.parse().ok()?)
        },
    ))
}

/// Ask the server about `url` (HEAD): the name and size it would have.
/// Servers that don't answer HEAD give an error; the download itself
/// still works.
pub fn probe(url: &Url, opts: &FetchOpts) -> Result<Probe, FetchError> {
    let agent = agent(opts.allow_lan);
    let (resp, final_url) = open(&agent, "HEAD", url, &[], opts)?;
    if resp.status() >= 400 {
        return Err(FetchError::Http(resp.status()));
    }
    Ok(Probe {
        name: pick_name(
            final_url.as_str(),
            resp.header("content-disposition"),
            resp.header("content-type"),
        ),
        size: header_u64(&resp, "content-length"),
        final_url: final_url.to_string(),
    })
}

/// What a fetch knows between tries.
#[derive(Default)]
struct Known {
    name: Option<String>,
    total: Option<u64>,
    /// `ETag` (strong) or `Last-Modified`, for `If-Range`.
    validator: Option<String>,
    final_url: String,
}

/// Download `url` into `part` (carrying on from what's there), calling
/// `progress(done, total, name)` as bytes arrive. Retries a dropped
/// connection from where it stopped.
pub fn download(
    url: &Url,
    part: &Path,
    opts: &FetchOpts,
    progress: &mut dyn FnMut(u64, Option<u64>, &str),
) -> Result<Fetched, FetchError> {
    let agent = agent(opts.allow_lan);
    let mut known = Known {
        final_url: url.to_string(),
        ..Known::default()
    };
    let mut tries = 0;
    loop {
        match fetch_once(&agent, url, part, opts, &mut known, progress) {
            Ok(size) => {
                return Ok(Fetched {
                    name: known
                        .name
                        .unwrap_or_else(|| pick_name(url.as_str(), None, None)),
                    size,
                    final_url: known.final_url,
                })
            }
            Err(e) if e.retryable() && tries < opts.retries => {
                tries += 1;
                let wait = opts.retry_wait * 2u32.saturating_pow(tries - 1);
                let until = Instant::now() + wait;
                while Instant::now() < until {
                    if opts.cancel.load(Ordering::Relaxed) {
                        return Err(FetchError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(50).min(wait));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn part_len(part: &Path) -> u64 {
    std::fs::metadata(part).map(|m| m.len()).unwrap_or(0)
}

fn disk(e: std::io::Error) -> FetchError {
    FetchError::Disk(e.to_string())
}

/// One request: the bytes from where `part` ends. The file's size when
/// it's all there.
fn fetch_once(
    agent: &ureq::Agent,
    url: &Url,
    part: &Path,
    opts: &FetchOpts,
    known: &mut Known,
    progress: &mut dyn FnMut(u64, Option<u64>, &str),
) -> Result<u64, FetchError> {
    let have = part_len(part);
    let mut headers: Vec<(&str, String)> = Vec::new();
    if have > 0 {
        headers.push(("Range", format!("bytes={have}-")));
        if let Some(v) = &known.validator {
            headers.push(("If-Range", v.clone()));
        }
    }
    let (resp, final_url) = open(agent, "GET", url, &headers, opts)?;
    let status = resp.status();
    let mut start = 0;
    match status {
        206 if have > 0 => {
            let (from, total) = resp
                .header("content-range")
                .and_then(content_range)
                .ok_or_else(|| FetchError::Net("a bad partial answer".into()))?;
            if from != have {
                // Not where we asked: start over on the next try.
                std::fs::remove_file(part).map_err(disk)?;
                return Err(FetchError::Net("the server sent another part".into()));
            }
            start = have;
            if total.is_some() {
                known.total = total;
            }
        }
        200 | 206 => {
            known.total = header_u64(&resp, "content-length");
        }
        416 if have > 0 => {
            // Nothing after what we have: it's complete if the server says
            // the file is that long.
            let total = resp
                .header("content-range")
                .and_then(content_range)
                .and_then(|(_, t)| t);
            if total == Some(have) && known.name.is_some() {
                return Ok(have);
            }
            std::fs::remove_file(part).map_err(disk)?;
            return Err(FetchError::Net("the partial file didn't match".into()));
        }
        _ => return Err(FetchError::Http(status)),
    }
    if known.name.is_none() {
        known.name = Some(pick_name(
            final_url.as_str(),
            resp.header("content-disposition"),
            resp.header("content-type"),
        ));
    }
    known.final_url = final_url.to_string();
    if known.validator.is_none() {
        known.validator = resp
            .header("etag")
            .filter(|e| !e.starts_with("W/"))
            .or_else(|| resp.header("last-modified"))
            .map(str::to_string);
    }
    let name = known.name.clone().unwrap_or_default();
    if opts.max_bytes > 0 && known.total.is_some_and(|t| t > opts.max_bytes) {
        return Err(FetchError::TooBig(known.total.unwrap_or(0)));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(part)
        .map_err(disk)?;
    let mut done = start;
    progress(done, known.total, &name);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut stall = Stall::new(std::time::Instant::now());
    loop {
        if opts.cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = file.flush();
                return Err(FetchError::Net(e.to_string()));
            }
        };
        file.write_all(&buf[..n]).map_err(disk)?;
        done += n as u64;
        if stall.too_slow(n as u64, std::time::Instant::now()) {
            let _ = file.flush();
            return Err(FetchError::Net("the server is too slow".into()));
        }
        if opts.max_bytes > 0 && done > opts.max_bytes {
            return Err(FetchError::TooBig(done));
        }
        progress(done, known.total, &name);
    }
    file.flush().map_err(disk)?;
    if let Some(total) = known.total {
        if done < total {
            return Err(FetchError::Net("the connection dropped".into()));
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_forms_that_carry_a_private_ipv4_are_not_public() {
        for a in [
            "::7f00:1",         // ::127.0.0.1, IPv4-compatible
            "::a00:1",          // ::10.0.0.1
            "::c0a8:101",       // ::192.168.1.1
            "2001:0:1:2::1",    // Teredo
            "64:ff9b:1::a00:1", // local-use NAT64
        ] {
            let ip: IpAddr = a.parse().unwrap();
            assert!(!ip_allowed(ip, false), "{a}");
        }
        assert!(ip_allowed("::808:808".parse().unwrap(), false), "::8.8.8.8");
        assert!(ip_allowed("2606:4700::1111".parse().unwrap(), false));
    }

    #[test]
    fn a_trickling_server_is_given_up_but_a_slow_one_is_not() {
        let t0 = std::time::Instant::now();
        let mut s = Stall::new(t0);
        // One byte every 29 s (just inside the read timeout).
        assert!(!s.too_slow(1, t0 + std::time::Duration::from_secs(29)));
        assert!(s.too_slow(1, t0 + std::time::Duration::from_secs(58 + 2)));
        // 2 KB/s: slow, but going.
        let mut s = Stall::new(t0);
        for i in 1..=180u64 {
            assert!(!s.too_slow(2048, t0 + std::time::Duration::from_secs(i)));
        }
    }
    use std::collections::HashMap;
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn only_web_links() {
        assert!(check_url("https://example.com/a.zip", false).is_ok());
        assert!(check_url("HTTP://Example.com/a", false).is_ok());
        for bad in [
            "ftp://example.com/a",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.c",
            "data:text/plain,hi",
            "example.com/a",
            "https://",
            "",
            "https://exa mple.com",
        ] {
            assert_eq!(check_url(bad, true), Err(LinkProblem::NotWeb), "{bad}");
        }
        assert_eq!(
            check_url("https://user:pw@example.com/a", false),
            Err(LinkProblem::Login)
        );
    }

    #[test]
    fn local_and_private_addresses_need_the_setting() {
        for local in [
            "http://127.0.0.1/",
            "http://127.1/",
            "http://0x7f000001/",
            "http://2130706433/",
            "http://10.0.0.5/",
            "http://172.16.3.4/",
            "http://192.168.1.1:8080/x",
            "http://169.254.169.254/latest/meta-data",
            "http://100.64.0.1/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:192.168.0.1]/",
            "http://[64:ff9b::a00:1]/",
            "http://localhost:3000/",
            "http://foo.localhost/",
            "http://printer.local/",
            "http://nas.lan/",
            "http://router/",
            "http://svc.internal/",
            "http://box.home.arpa/",
        ] {
            assert_eq!(check_url(local, false), Err(LinkProblem::Local), "{local}");
            assert!(check_url(local, true).is_ok(), "{local} with LAN links on");
        }
        for never in [
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://224.0.0.1/",
            "http://[::]/",
            "http://[ff02::1]/",
        ] {
            assert_eq!(check_url(never, true), Err(LinkProblem::Never), "{never}");
        }
        for public in [
            "http://8.8.8.8/",
            "http://[2606:4700::1111]/",
            "http://[::ffff:8.8.8.8]/",
        ] {
            assert!(check_url(public, false).is_ok(), "{public}");
        }
    }

    #[test]
    fn international_names_become_ascii() {
        let u = check_url(
            "https://bücher.example/ä ö.pdf"
                .replace(' ', "%20")
                .as_str(),
            false,
        )
        .unwrap();
        assert_eq!(u.host_str(), Some("xn--bcher-kva.example"));
        assert_eq!(host_of(u.as_str()), "bücher.example");
        assert_eq!(host_of("https://www.Example.com:8080/a"), "example.com");
        // A dotless international name is still local.
        assert_eq!(check_url("http://bücher/", false), Err(LinkProblem::Local));
    }

    #[test]
    fn the_resolver_drops_local_addresses() {
        let g = Guard { allow_lan: false };
        let e = ureq::Resolver::resolve(&g, "127.0.0.1:80").unwrap_err();
        assert!(e.to_string().contains(BLOCKED));
        assert!(ureq::Resolver::resolve(&Guard { allow_lan: true }, "127.0.0.1:80").is_ok());
        assert!(ureq::Resolver::resolve(&g, "8.8.8.8:80").is_ok());
        assert!(ureq::Resolver::resolve(&Guard { allow_lan: true }, "0.0.0.0:80").is_err());
    }

    #[test]
    fn content_disposition() {
        assert_eq!(
            disposition_name("attachment; filename=\"report final.zip\"").as_deref(),
            Some("report final.zip")
        );
        assert_eq!(
            disposition_name("attachment; filename=plain.txt").as_deref(),
            Some("plain.txt")
        );
        assert_eq!(
            disposition_name(
                "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"
            )
            .as_deref(),
            Some("résumé.pdf")
        );
        assert_eq!(
            disposition_name("attachment; filename*=iso-8859-1'en'caf%E9.txt").as_deref(),
            Some("café.txt")
        );
        assert_eq!(
            disposition_name(r#"inline; filename="a \"quoted\" ; name.txt""#).as_deref(),
            Some(r#"a "quoted" ; name.txt"#)
        );
        assert_eq!(disposition_name("attachment"), None);
        assert_eq!(disposition_name("attachment; filename=\"\""), None);
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name("a\u{202E}fdp.exe"), "afdp.exe");
        assert_eq!(safe_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_name("com1"), "_com1");
        assert_eq!(safe_name("console.txt"), "console.txt");
        assert_eq!(safe_name("..."), "download");
        assert_eq!(safe_name(""), "download");
        assert_eq!(safe_name("a<b>c?.zip"), "abc.zip");
        let long = format!("{}.tar.gz", "é".repeat(200));
        let s = safe_name(&long);
        assert!(s.len() <= MAX_NAME, "{}", s.len());
        assert!(s.ends_with(".gz"));
    }

    #[test]
    fn names_come_from_the_header_the_link_or_the_host() {
        assert_eq!(
            pick_name("https://x.io/files/report%202024.pdf?dl=1", None, None),
            "report 2024.pdf"
        );
        assert_eq!(
            pick_name(
                "https://x.io/get?id=4",
                Some("attachment; filename=\"data.csv\""),
                Some("text/csv")
            ),
            "data.csv"
        );
        assert_eq!(pick_name("https://x.io/", None, Some("text/html")), "x.io");
        assert_eq!(
            pick_name("https://x.io/page", None, Some("text/html; charset=utf-8")),
            "page.html"
        );
        assert_eq!(
            pick_name("https://x.io/a/..%2F..%2Fsecret", None, None),
            "secret"
        );
    }

    #[test]
    fn content_ranges() {
        assert_eq!(content_range("bytes 100-199/200"), Some((100, Some(200))));
        assert_eq!(content_range("bytes 5-9/*"), Some((5, None)));
        assert_eq!(content_range("bytes */200"), None);
        assert_eq!(content_range("items 1-2/3"), None);
    }

    // --- a small HTTP server on 127.0.0.1 (no network) -----------------------

    #[derive(Default)]
    struct Served {
        /// The Range header of each GET, in order.
        ranges: Vec<Option<String>>,
        gets: usize,
    }

    /// Serves `body` at `/file.bin`, redirects `/go` there, answers HEAD.
    /// The first `cut` GETs stop halfway through.
    fn server(body: Vec<u8>, cut: usize, range_ok: bool) -> (String, Arc<Mutex<Served>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let served = Arc::new(Mutex::new(Served::default()));
        let s = served.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                let mut reader = std::io::BufReader::new(conn.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let (method, path) = (
                    parts.next().unwrap_or("").to_string(),
                    parts.next().unwrap_or("").to_string(),
                );
                let mut headers = HashMap::new();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                let reply = |conn: &mut std::net::TcpStream, head: String, data: &[u8]| {
                    let _ = conn.write_all(head.as_bytes());
                    let _ = conn.write_all(data);
                    let _ = conn.flush();
                };
                if path == "/go" {
                    reply(
                        &mut conn,
                        "HTTP/1.1 302 Found\r\nLocation: /file.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        b"",
                    );
                    continue;
                }
                if path == "/elsewhere" {
                    reply(
                        &mut conn,
                        "HTTP/1.1 301 Moved\r\nLocation: file:///etc/passwd\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        b"",
                    );
                    continue;
                }
                if path != "/file.bin" {
                    reply(
                        &mut conn,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .into(),
                        b"",
                    );
                    continue;
                }
                let cd =
                    "Content-Disposition: attachment; filename=\"report.zip\"\r\nETag: \"v1\"\r\n";
                if method == "HEAD" {
                    reply(
                        &mut conn,
                        format!("HTTP/1.1 200 OK\r\n{cd}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                        b"",
                    );
                    continue;
                }
                let range = headers.get("range").cloned();
                let n = {
                    let mut s = s.lock().unwrap();
                    s.ranges.push(range.clone());
                    s.gets += 1;
                    s.gets
                };
                let from = range
                    .as_deref()
                    .filter(|_| range_ok)
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                let (head, data) = match from {
                    Some(f) => (
                        format!(
                            "HTTP/1.1 206 Partial Content\r\n{cd}Content-Range: bytes {f}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len() - 1,
                            body.len(),
                            body.len() - f
                        ),
                        &body[f..],
                    ),
                    None => (
                        format!("HTTP/1.1 200 OK\r\n{cd}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                        &body[..],
                    ),
                };
                let data = if n <= cut {
                    &data[..data.len() / 2]
                } else {
                    data
                };
                reply(&mut conn, head, data);
                // Dropping the connection here cuts a short answer off.
            }
        });
        (base, served)
    }

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn opts(cancel: &AtomicBool) -> FetchOpts<'_> {
        FetchOpts {
            allow_lan: true,
            max_bytes: 0,
            retries: 3,
            retry_wait: Duration::from_millis(10),
            cancel,
        }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mdcore-link-fetch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("x.part")
    }

    #[test]
    fn downloads_follow_redirects_and_take_the_header_name() {
        let data = body(300_000);
        let (base, _) = server(data.clone(), 0, true);
        let cancel = AtomicBool::new(false);
        let part = tmp("redirect");
        let url = check_url(&format!("{base}/go"), true).unwrap();
        let mut last = 0;
        let got = download(&url, &part, &opts(&cancel), &mut |d, _, _| last = d).unwrap();
        assert_eq!(got.name, "report.zip");
        assert_eq!(got.size, data.len() as u64);
        assert!(got.final_url.ends_with("/file.bin"));
        assert_eq!(last, data.len() as u64);
        assert_eq!(std::fs::read(&part).unwrap(), data);
        let p = probe(&url, &opts(&cancel)).unwrap();
        assert_eq!((p.name.as_str(), p.size), ("report.zip", Some(300_000)));
    }

    #[test]
    fn a_dropped_connection_resumes_with_range() {
        let data = body(500_000);
        let (base, served) = server(data.clone(), 1, true);
        let cancel = AtomicBool::new(false);
        let part = tmp("resume");
        let url = check_url(&format!("{base}/file.bin"), true).unwrap();
        let got = download(&url, &part, &opts(&cancel), &mut |_, _, _| {}).unwrap();
        assert_eq!(got.size, data.len() as u64);
        assert_eq!(std::fs::read(&part).unwrap(), data);
        let s = served.lock().unwrap();
        assert_eq!(s.gets, 2);
        assert_eq!(s.ranges[0], None);
        assert_eq!(s.ranges[1].as_deref(), Some("bytes=250000-"));
    }

    #[test]
    fn a_server_without_ranges_sends_it_all_again() {
        let data = body(200_000);
        let (base, served) = server(data.clone(), 1, false);
        let cancel = AtomicBool::new(false);
        let part = tmp("norange");
        let url = check_url(&format!("{base}/file.bin"), true).unwrap();
        download(&url, &part, &opts(&cancel), &mut |_, _, _| {}).unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), data);
        assert_eq!(served.lock().unwrap().gets, 2);
    }

    #[test]
    fn errors_limits_and_local_redirects() {
        let data = body(100_000);
        let (base, _) = server(data, 0, true);
        let cancel = AtomicBool::new(false);
        let part = tmp("errors");
        let missing = check_url(&format!("{base}/nope"), true).unwrap();
        assert_eq!(
            download(&missing, &part, &opts(&cancel), &mut |_, _, _| {}),
            Err(FetchError::Http(404))
        );
        let url = check_url(&format!("{base}/file.bin"), true).unwrap();
        let small = FetchOpts {
            max_bytes: 10_000,
            ..opts(&cancel)
        };
        assert_eq!(
            download(&url, &part, &small, &mut |_, _, _| {}),
            Err(FetchError::TooBig(100_000))
        );
        // Without LAN links, the loopback server itself is refused.
        let strict = FetchOpts {
            allow_lan: false,
            ..opts(&cancel)
        };
        assert_eq!(
            download(&url, &part, &strict, &mut |_, _, _| {}),
            Err(FetchError::Link(LinkProblem::Local))
        );
        // Every redirect is checked like the link itself.
        let r = check_url(&format!("{base}/elsewhere"), true).unwrap();
        assert_eq!(
            download(&r, &part, &opts(&cancel), &mut |_, _, _| {}),
            Err(FetchError::Link(LinkProblem::NotWeb))
        );
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            download(&url, &part, &opts(&cancel), &mut |_, _, _| {}),
            Err(FetchError::Cancelled)
        );
    }
}
