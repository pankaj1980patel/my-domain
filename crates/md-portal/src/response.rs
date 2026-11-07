//! What the portals answer. A portal method returns a *request* object at
//! once; the answer comes later as that request's `Response` signal: a
//! code (0 done, 1 cancelled by the user, 2 failed) and a dictionary of
//! results. These read those dictionaries (and the signals' options),
//! whatever nesting of variants the bus delivers them in.

use std::collections::HashMap;
use std::path::PathBuf;

use zvariant::{OwnedValue, Value};

use crate::geom::Rect;

/// A portal dictionary (`a{sv}`) as read from the bus.
pub type Dict = HashMap<String, OwnedValue>;

/// How a request ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done(Dict),
    /// The user said no (or closed the dialog).
    Cancelled,
    Failed,
}

pub fn outcome(code: u32, results: Dict) -> Outcome {
    match code {
        0 => Outcome::Done(results),
        1 => Outcome::Cancelled,
        _ => Outcome::Failed,
    }
}

/// This connection's unique name as it appears in request and session
/// paths: ":1.42" → "1_42".
pub fn sender_part(unique_name: &str) -> String {
    unique_name.trim_start_matches(':').replace('.', "_")
}

/// The request object a call made with `handle_token` = `token` answers on.
pub fn request_path(unique_name: &str, token: &str) -> String {
    format!(
        "/org/freedesktop/portal/desktop/request/{}/{token}",
        sender_part(unique_name)
    )
}

/// A handle token: letters, digits and `_` only (an object path element).
pub fn token(prefix: &str, n: u64) -> String {
    let clean: String = prefix
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    format!("md_{clean}_{}_{n}", std::process::id())
}

fn peel<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(inner) => peel(inner),
        other => other,
    }
}

pub(crate) fn as_u32(v: &Value<'_>) -> Option<u32> {
    match peel(v) {
        Value::U32(n) => Some(*n),
        Value::I32(n) => u32::try_from(*n).ok(),
        Value::U16(n) => Some(u32::from(*n)),
        Value::U8(n) => Some(u32::from(*n)),
        Value::U64(n) => u32::try_from(*n).ok(),
        _ => None,
    }
}

fn as_i32(v: &Value<'_>) -> Option<i32> {
    match peel(v) {
        Value::I32(n) => Some(*n),
        Value::U32(n) => i32::try_from(*n).ok(),
        Value::I16(n) => Some(i32::from(*n)),
        Value::I64(n) => i32::try_from(*n).ok(),
        _ => None,
    }
}

fn as_f64(v: &Value<'_>) -> Option<f64> {
    match peel(v) {
        Value::F64(n) => Some(*n),
        other => as_i32(other).map(f64::from),
    }
}

pub(crate) fn as_string(v: &Value<'_>) -> Option<String> {
    match peel(v) {
        Value::Str(s) => Some(s.as_str().to_string()),
        Value::ObjectPath(p) => Some(p.as_str().to_string()),
        _ => None,
    }
}

pub(crate) fn fields<'a>(v: &'a Value<'a>) -> Option<&'a [Value<'a>]> {
    match peel(v) {
        Value::Structure(s) => Some(s.fields()),
        _ => None,
    }
}

pub(crate) fn items<'a>(v: &'a Value<'a>) -> Option<&'a [Value<'a>]> {
    match peel(v) {
        Value::Array(a) => Some(a.inner()),
        _ => None,
    }
}

/// A pair of numbers, `(ii)` or `(dd)`.
fn pair(v: &Value<'_>) -> Option<(f64, f64)> {
    let f = fields(v)?;
    Some((as_f64(f.first()?)?, as_f64(f.get(1)?)?))
}

/// A nested dictionary's entries.
fn entries<'a>(v: &'a Value<'a>) -> Vec<(String, &'a Value<'a>)> {
    match peel(v) {
        Value::Dict(d) => d
            .iter()
            .filter_map(|(k, v)| Some((as_string(k)?, v)))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn get_u32(d: &Dict, key: &str) -> Option<u32> {
    d.get(key).and_then(|v| as_u32(v))
}

pub fn get_string(d: &Dict, key: &str) -> Option<String> {
    d.get(key)
        .and_then(|v| as_string(v))
        .filter(|s| !s.is_empty())
}

/// A screen cast stream: its PipeWire node and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub node: u32,
    /// Where it sits in the desktop and how big (logical pixels), when
    /// the portal says (monitors do).
    pub rect: Option<Rect>,
}

