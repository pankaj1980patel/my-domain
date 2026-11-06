//! Verification codes on the computer: the phone finds a one-time code in a
//! notification (its host app does, from the notification listener: never
//! from SMS) and sends just the code to the computers you chose, which show
//! "Code 482913 from Bank · Copy".
//!
//!   otp_code { from, code, app, issuer?, expires_at }   phone → a computer
//!
//! Privacy, by construction:
//! - Only `{code, app, issuer, expires_at}` goes: never the notification's
//!   text. It rides the same end-to-end encrypted control channel as every
//!   other message.
//! - Only to your own computers (a paired device whose kind says macOS,
//!   Windows or Linux) that said `otp_relay: 1` in their status or
//!   `addr_update`: an older app, or a phone, never gets one.
//! - Only over a live link: without one, it waits at most `QUEUE_FOR` for
//!   the link to come up (it starts connecting), in memory only, never in
//!   the outbox or `kv`.
//! - A code lives `TTL` (`expires_at`, unix ms; a receiver never keeps one
//!   longer than `TTL` from when it arrived, whatever the sender said). Then
//!   it's wiped from memory (its bytes zeroed) and `VerificationCodeGone`
//!   says so. It's never logged (`VerificationCode`'s `Debug` hides it), and
//!   never kept in the clipboard history, activity or any other history.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag saying this device shows
/// verification codes sent to it, and its version.
pub const FLAG: &str = "otp_relay";
const VERSION: u64 = 1;
/// How long a code lives.
pub const TTL: Duration = Duration::from_secs(5 * 60);
/// How long a code waits for a link to come up before it's dropped.
pub const QUEUE_FOR: Duration = Duration::from_secs(2 * 60);
/// Codes kept on a receiving device at once (newest first).
pub const MAX_KEPT: usize = 4;
/// Longest app or issuer name.
pub const MAX_LABEL: usize = 60;
/// The operating systems a "computer" runs.
pub const COMPUTER_OSES: [&str; 3] = ["macos", "windows", "linux"];

const OLD_PEER: &str = "that computer's app is too old for verification codes: update it";

/// A verification code another device sent here. Its `code` is zeroed when
/// it's dropped, and `Debug` never prints it.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct VerificationCode {
    /// This code's id here (to copy or dismiss it).
    pub id: String,
    /// The device that sent it.
    pub from: String,
    pub code: String,
    /// The app whose notification carried it ("Messages", "HDFC Bank").
    pub app: String,
    /// Who it's from, when the text said ("Google", "Amazon"): may be
    /// empty.
    pub issuer: String,
    /// When it stops being shown (unix ms).
    pub expires_at: u64,
}

impl std::fmt::Debug for VerificationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerificationCode")
            .field("id", &self.id)
            .field("from", &self.from)
            .field("code", &"<hidden>")
            .field("app", &self.app)
            .field("issuer", &self.issuer)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Drop for VerificationCode {
    fn drop(&mut self) {
        wipe(&mut self.code);
    }
}

/// Zero a string's bytes before its memory is given back.
pub fn wipe(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    for b in bytes.iter_mut() {
        *b = 0;
    }
    // Keeps the zeroing from being optimised away as a dead store.
    std::hint::black_box(&bytes);
}

/// A code waiting for a link (sender side).
struct Pending {
    node: String,
    code: String,
    app: String,
    issuer: String,
    expires_at: u64,
    queued: Instant,
}

impl Drop for Pending {
    fn drop(&mut self) {
        wipe(&mut self.code);
    }
}

