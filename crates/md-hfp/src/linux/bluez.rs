//! BlueZ over D-Bus (system bus): the hands-free profile we register, and
//! the few calls we make.
//!
//! BlueZ owns pairing and the RFCOMM channel: we register an external
//! profile (`org.bluez.ProfileManager1.RegisterProfile`, HFP HF UUID,
//! client role) and BlueZ hands us the channel's fd in
//! `org.bluez.Profile1.NewConnection` once `Device1.ConnectProfile` (or the
//! phone) connects it.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::Msg;
use crate::{Gateway, HFP_AG_UUID, HFP_HF_UUID};

/// Where our `org.bluez.Profile1` object lives.
pub const PROFILE_PATH: &str = "/org/mydomain/hfp";

/// BlueZ's calls into us.
pub struct Profile {
    pub tx: Sender<Msg>,
}

#[zbus::interface(name = "org.bluez.Profile1")]
impl Profile {
    /// BlueZ dropped the profile (it's shutting down or restarting).
    fn release(&self) {
        let _ = self.tx.send(Msg::Released);
    }

    /// A phone's hands-free channel is connected: here is its RFCOMM fd.
    fn new_connection(
        &self,
        device: OwnedObjectPath,
        fd: zbus::zvariant::OwnedFd,
        _properties: HashMap<String, OwnedValue>,
    ) {
        let _ = self.tx.send(Msg::NewConnection {
            device: device.to_string(),
            fd: fd.into(),
        });
    }

    /// BlueZ wants the channel closed (the phone or the user disconnected).
    fn request_disconnection(&self, device: OwnedObjectPath) {
        let _ = self.tx.send(Msg::RequestDisconnection(device.to_string()));
    }
}

/// The system bus, with our profile object served on it.
pub fn connect(tx: Sender<Msg>) -> zbus::Result<Connection> {
    zbus::blocking::connection::Builder::system()?
        .serve_at(PROFILE_PATH, Profile { tx })?
        .build()
}

/// How registering went.
pub enum Registered {
    Ok,
    /// BlueZ said this hands-free profile is already registered.
    Taken,
}

/// Register as an HFP hands-free (client role: we connect to the phone;
/// BlueZ connects us too when the phone connects, `AutoConnect`).
pub fn register(conn: &Connection, features: u16) -> Result<Registered, String> {
    let pm = Proxy::new(conn, "org.bluez", "/org/bluez", "org.bluez.ProfileManager1")
        .map_err(|e| e.to_string())?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("Name", Value::from("my-domain hands-free"));
    opts.insert("Role", Value::from("client"));
    opts.insert("Features", Value::from(features));
    opts.insert("RequireAuthentication", Value::from(true));
    opts.insert("AutoConnect", Value::from(true));
    let path = ObjectPath::try_from(PROFILE_PATH).map_err(|e| e.to_string())?;
    match pm.call_method("RegisterProfile", &(path, HFP_HF_UUID, opts)) {
        Ok(_) => Ok(Registered::Ok),
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.bluez.Error.AlreadyExists" =>
        {
            Ok(Registered::Taken)
        }
        Err(e) => Err(e.to_string()),
    }
}

type Props = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

fn objects(conn: &Connection) -> Result<Objects, String> {
    let om = Proxy::new(conn, "org.bluez", "/", "org.freedesktop.DBus.ObjectManager")
        .map_err(|e| e.to_string())?;
    om.call("GetManagedObjects", &())
        .map_err(|e| format!("Bluetooth (BlueZ) didn\u{2019}t answer: {e}"))
}

