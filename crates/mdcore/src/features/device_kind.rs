//! What kind of device each of your devices is, so the apps draw the right
//! icon for it: a Mac laptop, a Windows desktop, an Android phone...
//!
//! The host app works out its own kind once (OS, form, model, maker) and
//! hands it to the core (`set_own`). The core then says it, as an optional
//! object, everywhere a device describes itself:
//!
//!   status      { ..., device: {os, form, model, maker} }
//!   addr_update { ..., device: {os, form, model, maker} }
//!   registry    POST /devices/register { ..., kind: {...} }
//!
//! Older apps ignore the field, and an older app saying nothing gets a
//! guess from what it did say (`platform: "android"` in its status, its
//! `kvm_os`), else the `"unknown"` icon. What each device said is kept
//! across restarts (`kv`, key `device_kinds`), and a change makes a
//! `CoreEvent::DeviceKind`.
//!
//! The icon is worked out here (`icon_for`) from the OS and the form; the
//! apps only draw it. A user's own choice of icon is a device label
//! (`features::labels`) and wins over this one in the apps. `ICONS` is the
//! set both apps draw (and offer in "Change icon").
//!
//! The pure tables the apps use to find their own kind live here too, so
//! they're tested once: SMBIOS chassis types (`form_from_chassis`, Windows
//! and Linux) and Mac model names (`form_from_mac_model`).

use std::collections::BTreeMap;

use parking_lot::Mutex;
use serde_json::Value;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::model::DeviceKind;
use crate::platform::Platform;

/// The status / `addr_update` field carrying this device's kind.
pub const FLAG: &str = "device";
const MAP_KEY: &str = "device_kinds";
/// The longest model or maker kept, in characters.
pub const MAX_TEXT: usize = 60;
/// The most devices remembered.
const MAX_DEVICES: usize = 256;

pub const OSES: [&str; 5] = ["macos", "windows", "linux", "android", "ios"];
pub const FORMS: [&str; 8] = [
    "phone",
    "tablet",
    "foldable",
    "laptop",
    "desktop",
    "all_in_one",
    "tv",
    "other",
];

/// Every icon the apps draw: the automatic ones and the older label set
/// (`phone`, `tablet`, `laptop`, `desktop`, `tv`, `watch`).
pub const ICONS: [&str; 17] = [
    "android_phone",
    "android_tablet",
    "android_foldable",
    "phone",
    "tablet",
    "mac_laptop",
    "imac",
    "mac_desktop",
    "win_laptop",
    "win_desktop",
    "linux_laptop",
    "linux_desktop",
    "laptop",
    "desktop",
    "tv",
    "watch",
    "unknown",
];

/// The icon for a device of `os` and `form` (either may be empty).
pub fn icon_for(os: &str, form: &str) -> &'static str {
    let by_os =
        |mac: &'static str, win: &'static str, linux: &'static str, other: &'static str| match os {
            "macos" => mac,
            "windows" => win,
            "linux" => linux,
            _ => other,
        };
    match form {
        "tv" => "tv",
        "phone" | "foldable" | "tablet" if os == "android" => match form {
            "tablet" => "android_tablet",
            "foldable" => "android_foldable",
            _ => "android_phone",
        },
        "phone" | "foldable" => "phone",
        "tablet" => "tablet",
        "laptop" => by_os("mac_laptop", "win_laptop", "linux_laptop", "laptop"),
        "all_in_one" => by_os("imac", "win_desktop", "linux_desktop", "desktop"),
        "desktop" => by_os("mac_desktop", "win_desktop", "linux_desktop", "desktop"),
        // Form not said: most phones are phones, most computers laptops.
        _ => match os {
            "android" => "android_phone",
            "ios" => "phone",
            _ => by_os("mac_laptop", "win_laptop", "linux_laptop", "unknown"),
        },
    }
}

/// The form for an SMBIOS chassis type (Windows `Win32_SystemEnclosure`,
/// Linux `/sys/class/dmi/id/chassis_type`); `None` when it says nothing
/// useful (other, unknown, a server part...).
pub fn form_from_chassis(t: u32) -> Option<&'static str> {
    match t {
        8 | 9 | 10 | 14 | 31 | 32 => Some("laptop"),
        13 => Some("all_in_one"),
        3..=7 | 15 | 16 | 35 | 36 => Some("desktop"),
        30 => Some("tablet"),
        _ => None,
    }
}

