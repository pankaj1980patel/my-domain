//! Rich phone notifications: what a notification carries beyond its title,
//! text and buttons, the subscriber's filters, and app icons.
//!
//! Only between devices that said `notif_rich: 1` (in their status and the
//! `addr_update` a link starts with). To any other device a notification
//! goes exactly as before, and the new messages never go at all.
//!
//! Extra, optional fields of `notification` (all may be missing):
//!
//!   pkg         the app's package ("com.whatsapp")
//!   conv        the conversation it belongs to (same value: same chat)
//!   conv_title  the conversation's name ("Family"), group_chat when a group
//!   sender      who wrote the newest message
//!   messages    the last few messages [{sender, text, ts}], oldest first
//!   avatar      the sender's picture: a small JPEG, base64
//!   picture     a picture in the notification: a small JPEG, base64
//!   icon        the app icon's id (see below)
//!   sensitive   the phone thinks it holds a secret (a code, a bank)
//!   private     its content was left out (the subscriber's private mode)
//!   silent      an update that shouldn't alert again
//!   ts          when it was posted, ms since 1970
//!
//! Pictures over their cap ([`MAX_AVATAR`], [`MAX_PICTURE`]) or that aren't
//! JPEG are dropped on both ends, and none go over a Bluetooth link.
//!
//! The subscriber says what it wants in `subscribe_apps` (optional fields
//! an older phone ignores): `only_locked` (only while the phone isn't in
//! use) and `private` (leave the content of a sensitive one out: it goes
//! with an empty title and text and no buttons, marked `private`). The
//! producer keeps the subscriptions across restarts.
//!
//! App icons: the producer knows each app's icon (`Engine::set_app_icon`)
//! and puts its id in `icon`; a subscriber that hasn't got that one asks:
//!
//!   notif_icon_request { from, pkg }               subscriber → producer
//!   notif_icon         { from, pkg, hash, data }   producer → subscriber
//!
//! (only for an app it's subscribed to), arriving as `NotificationIcon`.

use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::NetCtx;
use crate::events::CoreEvent;
use crate::platform::Platform;

use super::{ControlHandler, ControlMsg};

/// The status (and `addr_update`) flag, and its version.
pub const FLAG: &str = "notif_rich";
const VERSION: u64 = 1;
/// The biggest sender picture, in bytes of JPEG.
pub const MAX_AVATAR: usize = 16 * 1024;
/// The biggest picture in a notification, in bytes of JPEG.
pub const MAX_PICTURE: usize = 48 * 1024;
/// The biggest app icon, in bytes of JPEG.
pub const MAX_ICON: usize = 16 * 1024;
/// The most messages of a conversation carried.
pub const MAX_MESSAGES: usize = 6;
/// The longest message text, in characters.
const MAX_TEXT: usize = 1000;
/// The longest name (sender, conversation, package), in characters.
const MAX_NAME: usize = 200;
/// The kv key the subscriptions made here are kept under.
const SUBS_KEY: &str = "notif_subs";
const CAPS_PREFIX: &str = "notif_rich_cap:";

fn is_false(b: &bool) -> bool {
    !*b
}

/// One message of a conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifMessage {
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub text: String,
    /// When, in ms since 1970 (0: unknown).
    #[serde(default)]
    pub ts: u64,
}

/// What a notification carries beyond its title, text and buttons (see the
/// module docs). Every field is optional on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifExtra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_title: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub group_chat: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<NotifMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub silent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<u64>,
}

/// `s` cut to `max` characters, without surrounding spaces.
fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

fn clip_opt(s: Option<String>, max: usize) -> Option<String> {
    s.map(|s| clip(&s, max)).filter(|s| !s.is_empty())
}

/// `b64` is a JPEG of at most `max` bytes, in standard base64.
pub fn jpeg_ok(b64: &str, max: usize) -> bool {
    if b64.is_empty() || b64.len() > max.div_ceil(3) * 4 + 4 {
        return false;
    }
    match base64::engine::general_purpose::STANDARD.decode(b64) {
        Ok(bytes) => bytes.len() <= max && bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        Err(_) => false,
    }
}

