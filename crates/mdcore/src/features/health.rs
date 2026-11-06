//! The health check both apps show (Settings → Health, and from a
//! device's page when something is wrong): a list of checks, each ok
//! (green), info (grey: nothing to do, or can't be read), warn (amber) or
//! bad (red), with the one thing to tap that fixes it.
//!
//! The core knows the server, pushes, each device's link (and why it
//! isn't direct, from the links map), keys that changed and each device's
//! app version; the app adds what only it can read (`PlatformFacts`):
//! permissions with the feature groups that need them, and free space
//! where files are saved. `evaluate` turns both into the report; it's
//! pure, so every rule is unit-tested. Permissions only count for the
//! feature groups the user picked in first-run setup
//! (`features::onboarding`; all of them until something was picked).
//!
//! Every `reason` and `fix` is a short code the apps put into words.

use serde::{Deserialize, Serialize};

use crate::engine::NetCtx;
use crate::platform::Platform;

use super::app_info::{self, PeerApp};
use super::reconnect::DeviceReach;

/// How a check came out, worst last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum HealthLevel {
    Ok,
    /// Nothing to fix (or it can't be read): shown grey.
    Info,
    Warn,
    Bad,
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct HealthCheck {
    /// What's checked: "server", "push", "disk", "permission", "key",
    /// "link", "version".
    pub id: String,
    pub level: HealthLevel,
    /// A code saying how it came out (see each check).
    pub reason: String,
    /// The device it's about, and its name.
    pub node_id: Option<String>,
    pub name: Option<String>,
    /// More to show: the permission's id, how a link is carried, the
    /// reason it isn't direct, a device's version, free space in MB.
    pub detail: Option<String>,
    /// Feature flags an older app lacks (`version`), or the feature groups
    /// that need a permission (`permission`).
    pub features: Vec<String>,
    /// The last reconnect's time (`link`).
    pub reconnect_ms: Option<u32>,
    /// What the button does: "retry", "push_test", "open_permission",
    /// "free_space", "trust_key", "encryption_key", "connect",
    /// "links_map", "update_other", "update_app". None: no button.
    pub fix: Option<String>,
}

impl HealthCheck {
    fn new(id: &str, level: HealthLevel, reason: &str) -> Self {
        HealthCheck {
            id: id.into(),
            level,
            reason: reason.into(),
            node_id: None,
            name: None,
            detail: None,
            features: Vec::new(),
            reconnect_ms: None,
            fix: None,
        }
    }
    fn device(mut self, node: &str, name: &str) -> Self {
        self.node_id = Some(node.into());
        self.name = Some(name.into());
        self
    }
    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
    fn fix(mut self, f: &str) -> Self {
        self.fix = Some(f.into());
        self
    }
}

/// The whole report, worst first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct HealthReport {
    pub checks: Vec<HealthCheck>,
    /// The worst level (`Ok` when there's nothing).
    pub worst: HealthLevel,
    /// How many are red, and amber.
    pub bad: u32,
    pub warn: u32,
}

/// A permission or setting as the app read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PermissionFact {
    /// The app's id for it ("accessibility", "notification_access",
    /// "battery", "autostart", "local_network", ...): the fix opens its
    /// page.
    pub id: String,
    /// The feature groups that need it; empty: everything does.
    #[serde(default)]
    pub features: Vec<String>,
    /// "granted", "denied", "unknown" (it can't be read: shown grey with
    /// its page) or "not_needed" (left out).
    pub state: String,
    /// Off breaks what it's for (red); else amber.
    #[serde(default)]
    pub critical: bool,
}

/// What only the app can read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct PlatformFacts {
    #[serde(default)]
    pub permissions: Vec<PermissionFact>,
    /// Free and total space where received files are saved, in MB (none:
    /// not known).
    #[serde(default)]
    pub disk_free_mb: Option<u32>,
    #[serde(default)]
    pub disk_total_mb: Option<u32>,
}

/// One of your devices, as the core sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceFacts {
    pub node_id: String,
    pub name: String,
    /// The links map's state: a link's `how`, "unreachable" or
    /// "not_connected".
    pub state: String,
    /// Why it's unreachable, or why a relayed link isn't direct.
    pub reason: Option<String>,
    /// "key_changed" or "bad_key".
    pub problem: Option<String>,
    pub reach: Option<DeviceReach>,
    pub reconnect_ms: Option<u32>,
    /// What it said about its app (none: nothing yet).
    pub app: Option<PeerApp>,
}

