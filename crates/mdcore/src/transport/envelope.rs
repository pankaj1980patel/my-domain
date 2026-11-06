//! Envelope version 2: every message says who sent it, to whom, when, and
//! a message id, and the encryption vouches for all four.
//!
//! A version 1 envelope (`crypto::encrypt`) only proves that one of the
//! account's devices made it, once. So the relay server, or anyone who
//! recorded the relay or Bluetooth traffic, could send an old one again
//! (`input` text plus Enter, `open_url`, `action: lock`, ...) and it would
//! be acted on as coming from whichever device the server named.
//!
//! Version 2 puts a header in the clear and passes it to XChaCha20-Poly1305
//! as associated data ([`aad`]):
//!
//!   v: 2, src: sender node id, dst: recipient node id,
//!   mid: random 128-bit id (hex), ts: send time (unix ms)
//!
//! Changing any of it breaks the tag. The receiver ([`Guard::check`])
//! then wants:
//! - `dst` is this device. Every envelope is sealed for one device: a
//!   message to several (`broadcast_control`, `addr_update` to every link)
//!   is sealed once per recipient, so no envelope is ever good for two.
//! - `src` is the device the link says it came from: the pinned device key
//!   over QUIC (TLS), the server's `from` on the relay, the `Hello` on
//!   Bluetooth. On the relay and Bluetooth only the bound `src` and the
//!   account key vouch for the sender: a frame passed off as another
//!   device's is refused.
//! - `ts` within [`WINDOW_MS`] (5 minutes) of this device's clock either
//!   way. The difference is kept per device (clock skew, in diagnostics).
//! - `mid` not seen from that device before. Ids are kept per sender for
//!   as long as their `ts` is in the window, so the set stays small; those
//!   that came over the relay or Bluetooth (the links where a copy can be
//!   recorded: QUIC and WebRTC are encrypted end to end) are also written
//!   to disk as they arrive, so a restart doesn't open the window again.
//!   A sender that went over [`MAX_PER_PEER`] ids in the window raises a
//!   floor instead: anything at or before the oldest id let go is refused.
//!
//! Negotiation: a device says `env: 2` in its status and `addr_update`.
//! Version 2 goes only to devices that said so (or sent one); others get
//! version 1, which is still taken from a device until it is seen sending
//! version 2. From then on (remembered across restarts) its version 1
//! envelopes are refused: a downgrade. Advertising alone doesn't switch
//! that on, because a device's own messages stay version 1 until it hears
//! our `env: 2`, and those already on their way would be refused.
//! `CoreConfig::envelope_v2` off plays an older app: version 1 only, and no
//! checks.
//!
//! Every refusal is counted per device (`Engine::envelope_info`, the
//! connection report) and logged without content.

use std::collections::HashMap;
use std::io::Write as _;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use parking_lot::Mutex;
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use serde_json::Value;

use crate::model::Envelope;
use crate::platform::Platform;

/// This envelope version.
pub const VERSION: u8 = 2;
/// The status and `addr_update` flag saying which version a device reads.
pub const FLAG: &str = "env";
/// How far a message's send time may be from this device's clock, either way.
pub const WINDOW_MS: u64 = 5 * 60 * 1000;
/// Message ids kept per sender (in the window); past it the floor rises.
pub const MAX_PER_PEER: usize = 65_536;
/// Devices whose ids and counters are kept.
const MAX_PEERS: usize = 256;
/// kv key: node id -> 1 (it said `env: 2`) or 2 (it sent version 2).
const PEERS_KEY: &str = "env_peers";
/// Where the ids are written as they arrive (or, without a data file, kv).
const SEEN_FILE: &str = "env_seen.log";
const SEEN_KEY: &str = "env_seen";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The associated data the AEAD binds: a label, the version, and each field
/// with its length in front (so no two headers give the same bytes).
pub fn aad(src: &str, dst: &str, mid: &[u8; 16], ts: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + src.len() + dst.len());
    out.extend_from_slice(b"my-domain envelope\0");
    out.push(VERSION);
    for s in [src, dst] {
        out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out.extend_from_slice(mid);
    out.extend_from_slice(&ts.to_be_bytes());
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_mid(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Seal `plaintext` from `src` to `dst` with the given id and time. Tests
/// use it to make envelopes a device wouldn't (old, misaddressed).
pub fn seal_with(
    key: &[u8; 32],
    src: &str,
    dst: &str,
    mid: [u8; 16],
    ts: u64,
    plaintext: &[u8],
) -> Option<Envelope> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).ok()?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ad = aad(src, dst, &mid, ts);
    let ct = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &ad,
            },
        )
        .ok()?;
    Some(Envelope {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ct),
        v: VERSION,
        src: src.to_string(),
        dst: dst.to_string(),
        mid: hex(&mid),
        ts,
    })
}