/// An app icon's id: the first 8 bytes of its SHA-256, in hex.
pub fn icon_hash(b64: &str) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(b64.as_bytes());
    d[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// An icon id as one may look: 1 to 64 hex digits.
fn hash_ok(h: &str) -> bool {
    !h.is_empty() && h.len() <= 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A package name as one may look: letters, digits, `.`, `_`, `*`.
pub fn pkg_ok(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('.')
        && p.len() <= MAX_NAME
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'*')
}

impl NotifExtra {
    /// The extra fields of a message (anything malformed left out).
    pub fn from_value(v: &Value) -> Self {
        let mut e = NotifExtra::default();
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        e.pkg = s("pkg");
        e.conv = s("conv");
        e.conv_title = s("conv_title");
        e.group_chat = b("group_chat");
        e.sender = s("sender");
        e.avatar = s("avatar");
        e.picture = s("picture");
        e.icon = s("icon");
        e.sensitive = b("sensitive");
        e.private = b("private");
        e.silent = b("silent");
        e.ts = v.get("ts").and_then(Value::as_u64);
        e.messages = v
            .get("messages")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| serde_json::from_value::<NotifMessage>(m.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        e.sanitized()
    }

    /// Within its caps: names and texts cut, the last [`MAX_MESSAGES`]
    /// messages, pictures that aren't small JPEGs dropped.
    pub fn sanitized(mut self) -> Self {
        self.pkg = self.pkg.filter(|p| pkg_ok(p));
        self.conv = clip_opt(self.conv, MAX_NAME);
        self.conv_title = clip_opt(self.conv_title, MAX_NAME);
        self.sender = clip_opt(self.sender, MAX_NAME);
        self.avatar = self.avatar.filter(|a| jpeg_ok(a, MAX_AVATAR));
        self.picture = self.picture.filter(|p| jpeg_ok(p, MAX_PICTURE));
        self.icon = self.icon.filter(|h| hash_ok(h));
        let mut msgs: Vec<NotifMessage> = self
            .messages
            .into_iter()
            .map(|m| NotifMessage {
                sender: clip(&m.sender, MAX_NAME),
                text: clip(&m.text, MAX_TEXT),
                ts: m.ts,
            })
            .filter(|m| !m.text.is_empty())
            .collect();
        if msgs.len() > MAX_MESSAGES {
            msgs.drain(..msgs.len() - MAX_MESSAGES);
        }
        self.messages = msgs;
        self
    }

    /// Its content left out (private mode): who and what are gone, the app
    /// and its icon stay.
    pub fn redacted(self) -> Self {
        NotifExtra {
            pkg: self.pkg,
            icon: self.icon,
            ts: self.ts,
            silent: self.silent,
            sensitive: true,
            private: true,
            ..NotifExtra::default()
        }
    }

    /// The pictures left out (a slow link).
    pub fn without_pictures(mut self) -> Self {
        self.avatar = None;
        self.picture = None;
        self
    }
}

/// What a subscriber asked for besides its apps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubOpts {
    /// Only while the phone isn't in use (locked or screen off).
    #[serde(default)]
    pub only_locked: bool,
    /// Leave the content of a sensitive notification out.
    #[serde(default)]
    pub private: bool,
}

/// One subscriber's subscription, as kept across restarts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sub {
    pub apps: Vec<String>,
    #[serde(default)]
    pub removals: bool,
    #[serde(default)]
    pub opts: SubOpts,
}

/// The message a subscriber gets for a notification: `base` (type, from,
/// title, body, app, key, actions), plus `extra` when it understands it
/// (`rich`), without pictures unless `pictures`. None when it asked for
/// none now (`only_locked` while the phone is `in_use`).
pub fn outgoing(
    base: &Value,
    extra: &NotifExtra,
    opts: SubOpts,
    in_use: bool,
    rich: bool,
    pictures: bool,
) -> Option<Value> {
    if opts.only_locked && in_use {
        return None;
    }
    let mut m = base.clone();
    if !rich {
        return Some(m);
    }
    let mut extra = extra.clone();
    if opts.private && extra.sensitive {
        extra = extra.redacted();
        m["title"] = "".into();
        m["body"] = "".into();
        if m.get("actions").is_some() {
            m["actions"] = Value::Array(vec![]);
        }
    }
    if !pictures {
        extra = extra.without_pictures();
    }
    if let (Value::Object(fields), Ok(Value::Object(more))) = (&mut m, serde_json::to_value(&extra))
    {
        for (k, v) in more {
            fields.entry(k).or_insert(v);
        }
    }
    Some(m)
}

