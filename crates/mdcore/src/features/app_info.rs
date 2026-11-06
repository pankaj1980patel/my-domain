//! Which app each of your devices runs, for the health check's "app
//! version" line: this device says its version (`app_v`) in its status and
//! the `addr_update` every link starts with, and keeps what each of the
//! others said, along with which of the newer features' flags they sent
//! (`FEATURE_FLAGS`). A device that sends no `app_v` runs an app from
//! before this was added; what it lacks is read from its flags.
//!
//!   status / addr_update: { ..., app_v: "0.4.1" }
//!
//! Also kept here: when a push last reached this device (`push_seen`),
//! for the health check's "push working" line. Both are kept in the
//! platform's store (`peer_apps`, `push_seen_at`) so the check knows them
//! after a restart.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::platform::Platform;
use crate::transport::now_secs;

/// The status / `addr_update` field carrying this device's app version.
pub const FLAG: &str = "app_v";

/// The newer features' flags, as they appear in a status or `addr_update`
/// (each feature's own `FLAG`). Each one is sent by every app that knows
/// the feature, whatever the platform, so a device that leaves one out
/// runs an app from before it (or has it turned off).
pub const FEATURE_FLAGS: [&str; 12] = [
    "ping",
    "links_map",
    "nearby",
    "camera_view",
    "timers",
    "notes",
    "read_later",
    "insert_capture",
    "remote_download",
    "notif_rich",
    "xfer",
    "media_ctl",
];

const PEERS_KEY: &str = "peer_apps";
const PUSH_KEY: &str = "push_seen_at";
/// The longest version string kept.
pub const MAX_VERSION: usize = 32;
const MAX_DEVICES: usize = 256;

/// What one of your devices said about its app.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PeerApp {
    /// Its version; none: an app from before versions were said.
    pub version: Option<String>,
    /// Which of `FEATURE_FLAGS` it sent.
    pub flags: Vec<String>,
}

/// Everything this feature keeps.
#[derive(Default)]
pub struct AppInfo {
    own: Mutex<Option<String>>,
    /// node -> what it said; `None` until read from `kv`.
    peers: Mutex<Option<BTreeMap<String, PeerApp>>>,
    /// When a push last arrived (unix secs); outer `None` until read.
    push_seen: Mutex<Option<Option<u64>>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &AppInfo {
    &ctx.extras.app_info
}

/// A version as said: trimmed, at most `MAX_VERSION` characters of digits,
/// letters, `.`, `-` and `+`, starting with a digit (a leading `v` is
/// dropped). Anything else: none.
pub fn clean_version(v: &str) -> Option<String> {
    let v = v.trim();
    let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
    let ok = !v.is_empty()
        && v.len() <= MAX_VERSION
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    ok.then(|| v.to_string())
}

/// Compare two versions the way people number apps: dot-separated parts,
/// numbers as numbers (`0.10` after `0.9`), a missing part is 0 (`1.2` ==
/// `1.2.0`), a pre-release (`1.0.0-beta.2`) before its release, and build
/// metadata (`+abc`) ignored. Parts that aren't numbers compare as text,
/// after numbers.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (&str, Option<&str>) {
        let v = v.trim();
        let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
        let v = v.split('+').next().unwrap_or(v);
        match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (v, None),
        }
    }
    fn parts(s: &str) -> Vec<&str> {
        s.split('.').collect()
    }
    fn cmp_part(x: &str, y: &str) -> Ordering {
        match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        }
    }
    let (ca, pa) = split(a);
    let (cb, pb) = split(b);
    let (xa, xb) = (parts(ca), parts(cb));
    for i in 0..xa.len().max(xb.len()) {
        let x = xa.get(i).copied().filter(|s| !s.is_empty()).unwrap_or("0");
        let y = xb.get(i).copied().filter(|s| !s.is_empty()).unwrap_or("0");
        let o = cmp_part(x, y);
        if o != Ordering::Equal {
            return o;
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (px, py) = (parts(x), parts(y));
            for i in 0..px.len().max(py.len()) {
                match (px.get(i), py.get(i)) {
                    (Some(a), Some(b)) => {
                        let o = cmp_part(a, b);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (None, None) => break,
                }
            }
            Ordering::Equal
        }
    }
}

/// Which of `FEATURE_FLAGS` `msg` (a status or an `addr_update`) carries,
/// in `FEATURE_FLAGS` order.
pub fn flags_in(msg: &Value) -> Vec<String> {
    FEATURE_FLAGS
        .iter()
        .filter(|f| msg.get(**f).is_some_and(|v| !v.is_null()))
        .map(|f| f.to_string())
        .collect()
}

/// The flags `own` has that `theirs` lacks, in `own`'s order.
pub fn missing(own: &[String], theirs: &[String]) -> Vec<String> {
    own.iter()
        .filter(|f| !theirs.contains(f))
        .cloned()
        .collect()
}

// --- this device -------------------------------------------------------------

/// The app said its version. Linked devices hear a change at once.
pub fn set_own<P: Platform>(ctx: &NetCtx<P>, version: &str) {
    let Some(v) = clean_version(version) else {
        return;
    };
    let changed = {
        let mut own = st(ctx).own.lock();
        let changed = own.as_deref() != Some(v.as_str());
        *own = Some(v);
        changed
    };
    if changed && !ctx.p2p_conns.lock().is_empty() {
        super::addr_update::announce(ctx, None);
    }
}

/// This device's app version (`None` until the app said it).
pub fn own<P: Platform>(ctx: &NetCtx<P>) -> Option<String> {
    st(ctx).own.lock().clone()
}

