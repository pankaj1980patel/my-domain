//! Per-device permissions for what lets another device act on this one or
//! see it: off by default, allowed device by device.
//!
//! - `control`: remote input (a phone as touchpad and keyboard,
//!   `features::input`) plays back here.
//! - `screen`: another device may watch this screen (`features::screen`).
//! - `kvm`: another computer may take over this one's mouse and keyboard
//!   (`features::kvm`).
//!
//! Each is an allow-list of node ids, kept in `kv` (`perm_allow:<perm>`),
//! empty on a fresh install and for every new pairing. Turning one off
//! takes effect at once: a watch of this screen by that device ends, and
//! a mouse-and-keyboard session it drives lets go.
//!
//!   perm_refused { from, perm, why }   target → requester: "not allowed
//!                                       here" (`why`: "not_allowed")
//!
//! - A request that isn't allowed is refused, and the requester told why
//!   (`perm_refused`, only to devices that say `perms` in their status:
//!   an older one would show it as chat; it's simply refused. Share mouse
//!   and keyboard says it in its own `kvm_ack { ok: false, why:
//!   "not_allowed" }`, which an older computer reads as "ended"). The
//!   requester gets `CoreEvent::PermissionRefused`.
//! - The target's host is asked (`CoreEvent::PermissionAsk`, at most every
//!   `ASK_EVERY` per device and permission, and not for `DENY_QUIET` after
//!   "Don't allow"): it shows Allow once / Always / Don't allow, and
//!   answers with `answer`. Nothing is ever allowed without that answer.
//! - "Allow once" lasts while it's used: it ends after `ONCE_IDLE` without
//!   a request from that device, when the session ends (the watch stops,
//!   the pointer comes back), or when the app quits.
//! - Upgrading from the global switches (on unless turned off) doesn't keep
//!   them on: the first run of this version turns them all off and keeps a
//!   one-time notice (`notice`) listing what was on, for the host to show
//!   with "Allow for <device>" buttons.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::NetCtx;
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

pub const FLAG: &str = "perms";
const VERSION: u64 = 1;

/// The host is asked again for the same device and permission at most this
/// often.
pub const ASK_EVERY: Duration = Duration::from_secs(30);
/// After "Don't allow", the host isn't asked again for this long.
pub const DENY_QUIET: Duration = Duration::from_secs(10 * 60);
/// A refusal is sent to the same device for the same permission at most
/// this often (input comes many times a second).
pub const REFUSE_EVERY: Duration = Duration::from_secs(3);
/// "Allow once" ends after this long without a request.
pub const ONCE_IDLE: Duration = Duration::from_secs(10 * 60);

const LIST_PREFIX: &str = "perm_allow:";
const MIGRATED_KEY: &str = "perms_migrated";
const NOTICE_KEY: &str = "perms_notice";
/// The global switches from before (on unless set to "off").
const OLD_CONTROL_KEY: &str = "remote_input";
const OLD_SCREEN_KEY: &str = "screen_share";
const OLD_KVM_KEY: &str = "kvm_allowed";

/// What a device may be allowed to do here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Perm {
    /// Remote input: control this computer.
    Control,
    /// See this screen.
    Screen,
    /// Take over this computer's mouse and keyboard.
    Kvm,
}

impl Perm {
    pub const ALL: [Perm; 3] = [Perm::Control, Perm::Screen, Perm::Kvm];

    pub fn parse(s: &str) -> Option<Perm> {
        match s {
            "control" => Some(Perm::Control),
            "screen" => Some(Perm::Screen),
            "kvm" => Some(Perm::Kvm),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Perm::Control => "control",
            Perm::Screen => "screen",
            Perm::Kvm => "kvm",
        }
    }

    fn key(self) -> String {
        format!("{LIST_PREFIX}{}", self.as_str())
    }
}

/// What one device is allowed to do here (always, not "once").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PeerPermissions {
    pub node_id: String,
    /// Control this computer (remote input).
    pub control: bool,
    /// See this screen.
    pub screen: bool,
    /// Take over this computer's mouse and keyboard.
    pub kvm: bool,
}

impl PeerPermissions {
    pub fn any(&self) -> bool {
        self.control || self.screen || self.kvm
    }
}

/// The one-time notice after the upgrade: these were on for every device
/// and are now off until allowed device by device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PermissionNotice {
    /// "control", "screen", "kvm".
    pub features: Vec<String>,
}