/// Seal `plaintext` from `src` to `dst`: a fresh id, sent now.
pub fn seal(key: &[u8; 32], src: &str, dst: &str, plaintext: &[u8]) -> Option<Envelope> {
    let mut mid = [0u8; 16];
    OsRng.fill_bytes(&mut mid);
    seal_with(key, src, dst, mid, now_ms(), plaintext)
}

/// A version 2 envelope's header, once the tag vouched for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub src: String,
    pub dst: String,
    pub mid: [u8; 16],
    pub ts: u64,
}

/// Open a version 2 envelope: its header and plaintext, or `None` if it
/// isn't one, was changed (header included), or was sealed under another key.
pub fn open(key: &[u8; 32], env: &Envelope) -> Option<(Header, Vec<u8>)> {
    if env.v != VERSION || env.src.is_empty() || env.dst.is_empty() || env.ts == 0 {
        return None;
    }
    let mid = parse_mid(&env.mid)?;
    let nonce = STANDARD.decode(&env.nonce).ok()?;
    if nonce.len() != 24 {
        return None;
    }
    let ct = STANDARD.decode(&env.ciphertext).ok()?;
    let cipher = XChaCha20Poly1305::new_from_slice(key).ok()?;
    let ad = aad(&env.src, &env.dst, &mid, env.ts);
    let pt = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ct.as_ref(),
                aad: &ad,
            },
        )
        .ok()?;
    Some((
        Header {
            src: env.src.clone(),
            dst: env.dst.clone(),
            mid,
            ts: env.ts,
        },
        pt,
    ))
}

/// Why an envelope that opened was still refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Its id was seen before (or it's older than the floor).
    Replay,
    /// Its send time is more than [`WINDOW_MS`] from this device's clock.
    Stale,
    /// Version 1 from a device seen sending version 2.
    Downgrade,
    /// Sealed for another device.
    WrongRecipient,
    /// Its sender isn't the device the link says it came from.
    Spoofed,
}

impl Reject {
    pub fn as_str(self) -> &'static str {
        match self {
            Reject::Replay => "replay",
            Reject::Stale => "stale",
            Reject::Downgrade => "downgrade",
            Reject::WrongRecipient => "wrong_recipient",
            Reject::Spoofed => "spoofed",
        }
    }
}

/// What the connection report shows about one device's envelopes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct EnvelopeInfo {
    /// The version it reads, as far as this device knows: 2 (it said so,
    /// or sent one) or 1.
    pub version: u8,
    /// It was seen sending version 2: its version 1 envelopes are refused.
    pub v2_only: bool,
    /// Version 2 envelopes taken from it.
    pub accepted: u32,
    /// Refused: an id seen before (a copy sent again).
    pub replays: u32,
    /// Refused: sent more than 5 minutes from this device's clock.
    pub stale: u32,
    /// Refused: version 1 after it was seen sending version 2.
    pub downgrades: u32,
    /// Refused: sealed for another device.
    pub wrong_recipient: u32,
    /// Refused: sealed by another device than the link's.
    pub spoofed: u32,
    /// Its clock minus this one's, in ms (with the time on the way), from
    /// its last version 2 envelope.
    pub skew_ms: Option<i32>,
}