/// The `streams` a ScreenCast or RemoteDesktop `Start` answered, `a(ua{sv})`.
pub fn streams(d: &Dict) -> Vec<Stream> {
    let Some(list) = d.get("streams").and_then(|v| items(v)) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|s| {
            let f = fields(s)?;
            let node = as_u32(f.first()?)?;
            let props = f.get(1).map(entries).unwrap_or_default();
            let find = |k: &str| props.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
            let at = find("position").and_then(pair);
            let size = find("size").and_then(pair);
            let rect = match (at, size) {
                (Some((x, y)), Some((w, h))) if w > 0.0 && h > 0.0 => Some(Rect { x, y, w, h }),
                (None, Some((w, h))) if w > 0.0 && h > 0.0 => Some(Rect {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                }),
                _ => None,
            };
            Some(Stream { node, rect })
        })
        .collect()
}

/// The token that skips the prompt next time (ScreenCast v4+,
/// RemoteDesktop v2+).
pub fn restore_token(d: &Dict) -> Option<String> {
    get_string(d, "restore_token")
}

/// The session a `CreateSession` made.
pub fn session_handle(d: &Dict) -> Option<String> {
    get_string(d, "session_handle")
}

/// A `file://` URI as a path (percent-escapes decoded); none for any other
/// kind.
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // "file://host/path": only this computer's.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(i) if &rest[..i] == "localhost" => &rest[i..],
        _ => return None,
    };
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

/// Where a `Screenshot` put the picture.
pub fn screenshot_path(d: &Dict) -> Option<PathBuf> {
    get_string(d, "uri").and_then(|u| uri_path(&u))
}

/// InputCapture's `GetZones`: the screens, and the zone set the barriers
/// must name.
pub fn zones(d: &Dict) -> (Vec<Rect>, u32) {
    let set = get_u32(d, "zone_set").unwrap_or(0);
    let zones = d
        .get("zones")
        .and_then(|v| items(v))
        .map(|list| {
            list.iter()
                .filter_map(|z| {
                    let f = fields(z)?;
                    // (width u, height u, x i, y i)
                    Some(Rect {
                        w: f64::from(as_u32(f.first()?)?),
                        h: f64::from(as_u32(f.get(1)?)?),
                        x: f64::from(as_i32(f.get(2)?)?),
                        y: f64::from(as_i32(f.get(3)?)?),
                    })
                })
                .filter(|r| r.w > 0.0 && r.h > 0.0)
                .collect()
        })
        .unwrap_or_default();
    (zones, set)
}

/// The barriers `SetPointerBarriers` refused.
pub fn failed_barriers(d: &Dict) -> Vec<u32> {
    d.get("failed_barriers")
        .and_then(|v| items(v))
        .map(|l| l.iter().filter_map(as_u32).collect())
        .unwrap_or_default()
}

/// InputCapture's `Activated` options: the capture started.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activation {
    pub id: Option<u32>,
    pub barrier: Option<u32>,
    /// Where the pointer was.
    pub cursor: Option<(f64, f64)>,
}

pub fn activation(d: &Dict) -> Activation {
    Activation {
        id: get_u32(d, "activation_id"),
        barrier: get_u32(d, "barrier_id"),
        cursor: d.get("cursor_position").and_then(|v| pair(v)),
    }
}