/// The subscriptions as kept, from what's in memory.
pub fn subs_json(
    subs: &HashMap<String, Vec<String>>,
    removals: &HashSet<String>,
    opts: &HashMap<String, SubOpts>,
) -> String {
    let all: HashMap<&String, Sub> = subs
        .iter()
        .map(|(n, apps)| {
            (
                n,
                Sub {
                    apps: apps.clone(),
                    removals: removals.contains(n),
                    opts: opts.get(n).copied().unwrap_or_default(),
                },
            )
        })
        .collect();
    serde_json::to_string(&all).unwrap_or_default()
}

/// The subscriptions kept (empty when none or unreadable).
pub fn parse_subs(json: &str) -> HashMap<String, Sub> {
    serde_json::from_str(json).unwrap_or_default()
}

#[derive(Default)]
pub struct NotifRich {
    /// Device -> whether it said `notif_rich`.
    caps: Mutex<HashMap<String, bool>>,
    /// Subscriber -> its options.
    opts: Mutex<HashMap<String, SubOpts>>,
    /// Package -> (icon id, JPEG base64): this device's apps' icons.
    icons: Mutex<HashMap<String, (String, String)>>,
}

fn st<P: Platform>(ctx: &NetCtx<P>) -> &NotifRich {
    &ctx.extras.notif
}

fn me<P: Platform>(ctx: &NetCtx<P>) -> String {
    ctx.identity.lock().node_id.clone()
}

// --- capability ------------------------------------------------------------

pub fn extend_status<P: Platform>(ctx: &NetCtx<P>, status: &mut Value) {
    if ctx.config.notif_rich {
        status[FLAG] = VERSION.into();
    }
}

/// `node` said (`flag`, from its status or `addr_update`) whether it
/// understands rich notifications. Kept across restarts.
pub fn remember<P: Platform>(ctx: &NetCtx<P>, node: &str, flag: &Value) {
    if node.is_empty() || !ctx.config.notif_rich {
        return;
    }
    let on = flag.as_u64().is_some_and(|v| v >= VERSION);
    if supports(ctx, node) == on {
        return;
    }
    st(ctx).caps.lock().insert(node.to_string(), on);
    ctx.platform
        .kv_set(&format!("{CAPS_PREFIX}{node}"), if on { "1" } else { "" });
}

pub fn supports<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    if !ctx.config.notif_rich || node.is_empty() {
        return false;
    }
    if let Some(on) = st(ctx).caps.lock().get(node) {
        return *on;
    }
    let on = ctx
        .platform
        .kv_get(&format!("{CAPS_PREFIX}{node}"))
        .is_some_and(|v| v == "1");
    st(ctx).caps.lock().insert(node.to_string(), on);
    on
}

// --- subscriptions -----------------------------------------------------------

/// Bring back the subscriptions kept last time (at start).
pub fn load_subs<P: Platform>(ctx: &NetCtx<P>) {
    let Some(json) = ctx.platform.kv_get(SUBS_KEY) else {
        return;
    };
    let kept = parse_subs(&json);
    let mut subs = ctx.subs.lock();
    let mut removals = ctx.removal_subs.lock();
    let mut opts = st(ctx).opts.lock();
    for (node, sub) in kept {
        if node.is_empty() || sub.apps.is_empty() {
            continue;
        }
        if sub.removals {
            removals.insert(node.clone());
        }
        opts.insert(node.clone(), sub.opts);
        subs.insert(node, sub.apps);
    }
}

/// A subscriber's options came with its `subscribe_apps`; everything is
/// written down for the next start.
pub fn subscribed<P: Platform>(ctx: &NetCtx<P>, node: &str, msg: &ControlMsg) {
    let opts = SubOpts {
        only_locked: msg.json("only_locked").as_bool() == Some(true),
        private: msg.json("private").as_bool() == Some(true),
    };
    st(ctx).opts.lock().insert(node.to_string(), opts);
    save_subs(ctx);
}