/// What the core knows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoreFacts {
    pub signed_in: bool,
    /// The server answered (none: not asked this time).
    pub server_ok: Option<bool>,
    /// This device has a push token.
    pub push_token: bool,
    pub push_seen_ago_s: Option<u64>,
    pub own_version: Option<String>,
    pub own_flags: Vec<String>,
    pub devices: Vec<DeviceFacts>,
    /// Devices whose key changed: (node, name, dismissed).
    pub key_changes: Vec<(String, String, bool)>,
    /// The feature groups in use.
    pub features: Vec<String>,
}

/// A push this recent says pushes work.
pub const PUSH_FRESH_S: u64 = 14 * 24 * 3600;
/// Less free space than this is red; than `DISK_LOW_MB` (or 5% of the
/// disk) amber.
pub const DISK_BAD_MB: u32 = 1024;
pub const DISK_LOW_MB: u32 = 5 * 1024;
/// Links carried directly (not through the relay or Bluetooth).
const DIRECT: [&str; 6] = [
    "lan",
    "direct",
    "punched",
    "webrtc",
    "wifi_direct",
    "hotspot",
];

/// The report from what the core and the app know.
pub fn evaluate(core: &CoreFacts, platform: &PlatformFacts) -> HealthReport {
    use HealthLevel::*;
    let mut out = Vec::new();

    // The server: signing in, finding your devices, pushes, the relay.
    if core.signed_in {
        match core.server_ok {
            Some(true) => out.push(HealthCheck::new("server", Ok, "ok")),
            Some(false) => out.push(HealthCheck::new("server", Bad, "unreachable").fix("retry")),
            None => {}
        }
        // Pushes wake a sleeping device.
        out.push(match (core.push_token, core.push_seen_ago_s) {
            (false, _) => HealthCheck::new("push", Warn, "no_token").fix("push_test"),
            (true, Some(ago)) if ago <= PUSH_FRESH_S => HealthCheck::new("push", Ok, "working"),
            (true, _) => HealthCheck::new("push", Warn, "unconfirmed").fix("push_test"),
        });
    }

    // Space for received files.
    if let Some(free) = platform.disk_free_mb {
        let tight = platform
            .disk_total_mb
            .is_some_and(|t| t > 0 && u64::from(free) * 100 < u64::from(t) * 5);
        let c = if free < DISK_BAD_MB {
            HealthCheck::new("disk", Bad, "full").fix("free_space")
        } else if free < DISK_LOW_MB || tight {
            HealthCheck::new("disk", Warn, "low").fix("free_space")
        } else {
            HealthCheck::new("disk", Ok, "ok")
        };
        out.push(c.detail(free.to_string()));
    }

    // Permissions for the features in use.
    for p in &platform.permissions {
        let used = p.features.is_empty() || p.features.iter().any(|f| core.features.contains(f));
        if !used {
            continue;
        }
        let level = match p.state.as_str() {
            "granted" => Ok,
            "denied" if p.critical => Bad,
            "denied" => Warn,
            "unknown" => Info,
            _ => continue,
        };
        let mut c = HealthCheck::new("permission", level, &p.state).detail(p.id.clone());
        c.features = p.features.clone();
        if level != Ok {
            c = c.fix("open_permission");
        }
        out.push(c);
    }

    // Keys that changed: refused until the user trusts the new one.
    for (node, name, dismissed) in &core.key_changes {
        let level = if *dismissed { Warn } else { Bad };
        out.push(
            HealthCheck::new("key", level, "changed")
                .device(node, name)
                .fix("trust_key"),
        );
    }

    for d in &core.devices {
        // How it's linked, and why not directly.
        if let Some(c) = link_check(d, core) {
            out.push(c);
        }
        if let Some(c) = version_check(d, core) {
            out.push(c);
        }
    }

    // Worst first; the order above within a level.
    out.sort_by_key(|c| std::cmp::Reverse(c.level));
    let bad = out.iter().filter(|c| c.level == Bad).count();
    let warn = out.iter().filter(|c| c.level == Warn).count();
    HealthReport {
        worst: out.first().map(|c| c.level).unwrap_or(Ok),
        bad: u32::try_from(bad).unwrap_or(u32::MAX),
        warn: u32::try_from(warn).unwrap_or(u32::MAX),
        checks: out,
    }
}