/// The ids of the shortcuts `BindShortcuts` bound, `a(sa{sv})`.
pub fn shortcut_ids(d: &Dict) -> Vec<String> {
    d.get("shortcuts")
        .and_then(|v| items(v))
        .map(|l| {
            l.iter()
                .filter_map(|s| as_string(fields(s)?.first()?))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvariant::serialized::{Context, Data};
    use zvariant::LE;

    #[test]
    fn bound_shortcuts() {
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("description", Value::from("Send"));
        props.insert("trigger_description", Value::from("Ctrl+Shift+Y"));
        let mut d = HashMap::new();
        d.insert(
            "shortcuts",
            Value::from(vec![("send-clipboard", props.clone()), ("ring", props)]),
        );
        assert_eq!(shortcut_ids(&dict(d)), vec!["send-clipboard", "ring"]);
        assert!(shortcut_ids(&Dict::new()).is_empty());
    }

    /// `value` sent over the bus and read back as a portal dictionary,
    /// what zbus's `Body::deserialize` hands over.
    fn dict(value: HashMap<&str, Value<'_>>) -> Dict {
        let ctxt = Context::new_dbus(LE, 0);
        let data: Data<'static, 'static> = zvariant::to_bytes(ctxt, &value).unwrap();
        data.deserialize::<Dict>().unwrap().0
    }

    #[test]
    fn response_codes() {
        assert!(matches!(outcome(0, Dict::new()), Outcome::Done(_)));
        assert_eq!(outcome(1, Dict::new()), Outcome::Cancelled);
        assert_eq!(outcome(2, Dict::new()), Outcome::Failed);
    }

    #[test]
    fn request_paths_and_tokens() {
        assert_eq!(
            request_path(":1.234", "md_x_1"),
            "/org/freedesktop/portal/desktop/request/1_234/md_x_1"
        );
        let t = token("screen-cast!", 7);
        assert!(t.starts_with("md_screencast_"));
        assert!(t.ends_with("_7"));
        assert!(t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn streams_with_their_places() {
        let mut a: HashMap<&str, Value<'_>> = HashMap::new();
        a.insert("position", Value::from((1920i32, 0i32)));
        a.insert("size", Value::from((1280i32, 1024i32)));
        a.insert("source_type", Value::from(1u32));
        let b: HashMap<&str, Value<'_>> = HashMap::new();
        let mut d = HashMap::new();
        d.insert("streams", Value::from(vec![(57u32, a), (58u32, b)]));
        d.insert("restore_token", Value::from("tok-123"));
        d.insert("devices", Value::from(3u32));
        let d = dict(d);
        let s = streams(&d);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].node, 57);
        assert_eq!(
            s[0].rect,
            Some(Rect {
                x: 1920.0,
                y: 0.0,
                w: 1280.0,
                h: 1024.0
            })
        );
        assert_eq!(
            s[1],
            Stream {
                node: 58,
                rect: None
            }
        );
        assert_eq!(restore_token(&d).as_deref(), Some("tok-123"));
        assert_eq!(get_u32(&d, "devices"), Some(3));
        assert!(streams(&Dict::new()).is_empty());
    }

    #[test]
    fn an_empty_restore_token_is_none() {
        let mut d = HashMap::new();
        d.insert("restore_token", Value::from(""));
        assert_eq!(restore_token(&dict(d)), None);
    }

    #[test]
    fn screenshot_uris_become_paths() {
        assert_eq!(
            uri_path("file:///home/me/Pictures/Screenshot%20from%202026.png"),
            Some(PathBuf::from("/home/me/Pictures/Screenshot from 2026.png"))
        );
        assert_eq!(
            uri_path("file://localhost/tmp/a.png"),
            Some(PathBuf::from("/tmp/a.png"))
        );
        assert_eq!(uri_path("file://other/tmp/a.png"), None);
        assert_eq!(uri_path("https://example.com/a.png"), None);
        assert_eq!(
            uri_path("file:///tmp/100%"),
            Some(PathBuf::from("/tmp/100%"))
        );
        let mut d = HashMap::new();
        d.insert("uri", Value::from("file:///tmp/s.png"));
        assert_eq!(screenshot_path(&dict(d)), Some(PathBuf::from("/tmp/s.png")));
    }

    #[test]
    fn zones_barriers_and_activation() {
        let mut d = HashMap::new();
        d.insert(
            "zones",
            Value::from(vec![
                (1920u32, 1080u32, 0i32, 0i32),
                (0u32, 0u32, 5i32, 5i32),
            ]),
        );
        d.insert("zone_set", Value::from(4u32));
        let (z, set) = zones(&dict(d));
        assert_eq!(set, 4);
        assert_eq!(
            z,
            vec![Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0
            }]
        );

        let mut f = HashMap::new();
        f.insert("failed_barriers", Value::from(vec![2u32, 5u32]));
        assert_eq!(failed_barriers(&dict(f)), vec![2, 5]);

        let mut a = HashMap::new();
        a.insert("activation_id", Value::from(9u32));
        a.insert("barrier_id", Value::from(1u32));
        a.insert("cursor_position", Value::from((1919.5f64, 300.0f64)));
        assert_eq!(
            activation(&dict(a)),
            Activation {
                id: Some(9),
                barrier: Some(1),
                cursor: Some((1919.5, 300.0))
            }
        );
        assert_eq!(activation(&Dict::new()), Activation::default());
    }

    #[test]
    fn a_session_handle_may_be_a_path_or_a_string() {
        let mut d = HashMap::new();
        d.insert(
            "session_handle",
            Value::from(
                zvariant::ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_2/t")
                    .unwrap(),
            ),
        );
        assert_eq!(
            session_handle(&dict(d)).as_deref(),
            Some("/org/freedesktop/portal/desktop/session/1_2/t")
        );
        let mut s = HashMap::new();
        s.insert("session_handle", Value::from("/x/y"));
        assert_eq!(session_handle(&dict(s)).as_deref(), Some("/x/y"));
    }
}