type Key = (Perm, String);

#[derive(Default)]
pub struct Perms {
    /// `None` until read from `kv`.
    lists: Mutex<Option<HashMap<Perm, BTreeSet<String>>>>,
    /// "Allow once": when last used.
    once: Mutex<HashMap<Key, Instant>>,
    /// When the host was last asked.
    asked: Mutex<HashMap<Key, Instant>>,
    /// When the owner last said "Don't allow".
    denied: Mutex<HashMap<Key, Instant>>,
    /// When a refusal was last sent.
    refused: Mutex<HashMap<Key, Instant>>,
    /// node → it says `perms` (reads `perm_refused`).
    caps: Mutex<HashMap<String, bool>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &Perms {
    &ctx.extras.perms
}

fn with_lists<P: Platform, R>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut HashMap<Perm, BTreeSet<String>>) -> R,
) -> R {
    let mut g = st(ctx).lists.lock();
    let lists = g.get_or_insert_with(|| {
        Perm::ALL
            .iter()
            .map(|p| {
                let set: BTreeSet<String> = ctx
                    .platform
                    .kv_get(&p.key())
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default();
                (*p, set)
            })
            .collect()
    });
    f(lists)
}

fn save<P: Platform>(ctx: &NetCtx<P>, perm: Perm, set: &BTreeSet<String>) {
    let s = serde_json::to_string(set).unwrap_or_else(|_| "[]".into());
    ctx.platform.kv_set(&perm.key(), &s);
}

// --- capability ---------------------------------------------------------------

/// Say in this device's status (and `addr_update`) that it reads
/// `perm_refused`.
pub fn extend_status<P: Platform>(_ctx: &NetCtx<P>, status: &mut Value) {
    status[FLAG] = VERSION.into();
}

/// `node` said (`flag`: its status' or `addr_update`'s `perms`) whether it
/// reads `perm_refused`.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    st(ctx).caps.lock().insert(node.to_string(), on);
}

/// `node` reads `perm_refused`.
pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    st(ctx).caps.lock().get(node).copied().unwrap_or(false)
}

// --- the lists ------------------------------------------------------------------

/// Whether `node` may do `perm` here now: allowed always, or once and
/// still in use (which this keeps alive).
pub fn allowed<P: Platform>(ctx: &NetCtx<P>, perm: Perm, node: &str) -> bool {
    if node.is_empty() {
        return false;
    }
    if with_lists(ctx, |l| l.get(&perm).is_some_and(|s| s.contains(node))) {
        return true;
    }
    let mut once = st(ctx).once.lock();
    let key = (perm, node.to_string());
    match once.get_mut(&key) {
        Some(at) if at.elapsed() < ONCE_IDLE => {
            *at = Instant::now();
            true
        }
        Some(_) => {
            once.remove(&key);
            false
        }
        None => false,
    }
}

/// Whether `node` is allowed `perm` always (the switch on its page).
pub fn always<P: Platform>(ctx: &NetCtx<P>, perm: Perm, node: &str) -> bool {
    with_lists(ctx, |l| l.get(&perm).is_some_and(|s| s.contains(node)))
}

/// What `node` is allowed here.
pub fn of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> PeerPermissions {
    PeerPermissions {
        node_id: node.to_string(),
        control: always(ctx, Perm::Control, node),
        screen: always(ctx, Perm::Screen, node),
        kvm: always(ctx, Perm::Kvm, node),
    }
}

/// Every device allowed anything here, by node id.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> Vec<PeerPermissions> {
    let nodes: BTreeSet<String> = with_lists(ctx, |l| l.values().flatten().cloned().collect());
    nodes.iter().map(|n| of(ctx, n)).collect()
}

/// Allow `node` to do `perm` here always (`on`), or not. Off takes effect
/// at once: a session of that kind with it ends, and a once ends too.
pub fn set<P: Platform>(ctx: &NetCtx<P>, perm: Perm, node: &str, on: bool) -> crate::Result<()> {
    if node.is_empty() || node.len() > 128 {
        return Err(CoreError::InvalidInput("not a device".into()));
    }
    let changed = with_lists(ctx, |l| {
        let set = l.entry(perm).or_default();
        let changed = if on {
            set.insert(node.to_string())
        } else {
            set.remove(node)
        };
        if changed {
            save(ctx, perm, set);
        }
        changed
    });
    if !on {
        let had_once = st(ctx)
            .once
            .lock()
            .remove(&(perm, node.to_string()))
            .is_some();
        if changed || had_once {
            end_sessions(ctx, perm, node);
        }
    } else {
        st(ctx).denied.lock().remove(&(perm, node.to_string()));
    }
    if changed {
        ctx.sink.emit(CoreEvent::PermissionsChanged {
            node_id: node.to_string(),
        });
    }
    Ok(())
}