/// Who takes codes, what waits for a link, and the codes shown here.
#[derive(Default)]
pub struct OtpRelay {
    /// Device -> whether it said `otp_relay`.
    caps: Mutex<HashMap<String, bool>>,
    pending: Mutex<Vec<Pending>>,
    received: Mutex<VecDeque<VerificationCode>>,
    next_id: AtomicU64,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &OtpRelay {
    &ctx.extras.otp
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// --- checks (pure) --------------------------------------------------------------

/// A code as sent: the spaces and dashes splitting it ("123-456") taken
/// out, letters in capitals. 4–8 digits, or 6–8 letters and digits.
pub fn clean_code(code: &str) -> Option<String> {
    let c: String = code
        .trim()
        .chars()
        .filter(|c| *c != ' ' && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !c.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let digits_only = c.chars().all(|c| c.is_ascii_digit());
    let ok = if digits_only {
        (4..=8).contains(&c.len())
    } else {
        (6..=8).contains(&c.len())
    };
    ok.then_some(c)
}

/// An app or issuer name: trimmed, no control characters, at most
/// `MAX_LABEL` characters.
pub fn clean_label(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_LABEL)
        .collect::<String>()
        .trim()
        .to_string()
}

/// When a code that arrived at `now` (unix ms) saying `expires_at` stops
/// being shown: never later than `TTL` from now; `None` if already gone.
pub fn expiry(expires_at: u64, now: u64) -> Option<u64> {
    let latest = now.saturating_add(TTL.as_millis() as u64);
    let at = expires_at.min(latest);
    (at > now).then_some(at)
}

/// Whether a code queued at `queued` (expiring at `expires_at`, unix ms)
/// may still go at `now` / `now_ms`.
pub fn still_due(queued: Instant, now: Instant, expires_at: u64, now_ms: u64) -> bool {
    now.saturating_duration_since(queued) < QUEUE_FOR && expires_at > now_ms
}

/// Whether `os` is a computer's.
pub fn is_computer_os(os: &str) -> bool {
    COMPUTER_OSES.contains(&os)
}

// --- capability -----------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it shows codes.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.otp_relay {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: its status' or `addr_update`'s `otp_relay`)
/// whether it shows codes.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    st(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` said it shows verification codes.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    ctx.config.otp_relay && st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

/// `node` is one of your computers (paired, and its kind says so).
pub fn is_computer<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    !node.starts_with("manual:")
        && ctx.peers.lock().contains_key(node)
        && is_computer_os(&super::device_kind::of(ctx, node).os)
}

/// Whether a code may go to `node`: one of your computers that takes them.
pub fn eligible<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    is_computer(ctx, node) && supports(ctx, node)
}

/// Your computers, each with whether it takes verification codes (for the
/// phone's "choose computers"), sorted by node id.
pub fn computers<P: Platform>(ctx: &NetCtx<P>) -> Vec<(String, bool)> {
    let nodes: Vec<String> = ctx.peers.lock().keys().cloned().collect();
    let mut out: Vec<(String, bool)> = nodes
        .into_iter()
        .filter(|n| is_computer(ctx, n))
        .map(|n| {
            let ok = supports(ctx, &n);
            (n, ok)
        })
        .collect();
    out.sort();
    out
}

// --- sending (the phone) ----------------------------------------------------------

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

fn wire(me: &str, code: &str, app: &str, issuer: &str, expires_at: u64) -> String {
    let mut m = json!({
        "type": "otp_code", "from": me, "code": code, "app": app, "expires_at": expires_at,
    });
    if !issuer.is_empty() {
        m["issuer"] = issuer.into();
    }
    m.to_string()
}

fn send_now<P: Platform>(ctx: &NetCtx<P>, node: &str, p: &Pending) -> bool {
    let mut msg = wire(&me(ctx), &p.code, &p.app, &p.issuer, p.expires_at);
    let name = ctx.identity.lock().name.clone();
    let ok =
        crate::transport::send_payload(&ctx.ws_conns, &ctx.p2p_conns, &ctx.key, &name, node, &msg)
            .is_ok();
    wipe(&mut msg);
    ok
}