/// Ids seen from one sender, in the window.
#[derive(Default)]
struct PeerSeen {
    mids: HashMap<[u8; 16], u64>,
    /// Anything sent at or before this (ms) is refused (raised when ids
    /// over `MAX_PER_PEER` had to be let go).
    floor: u64,
    inserts: u32,
}

impl PeerSeen {
    /// Drop ids whose time is out of the window (any copy is stale now).
    fn prune(&mut self, now: u64) {
        let oldest = now.saturating_sub(WINDOW_MS);
        self.mids.retain(|_, ts| *ts >= oldest);
    }

    /// Remember `mid`; past the cap, let the oldest go and raise the floor.
    fn insert(&mut self, mid: [u8; 16], ts: u64, now: u64) -> Option<u64> {
        self.inserts = self.inserts.wrapping_add(1);
        if self.inserts.is_multiple_of(512) || self.mids.len() >= MAX_PER_PEER {
            self.prune(now);
        }
        let mut raised = None;
        while self.mids.len() >= MAX_PER_PEER {
            let Some((&old, &old_ts)) = self.mids.iter().min_by_key(|(_, t)| **t) else {
                break;
            };
            self.mids.remove(&old);
            self.floor = self.floor.max(old_ts);
            raised = Some(self.floor);
        }
        self.mids.insert(mid, ts);
        raised
    }
}

/// The ids on disk: an append-only file (a line per id), or kv on a
/// platform without data files.
#[derive(Default)]
struct Store {
    file: Option<std::path::PathBuf>,
    /// Open for appending (`file` only).
    out: Option<std::fs::File>,
    /// What kv holds (no `file`).
    kv_text: String,
    /// Lines appended since the store was last rewritten with only live ids.
    appended: usize,
}

/// Envelope state: who reads version 2, the ids seen, the refusals.
#[derive(Default)]
pub struct Guard {
    /// node id -> 1 (said `env: 2`) or 2 (sent version 2).
    peers: Mutex<HashMap<String, u8>>,
    seen: Mutex<HashMap<String, PeerSeen>>,
    stats: Mutex<HashMap<String, EnvelopeInfo>>,
    store: Mutex<Store>,
}

fn line_ok(node: &str) -> bool {
    !node.is_empty() && !node.contains(['\t', '\n', '\r'])
}

impl Guard {
    /// The state from last run: who reads version 2, and the ids still in
    /// the window (the store is rewritten with only those).
    pub fn load<P: Platform + ?Sized>(platform: &P) -> Guard {
        let g = Guard::default();
        if let Some(text) = platform.kv_get(PEERS_KEY) {
            if let Ok(map) = serde_json::from_str::<HashMap<String, u8>>(&text) {
                *g.peers.lock() = map;
            }
        }
        let file = platform.data_file(SEEN_FILE);
        let text = match &file {
            Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
            None => platform.kv_get(SEEN_KEY).unwrap_or_default(),
        };
        let now = now_ms();
        {
            let mut seen = g.seen.lock();
            for line in text.lines() {
                let mut f = line.split('\t');
                let (Some(node), Some(mid), Some(ts)) = (f.next(), f.next(), f.next()) else {
                    continue;
                };
                let Ok(ts) = ts.parse::<u64>() else { continue };
                if !line_ok(node) || (seen.len() >= MAX_PEERS && !seen.contains_key(node)) {
                    continue;
                }
                let p = seen.entry(node.to_string()).or_default();
                if mid == "*" {
                    p.floor = p.floor.max(ts);
                } else if let Some(mid) = parse_mid(mid) {
                    if ts.saturating_add(WINDOW_MS) >= now {
                        p.insert(mid, ts, now);
                    }
                }
            }
            seen.retain(|_, p| !p.mids.is_empty() || p.floor.saturating_add(WINDOW_MS) >= now);
        }
        *g.store.lock() = Store {
            file,
            ..Default::default()
        };
        g.compact(platform);
        g
    }