/// `node` lost `perm`: what it was doing of that kind ends now.
fn end_sessions<P: Platform>(ctx: &NetCtx<P>, perm: Perm, node: &str) {
    match perm {
        Perm::Control => {}
        Perm::Screen => super::screen::drop_watcher(ctx, node),
        Perm::Kvm => super::kvm::revoked(ctx, node),
    }
}

/// A session `perm` with `node` ended: an "allow once" for it is used up.
pub fn session_ended<P: Platform>(ctx: &NetCtx<P>, perm: Perm, node: &str) {
    st(ctx).once.lock().remove(&(perm, node.to_string()));
}

/// `node` was removed from your devices: it's allowed nothing any more
/// (paired again, it starts with nothing allowed).
pub fn forget<P: Platform>(ctx: &NetCtx<P>, node: &str) {
    for perm in Perm::ALL {
        let _ = set(ctx, perm, node, false);
    }
    let key_match = |k: &Key| k.1 == node;
    st(ctx).once.lock().retain(|k, _| !key_match(k));
    st(ctx).asked.lock().retain(|k, _| !key_match(k));
    st(ctx).denied.lock().retain(|k, _| !key_match(k));
}

/// The owner's answer to `CoreEvent::PermissionAsk`: "once", "always" or
/// "deny".
pub fn answer<P: Platform>(
    ctx: &NetCtx<P>,
    perm: Perm,
    node: &str,
    answer: &str,
) -> crate::Result<()> {
    if !super::known(ctx, node) {
        return Err(CoreError::PeerNotFound);
    }
    let key = (perm, node.to_string());
    st(ctx).asked.lock().remove(&key);
    match answer {
        "once" => {
            st(ctx).denied.lock().remove(&key);
            st(ctx).once.lock().insert(key, Instant::now());
            Ok(())
        }
        "always" => set(ctx, perm, node, true),
        "deny" => {
            st(ctx).denied.lock().insert(key, Instant::now());
            Ok(())
        }
        other => Err(CoreError::InvalidInput(format!("not an answer: {other}"))),
    }
}

/// `from` asked for `perm` and it isn't allowed: tell it why (if it reads
/// `perm_refused` and `tell`), and ask the owner here (not too often,
/// never after a recent "Don't allow").
pub fn refuse<P: Platform>(ctx: &NetCtx<P>, perm: Perm, from: &str, tell: bool) {
    let key = (perm, from.to_string());
    if tell && supports(ctx, from) && due(&st(ctx).refused, &key, REFUSE_EVERY) {
        let me = ctx.identity.lock().node_id.clone();
        let msg = json!({
            "type": "perm_refused", "from": me, "perm": perm.as_str(), "why": "not_allowed",
        });
        crate::transport::send_control_to(ctx, from, &msg.to_string());
    }
    let quiet = st(ctx)
        .denied
        .lock()
        .get(&key)
        .is_some_and(|at| at.elapsed() < DENY_QUIET);
    if !quiet && due(&st(ctx).asked, &key, ASK_EVERY) {
        ctx.sink.emit(CoreEvent::PermissionAsk {
            from: from.to_string(),
            perm: perm.as_str().to_string(),
        });
    }
}

/// Whether `every` has passed since `key` was last stamped in `map`;
/// stamps it if so.
fn due(map: &Mutex<HashMap<Key, Instant>>, key: &Key, every: Duration) -> bool {
    let mut m = map.lock();
    if m.get(key).is_some_and(|at| at.elapsed() < every) {
        return false;
    }
    if m.len() > 1024 {
        m.retain(|_, at| at.elapsed() < DENY_QUIET);
    }
    m.insert(key.clone(), Instant::now());
    true
}

// --- the upgrade from the global switches ---------------------------------------