/// Send `code` (found in a notification of `app`, from `issuer`) to those
/// of `nodes` that may have it ([`eligible`]). Over a live link it goes at
/// once; else it waits up to `QUEUE_FOR` (connecting meanwhile). Returns
/// the computers it went or will go to.
pub fn send<P: Platform>(
    ctx: &NetCtx<P>,
    code: &str,
    app: &str,
    issuer: &str,
    nodes: &[String],
) -> crate::Result<Vec<String>> {
    if !ctx.config.otp_relay {
        return Err(CoreError::Unsupported("verification codes are off here"));
    }
    let Some(code) = clean_code(code) else {
        return Err(CoreError::InvalidInput(
            "that doesn't look like a code".into(),
        ));
    };
    let (app, issuer) = (clean_label(app), clean_label(issuer));
    let expires_at = now_ms().saturating_add(TTL.as_millis() as u64);
    let mut to: Vec<String> = Vec::new();
    let mut known = false;
    for node in nodes {
        if to.contains(node) || !is_computer(ctx, node) {
            continue;
        }
        known = true;
        if !supports(ctx, node) {
            continue;
        }
        let p = Pending {
            node: node.clone(),
            code: code.clone(),
            app: app.clone(),
            issuer: issuer.clone(),
            expires_at,
            queued: Instant::now(),
        };
        if crate::transport::is_live(ctx, node) && send_now(ctx, node, &p) {
            to.push(node.clone());
            continue;
        }
        {
            let mut q = st(ctx).pending.lock();
            // Only the newest code waits for each computer.
            q.retain(|o| o.node != *node);
            q.push(p);
        }
        crate::engine::start_ladder(ctx, node);
        sweep_later(ctx);
        to.push(node.clone());
    }
    let mut code = code;
    wipe(&mut code);
    if to.is_empty() && known {
        return Err(CoreError::Unsupported(OLD_PEER));
    }
    Ok(to)
}

/// Drop what waited too long (wiped), a moment after `QUEUE_FOR`.
fn sweep_later<P: Platform>(ctx: &NetCtx<P>) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if ctx.shutdown.wait(QUEUE_FOR + Duration::from_secs(1)) {
            sweep(&ctx);
        }
    });
}

fn sweep<P: Platform>(ctx: &NetCtx<P>) {
    let (now, now_ms) = (Instant::now(), now_ms());
    st(ctx)
        .pending
        .lock()
        .retain(|p| still_due(p.queued, now, p.expires_at, now_ms));
}

/// A link to `node` came up: a code waiting for it goes now, if it's still
/// in time.
pub fn link_up<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    let due: Vec<Pending> = {
        let mut q = st(ctx).pending.lock();
        let (now, now_ms) = (Instant::now(), now_ms());
        q.retain(|p| still_due(p.queued, now, p.expires_at, now_ms));
        let mut due = Vec::new();
        let mut i = 0;
        while i < q.len() {
            if q[i].node == node {
                due.push(q.remove(i));
            } else {
                i += 1;
            }
        }
        due
    };
    for p in due {
        if eligible(ctx, node) {
            let _ = send_now(ctx, node, &p);
        }
    }
}

/// How many codes wait for a link (tests).
pub fn waiting<P: Platform>(ctx: &NetCtx<P>) -> usize {
    st(ctx).pending.lock().len()
}

// --- receiving (the computer) -------------------------------------------------------

/// The codes shown here that haven't expired, newest first.
pub fn current<P: Platform>(ctx: &NetCtx<P>) -> Vec<VerificationCode> {
    let now = now_ms();
    let mut r = st(ctx).received.lock();
    r.retain(|c| c.expires_at > now);
    r.iter().cloned().collect()
}

/// Forget code `id` here (wiped): `VerificationCodeGone` says so.
pub fn dismiss<P: Platform>(ctx: &NetCtx<P>, id: &str) -> bool {
    let gone = {
        let mut r = st(ctx).received.lock();
        let before = r.len();
        r.retain(|c| c.id != id);
        before != r.len()
    };
    if gone {
        ctx.sink
            .emit(CoreEvent::VerificationCodeGone { id: id.to_string() });
    }
    gone
}

/// Drop code `id` when it expires.
fn expire_later<P: Platform>(ctx: &NetCtx<P>, id: String, at: u64) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let wait = Duration::from_millis(at.saturating_sub(now_ms()));
        if ctx.shutdown.wait(wait) {
            dismiss(&ctx, &id);
        }
    });
}

pub struct OtpRelayHandler;

