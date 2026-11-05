//! The parts that don't touch the bus: what a Notify call (or GNOME's
//! `org.gtk.Notifications.AddNotification`) says, its text cleaned up, and
//! which notifications are on screen, so a close is reported only for one
//! that was seen and a dismiss only touches one that's still there.
//!
//! A freedesktop notification's id is only in the daemon's *reply* to
//! Notify, so a call waits (`pending`) until its reply comes; one that
//! replaces an earlier notification already names its id.

use std::collections::{HashMap, HashSet, VecDeque};

/// The freedesktop notifications interface (every desktop but GNOME's own
/// apps).
pub const FD_IFACE: &str = "org.freedesktop.Notifications";
/// GNOME Shell's interface (GTK's `GNotification`, Flatpak apps through the
/// portal on GNOME).
pub const GTK_IFACE: &str = "org.gtk.Notifications";

/// The most notifications remembered as on screen (the oldest go first).
const MAX_LIVE: usize = 500;
/// The most Notify calls waiting for their reply.
const MAX_PENDING: usize = 200;
/// The longest title and body passed on, in characters.
const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 1000;

/// A notification seen here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Names it while it's on screen: `fd:<id>` or `gtk:<app id>:<id>`.
    pub key: String,
    /// The app, as the per-app list keeps it (its name, or GNOME's app id).
    pub app_id: String,
    /// The app's name to show.
    pub app: String,
    pub title: String,
    pub body: String,
}

/// What happened on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// A notification appeared (or an earlier one with the same key changed).
    Posted(Seen),
    /// The notification with this key went away.
    Closed(String),
}

/// What a Notify call asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotifyCall {
    pub app_name: String,
    /// The notification it replaces (0: a new one).
    pub replaces_id: u32,
    pub summary: String,
    pub body: String,
    /// The `desktop-entry` hint: the app's desktop file name.
    pub desktop_entry: Option<String>,
}

/// What an `AddNotification` call says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GtkCall {
    pub app_id: String,
    pub id: String,
    pub title: String,
    pub body: String,
}

/// A Notify call's arguments as they are on the bus, `susssasa{sv}i`:
/// app name, replaces id, icon, summary, body, actions, hints, timeout.
#[cfg(any(target_os = "linux", test))]
pub type NotifyArgs = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, zvariant::OwnedValue>,
    i32,
);

/// An `AddNotification` call's arguments, `ssa{sv}`: app id, notification
/// id, and the notification (`title`, `body`, ...).
#[cfg(any(target_os = "linux", test))]
pub type GtkAddArgs = (String, String, HashMap<String, zvariant::OwnedValue>);

/// A string entry of a D-Bus dictionary.
#[cfg(any(target_os = "linux", test))]
fn dict_str(dict: &HashMap<String, zvariant::OwnedValue>, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(|v| String::try_from(&**v).ok())
        .filter(|s| !s.trim().is_empty())
}

/// What a Notify call's arguments say.
#[cfg(any(target_os = "linux", test))]
pub fn notify_call(args: NotifyArgs) -> NotifyCall {
    let (app_name, replaces_id, _icon, summary, body, _actions, hints, _timeout) = args;
    NotifyCall {
        app_name,
        replaces_id,
        summary,
        body,
        desktop_entry: dict_str(&hints, "desktop-entry"),
    }
}

/// What an `AddNotification` call's arguments say.
#[cfg(any(target_os = "linux", test))]
pub fn gtk_call(args: GtkAddArgs) -> GtkCall {
    let (app_id, id, n) = args;
    GtkCall {
        app_id,
        id,
        title: dict_str(&n, "title").unwrap_or_default(),
        body: dict_str(&n, "body").unwrap_or_default(),
    }
}

/// A notification body as plain text: the markup some apps send (`<b>`,
/// `<a href=...>`, `<img .../>`) taken out, entities (`&amp;`) decoded,
/// runs of blank lines and spaces at the ends trimmed. A `<` that doesn't
/// start a tag stays.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(inner) = tail.strip_prefix('<') {
            let is_tag = inner
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '/');
            match (is_tag, tail.find('>')) {
                (true, Some(end)) => {
                    let tag = tail[1..end].trim_start_matches('/').to_ascii_lowercase();
                    if tag.starts_with("br") || tag.starts_with("p ") || tag == "p" {
                        out.push('\n');
                    }
                    rest = &tail[end + 1..];
                }
                _ => {
                    out.push('<');
                    rest = inner;
                }
            }
        } else {
            match tail.find(';').filter(|&end| end <= 10) {
                Some(end) => match entity(&tail[1..end]) {
                    Some(c) => {
                        out.push(c);
                        rest = &tail[end + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &tail[1..];
                    }
                },
                None => {
                    out.push('&');
                    rest = &tail[1..];
                }
            }
        }
    }
    out.push_str(rest);
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    let mut text = String::new();
    let mut blank = false;
    for line in lines {
        if line.trim().is_empty() {
            blank = !text.is_empty();
            continue;
        }
        if !text.is_empty() {
            text.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        text.push_str(line);
    }
    text.trim().to_string()
}