/// Which of the old global switches were on, as far as this install goes:
/// "on" explicitly, or never set on an install that had paired devices
/// (they were on until turned off). A fresh install (no devices yet)
/// had nothing to lose. `applies`: the permission means something on this
/// device.
pub fn were_on(
    old: impl Fn(Perm) -> Option<String>,
    had_devices: bool,
    applies: impl Fn(Perm) -> bool,
) -> Vec<Perm> {
    Perm::ALL
        .into_iter()
        .filter(|p| applies(*p))
        .filter(|p| match old(*p).as_deref() {
            Some("off") => false,
            Some(_) => true,
            None => had_devices,
        })
        .collect()
}

fn old_key(perm: Perm) -> &'static str {
    match perm {
        Perm::Control => OLD_CONTROL_KEY,
        Perm::Screen => OLD_SCREEN_KEY,
        Perm::Kvm => OLD_KVM_KEY,
    }
}

/// Whether `perm` means anything on this device: controlling it and
/// watching its screen from the core's side are a computer's (a phone
/// shares its screen only when its owner starts it, and asks Android
/// each time), sharing mouse and keyboard where it's available.
fn applies<P: Platform>(ctx: &NetCtx<P>, perm: Perm) -> bool {
    match perm {
        Perm::Control | Perm::Screen => ctx.platform.shares_input(),
        Perm::Kvm => super::kvm::available(ctx),
    }
}

/// Once, at the first start of this version: the global switches are gone
/// (everything starts off), and what was on is kept for the one-time
/// notice.
pub fn migrate<P: Platform>(ctx: &NetCtx<P>) {
    if ctx.platform.kv_get(MIGRATED_KEY).is_some() {
        return;
    }
    let had_devices = ctx.peers.lock().keys().any(|n| !n.starts_with("manual:"));
    let on = were_on(
        |p| ctx.platform.kv_get(old_key(p)),
        had_devices,
        |p| applies(ctx, p),
    );
    if !on.is_empty() {
        let notice = PermissionNotice {
            features: on.iter().map(|p| p.as_str().to_string()).collect(),
        };
        if let Ok(s) = serde_json::to_string(&notice) {
            ctx.platform.kv_set(NOTICE_KEY, &s);
        }
    }
    for p in Perm::ALL {
        ctx.platform.kv_set(old_key(p), "off");
    }
    ctx.platform.kv_set(MIGRATED_KEY, "1");
}

/// The one-time notice, until dismissed.
pub fn notice<P: Platform>(ctx: &NetCtx<P>) -> Option<PermissionNotice> {
    ctx.platform
        .kv_get(NOTICE_KEY)
        .and_then(|s| serde_json::from_str::<PermissionNotice>(&s).ok())
        .filter(|n| !n.features.is_empty())
}

pub fn dismiss_notice<P: Platform>(ctx: &NetCtx<P>) {
    ctx.platform.kv_set(NOTICE_KEY, "");
}

// --- messages -------------------------------------------------------------------

pub struct PermsHandler;

impl<P: Platform> ControlHandler<P> for PermsHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["perm_refused"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        if !super::known(ctx, &from) {
            return;
        }
        let Some(perm) = Perm::parse(&msg.str("perm")) else {
            return;
        };
        let why = match msg.str("why").as_str() {
            "not_allowed" => "not_allowed",
            _ => "refused",
        };
        ctx.sink.emit(CoreEvent::PermissionRefused {
            from,
            perm: perm.as_str().to_string(),
            why: why.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for p in Perm::ALL {
            assert_eq!(Perm::parse(p.as_str()), Some(p));
        }
        assert_eq!(Perm::parse("input"), None);
    }

    #[test]
    fn what_was_on_before_the_upgrade() {
        let all = |_: Perm| true;
        // A fresh install: nothing set, no devices: nothing to tell.
        assert!(were_on(|_| None, false, all).is_empty());
        // Devices paired, never touched: they were all on.
        assert_eq!(were_on(|_| None, true, all), Perm::ALL.to_vec());
        // Turned off before: not listed; turned on explicitly: listed.
        let old = |p: Perm| match p {
            Perm::Control => Some("off".to_string()),
            Perm::Screen => Some("on".to_string()),
            Perm::Kvm => None,
        };
        assert_eq!(were_on(old, true, all), [Perm::Screen, Perm::Kvm]);
        assert_eq!(were_on(old, false, all), [Perm::Screen]);
        // Only what means something here (a phone: none of these).
        assert!(were_on(|_| None, true, |_| false).is_empty());
        assert_eq!(were_on(|_| None, true, |p| p == Perm::Kvm), [Perm::Kvm]);
    }
}