/// The form for a Mac's model name or identifier ("MacBook Pro",
/// "MacBookAir10,1", "iMac21,1", "Macmini9,1", "Mac Studio"); `None` for
/// the bare "Mac14,3" style identifiers newer Macs have (ask the battery).
pub fn form_from_mac_model(model: &str) -> Option<&'static str> {
    let m: String = model
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if m.starts_with("macbook") {
        Some("laptop")
    } else if m.starts_with("imac") {
        Some("all_in_one")
    } else if m.starts_with("macmini") || m.starts_with("macstudio") || m.starts_with("macpro") {
        Some("desktop")
    } else {
        None
    }
}

/// A readable Mac name from its identifier when nothing better is known
/// ("MacBookPro18,3" -> "MacBook Pro"); else the identifier.
pub fn mac_name(identifier: &str) -> String {
    let base: String = identifier
        .chars()
        .take_while(|c| !c.is_ascii_digit() && *c != ',')
        .collect();
    match base.as_str() {
        "MacBookPro" => "MacBook Pro".into(),
        "MacBookAir" => "MacBook Air".into(),
        "MacBook" => "MacBook".into(),
        "iMac" => "iMac".into(),
        "iMacPro" => "iMac Pro".into(),
        "Macmini" => "Mac mini".into(),
        "MacPro" => "Mac Pro".into(),
        _ => identifier.trim().to_string(),
    }
}

