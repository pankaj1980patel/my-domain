//! The session-bus side: asking the portals, and what's installed.

pub mod capture;
pub mod inject;
pub mod remote;
pub mod screen;
pub mod service;
pub mod shortcuts;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::MatchRule;
use zvariant::{OwnedObjectPath, Value};

use crate::caps::{Portals, Tools, DEST, INTERFACES, PATH};
use crate::response::{self, Dict, Outcome};
use crate::{gst, uinput};

/// How long a portal's prompt may stay open.
const ANSWER_WAIT: Duration = Duration::from_secs(180);

/// What the user sees when they said no.
pub const CANCELLED: &str = "You didn\u{2019}t allow it.";

/// Options or results as sent (`a{sv}`).
pub(crate) type Opts = HashMap<&'static str, Value<'static>>;

fn bus_error(e: impl std::fmt::Display) -> String {
    format!("the desktop portal didn\u{2019}t answer: {e}")
}

/// The portals present, and their versions.
pub fn portals() -> Portals {
    let Ok(conn) = Connection::session() else {
        return Portals::default();
    };
    let found: Vec<(&str, u32)> = INTERFACES
        .iter()
        .filter_map(|i| version(&conn, i).map(|v| (*i, v)))
        .collect();
    Portals::from_versions(found)
}

/// A property of a portal interface (`version`, `AvailableCursorModes`).
pub(crate) fn property(conn: &Connection, iface: &str, name: &str) -> Option<u32> {
    let reply = conn
        .call_method(
            Some(DEST),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(iface, name),
        )
        .ok()?;
    let v: zvariant::OwnedValue = reply.body().deserialize().ok()?;
    response::as_u32(&v)
}

fn version(conn: &Connection, iface: &str) -> Option<u32> {
    property(conn, iface, "version")
}

/// `cmd` is an executable file on `PATH`.
pub(crate) fn on_path(cmd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(cmd))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}

/// GStreamer can read a PipeWire stream into JPEGs here.
fn gst_ready() -> bool {
    on_path(gst::LAUNCH)
        && on_path(gst::INSPECT)
        && gst::ELEMENTS.iter().all(|e| {
            Command::new(gst::INSPECT)
                .arg(e)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
}

/// Someone on the session bus owns `name`.
fn has_owner(name: &str) -> bool {
    let Ok(conn) = Connection::session() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(name,),
    )
    .ok()
    .and_then(|r| r.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

/// The helper programs and devices found here (runs `gst-inspect-1.0`:
/// keep the answer a while).
pub fn tools() -> Tools {
    Tools {
        pipewire: runtime_dir().is_some_and(|d| d.join("pipewire-0").exists()),
        gst_pipewire: gst_ready(),
        grim: on_path("grim"),
        wl_clipboard: on_path("wl-copy") && on_path("wl-paste"),
        uinput: uinput::writable(),
        tray: has_owner("org.kde.StatusNotifierWatcher"),
    }
}

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// One connection to the portals. A portal session lives as long as the
/// connection that made it: dropping this ends it.
pub(crate) struct Portal {
    pub conn: Connection,
    unique: String,
}

impl Portal {
    pub fn new() -> Result<Portal, String> {
        let conn = Connection::session().map_err(bus_error)?;
        let unique = conn
            .unique_name()
            .map(|n| n.as_str().to_string())
            .ok_or_else(|| bus_error("no name on the bus"))?;
        Ok(Portal { conn, unique })
    }

    pub fn token(what: &str) -> String {
        response::token(what, NEXT_TOKEN.fetch_add(1, Ordering::Relaxed))
    }

    pub fn version(&self, iface: &str) -> Option<u32> {
        version(&self.conn, iface)
    }

    /// Listen for the answer to the request made with `token`, before
    /// making it (so it can't be missed).
    pub fn listen(&self, token: &str) -> Result<MessageIterator, String> {
        let path = response::request_path(&self.unique, token);
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .and_then(|b| b.member("Response"))
            .and_then(|b| b.path(path.as_str()))
            .map_err(bus_error)?
            .build();
        MessageIterator::for_match_rule(rule, &self.conn, Some(4)).map_err(bus_error)
    }

    /// Wait for the answer (the user may be looking at a prompt).
    pub fn wait(&self, it: MessageIterator) -> Result<Dict, String> {
        let (tx, rx) = mpsc::channel::<(u32, Dict)>();
        let _ = std::thread::Builder::new()
            .name("portal-answer".into())
            .spawn(move || {
                for m in it {
                    let Ok(m) = m else { break };
                    if let Ok(r) = m.body().deserialize::<(u32, Dict)>() {
                        let _ = tx.send(r);
                        break;
                    }
                }
            });
        match rx.recv_timeout(ANSWER_WAIT) {
            Ok((code, d)) => match response::outcome(code, d) {
                Outcome::Done(d) => Ok(d),
                Outcome::Cancelled => Err(CANCELLED.into()),
                Outcome::Failed => Err("The desktop portal refused.".into()),
            },
            Err(_) => Err("No answer from the desktop portal.".into()),
        }
    }

    /// `opts` with a fresh `handle_token`, and the listener for its answer.
    pub fn ask(&self, what: &str, mut opts: Opts) -> Result<(Opts, MessageIterator), String> {
        let token = Self::token(what);
        let it = self.listen(&token)?;
        opts.insert("handle_token", Value::from(token));
        Ok((opts, it))
    }

    /// `CreateSession` on `iface` (InputCapture's takes a parent window
    /// first).
    pub fn create_session(&self, iface: &str, extra: Opts) -> Result<OwnedObjectPath, String> {
        let (mut opts, it) = self.ask("create", extra)?;
        opts.insert("session_handle_token", Value::from(Self::token("session")));
        let sent = if iface == crate::caps::INPUT_CAPTURE {
            self.conn
                .call_method(Some(DEST), PATH, Some(iface), "CreateSession", &("", opts))
        } else {
            self.conn
                .call_method(Some(DEST), PATH, Some(iface), "CreateSession", &(opts,))
        };
        sent.map_err(bus_error)?;
        let d = self.wait(it)?;
        let handle = response::session_handle(&d).ok_or_else(|| bus_error("no session"))?;
        OwnedObjectPath::try_from(handle).map_err(bus_error)
    }

    /// A session method that answers through a request (`SelectSources`,
    /// `SelectDevices`, `GetZones`...): `(session, options)`.
    pub fn on_session(
        &self,
        iface: &str,
        method: &str,
        session: &OwnedObjectPath,
        opts: Opts,
    ) -> Result<Dict, String> {
        let (opts, it) = self.ask(method, opts)?;
        self.conn
            .call_method(Some(DEST), PATH, Some(iface), method, &(session, opts))
            .map_err(bus_error)?;
        self.wait(it)
    }

    /// `Start(session, parent_window, options)`: the prompt, if any.
    pub fn start(&self, iface: &str, session: &OwnedObjectPath) -> Result<Dict, String> {
        let (opts, it) = self.ask("start", Opts::new())?;
        self.conn
            .call_method(Some(DEST), PATH, Some(iface), "Start", &(session, "", opts))
            .map_err(bus_error)?;
        self.wait(it)
    }

    /// A call with no answer to wait for; false if it failed.
    pub fn call<B>(&self, iface: &str, method: &str, body: &B) -> bool
    where
        B: zbus::export::serde::Serialize + zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(DEST), PATH, Some(iface), method, body)
            .is_ok()
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        // Ends the portal session, and any thread still waiting on it.
        let _ = self.conn.clone().close();
    }
}