    /// Rewrite the store with only the live ids (and floors).
    fn compact<P: Platform + ?Sized>(&self, platform: &P) {
        let text = {
            let seen = self.seen.lock();
            let mut text = String::new();
            for (node, p) in seen.iter() {
                if p.floor > 0 {
                    text.push_str(&format!("{node}\t*\t{}\n", p.floor));
                }
                for (mid, ts) in &p.mids {
                    text.push_str(&format!("{node}\t{}\t{ts}\n", hex(mid)));
                }
            }
            text
        };
        let mut store = self.store.lock();
        store.appended = 0;
        store.out = None;
        match store.file.clone() {
            Some(path) => {
                let tmp = path.with_extension("tmp");
                if std::fs::write(&tmp, &text).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            }
            None => {
                platform.kv_set(SEEN_KEY, &text);
                store.kv_text = text;
            }
        }
    }

    /// Write one line to the store as it arrives (before it's acted on).
    fn persist<P: Platform + ?Sized>(&self, platform: &P, line: &str) {
        let live: usize = self.seen.lock().values().map(|p| p.mids.len()).sum();
        let compact = {
            let mut store = self.store.lock();
            store.appended += 1;
            match store.file.clone() {
                Some(path) => {
                    if store.out.is_none() {
                        store.out = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&path)
                            .ok();
                    }
                    if let Some(f) = store.out.as_mut() {
                        if f.write_all(line.as_bytes()).is_err() {
                            store.out = None;
                        }
                    }
                }
                None => {
                    store.kv_text.push_str(line);
                    platform.kv_set(SEEN_KEY, &store.kv_text);
                }
            }
            store.appended > 4096.max(live * 2)
        };
        if compact {
            self.compact(platform);
        }
    }

    /// `node` said (`flag`: its status' or `addr_update`'s `env`) which
    /// envelope version it reads.
    pub fn advertised<P: Platform + ?Sized>(&self, platform: &P, node: &str, flag: &Value) {
        if flag.as_u64().is_some_and(|v| v >= u64::from(VERSION)) {
            self.raise(platform, node, 1);
        }
    }

    /// Note what `node` reads (never lowers it), and keep it if it changed.
    fn raise<P: Platform + ?Sized>(&self, platform: &P, node: &str, level: u8) {
        if node.is_empty() || node.starts_with("manual:") {
            return;
        }
        let text = {
            let mut peers = self.peers.lock();
            if peers.get(node).copied().unwrap_or(0) >= level
                || (peers.len() >= MAX_PEERS && !peers.contains_key(node))
            {
                return;
            }
            peers.insert(node.to_string(), level);
            serde_json::to_string(&*peers).unwrap_or_default()
        };
        platform.kv_set(PEERS_KEY, &text);
    }

    /// Send `node` version 2 (it said it reads it, or sent one).
    pub fn sends_v2(&self, node: &str) -> bool {
        self.peers.lock().get(node).copied().unwrap_or(0) >= 1
    }

    /// `node` was seen sending version 2: refuse its version 1.
    pub fn requires_v2(&self, node: &str) -> bool {
        self.peers.lock().get(node).copied().unwrap_or(0) >= 2
    }

    /// A version 1 envelope from `node` opened: may it be used?
    pub fn check_v1(&self, node: &str) -> Result<(), Reject> {
        if self.requires_v2(node) {
            return Err(Reject::Downgrade);
        }
        Ok(())
    }

    /// A version 2 envelope that opened (`hdr`), which came from `from` (the
    /// link's sender) to `me`: may it be used? On yes, its id is kept (and
    /// written down when `persist`: it came over the relay or Bluetooth).
    pub fn check<P: Platform + ?Sized>(
        &self,
        platform: &P,
        me: &str,
        from: &str,
        hdr: &Header,
        persist: bool,
    ) -> Result<(), Reject> {
        self.check_at(platform, me, from, hdr, persist, now_ms())
    }

    fn check_at<P: Platform + ?Sized>(
        &self,
        platform: &P,
        me: &str,
        from: &str,
        hdr: &Header,
        persist: bool,
        now: u64,
    ) -> Result<(), Reject> {
        if hdr.dst != me {
            return Err(Reject::WrongRecipient);
        }
        if hdr.src != from {
            return Err(Reject::Spoofed);
        }
        let skew = i128::from(hdr.ts) - i128::from(now);
        let shown = i32::try_from(skew).unwrap_or(if skew < 0 { i32::MIN } else { i32::MAX });
        self.with_stats(from, |s| s.skew_ms = Some(shown));
        if skew.unsigned_abs() > u128::from(WINDOW_MS) {
            return Err(Reject::Stale);
        }
        let raised = {
            let mut seen = self.seen.lock();
            if seen.len() >= MAX_PEERS && !seen.contains_key(from) {
                // Only an account device gets this far; there aren't 256.
                seen.retain(|_, p| !p.mids.is_empty());
            }
            let p = seen.entry(from.to_string()).or_default();
            if hdr.ts <= p.floor || p.mids.contains_key(&hdr.mid) {
                return Err(Reject::Replay);
            }
            p.insert(hdr.mid, hdr.ts, now)
        };
        if persist && line_ok(from) {
            let mut line = format!("{from}\t{}\t{}\n", hex(&hdr.mid), hdr.ts);
            if let Some(floor) = raised {
                line.push_str(&format!("{from}\t*\t{floor}\n"));
            }
            self.persist(platform, &line);
        }
        self.raise(platform, from, 2);
        self.with_stats(from, |s| s.accepted = s.accepted.saturating_add(1));
        Ok(())
    }

    fn with_stats(&self, node: &str, f: impl FnOnce(&mut EnvelopeInfo)) {
        let mut stats = self.stats.lock();
        if stats.len() >= MAX_PEERS && !stats.contains_key(node) {
            return;
        }
        f(stats.entry(node.to_string()).or_default());
    }

    /// Count a refusal from `node`; returns how many of that kind so far.
    pub fn count(&self, node: &str, why: Reject) -> u32 {
        let mut n = 0;
        self.with_stats(node, |s| {
            let c = match why {
                Reject::Replay => &mut s.replays,
                Reject::Stale => &mut s.stale,
                Reject::Downgrade => &mut s.downgrades,
                Reject::WrongRecipient => &mut s.wrong_recipient,
                Reject::Spoofed => &mut s.spoofed,
            };
            *c = c.saturating_add(1);
            n = *c;
        });
        n
    }

    /// The report on `node`'s envelopes.
    pub fn info(&self, node: &str) -> EnvelopeInfo {
        let level = self.peers.lock().get(node).copied().unwrap_or(0);
        let mut info = self.stats.lock().get(node).cloned().unwrap_or_default();
        info.version = if level >= 1 { VERSION } else { 1 };
        info.v2_only = level >= 2;
        info
    }
}