fn prop_str(p: &Props, key: &str) -> Option<String> {
    match &**p.get(key)? {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn prop_bool(p: &Props, key: &str) -> bool {
    matches!(p.get(key).map(|v| &**v), Some(Value::Bool(true)))
}

fn prop_strs(p: &Props, key: &str) -> Vec<String> {
    match p.get(key).map(|v| &**v) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A paired phone: its gateway entry and D-Bus path.
pub struct Device {
    pub gateway: Gateway,
    pub path: String,
}

/// Paired devices offering the Hands-Free Audio Gateway service.
pub fn gateways(conn: &Connection) -> Result<Vec<Device>, String> {
    let mut out = Vec::new();
    for (path, ifaces) in objects(conn)? {
        let Some(p) = ifaces.get("org.bluez.Device1") else {
            continue;
        };
        let ag = prop_strs(p, "UUIDs")
            .iter()
            .any(|u| u.eq_ignore_ascii_case(HFP_AG_UUID));
        if !ag || !prop_bool(p, "Paired") {
            continue;
        }
        let Some(address) = prop_str(p, "Address") else {
            continue;
        };
        let name = prop_str(p, "Alias")
            .or_else(|| prop_str(p, "Name"))
            .unwrap_or_else(|| address.clone());
        out.push(Device {
            gateway: Gateway {
                address,
                name,
                connected: prop_bool(p, "Connected"),
            },
            path: path.to_string(),
        });
    }
    out.sort_by_key(|d| d.gateway.name.to_lowercase());
    Ok(out)
}

/// The first adapter's address (a powered one if any), as text.
pub fn adapter_address(conn: &Connection) -> Option<String> {
    let objs = objects(conn).ok()?;
    let mut adapters: Vec<(bool, String)> = objs
        .values()
        .filter_map(|i| i.get("org.bluez.Adapter1"))
        .filter_map(|p| Some((prop_bool(p, "Powered"), prop_str(p, "Address")?)))
        .collect();
    adapters.sort_by_key(|(powered, _)| !*powered);
    adapters.into_iter().next().map(|(_, a)| a)
}

/// A device's display name.
pub fn alias(conn: &Connection, path: &str) -> Option<String> {
    let d = Proxy::new(conn, "org.bluez", path.to_string(), "org.bluez.Device1").ok()?;
    d.get_property::<String>("Alias").ok()
}

/// Ask BlueZ to connect (`true`) or drop (`false`) the phone's
/// hands-free channel. Blocks until BlueZ answers.
pub fn profile(conn: &Connection, path: &str, connect: bool) -> Result<(), zbus::Error> {
    let d = Proxy::new(conn, "org.bluez", path.to_string(), "org.bluez.Device1")?;
    let method = if connect {
        "ConnectProfile"
    } else {
        "DisconnectProfile"
    };
    d.call_method(method, &(HFP_AG_UUID,))?;
    Ok(())
}

/// oFono (another hands-free implementation) is running.
pub fn ofono_running(conn: &Connection) -> bool {
    Proxy::new(
        conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .and_then(|p| p.call::<_, _, bool>("NameHasOwner", &("org.ofono",)))
    .unwrap_or(false)
}

/// What BlueZ's connect error means, for a person (`None`: someone else
/// has the hands-free channel).
pub fn connect_error(e: &zbus::Error) -> Option<String> {
    let (name, desc) = match e {
        zbus::Error::MethodError(name, desc, _) => {
            (name.as_str().to_string(), desc.clone().unwrap_or_default())
        }
        other => return Some(format!("Bluetooth said: {other}")),
    };
    Some(match name.as_str() {
        "org.bluez.Error.AlreadyConnected" | "org.bluez.Error.InProgress" => return None,
        "org.bluez.Error.NotAvailable" | "org.bluez.Error.NotSupported" => {
            "Your phone doesn\u{2019}t offer calls to this computer. Allow calls (phone audio) for this computer in the phone\u{2019}s Bluetooth settings.".into()
        }
        _ if desc.contains("page-timeout") || desc.contains("Host is down") => {
            "Your phone didn\u{2019}t answer. Make sure Bluetooth is on and the phone is nearby, then try Connect again.".into()
        }
        _ => format!("Couldn\u{2019}t connect to your phone for calls ({desc})."),
    })
}