fn link_check(d: &DeviceFacts, core: &CoreFacts) -> Option<HealthCheck> {
    use HealthLevel::*;
    let base = |level, reason: &str| {
        let mut c = HealthCheck::new("link", level, reason).device(&d.node_id, &d.name);
        c.reconnect_ms = d.reconnect_ms;
        c
    };
    match d.problem.as_deref() {
        // Said by the key check already.
        Some("key_changed") if core.key_changes.iter().any(|k| k.0 == d.node_id) => return None,
        Some("key_changed") => return Some(base(Bad, "key_changed").fix("trust_key")),
        Some("bad_key") => return Some(base(Bad, "bad_key").fix("encryption_key")),
        _ => {}
    }
    let s = d.state.as_str();
    let c = if DIRECT.contains(&s) {
        base(Ok, "direct").detail(s)
    } else if s == "relay" || s == "ble" {
        let c = base(Warn, s).fix("links_map");
        match &d.reason {
            Some(r) => c.detail(r.clone()),
            None => c,
        }
    } else {
        match d.reach {
            Some(DeviceReach::Asleep) => base(Info, "asleep"),
            Some(DeviceReach::Waking) => base(Info, "waking"),
            _ if s == "unreachable" => {
                let c = base(Warn, "unreachable").fix("connect");
                match &d.reason {
                    Some(r) => c.detail(r.clone()),
                    None => c,
                }
            }
            _ => base(Info, "not_connected").fix("connect"),
        }
    };
    Some(c)
}

fn version_check(d: &DeviceFacts, core: &CoreFacts) -> Option<HealthCheck> {
    use std::cmp::Ordering;
    use HealthLevel::*;
    let own = core.own_version.as_deref()?;
    let app = d.app.as_ref()?;
    let lacks = app_info::missing(&core.own_flags, &app.flags);
    let order = app
        .version
        .as_deref()
        .map(|v| app_info::compare_versions(v, own))
        // No version said: an app from before versions were said.
        .unwrap_or(Ordering::Less);
    let mut c = match order {
        Ordering::Less if !lacks.is_empty() => {
            HealthCheck::new("version", Warn, "older").fix("update_other")
        }
        Ordering::Less => HealthCheck::new("version", Info, "older").fix("update_other"),
        Ordering::Greater => HealthCheck::new("version", Info, "newer").fix("update_app"),
        Ordering::Equal => HealthCheck::new("version", Ok, "same"),
    }
    .device(&d.node_id, &d.name);
    if let Some(v) = &app.version {
        c = c.detail(v.clone());
    }
    if order == Ordering::Less {
        c.features = lacks;
    }
    Some(c)
}