/// Say in this device's status (and `addr_update`) that it reads version 2.
pub fn extend_status<P: Platform>(ctx: &crate::engine::NetCtx<P>, status: &mut Value) {
    if ctx.config.envelope_v2 {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`: its status' or `addr_update`'s `env`) which version
/// it reads.
pub fn remember<P: Platform>(ctx: &crate::engine::NetCtx<P>, node: &str, flag: &Value) {
    if ctx.config.envelope_v2 {
        ctx.env.advertised(&*ctx.platform, node, flag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Kv(Mutex<HashMap<String, String>>);
    impl Platform for Kv {
        fn device_id(&self) -> String {
            "me".into()
        }
        fn device_name(&self) -> String {
            "me".into()
        }
        fn iface_mode(&self) -> crate::IfaceMode {
            crate::IfaceMode::Single(std::net::Ipv4Addr::LOCALHOST)
        }
        fn kv_get(&self, k: &str) -> Option<String> {
            self.0.lock().get(k).cloned()
        }
        fn kv_set(&self, k: &str, v: &str) {
            self.0.lock().insert(k.into(), v.into());
        }
    }
    fn kv() -> Kv {
        Kv(Mutex::new(HashMap::new()))
    }

    const KEY: [u8; 32] = [9u8; 32];

    fn hdr(mid: u8, ts: u64) -> Header {
        Header {
            src: "a".into(),
            dst: "me".into(),
            mid: [mid; 16],
            ts,
        }
    }

    #[test]
    fn seal_then_open_round_trips_and_binds_the_header() {
        let env = seal(&KEY, "a", "b", b"hi").unwrap();
        let (h, pt) = open(&KEY, &env).unwrap();
        assert_eq!(pt, b"hi");
        assert_eq!((h.src.as_str(), h.dst.as_str()), ("a", "b"));
        let tweak: [fn(&mut Envelope); 5] = [
            |e| e.src = "c".into(),
            |e| e.dst = "c".into(),
            |e| e.ts += 1,
            |e| e.mid = "00".repeat(16),
            |e| e.v = 1,
        ];
        for t in tweak {
            let mut e = env.clone();
            t(&mut e);
            assert!(open(&KEY, &e).is_none(), "{e:?}");
        }
        assert!(open(&[1u8; 32], &env).is_none(), "other key");
        // Not a version 1 envelope either.
        assert!(crate::crypto::decrypt(&KEY, &env).is_none());
    }

    #[test]
    fn the_aad_is_unambiguous() {
        let m = [0u8; 16];
        assert_ne!(aad("ab", "c", &m, 1), aad("a", "bc", &m, 1));
        assert_ne!(aad("a", "b", &m, 1), aad("a", "b", &m, 2));
    }

    #[test]
    fn checks_recipient_sender_window_and_id() {
        let p = kv();
        let g = Guard::load(&p);
        let now = 10 * WINDOW_MS;
        let ok = hdr(1, now);
        assert_eq!(
            g.check_at(&p, "other", "a", &ok, true, now),
            Err(Reject::WrongRecipient)
        );
        assert_eq!(
            g.check_at(&p, "me", "b", &ok, true, now),
            Err(Reject::Spoofed)
        );
        let old = hdr(2, now - WINDOW_MS - 1);
        assert_eq!(
            g.check_at(&p, "me", "a", &old, true, now),
            Err(Reject::Stale)
        );
        let ahead = hdr(3, now + WINDOW_MS + 1);
        assert_eq!(
            g.check_at(&p, "me", "a", &ahead, true, now),
            Err(Reject::Stale)
        );
        assert_eq!(
            g.check_at(&p, "me", "a", &hdr(4, now - WINDOW_MS), true, now),
            Ok(())
        );
        assert_eq!(g.check_at(&p, "me", "a", &ok, true, now), Ok(()));
        assert_eq!(
            g.check_at(&p, "me", "a", &ok, true, now),
            Err(Reject::Replay)
        );
        assert_eq!(g.info("a").skew_ms, Some(0));
        // Seen sending version 2: version 1 is a downgrade now.
        assert!(g.requires_v2("a"));
        assert_eq!(g.check_v1("a"), Err(Reject::Downgrade));
        assert_eq!(g.check_v1("b"), Ok(()));
    }

    #[test]
    fn a_restart_keeps_the_ids_and_the_versions() {
        let p = kv();
        let now = now_ms();
        {
            let g = Guard::load(&p);
            assert_eq!(g.check(&p, "me", "a", &hdr(1, now), true), Ok(()));
            // Over QUIC: not written down.
            assert_eq!(g.check(&p, "me", "a", &hdr(2, now), false), Ok(()));
            g.advertised(&p, "c", &Value::from(2));
        }
        let g = Guard::load(&p);
        assert_eq!(
            g.check(&p, "me", "a", &hdr(1, now), true),
            Err(Reject::Replay)
        );
        assert!(g.requires_v2("a"));
        assert!(g.sends_v2("c") && !g.requires_v2("c"));
    }

    #[test]
    fn past_the_cap_the_floor_rises() {
        let mut p = PeerSeen::default();
        let now = 10 * WINDOW_MS;
        for i in 0..MAX_PER_PEER as u64 {
            let mut mid = [0u8; 16];
            mid[..8].copy_from_slice(&i.to_be_bytes());
            assert!(p.insert(mid, now - 1000 + i % 7, now).is_none());
        }
        assert_eq!(p.insert([0xff; 16], now, now), Some(now - 1000));
        assert_eq!(p.mids.len(), MAX_PER_PEER);
    }
}