fn save_subs<P: Platform>(ctx: &NetCtx<P>) {
    let json = {
        let subs = ctx.subs.lock();
        let live: HashMap<String, Vec<String>> = subs
            .iter()
            .filter(|(_, apps)| !apps.is_empty())
            .map(|(n, a)| (n.clone(), a.clone()))
            .collect();
        subs_json(&live, &ctx.removal_subs.lock(), &st(ctx).opts.lock())
    };
    ctx.platform.kv_set(SUBS_KEY, &json);
}

/// What subscriber `node` asked for besides its apps.
pub fn opts_of<P: Platform>(ctx: &NetCtx<P>, node: &str) -> SubOpts {
    st(ctx).opts.lock().get(node).copied().unwrap_or_default()
}

// --- the producer --------------------------------------------------------------

/// The link to `node` is Bluetooth (no pictures over it).
fn slow_link<P: Platform>(ctx: &NetCtx<P>, node: &str) -> bool {
    let id = ctx.ws_conns.lock().get(node).map(|(id, _)| *id);
    id.is_some_and(|id| crate::engine::is_ble_link(ctx, id))
}

/// Share notification `notif` (JSON: title, body, app, key, actions, the
/// extra fields and `in_use`) of package `pkg` with its subscribers.
pub fn share<P: Platform>(ctx: &NetCtx<P>, pkg: &str, notif: &Value) {
    let me = me(ctx);
    let s = |k: &str| notif.get(k).and_then(Value::as_str).unwrap_or("");
    let mut base = serde_json::json!({
        "type": "notification", "from": me, "title": s("title"), "body": s("body"),
        "app": s("app"),
    });
    let key = s("key");
    if !key.is_empty() {
        base["key"] = key.into();
        base["actions"] = notif
            .get("actions")
            .filter(|a| a.is_array())
            .cloned()
            .unwrap_or(Value::Array(vec![]));
    }
    let mut extra = NotifExtra::from_value(notif);
    if extra.pkg.is_none() && pkg_ok(pkg) && pkg != super::notif_apps::ALL_APPS {
        extra.pkg = Some(pkg.to_string());
    }
    if extra.icon.is_none() {
        extra.icon = st(ctx).icons.lock().get(pkg).map(|(h, _)| h.clone());
    }
    let in_use = notif
        .get("in_use")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let targets: Vec<String> = ctx
        .subs
        .lock()
        .iter()
        .filter(|(_, pkgs)| pkgs.iter().any(|p| p == pkg))
        .map(|(n, _)| n.clone())
        .collect();
    for n in targets {
        // Only over a link that's up, as always: a notification isn't
        // worth connecting for, and one late is worth little.
        if !ctx.ws_conns.lock().contains_key(&n) {
            continue;
        }
        let rich = supports(ctx, &n);
        let pictures = rich && !slow_link(ctx, &n);
        if let Some(m) = outgoing(&base, &extra, opts_of(ctx, &n), in_use, rich, pictures) {
            crate::transport::send_control_to(ctx, &n, &m.to_string());
        }
    }
}

/// Producer: app `pkg`'s icon is `b64` (a small JPEG). False when it isn't
/// one.
pub fn set_icon<P: Platform>(ctx: &NetCtx<P>, pkg: &str, b64: &str) -> bool {
    if !pkg_ok(pkg) || !jpeg_ok(b64, MAX_ICON) {
        return false;
    }
    st(ctx)
        .icons
        .lock()
        .insert(pkg.to_string(), (icon_hash(b64), b64.to_string()));
    true
}