/// What the core knows now. `server_ok` is asked by the caller (it blocks).
pub fn core_facts<P: Platform>(
    ctx: &NetCtx<P>,
    signed_in: bool,
    server_ok: Option<bool>,
    features: Vec<String>,
) -> CoreFacts {
    let map = super::links_map::snapshot(ctx);
    let devices = map
        .devices
        .iter()
        .filter(|d| !d.node_id.starts_with("manual:"))
        .map(|d| {
            let rc = super::reconnect::report(ctx, &d.node_id);
            // A relayed link: why the direct paths didn't work.
            let reason = if d.link.is_some() {
                d.tries
                    .iter()
                    .find(|t| t.path == "internet" || t.path == "lan")
                    .map(|t| format!("{}:{}", t.path, t.reason))
            } else {
                d.reason.clone()
            };
            DeviceFacts {
                node_id: d.node_id.clone(),
                name: d.name.clone(),
                state: d.state.clone(),
                reason,
                problem: d.problem.clone(),
                reach: rc.reach,
                reconnect_ms: rc.last_ms,
                app: app_info::peer(ctx, &d.node_id),
            }
        })
        .collect();
    let key_changes = crate::device_key::changes(&ctx.peer_keys, &*ctx.platform, |node| {
        ctx.peers
            .lock()
            .get(node)
            .map(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| node.to_string())
    })
    .into_iter()
    .map(|k| (k.node_id, k.name, k.dismissed))
    .collect();
    CoreFacts {
        signed_in,
        server_ok,
        push_token: ctx
            .platform
            .fcm_token()
            .is_some_and(|t| !t.trim().is_empty()),
        push_seen_ago_s: app_info::push_seen_ago(ctx),
        own_version: app_info::own(ctx),
        own_flags: app_info::own_flags(ctx),
        devices,
        key_changes,
        features,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HealthLevel::*;

    fn all_features() -> Vec<String> {
        super::super::onboarding::GROUPS.map(String::from).to_vec()
    }

    fn core() -> CoreFacts {
        CoreFacts {
            signed_in: true,
            server_ok: Some(true),
            push_token: true,
            push_seen_ago_s: Some(60),
            own_version: Some("0.3.0".into()),
            own_flags: vec!["xfer".into(), "media_ctl".into()],
            features: all_features(),
            ..Default::default()
        }
    }

    fn device(state: &str) -> DeviceFacts {
        DeviceFacts {
            node_id: "n1".into(),
            name: "Phone".into(),
            state: state.into(),
            ..Default::default()
        }
    }

    fn find<'a>(r: &'a HealthReport, id: &str) -> &'a HealthCheck {
        r.checks
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn all_well_is_green() {
        let r = evaluate(&core(), &PlatformFacts::default());
        assert_eq!(r.worst, Ok);
        assert_eq!((r.bad, r.warn), (0, 0));
        assert_eq!(find(&r, "server").reason, "ok");
        assert_eq!(find(&r, "push").reason, "working");
    }

    #[test]
    fn server_and_push() {
        let mut c = core();
        c.server_ok = Some(false);
        c.push_token = false;
        let r = evaluate(&c, &PlatformFacts::default());
        let s = find(&r, "server");
        assert_eq!(
            (s.level, s.reason.as_str(), s.fix.as_deref()),
            (Bad, "unreachable", Some("retry"))
        );
        assert_eq!(find(&r, "push").reason, "no_token");
        assert_eq!(r.worst, Bad);
        assert_eq!(r.checks[0].id, "server", "worst first");

        // A token, but no push for weeks: not confirmed.
        let mut c = core();
        c.push_seen_ago_s = Some(PUSH_FRESH_S + 1);
        let p = evaluate(&c, &PlatformFacts::default());
        let p = find(&p, "push");
        assert_eq!(
            (p.level, p.reason.as_str(), p.fix.as_deref()),
            (Warn, "unconfirmed", Some("push_test"))
        );
        c.push_seen_ago_s = None;
        assert_eq!(
            find(&evaluate(&c, &PlatformFacts::default()), "push").reason,
            "unconfirmed"
        );

        // Not asked this time: no server line. Signed out: neither.
        let mut c = core();
        c.server_ok = None;
        assert!(evaluate(&c, &PlatformFacts::default())
            .checks
            .iter()
            .all(|x| x.id != "server"));
        c.signed_in = false;
        assert!(evaluate(&c, &PlatformFacts::default()).checks.is_empty());
    }

    #[test]
    fn disk_space() {
        let level = |free: u32, total: Option<u32>| {
            let p = PlatformFacts {
                disk_free_mb: Some(free),
                disk_total_mb: total,
                ..Default::default()
            };
            let r = evaluate(&core(), &p);
            let d = find(&r, "disk");
            (d.level, d.detail.clone().unwrap())
        };
        assert_eq!(level(500, None), (Bad, "500".into()));
        assert_eq!(level(3000, None), (Warn, "3000".into()));
        assert_eq!(
            level(20_000, Some(1_000_000)).0,
            Warn,
            "under 5% of the disk"
        );
        assert_eq!(level(80_000, Some(1_000_000)).0, Ok);
        assert!(evaluate(&core(), &PlatformFacts::default())
            .checks
            .iter()
            .all(|c| c.id != "disk"));
    }

    #[test]
    fn permissions_count_only_for_features_in_use() {
        let perm = |id: &str, f: &[&str], state: &str, critical: bool| PermissionFact {
            id: id.into(),
            features: f.iter().map(|s| s.to_string()).collect(),
            state: state.into(),
            critical,
        };
        let p = PlatformFacts {
            permissions: vec![
                perm("notification_access", &["notifications"], "denied", true),
                perm("accessibility", &["media"], "denied", false),
                perm("battery", &[], "granted", true),
                perm("autostart", &[], "unknown", false),
                perm("microphone", &["media"], "not_needed", false),
            ],
            ..Default::default()
        };
        let mut c = core();
        c.features = vec!["notifications".into(), "files".into()];
        let r = evaluate(&c, &p);
        let perms: Vec<(&str, HealthLevel, Option<&str>)> = r
            .checks
            .iter()
            .filter(|x| x.id == "permission")
            .map(|x| (x.detail.as_deref().unwrap(), x.level, x.fix.as_deref()))
            .collect();
        assert_eq!(
            perms,
            vec![
                ("notification_access", Bad, Some("open_permission")),
                ("autostart", Info, Some("open_permission")),
                ("battery", Ok, None),
            ],
            "media isn't used: accessibility left out; not_needed never shown"
        );
        assert_eq!(r.bad, 1);
        // With media in use, a non-critical one is amber.
        c.features.push("media".into());
        let r = evaluate(&c, &p);
        assert!(r
            .checks
            .iter()
            .any(|x| x.detail.as_deref() == Some("accessibility") && x.level == Warn));
    }

    #[test]
    fn links_say_how_and_why() {
        let link = |d: DeviceFacts| {
            let mut c = core();
            c.devices = vec![d];
            let r = evaluate(&c, &PlatformFacts::default());
            find(&r, "link").clone()
        };
        let mut lan = device("lan");
        lan.reconnect_ms = Some(800);
        let l = link(lan);
        assert_eq!(
            (l.level, l.reason.as_str(), l.detail.as_deref()),
            (Ok, "direct", Some("lan"))
        );
        assert_eq!(l.reconnect_ms, Some(800));
        assert_eq!(l.node_id.as_deref(), Some("n1"));

        let mut relay = device("relay");
        relay.reason = Some("internet:symmetric_nat".into());
        let l = link(relay);
        assert_eq!(
            (
                l.level,
                l.reason.as_str(),
                l.detail.as_deref(),
                l.fix.as_deref()
            ),
            (
                Warn,
                "relay",
                Some("internet:symmetric_nat"),
                Some("links_map")
            )
        );

        let mut gone = device("unreachable");
        gone.reason = Some("no_route".into());
        let l = link(gone);
        assert_eq!((l.level, l.fix.as_deref()), (Warn, Some("connect")));

        let mut asleep = device("unreachable");
        asleep.reach = Some(DeviceReach::Asleep);
        assert_eq!(link(asleep).reason, "asleep");
        assert_eq!(link(device("not_connected")).level, Info);

        let mut bad = device("not_connected");
        bad.problem = Some("bad_key".into());
        let l = link(bad);
        assert_eq!((l.level, l.fix.as_deref()), (Bad, Some("encryption_key")));
    }

    #[test]
    fn a_changed_key_is_said_once() {
        let mut c = core();
        let mut d = device("not_connected");
        d.problem = Some("key_changed".into());
        c.devices = vec![d];
        c.key_changes = vec![("n1".into(), "Phone".into(), false)];
        let r = evaluate(&c, &PlatformFacts::default());
        assert_eq!(
            r.checks
                .iter()
                .filter(|x| x.node_id.as_deref() == Some("n1") && x.level == Bad)
                .count(),
            1
        );
        let k = find(&r, "key");
        assert_eq!((k.level, k.fix.as_deref()), (Bad, Some("trust_key")));
        // "Not now": still refused, amber.
        c.key_changes[0].2 = true;
        assert_eq!(
            find(&evaluate(&c, &PlatformFacts::default()), "key").level,
            Warn
        );
    }

    #[test]
    fn versions_warn_when_an_older_app_lacks_features() {
        let check = |app: Option<PeerApp>| {
            let mut c = core();
            let mut d = device("lan");
            d.app = app;
            c.devices = vec![d];
            evaluate(&c, &PlatformFacts::default())
                .checks
                .into_iter()
                .find(|x| x.id == "version")
        };
        let app = |v: Option<&str>, flags: &[&str]| {
            Some(PeerApp {
                version: v.map(String::from),
                flags: flags.iter().map(|s| s.to_string()).collect(),
            })
        };
        assert!(check(None).is_none(), "nothing said yet");
        let same = check(app(Some("0.3.0"), &["xfer", "media_ctl"])).unwrap();
        assert_eq!((same.level, same.reason.as_str()), (Ok, "same"));
        let old = check(app(Some("0.2.9"), &["xfer"])).unwrap();
        assert_eq!(
            (
                old.level,
                old.reason.as_str(),
                old.features.clone(),
                old.fix.as_deref()
            ),
            (
                Warn,
                "older",
                vec!["media_ctl".to_string()],
                Some("update_other")
            )
        );
        assert_eq!(old.detail.as_deref(), Some("0.2.9"));
        let old_complete = check(app(Some("0.2.9"), &["xfer", "media_ctl"])).unwrap();
        assert_eq!(old_complete.level, Info);
        // No version at all: an app from before; what it lacks decides.
        let ancient = check(app(None, &[])).unwrap();
        assert_eq!((ancient.level, ancient.features.len()), (Warn, 2));
        let newer = check(app(Some("0.10.0"), &[])).unwrap();
        assert_eq!(
            (newer.level, newer.reason.as_str(), newer.fix.as_deref()),
            (Info, "newer", Some("update_app"))
        );
        assert!(newer.features.is_empty());
    }
}
