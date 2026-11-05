//! The session-bus side (see the crate docs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use zbus::blocking::fdo::MonitoringProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::{MatchRule, Message};

use crate::parse::{self, Change, Tracker, FD_IFACE, GTK_IFACE};

const FD_PATH: &str = "/org/freedesktop/Notifications";

/// Just the notification traffic. A reply is matched by the daemon's
/// well-known name (the bus checks who owns it), so other replies on the
/// bus never come here.
const RULES: &[&str] = &[
    "type='method_call',interface='org.freedesktop.Notifications',member='Notify'",
    "type='method_call',interface='org.freedesktop.Notifications',member='CloseNotification'",
    "type='method_return',sender='org.freedesktop.Notifications'",
    "type='signal',interface='org.freedesktop.Notifications',member='NotificationClosed'",
    "type='method_call',interface='org.gtk.Notifications',member='AddNotification'",
    "type='method_call',interface='org.gtk.Notifications',member='RemoveNotification'",
];

pub struct Watcher {
    /// The monitor connection (closed on drop, which ends the thread).
    conn: Connection,
    tracker: Arc<Mutex<Tracker>>,
    stopped: Arc<AtomicBool>,
}

impl Watcher {
    pub fn start(
        skip: Vec<String>,
        mut on_change: impl FnMut(Change) + Send + 'static,
    ) -> Result<Watcher, String> {
        let conn = Connection::session()
            .map_err(|e| format!("couldn't reach this session's D-Bus: {e}"))?;
        let rules = RULES
            .iter()
            .map(|r| MatchRule::try_from(*r))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        // Listening before becoming a monitor, so nothing in between is lost.
        let messages = MessageIterator::from(&conn);
        let refused = |e: &dyn std::fmt::Display| {
            format!("this session's D-Bus won't let my-domain read notifications: {e}")
        };
        MonitoringProxy::new(&conn)
            .map_err(|e| refused(&e))?
            .become_monitor(&rules, 0)
            .map_err(|e| refused(&e))?;
        let tracker = Arc::new(Mutex::new(Tracker::new(&skip)));
        let stopped = Arc::new(AtomicBool::new(false));
        let (t, stop) = (tracker.clone(), stopped.clone());
        std::thread::Builder::new()
            .name("notify-watch".into())
            .spawn(move || {
                for msg in messages {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(msg) = msg else { break };
                    let change = match t.lock() {
                        Ok(mut t) => handle(&mut t, &msg),
                        Err(_) => break,
                    };
                    if let Some(change) = change {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        on_change(change);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Watcher {
            conn,
            tracker,
            stopped,
        })
    }

    pub fn dismiss(&self, key: &str) -> bool {
        let Some(id) = parse::fd_id(key) else {
            return false;
        };
        let live = self.tracker.lock().map(|t| t.is_live(key)).unwrap_or(false);
        if !live {
            return false;
        }
        // A monitor can't send: a connection of its own.
        let Ok(conn) = Connection::session() else {
            return false;
        };
        conn.call_method(
            Some(FD_IFACE),
            FD_PATH,
            Some(FD_IFACE),
            "CloseNotification",
            &(id,),
        )
        .is_ok()
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.conn.clone().close();
    }
}

/// What one message on the bus changes.
fn handle(t: &mut Tracker, msg: &Message) -> Option<Change> {
    let header = msg.header();
    match header.message_type() {
        Type::MethodCall => {
            let iface = header.interface()?.as_str().to_string();
            let member = header.member()?.as_str().to_string();
            match (iface.as_str(), member.as_str()) {
                (FD_IFACE, "Notify") => {
                    let sender = header.sender()?.as_str().to_string();
                    let args: parse::NotifyArgs = msg.body().deserialize().ok()?;
                    let serial = msg.primary_header().serial_num().get();
                    t.notify(&sender, serial, parse::notify_call(args))
                }
                (FD_IFACE, "CloseNotification") => {
                    let id: u32 = msg.body().deserialize().ok()?;
                    t.closed(id)
                }
                (GTK_IFACE, "AddNotification") => {
                    let args: parse::GtkAddArgs = msg.body().deserialize().ok()?;
                    t.gtk_add(parse::gtk_call(args))
                }
                (GTK_IFACE, "RemoveNotification") => {
                    let (app, id): (String, String) = msg.body().deserialize().ok()?;
                    t.gtk_remove(&app, &id)
                }
                _ => None,
            }
        }
        Type::MethodReturn => {
            let dest = header.destination()?.as_str().to_string();
            let serial = header.reply_serial()?.get();
            // Only Notify's reply is one u32; others don't read as one.
            let id: u32 = msg.body().deserialize().ok()?;
            t.reply(&dest, serial, id)
        }
        Type::Signal
            if header
                .member()
                .is_some_and(|m| m.as_str() == "NotificationClosed") =>
        {
            let (id, _reason): (u32, u32) = msg.body().deserialize().ok()?;
            t.closed(id)
        }
        _ => None,
    }
}