/// Subscriber: ask `node` for app `pkg`'s icon (it arrives as
/// `NotificationIcon`). Only to a device that understands it.
pub fn request_icon<P: Platform>(ctx: &NetCtx<P>, node: &str, pkg: &str) -> crate::Result<()> {
    if !pkg_ok(pkg) {
        return Err(crate::error::CoreError::InvalidInput(
            "not a package name".into(),
        ));
    }
    if !supports(ctx, node) {
        return Err(crate::error::CoreError::Unsupported(
            "this device's app is too old to send app icons",
        ));
    }
    let m = serde_json::json!({ "type": "notif_icon_request", "from": me(ctx), "pkg": pkg });
    crate::transport::send_control_to(ctx, node, &m.to_string());
    Ok(())
}

/// The messages only devices that said `notif_rich` send.
pub struct NotifRichHandler;

impl<P: Platform> ControlHandler<P> for NotifRichHandler {
    fn kinds(&self) -> &'static [&'static str] {
        &["notif_icon_request", "notif_icon"]
    }

    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg) {
        let from = msg.sender();
        let pkg = msg.str("pkg");
        if from.is_empty() || !pkg_ok(&pkg) {
            return;
        }
        match msg.kind() {
            "notif_icon_request" => {
                // Only an app it follows, only to a device that understands.
                let follows = ctx
                    .subs
                    .lock()
                    .get(&from)
                    .is_some_and(|apps| apps.contains(&pkg));
                if !follows || !supports(ctx, &from) {
                    return;
                }
                let Some((hash, data)) = st(ctx).icons.lock().get(&pkg).cloned() else {
                    return;
                };
                let m = serde_json::json!({
                    "type": "notif_icon", "from": me(ctx), "pkg": pkg, "hash": hash, "data": data,
                });
                crate::transport::send_control_to(ctx, &from, &m.to_string());
            }
            "notif_icon" => {
                let (hash, data) = (msg.str("hash"), msg.str("data"));
                if !hash_ok(&hash) || !jpeg_ok(&data, MAX_ICON) || icon_hash(&data) != hash {
                    return;
                }
                ctx.sink.emit(CoreEvent::NotificationIcon {
                    from,
                    pkg,
                    hash,
                    data,
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(n: usize) -> String {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0];
        b.resize(n, 7);
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn base() -> Value {
        serde_json::json!({
            "type": "notification", "from": "p", "title": "Asha", "body": "Your code is 123456",
            "app": "Bank", "key": "k", "actions": [{"title": "Copy 123456", "reply": false}],
        })
    }

    #[test]
    fn only_small_jpegs_pass() {
        assert!(jpeg_ok(&jpeg(100), MAX_AVATAR));
        assert!(jpeg_ok(&jpeg(MAX_AVATAR), MAX_AVATAR));
        assert!(!jpeg_ok(&jpeg(MAX_AVATAR + 1), MAX_AVATAR));
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n....");
        assert!(!jpeg_ok(&png, MAX_AVATAR));
        assert!(!jpeg_ok("not base64!", MAX_AVATAR));
        assert!(!jpeg_ok("", MAX_AVATAR));
    }

    #[test]
    fn extras_are_brought_within_their_caps() {
        let long = "x".repeat(5000);
        let msgs: Vec<Value> = (0..10)
            .map(|i| serde_json::json!({"sender": "Asha", "text": format!("m{i}"), "ts": i}))
            .collect();
        let v = serde_json::json!({
            "pkg": "com.whatsapp", "conv": long, "sender": "  Asha  ", "messages": msgs,
            "avatar": jpeg(MAX_AVATAR + 10), "picture": jpeg(2000), "icon": "zz",
            "sensitive": true, "ts": 5,
        });
        let e = NotifExtra::from_value(&v);
        assert_eq!(e.pkg.as_deref(), Some("com.whatsapp"));
        assert_eq!(e.conv.as_ref().map(|c| c.chars().count()), Some(MAX_NAME));
        assert_eq!(e.sender.as_deref(), Some("Asha"));
        assert_eq!(e.messages.len(), MAX_MESSAGES);
        assert_eq!(e.messages[0].text, "m4");
        assert_eq!(e.messages[5].text, "m9");
        assert!(e.avatar.is_none());
        assert!(e.picture.is_some());
        assert!(e.icon.is_none());
        assert!(e.sensitive);
        // Junk in a field: that field is left out, not the rest.
        let e = NotifExtra::from_value(&serde_json::json!({"messages": "x", "pkg": "a b"}));
        assert_eq!(e, NotifExtra::default());
    }

    #[test]
    fn an_older_subscriber_gets_the_old_shape() {
        let extra = NotifExtra {
            pkg: Some("com.bank".into()),
            sensitive: true,
            picture: Some(jpeg(100)),
            ..Default::default()
        };
        let opts = SubOpts {
            only_locked: false,
            private: true,
        };
        let m = outgoing(&base(), &extra, opts, false, false, false).unwrap();
        assert_eq!(m, base());
    }

    #[test]
    fn private_mode_leaves_a_sensitive_ones_content_out() {
        let extra = NotifExtra {
            pkg: Some("com.bank".into()),
            sender: Some("Bank".into()),
            messages: vec![NotifMessage {
                sender: "Bank".into(),
                text: "123456".into(),
                ts: 1,
            }],
            picture: Some(jpeg(100)),
            icon: Some("ab12".into()),
            sensitive: true,
            ..Default::default()
        };
        let private = SubOpts {
            only_locked: false,
            private: true,
        };
        let m = outgoing(&base(), &extra, private, false, true, true).unwrap();
        assert_eq!(m["title"], "");
        assert_eq!(m["body"], "");
        assert_eq!(m["actions"], serde_json::json!([]));
        assert_eq!(m["private"], true);
        assert_eq!(m["pkg"], "com.bank");
        assert_eq!(m["icon"], "ab12");
        assert_eq!(m["key"], "k");
        assert!(m.get("messages").is_none() && m.get("picture").is_none());
        assert!(!m.to_string().contains("123456"));
        // Not sensitive, or private mode off: all of it.
        let open = outgoing(&base(), &extra, SubOpts::default(), false, true, true).unwrap();
        assert_eq!(open["body"], "Your code is 123456");
        assert!(open.get("private").is_none());
        let plain = NotifExtra {
            sensitive: false,
            ..extra
        };
        let m = outgoing(&base(), &plain, private, false, true, true).unwrap();
        assert_eq!(m["title"], "Asha");
    }

    #[test]
    fn only_while_locked_and_no_pictures_on_a_slow_link() {
        let extra = NotifExtra {
            avatar: Some(jpeg(100)),
            picture: Some(jpeg(100)),
            ..Default::default()
        };
        let locked = SubOpts {
            only_locked: true,
            private: false,
        };
        assert!(outgoing(&base(), &extra, locked, true, true, true).is_none());
        assert!(outgoing(&base(), &extra, locked, false, true, true).is_some());
        let m = outgoing(&base(), &extra, SubOpts::default(), true, true, false).unwrap();
        assert!(m.get("avatar").is_none() && m.get("picture").is_none());
        let m = outgoing(&base(), &extra, SubOpts::default(), true, true, true).unwrap();
        assert!(m.get("avatar").is_some() && m.get("picture").is_some());
    }

    #[test]
    fn subscriptions_round_trip() {
        let subs = HashMap::from([
            ("a".to_string(), vec!["com.x".to_string()]),
            ("b".to_string(), vec!["*".to_string()]),
        ]);
        let removals = HashSet::from(["a".to_string()]);
        let opts = HashMap::from([(
            "a".to_string(),
            SubOpts {
                only_locked: true,
                private: true,
            },
        )]);
        let back = parse_subs(&subs_json(&subs, &removals, &opts));
        assert_eq!(back.len(), 2);
        assert_eq!(back["a"].apps, ["com.x"]);
        assert!(back["a"].removals && back["a"].opts.only_locked && back["a"].opts.private);
        assert!(!back["b"].removals && back["b"].opts == SubOpts::default());
        assert!(parse_subs("junk").is_empty());
    }

    #[test]
    fn icon_ids_and_packages() {
        let h = icon_hash(&jpeg(50));
        assert_eq!(h.len(), 16);
        assert!(hash_ok(&h));
        assert_ne!(h, icon_hash(&jpeg(51)));
        assert!(pkg_ok("com.whatsapp") && pkg_ok("*"));
        assert!(!pkg_ok("../etc") && !pkg_ok("..") && !pkg_ok("") && !pkg_ok("a b"));
    }
}