/// Add this device's version to a status reply or `addr_update`.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if let Some(v) = own(ctx) {
        status[FLAG] = v.into();
    }
}

/// Which of `FEATURE_FLAGS` this device sends (as configured).
pub fn own_flags<P: Platform>(ctx: &NetCtx<P>) -> Vec<String> {
    let mut v = json!({});
    super::diag::extend_status(ctx, &mut v);
    super::links_map::extend_status(ctx, &mut v);
    super::nearby::extend_status(ctx, &mut v);
    super::camera::extend_status(ctx, &mut v);
    super::timers::extend_status(ctx, &mut v);
    super::notes::extend_status(ctx, &mut v);
    super::read_later::extend_status(ctx, &mut v);
    super::insert_capture::extend_status(ctx, &mut v);
    super::remote_download::extend_status(ctx, &mut v);
    super::notif_rich::extend_status(ctx, &mut v);
    super::batches::extend_status(ctx, &mut v);
    super::media::extend_status(ctx, &mut v);
    flags_in(&v)
}

// --- your other devices ------------------------------------------------------

fn with_peers<P: Platform, R>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut BTreeMap<String, PeerApp>) -> R,
) -> R {
    let mut guard = st(ctx).peers.lock();
    let map = guard.get_or_insert_with(|| {
        ctx.platform
            .kv_get(PEERS_KEY)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    f(map)
}

/// `node`'s status or `addr_update` (`msg`, the whole object) arrived:
/// keep its version and flags.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &Value) {
    if node.is_empty() || node.len() > 128 || node.starts_with("manual:") || !msg.is_object() {
        return;
    }
    let app = PeerApp {
        version: msg
            .get(FLAG)
            .and_then(Value::as_str)
            .and_then(clean_version),
        flags: flags_in(msg),
    };
    let saved = with_peers(ctx, |map| {
        if map.get(node) == Some(&app) {
            return None;
        }
        map.insert(node.to_string(), app);
        while map.len() > MAX_DEVICES {
            let Some(first) = map.keys().find(|k| k.as_str() != node).cloned() else {
                break;
            };
            map.remove(&first);
        }
        serde_json::to_string(&*map).ok()
    });
    if let Some(s) = saved {
        ctx.platform.kv_set(PEERS_KEY, &s);
    }
}

/// What `node` said about its app (`None`: it hasn't said anything since
/// this device kept count).
pub fn peer<P: Platform>(ctx: &NetCtx<P>, node: &str) -> Option<PeerApp> {
    with_peers(ctx, |map| map.get(node).cloned())
}

/// Every device's app that's known.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> BTreeMap<String, PeerApp> {
    with_peers(ctx, |map| map.clone())
}

// --- pushes ------------------------------------------------------------------

/// A push reached this device now.
pub fn push_seen<P: Platform>(ctx: &NetCtx<P>) {
    let now = now_secs();
    *st(ctx).push_seen.lock() = Some(Some(now));
    ctx.platform.kv_set(PUSH_KEY, &now.to_string());
}

/// Seconds since a push last reached this device (none: never, as far as
/// this device kept count).
pub fn push_seen_ago<P: Platform>(ctx: &NetCtx<P>) -> Option<u64> {
    let at = *st(ctx).push_seen.lock().get_or_insert_with(|| {
        ctx.platform
            .kv_get(PUSH_KEY)
            .and_then(|s| s.trim().parse().ok())
    });
    at.map(|t| now_secs().saturating_sub(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_like_app_numbers() {
        use Ordering::*;
        let cases = [
            ("0.1.0", "0.1.0", Equal),
            ("0.9.0", "0.10.0", Less),
            ("1.2", "1.2.0", Equal),
            ("1.2.1", "1.2", Greater),
            ("2.0.0", "10.0.0", Less),
            ("1.0.0-beta.2", "1.0.0", Less),
            ("1.0.0-beta.2", "1.0.0-beta.10", Less),
            ("1.0.0-alpha", "1.0.0-beta", Less),
            ("1.0.0-rc.1", "1.0.0-rc.1.1", Less),
            ("1.0.0+abc", "1.0.0+xyz", Equal),
            ("v1.4.0", "1.4", Equal),
            ("1.x", "1.0", Greater),
        ];
        for (a, b, want) in cases {
            assert_eq!(compare_versions(a, b), want, "{a} vs {b}");
            assert_eq!(compare_versions(b, a), want.reverse(), "{b} vs {a}");
        }
    }

    #[test]
    fn versions_are_cleaned() {
        assert_eq!(clean_version(" 0.4.1 ").as_deref(), Some("0.4.1"));
        assert_eq!(
            clean_version("v1.0.0-beta+7").as_deref(),
            Some("1.0.0-beta+7")
        );
        assert_eq!(clean_version(""), None);
        assert_eq!(clean_version("latest"), None);
        assert_eq!(clean_version("1.0 beta"), None);
        assert_eq!(clean_version("1.0;rm"), None);
        assert_eq!(clean_version(&"1".repeat(MAX_VERSION + 1)), None);
    }

    #[test]
    fn flags_are_read_and_compared() {
        let msg =
            json!({"type": "addr_update", "xfer": 1, "links_map": 1, "media_ctl": null, "kvm": 1});
        assert_eq!(flags_in(&msg), vec!["links_map", "xfer"]);
        let own: Vec<String> = ["links_map", "xfer", "media_ctl"]
            .map(String::from)
            .to_vec();
        assert_eq!(missing(&own, &flags_in(&msg)), vec!["media_ctl"]);
        assert!(missing(&own, &own).is_empty());
    }
}