/// A maker name as manufacturers report it, tidied for display:
/// "samsung" -> "Samsung", "LENOVO" -> "Lenovo", "To Be Filled By O.E.M."
/// and the like -> "".
pub fn tidy_maker(maker: &str) -> String {
    let m = tidy_text(maker);
    let lower = m.to_ascii_lowercase();
    if lower.is_empty()
        || lower.contains("o.e.m")
        || lower.contains("default string")
        || lower == "system manufacturer"
        || lower == "unknown"
    {
        return String::new();
    }
    match lower.as_str() {
        "hp" | "hewlett-packard" => return "HP".into(),
        "asustek computer inc." => return "ASUS".into(),
        "micro-star international co., ltd." => return "MSI".into(),
        "lg" | "lge" => return "LG".into(),
        "iqoo" => return "iQOO".into(),
        _ => {}
    }
    let shouting = m.chars().any(char::is_alphabetic)
        && (m == m.to_ascii_lowercase() || m == m.to_ascii_uppercase());
    if !shouting {
        return m;
    }
    m.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f
                    .to_uppercase()
                    .chain(c.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Text for display: no control or hidden characters, spaces collapsed, cut
/// to `MAX_TEXT` characters.
pub fn tidy_text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A kind as given or as it arrived: OS and form only from their sets,
/// texts tidied, the icon worked out.
pub fn sanitize(k: DeviceKind) -> DeviceKind {
    let pick = |v: &str, set: &[&str]| {
        let v = v.trim().to_ascii_lowercase();
        if set.contains(&v.as_str()) {
            v
        } else {
            String::new()
        }
    };
    let os = pick(&k.os, &OSES);
    let form = pick(&k.form, &FORMS);
    let icon = icon_for(&os, &form).to_string();
    DeviceKind {
        model: tidy_text(&k.model),
        maker: tidy_text(&k.maker),
        os,
        form,
        icon,
    }
}

/// The kind in a status / `addr_update` / registry `device` object, if any.
pub fn parse(v: &Value) -> Option<DeviceKind> {
    if !v.is_object() {
        return None;
    }
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let k = sanitize(DeviceKind {
        os: s("os"),
        form: s("form"),
        model: s("model"),
        maker: s("maker"),
        icon: String::new(),
    });
    (!k.os.is_empty() || !k.form.is_empty() || !k.model.is_empty()).then_some(k)
}

/// What an older app's status said instead: its `platform` ("android") or
/// its `kvm_os`. `None` when neither says anything.
pub fn guess(platform: &Value, kvm_os: &Value) -> Option<DeviceKind> {
    let os = match platform.as_str() {
        Some("android") => "android",
        Some("ios") => "ios",
        _ => kvm_os.as_str().unwrap_or_default(),
    };
    let k = sanitize(DeviceKind {
        os: os.into(),
        ..Default::default()
    });
    (!k.os.is_empty()).then_some(k)
}

/// The kind as it goes on the wire (no icon).
pub fn to_wire(k: &DeviceKind) -> Value {
    serde_json::json!({ "os": k.os, "form": k.form, "model": k.model, "maker": k.maker })
}

/// A device nothing is known about.
pub fn unknown() -> DeviceKind {
    sanitize(DeviceKind::default())
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct DeviceKinds {
    own: Mutex<Option<DeviceKind>>,
    /// node -> what it said; `None` until read from `kv`.
    peers: Mutex<Option<BTreeMap<String, DeviceKind>>>,
}

fn with_peers<P: Platform, R>(
    ctx: &NetCtx<P>,
    f: impl FnOnce(&mut BTreeMap<String, DeviceKind>) -> R,
) -> R {
    let mut guard = ctx.extras.device_kind.peers.lock();
    let map = guard.get_or_insert_with(|| {
        ctx.platform
            .kv_get(MAP_KEY)
            .and_then(|s| serde_json::from_str::<BTreeMap<String, DeviceKind>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(n, k)| (n, sanitize(k)))
            .collect()
    });
    f(map)
}

/// This device's own kind, as the app found it. Said to linked devices at
/// once when it changes.
pub fn set_own<P: Platform>(ctx: &NetCtx<P>, kind: DeviceKind) {
    let kind = sanitize(kind);
    let changed = {
        let mut own = ctx.extras.device_kind.own.lock();
        let changed = own.as_ref() != Some(&kind);
        *own = Some(kind);
        changed
    };
    if changed && !ctx.p2p_conns.lock().is_empty() {
        super::addr_update::announce(ctx, None);
    }
}

/// This device's own kind (`None` until the app said it).
pub fn own<P: Platform>(ctx: &NetCtx<P>) -> Option<DeviceKind> {
    ctx.extras.device_kind.own.lock().clone()
}

/// Add this device's kind to a status reply or `addr_update`.
pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if let Some(k) = own(ctx) {
        status[FLAG] = to_wire(&k);
    }
}

/// `node`'s status or `addr_update` arrived: `device` is its `device`
/// field; `platform` and `kvm_os` what an older app says instead.
pub fn remember<P: Platform>(
    ctx: &NetCtx<P>,
    node: &str,
    device: &Value,
    platform: &Value,
    kvm_os: &Value,
) {
    if let Some(k) = parse(device) {
        learn(ctx, node, k, true);
    } else if let Some(k) = guess(platform, kvm_os) {
        learn(ctx, node, k, false);
    }
}

/// `node`'s kind as the server lists it (`kind` in its registry entry).
pub fn from_registry<P: Platform>(ctx: &NetCtx<P>, node: &str, kind: &Value) {
    if let Some(k) = parse(kind) {
        learn(ctx, node, k, true);
    }
}

/// Keep what `node` said. A guess never replaces what it said itself
/// (only fills in the OS when nothing better is known).
fn learn<P: Platform>(ctx: &NetCtx<P>, node: &str, kind: DeviceKind, said: bool) {
    if node.is_empty() || node.len() > 128 || node.starts_with("manual:") {
        return;
    }
    let saved = with_peers(ctx, |map| {
        let old = map.get(node);
        if old == Some(&kind) {
            return None;
        }
        if !said && old.is_some_and(|o| !o.form.is_empty() || o.os == kind.os) {
            return None;
        }
        map.insert(node.to_string(), kind.clone());
        while map.len() > MAX_DEVICES {
            let Some(first) = map.keys().find(|k| k.as_str() != node).cloned() else {
                break;
            };
            map.remove(&first);
        }
        Some(serde_json::to_string(&*map).unwrap_or_default())
    });
    if let Some(json) = saved {
        ctx.platform.kv_set(MAP_KEY, &json);
        ctx.sink.emit(CoreEvent::DeviceKind {
            node: node.to_string(),
            device: kind,
        });
    }
}

/// What `node` is (`unknown()` when it never said).
pub fn of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> DeviceKind {
    with_peers(ctx, |map| map.get(node).cloned()).unwrap_or_else(unknown)
}

/// Every device's kind that's known, by node id.
pub fn all<P: Platform>(ctx: &NetCtx<P>) -> BTreeMap<String, DeviceKind> {
    with_peers(ctx, |map| map.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(os: &str, form: &str) -> DeviceKind {
        sanitize(DeviceKind {
            os: os.into(),
            form: form.into(),
            ..Default::default()
        })
    }

    #[test]
    fn icons_combine_os_and_form() {
        let cases = [
            ("macos", "laptop", "mac_laptop"),
            ("macos", "all_in_one", "imac"),
            ("macos", "desktop", "mac_desktop"),
            ("windows", "laptop", "win_laptop"),
            ("windows", "desktop", "win_desktop"),
            ("windows", "all_in_one", "win_desktop"),
            ("linux", "laptop", "linux_laptop"),
            ("linux", "desktop", "linux_desktop"),
            ("android", "phone", "android_phone"),
            ("android", "tablet", "android_tablet"),
            ("android", "foldable", "android_foldable"),
            ("android", "tv", "tv"),
            ("ios", "phone", "phone"),
            ("ios", "tablet", "tablet"),
            ("", "laptop", "laptop"),
            ("", "desktop", "desktop"),
            ("", "", "unknown"),
            ("", "other", "unknown"),
            // Form not said: a guess from the OS.
            ("android", "", "android_phone"),
            ("macos", "", "mac_laptop"),
            ("windows", "", "win_laptop"),
            ("linux", "", "linux_laptop"),
        ];
        for (os, form, icon) in cases {
            assert_eq!(icon_for(os, form), icon, "{os} {form}");
            assert!(ICONS.contains(&icon), "{icon} is drawn");
        }
    }

    #[test]
    fn chassis_types_map_to_forms() {
        for t in [8, 9, 10, 14, 31, 32] {
            assert_eq!(form_from_chassis(t), Some("laptop"), "{t}");
        }
        for t in [3, 4, 5, 6, 7, 35, 36] {
            assert_eq!(form_from_chassis(t), Some("desktop"), "{t}");
        }
        assert_eq!(form_from_chassis(13), Some("all_in_one"));
        assert_eq!(form_from_chassis(30), Some("tablet"));
        for t in [0, 1, 2, 17, 23, 99] {
            assert_eq!(form_from_chassis(t), None, "{t}");
        }
    }

    #[test]
    fn mac_models_map_to_forms() {
        let cases = [
            ("MacBook Pro", Some("laptop")),
            ("MacBookAir10,1", Some("laptop")),
            ("MacBookPro18,3", Some("laptop")),
            ("iMac", Some("all_in_one")),
            ("iMac21,1", Some("all_in_one")),
            ("Mac mini", Some("desktop")),
            ("Macmini9,1", Some("desktop")),
            ("Mac Studio", Some("desktop")),
            ("Mac Pro", Some("desktop")),
            ("MacPro7,1", Some("desktop")),
            ("Mac14,3", None),
            ("", None),
        ];
        for (m, form) in cases {
            assert_eq!(form_from_mac_model(m), form, "{m}");
        }
        assert_eq!(mac_name("MacBookPro18,3"), "MacBook Pro");
        assert_eq!(mac_name("Macmini9,1"), "Mac mini");
        assert_eq!(mac_name("Mac14,3"), "Mac14,3");
    }

    #[test]
    fn makers_are_tidied() {
        assert_eq!(tidy_maker("samsung"), "Samsung");
        assert_eq!(tidy_maker("LENOVO"), "Lenovo");
        assert_eq!(tidy_maker("Dell Inc."), "Dell Inc.");
        assert_eq!(tidy_maker("HP"), "HP");
        assert_eq!(tidy_maker("Hewlett-Packard"), "HP");
        assert_eq!(tidy_maker("vivo"), "Vivo");
        assert_eq!(tidy_maker("To Be Filled By O.E.M."), "");
        assert_eq!(tidy_maker("Google"), "Google");
    }

    #[test]
    fn what_arrives_is_checked() {
        let k = parse(&serde_json::json!({
            "os": "MacOS", "form": "laptop", "model": "MacBook\u{202e} Pro\n", "maker": "x".repeat(200),
            "icon": "rocket"
        }))
        .unwrap();
        assert_eq!(k.os, "macos");
        assert_eq!(k.model, "MacBook Pro");
        assert_eq!(k.maker.chars().count(), MAX_TEXT);
        assert_eq!(k.icon, "mac_laptop", "the icon is worked out, never taken");
        let odd =
            parse(&serde_json::json!({ "os": "beos", "form": "toaster", "model": "Z" })).unwrap();
        assert_eq!(
            (odd.os.as_str(), odd.form.as_str(), odd.icon.as_str()),
            ("", "", "unknown")
        );
        assert!(parse(&Value::Null).is_none());
        assert!(parse(&serde_json::json!({})).is_none());
        assert!(parse(&serde_json::json!("phone")).is_none());
    }

    #[test]
    fn an_older_app_gets_a_guess_or_unknown() {
        assert_eq!(
            guess(&"android".into(), &Value::Null),
            Some(kind("android", ""))
        );
        assert_eq!(
            guess(&"desktop".into(), &"linux".into()),
            Some(kind("linux", ""))
        );
        assert_eq!(guess(&"desktop".into(), &Value::Null), None);
        assert_eq!(guess(&Value::Null, &Value::Null), None);
        assert_eq!(unknown().icon, "unknown");
        assert_eq!(
            guess(&"android".into(), &Value::Null).unwrap().icon,
            "android_phone"
        );
    }

    #[test]
    fn wire_form_has_no_icon() {
        let w = to_wire(&kind("android", "tablet"));
        assert!(w.get("icon").is_none());
        assert_eq!(parse(&w), Some(kind("android", "tablet")));
    }
}