impl<P: Platform> ControlHandler<P> for OtpRelayHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["otp_code"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        // Only while this end takes them, and only from your own devices.
        if !ctx.config.otp_relay
            || from.is_empty()
            || from.starts_with("manual:")
            || !ctx.peers.lock().contains_key(&from)
        {
            return;
        }
        let mut raw = msg.str("code");
        let code = clean_code(&raw);
        wipe(&mut raw);
        let Some(code) = code else {
            return;
        };
        let now = now_ms();
        let Some(expires_at) = msg
            .json("expires_at")
            .as_u64()
            .and_then(|at| expiry(at, now))
        else {
            let mut code = code;
            wipe(&mut code);
            return; // already expired (it waited too long somewhere)
        };
        let n = st(ctx).next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let c = VerificationCode {
            id: format!("otp-{n}"),
            from,
            code,
            app: clean_label(&msg.str("app")),
            issuer: clean_label(&msg.str("issuer")),
            expires_at,
        };
        {
            let mut r = st(ctx).received.lock();
            r.retain(|o| o.expires_at > now);
            // The same code again (sent twice): shown once.
            if r.iter().any(|o| o.from == c.from && o.code == c.code) {
                return;
            }
            r.push_front(c.clone());
            r.truncate(MAX_KEPT);
        }
        expire_later(ctx, c.id.clone(), c.expires_at);
        ctx.sink.emit(CoreEvent::VerificationCode(c));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_cleaned_and_checked() {
        assert_eq!(clean_code("482913").as_deref(), Some("482913"));
        assert_eq!(clean_code(" 123-456 ").as_deref(), Some("123456"));
        assert_eq!(clean_code("123 456").as_deref(), Some("123456"));
        assert_eq!(clean_code("1234").as_deref(), Some("1234"));
        assert_eq!(clean_code("12345678").as_deref(), Some("12345678"));
        assert_eq!(clean_code("ab12cd").as_deref(), Some("AB12CD"));
        assert_eq!(clean_code("K7XQ2PZ9").as_deref(), Some("K7XQ2PZ9"));
        for bad in [
            "",
            "123",
            "123456789",
            "abc",
            "ab12",
            "AB12CD34E",
            "12.34",
            "१२३४५६",
            "12_345",
        ] {
            assert_eq!(clean_code(bad), None, "{bad}");
        }
    }

    #[test]
    fn labels_are_short_and_plain() {
        assert_eq!(clean_label("  HDFC\u{0}Bank \n"), "HDFCBank");
        assert_eq!(clean_label(&"x".repeat(200)).len(), MAX_LABEL);
        assert_eq!(clean_label("स्टेट बैंक"), "स्टेट बैंक");
    }

    #[test]
    fn a_code_never_lives_past_five_minutes_here() {
        let now = 1_000_000;
        let ttl = TTL.as_millis() as u64;
        assert_eq!(expiry(now + 60_000, now), Some(now + 60_000));
        assert_eq!(expiry(now + 3_600_000, now), Some(now + ttl), "clamped");
        assert_eq!(expiry(now, now), None);
        assert_eq!(expiry(now - 1, now), None);
        assert_eq!(expiry(0, now), None);
    }

    #[test]
    fn a_queued_code_waits_two_minutes_at_most() {
        let t = Instant::now();
        let exp = 10_000_000;
        assert!(still_due(t, t, exp, 1));
        assert!(still_due(t, t + Duration::from_secs(119), exp, 1));
        assert!(!still_due(t, t + QUEUE_FOR, exp, 1));
        assert!(
            !still_due(t, t + Duration::from_secs(1), exp, exp),
            "expired"
        );
    }

    #[test]
    fn only_computers_count() {
        for os in ["macos", "windows", "linux"] {
            assert!(is_computer_os(os));
        }
        for os in ["android", "ios", "", "unknown"] {
            assert!(!is_computer_os(os));
        }
    }

    #[test]
    fn a_code_is_never_printed_and_is_wiped() {
        let c = VerificationCode {
            id: "otp-1".into(),
            from: "phone".into(),
            code: "482913".into(),
            app: "Bank".into(),
            issuer: String::new(),
            expires_at: 1,
        };
        let shown = format!("{c:?}");
        assert!(!shown.contains("482913"), "{shown}");
        assert!(shown.contains("<hidden>"));
        let mut s = String::from("482913");
        wipe(&mut s);
        assert!(s.is_empty());
    }

    #[test]
    fn only_the_four_fields_go_on_the_wire() {
        let m: Value = serde_json::from_str(&wire("me", "482913", "Bank", "", 5)).unwrap();
        let mut keys: Vec<&str> = m.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["app", "code", "expires_at", "from", "type"]);
        let m: Value = serde_json::from_str(&wire("me", "482913", "Bank", "Google", 5)).unwrap();
        assert_eq!(m["issuer"], "Google");
    }
}