/// The character an entity names (`amp`, `#39`, `#x27`).
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// At most `max` characters of `s`, with "…" when cut.
fn cut(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A GNOME app id's name to show: `org.gnome.Evolution` → `Evolution`.
pub fn app_label(app_id: &str) -> String {
    let id = app_id.trim().trim_end_matches(".desktop");
    let last = id.rsplit('.').next().unwrap_or(id);
    if last.is_empty() {
        id.to_string()
    } else {
        last.to_string()
    }
}

/// The freedesktop id in a key from [`Tracker`] (`fd:<id>`).
pub fn fd_id(key: &str) -> Option<u32> {
    key.strip_prefix("fd:")?.parse().ok()
}

/// Which notifications are on screen, from the calls, replies and signals
/// seen on the bus. Feed it what the bus says; it says what changed.
#[derive(Debug, Default)]
pub struct Tracker {
    /// App names (lowercase) never passed on: this app's own.
    skip: Vec<String>,
    /// Notify calls waiting for their reply: (caller, serial) -> call.
    pending: HashMap<(String, u32), NotifyCall>,
    /// The order the pending calls came in (the oldest go first).
    pending_order: VecDeque<(String, u32)>,
    /// Keys of the notifications on screen, oldest first.
    live: VecDeque<String>,
    live_set: HashSet<String>,
}

impl Tracker {
    /// `skip`: app names (or desktop file / GNOME app ids) whose
    /// notifications are never passed on, any case.
    pub fn new<S: AsRef<str>>(skip: &[S]) -> Self {
        Tracker {
            skip: skip
                .iter()
                .map(|s| s.as_ref().trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect(),
            ..Tracker::default()
        }
    }

    fn skipped(&self, names: &[Option<&str>]) -> bool {
        names.iter().flatten().any(|n| {
            let n = n.trim().trim_end_matches(".desktop").to_lowercase();
            self.skip.contains(&n)
        })
    }

    /// The notification with this key is on screen (as far as we saw).
    pub fn is_live(&self, key: &str) -> bool {
        self.live_set.contains(key)
    }

    fn posted(&mut self, seen: Seen) -> Option<Change> {
        if !self.live_set.contains(&seen.key) {
            self.live_set.insert(seen.key.clone());
            self.live.push_back(seen.key.clone());
            while self.live.len() > MAX_LIVE {
                if let Some(old) = self.live.pop_front() {
                    self.live_set.remove(&old);
                }
            }
        }
        Some(Change::Posted(seen))
    }

    fn gone(&mut self, key: String) -> Option<Change> {
        if !self.live_set.remove(&key) {
            return None;
        }
        self.live.retain(|k| *k != key);
        Some(Change::Closed(key))
    }

    /// A Notify call from `sender` (its unique bus name) with this serial.
    /// A new notification is reported once its reply says its id.
    pub fn notify(&mut self, sender: &str, serial: u32, call: NotifyCall) -> Option<Change> {
        if self.skipped(&[Some(&call.app_name), call.desktop_entry.as_deref()]) {
            return None;
        }
        if call.replaces_id != 0 {
            let id = call.replaces_id;
            return self.seen_fd(id, call);
        }
        let key = (sender.to_string(), serial);
        if self.pending.insert(key.clone(), call).is_none() {
            self.pending_order.push_back(key);
        }
        while self.pending_order.len() > MAX_PENDING {
            if let Some(old) = self.pending_order.pop_front() {
                self.pending.remove(&old);
            }
        }
        None
    }

    /// The daemon's reply to `destination`'s call `reply_serial`: the new
    /// notification's id.
    pub fn reply(&mut self, destination: &str, reply_serial: u32, id: u32) -> Option<Change> {
        let key = (destination.to_string(), reply_serial);
        let call = self.pending.remove(&key)?;
        self.pending_order.retain(|k| *k != key);
        self.seen_fd(id, call)
    }

    fn seen_fd(&mut self, id: u32, call: NotifyCall) -> Option<Change> {
        let title = cut(plain_text(&call.summary), MAX_TITLE);
        let body = cut(plain_text(&call.body), MAX_BODY);
        if title.is_empty() && body.is_empty() {
            return None;
        }
        let name = call.app_name.trim();
        let (app_id, app) = match (name.is_empty(), call.desktop_entry) {
            (false, _) => (name.to_string(), name.to_string()),
            (true, Some(entry)) => (entry.clone(), app_label(&entry)),
            (true, None) => (String::new(), String::new()),
        };
        self.posted(Seen {
            key: format!("fd:{id}"),
            app_id,
            app,
            title,
            body,
        })
    }

    /// The freedesktop notification `id` was closed (`NotificationClosed`,
    /// or a `CloseNotification` call).
    pub fn closed(&mut self, id: u32) -> Option<Change> {
        self.gone(format!("fd:{id}"))
    }

    /// A GNOME `AddNotification` call (a new notification, or one replaced).
    pub fn gtk_add(&mut self, call: GtkCall) -> Option<Change> {
        if call.app_id.is_empty() || self.skipped(&[Some(&call.app_id)]) {
            return None;
        }
        let title = cut(plain_text(&call.title), MAX_TITLE);
        let body = cut(plain_text(&call.body), MAX_BODY);
        if title.is_empty() && body.is_empty() {
            return None;
        }
        self.posted(Seen {
            key: format!("gtk:{}:{}", call.app_id, call.id),
            app: app_label(&call.app_id),
            app_id: call.app_id,
            title,
            body,
        })
    }

    /// A GNOME `RemoveNotification` call.
    pub fn gtk_remove(&mut self, app_id: &str, id: &str) -> Option<Change> {
        self.gone(format!("gtk:{app_id}:{id}"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use zvariant::serialized::{Context, Data};
    use zvariant::{OwnedValue, Value, LE};

    fn val(s: &str) -> OwnedValue {
        Value::from(s).try_into().unwrap()
    }

    /// `value` as a D-Bus message body (little-endian), then read back as
    /// `T`: what zbus's `Body::deserialize` does with a received message.
    fn over_the_bus<T>(value: &T) -> T
    where
        T: serde::Serialize + zvariant::Type + for<'d> serde::Deserialize<'d>,
    {
        let ctxt = Context::new_dbus(LE, 0);
        let data: Data<'static, 'static> = zvariant::to_bytes(ctxt, value).unwrap();
        data.deserialize::<T>().unwrap().0
    }

    fn notify_args(app: &str, replaces: u32, summary: &str, body: &str) -> NotifyArgs {
        let mut hints = HashMap::new();
        hints.insert("urgency".to_string(), OwnedValue::from(2u8));
        hints.insert("desktop-entry".to_string(), val("thunderbird"));
        (
            app.to_string(),
            replaces,
            "mail-unread".to_string(),
            summary.to_string(),
            body.to_string(),
            vec!["default".to_string(), "Open".to_string()],
            hints,
            -1,
        )
    }

    #[test]
    fn a_notify_body_reads_into_a_call() {
        let args = over_the_bus(&notify_args(
            "Thunderbird",
            0,
            "Asha",
            "Lunch <b>today</b>?",
        ));
        assert_eq!(
            <NotifyArgs as zvariant::Type>::SIGNATURE.to_string(),
            "(susssasa{sv}i)"
        );
        let call = notify_call(args);
        assert_eq!(
            call,
            NotifyCall {
                app_name: "Thunderbird".into(),
                replaces_id: 0,
                summary: "Asha".into(),
                body: "Lunch <b>today</b>?".into(),
                desktop_entry: Some("thunderbird".into()),
            }
        );
    }

    #[test]
    fn a_gtk_body_reads_into_a_call() {
        let mut n = HashMap::new();
        n.insert("title".to_string(), val("New mail"));
        n.insert("body".to_string(), val("From Asha"));
        n.insert("priority".to_string(), val("normal"));
        let args: GtkAddArgs = ("org.gnome.Evolution".into(), "new-mail-3".into(), n);
        assert_eq!(
            <GtkAddArgs as zvariant::Type>::SIGNATURE.to_string(),
            "(ssa{sv})"
        );
        let call = gtk_call(over_the_bus(&args));
        assert_eq!(
            call,
            GtkCall {
                app_id: "org.gnome.Evolution".into(),
                id: "new-mail-3".into(),
                title: "New mail".into(),
                body: "From Asha".into(),
            }
        );
    }

    #[test]
    fn a_notification_is_seen_once_its_reply_names_it() {
        let mut t = Tracker::new(&["my-domain"]);
        let call = notify_call(notify_args("Thunderbird", 0, "Asha", "Lunch &amp; a walk?"));
        assert_eq!(t.notify(":1.42", 7, call), None);
        // Someone else's reply with the same serial: not ours.
        assert_eq!(t.reply(":1.99", 7, 5), None);
        let got = t.reply(":1.42", 7, 12).unwrap();
        assert_eq!(
            got,
            Change::Posted(Seen {
                key: "fd:12".into(),
                app_id: "Thunderbird".into(),
                app: "Thunderbird".into(),
                title: "Asha".into(),
                body: "Lunch & a walk?".into(),
            })
        );
        assert!(t.is_live("fd:12"));
        // The reply comes once.
        assert_eq!(t.reply(":1.42", 7, 12), None);
        assert_eq!(t.closed(12), Some(Change::Closed("fd:12".into())));
        assert!(!t.is_live("fd:12"));
        // Closed again (the signal after a CloseNotification call): once.
        assert_eq!(t.closed(12), None);
        // Never seen: nothing to say.
        assert_eq!(t.closed(99), None);
    }

    #[test]
    fn a_replacing_notification_is_seen_at_once() {
        let mut t = Tracker::new::<&str>(&[]);
        let call = notify_call(notify_args("", 30, "Download", "80% done"));
        let Some(Change::Posted(seen)) = t.notify(":1.5", 1, call) else {
            panic!("not posted");
        };
        assert_eq!(seen.key, "fd:30");
        // No app name: the desktop file names it.
        assert_eq!(
            (seen.app_id.as_str(), seen.app.as_str()),
            ("thunderbird", "thunderbird")
        );
    }

    #[test]
    fn this_apps_own_notifications_and_empty_ones_are_skipped() {
        let mut t = Tracker::new(&["My-Domain", "com.landiscover.app"]);
        let own = notify_call(notify_args("my-domain", 0, "Pixel 8", "Asha: hi"));
        assert_eq!(t.notify(":1.1", 1, own), None);
        assert_eq!(t.reply(":1.1", 1, 3), None);
        let mut by_entry = notify_call(notify_args("", 0, "x", "y"));
        by_entry.desktop_entry = Some("com.landiscover.app.desktop".into());
        t.notify(":1.1", 2, by_entry);
        assert_eq!(t.reply(":1.1", 2, 4), None);
        let empty = notify_call(notify_args("Thunderbird", 0, " ", "<b></b>"));
        t.notify(":1.1", 3, empty);
        assert_eq!(t.reply(":1.1", 3, 5), None);
        let gtk_own = GtkCall {
            app_id: "com.landiscover.app".into(),
            id: "1".into(),
            title: "t".into(),
            body: String::new(),
        };
        assert_eq!(t.gtk_add(gtk_own), None);
    }

    #[test]
    fn gnome_notifications_come_and_go() {
        let mut t = Tracker::new(&["my-domain"]);
        let call = GtkCall {
            app_id: "org.gnome.Evolution".into(),
            id: "new-mail".into(),
            title: "New mail".into(),
            body: "From Asha".into(),
        };
        let Some(Change::Posted(seen)) = t.gtk_add(call) else {
            panic!("not posted");
        };
        assert_eq!(seen.key, "gtk:org.gnome.Evolution:new-mail");
        assert_eq!(seen.app, "Evolution");
        assert_eq!(seen.app_id, "org.gnome.Evolution");
        assert_eq!(
            t.gtk_remove("org.gnome.Evolution", "new-mail"),
            Some(Change::Closed("gtk:org.gnome.Evolution:new-mail".into()))
        );
        assert_eq!(fd_id(&seen.key), None);
        assert_eq!(fd_id("fd:17"), Some(17));
    }

    #[test]
    fn markup_and_entities_become_plain_text() {
        assert_eq!(plain_text("<b>Asha</b> &lt;asha@x&gt;"), "Asha <asha@x>");
        assert_eq!(
            plain_text(r#"See <a href="https://x.org">this</a><br/>now"#),
            "See this\nnow"
        );
        assert_eq!(plain_text("a < b && c"), "a < b && c");
        assert_eq!(
            plain_text("it&#39;s &#x2014; &bogus; done"),
            "it's — &bogus; done"
        );
        assert_eq!(plain_text("<img src=\"x.png\"/>  \n\n\n  hi  "), "hi");
        assert_eq!(cut("abcdef".into(), 4), "abc…");
        assert_eq!(cut("abc".into(), 4), "abc");
    }

    #[test]
    fn the_on_screen_list_stays_bounded() {
        let mut t = Tracker::new::<&str>(&[]);
        for id in 1..=(MAX_LIVE as u32 + 10) {
            let call = notify_call(notify_args("App", id, "t", "b"));
            t.notify(":1.1", id, call);
        }
        assert!(!t.is_live("fd:1"));
        assert!(t.is_live(&format!("fd:{}", MAX_LIVE + 10)));
        for serial in 0..(MAX_PENDING as u32 + 10) {
            t.notify(":1.1", serial, notify_call(notify_args("App", 0, "t", "b")));
        }
        assert_eq!(t.pending.len(), MAX_PENDING);
        assert_eq!(t.reply(":1.1", 0, 1), None);
    }
}
