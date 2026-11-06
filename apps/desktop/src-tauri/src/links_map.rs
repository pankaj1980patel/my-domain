// "How your devices are connected": the map's picture. The core says what
// is linked to what (`Engine::links_map`); this lays it out for the UI's
// SVG and words each line:
//
//  - this computer in the middle, your other devices on an ellipse around
//    it (the first at the top; two sit left and right), in the core's
//    order (by name), so a device keeps its place as links come and go;
//  - a straight line to each device, labelled by how it's linked (or "Not
//    connected", dashed and grey), with the round trip where it's known;
//  - the links your other devices reported between themselves, bowed
//    outwards so they don't run through this computer;
//  - `label_key` is an i18n key the UI translates; the UI adds names, "last
//    seen" and the details panel from `map`.

use mdcore::features::links_map::{
    LinksMap, MapDevice, OtherLink, BLE_REASONS, DEVICE_PROBLEMS, INTERNET_REASONS, LAN_REASONS,
    RELAY_REASONS,
};
use serde::Serialize;
use tauri::State;

use crate::Eng;

/// The picture's size (SVG user units; the UI scales it).
pub const WIDTH: f64 = 640.0;
pub const HEIGHT: f64 = 420.0;
/// Room left around the ring for the devices' names.
const MARGIN_X: f64 = 110.0;
const MARGIN_Y: f64 = 70.0;
/// How far along a line (from this computer) its label sits.
const LABEL_AT: f64 = 0.56;

/// A device's place.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct Spot {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
    pub connected: bool,
}

/// A line on the map.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct Edge {
    /// "me:<node>" for this computer's links, "<a>|<b>" for the others.
    pub id: String,
    pub a: String,
    pub b: String,
    /// One end is this computer.
    pub mine: bool,
    /// "lan", "direct", "punched", "webrtc", "relay", "ble", or "none".
    pub how: String,
    /// The colour family: "lan", "direct", "relay", "ble", "none".
    pub tone: String,
    /// An SVG stroke-dasharray (none: a solid line).
    pub dash: Option<String>,
    /// The SVG path.
    pub path: String,
    pub label_x: f64,
    pub label_y: f64,
    /// What the label says (an i18n key).
    pub label_key: String,
    /// The round trip, worded ("4 ms"); none when not known.
    pub latency: Option<String>,
    /// A media stream runs over it on the local network: the "Nearby ·
    /// low latency" badge on the line.
    pub nearby: bool,
    /// Why it isn't connected, or only through the relay or Bluetooth:
    /// per path tried, and anything wrong with the device itself (this
    /// computer's lines only; empty for a direct link).
    pub why: Vec<Why>,
}

/// One reason, worded by i18n keys.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct Why {
    /// "lan", "internet", "relay", "ble", or "device".
    pub path: String,
    pub reason: String,
    /// The path's name, what went wrong, and what you can do (i18n keys).
    pub path_key: String,
    pub what_key: String,
    pub fix_key: String,
    /// How long ago it was tried (none: a state, not an attempt).
    pub ago_s: Option<u32>,
}

/// The i18n keys for `reason` on `path`: (path, what went wrong, what you
/// can do). A reason this app doesn't know gets the general words.
pub fn why_keys(path: &str, reason: &str) -> (String, String, String) {
    let known: &[&str] = match path {
        "lan" => &LAN_REASONS,
        "internet" => &INTERNET_REASONS,
        "relay" => &RELAY_REASONS,
        "ble" => &BLE_REASONS,
        "device" => &DEVICE_PROBLEMS,
        _ => &[],
    };
    let path_key = if known.is_empty() {
        "map.path.unknown".to_string()
    } else {
        format!("map.via.{path}")
    };
    if known.contains(&reason) {
        (
            path_key,
            format!("map.why.{path}.{reason}"),
            format!("map.fix.{path}.{reason}"),
        )
    } else {
        (path_key, "map.why.unknown".into(), "map.fix.unknown".into())
    }
}

/// The reasons shown for a device's line: none while it's linked
/// directly; else what stopped each path, then the device's own problem.
pub fn whys(d: &MapDevice) -> Vec<Why> {
    let direct = d
        .link
        .as_ref()
        .is_some_and(|l| l.how != "relay" && l.how != "ble");
    if direct {
        return Vec::new();
    }
    let mut out: Vec<Why> = d
        .tries
        .iter()
        .map(|t| {
            let (path_key, what_key, fix_key) = why_keys(&t.path, &t.reason);
            Why {
                path: t.path.clone(),
                reason: t.reason.clone(),
                path_key,
                what_key,
                fix_key,
                ago_s: Some(t.ago_s),
            }
        })
        .collect();
    if let Some(p) = &d.problem {
        let (path_key, what_key, fix_key) = why_keys("device", p);
        out.push(Why {
            path: "device".into(),
            reason: p.clone(),
            path_key,
            what_key,
            fix_key,
            ago_s: None,
        });
    }
    out
}

/// Everything the map screen draws.
#[derive(Clone, Debug, PartialEq, Serialize, specta::Type)]
pub struct MapView {
    pub width: f64,
    pub height: f64,
    pub me: Spot,
    pub spots: Vec<Spot>,
    pub edges: Vec<Edge>,
    /// The core's data, for the list and the details.
    pub map: LinksMap,
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Where `n` devices sit around the middle of a `w` × `h` picture: on an
/// ellipse, evenly, the first at the top (two: left and right).
pub fn ring(n: usize, w: f64, h: f64) -> Vec<(f64, f64)> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (rx, ry) = ((cx - MARGIN_X).max(40.0), (cy - MARGIN_Y).max(40.0));
    let start = if n == 2 {
        std::f64::consts::PI
    } else {
        -std::f64::consts::FRAC_PI_2
    };
    (0..n)
        .map(|i| {
            let a = start + std::f64::consts::TAU * i as f64 / n as f64;
            (r1(cx + rx * a.cos()), r1(cy + ry * a.sin()))
        })
        .collect()
}

/// The colour family for a link's `how`.
pub fn tone(how: &str) -> &'static str {
    match how {
        "lan" => "lan",
        "direct" | "punched" | "webrtc" => "direct",
        "relay" => "relay",
        "ble" => "ble",
        _ => "none",
    }
}

/// Dashes: not connected is dashed, Bluetooth dotted, the rest solid.
pub fn dash(how: &str) -> Option<String> {
    match how {
        "ble" => Some("2 5".into()),
        "lan" | "direct" | "punched" | "webrtc" | "relay" => None,
        _ => Some("6 6".into()),
    }
}

/// The label's i18n key for a link's `how`.
pub fn label_key(how: &str) -> String {
    let k = match how {
        "lan" | "direct" | "punched" | "webrtc" | "relay" | "ble" => how,
        _ => "none",
    };
    format!("map.how.{k}")
}

/// A round trip in words: "<1 ms", "4 ms", or none.
pub fn latency(rtt_ms: Option<u32>) -> Option<String> {
    rtt_ms.map(|ms| {
        if ms == 0 {
            "<1 ms".to_string()
        } else {
            format!("{ms} ms")
        }
    })
}

/// A line from this computer to a device.
fn my_edge(me: &Spot, d: &MapDevice, at: (f64, f64)) -> Edge {
    let how = d.link.as_ref().map_or("none", |l| l.how.as_str());
    Edge {
        id: format!("me:{}", d.node_id),
        a: me.node_id.clone(),
        b: d.node_id.clone(),
        mine: true,
        how: how.to_string(),
        tone: tone(how).into(),
        dash: dash(how),
        path: format!("M {} {} L {} {}", me.x, me.y, at.0, at.1),
        label_x: r1(me.x + (at.0 - me.x) * LABEL_AT),
        label_y: r1(me.y + (at.1 - me.y) * LABEL_AT),
        label_key: label_key(how),
        latency: latency(d.link.as_ref().and_then(|l| l.rtt_ms)),
        nearby: d.link.as_ref().is_some_and(|l| l.nearby_media),
        why: whys(d),
    }
}

/// A line between two other devices: a curve bowed away from the middle
/// (`c`), labelled at its middle.
pub fn bowed(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> (String, (f64, f64)) {
    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (mut ox, mut oy) = (mid.0 - c.0, mid.1 - c.1);
    let mut len = (ox * ox + oy * oy).sqrt();
    if len < 1.0 {
        // Straight through the middle: bow to one side.
        ox = -(b.1 - a.1);
        oy = b.0 - a.0;
        len = (ox * ox + oy * oy).sqrt().max(1.0);
    }
    let span = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let bow = (span * 0.3).max(30.0);
    let ctrl = (r1(mid.0 + ox / len * bow), r1(mid.1 + oy / len * bow));
    let label = (
        r1((a.0 + 2.0 * ctrl.0 + b.0) / 4.0),
        r1((a.1 + 2.0 * ctrl.1 + b.1) / 4.0),
    );
    (
        format!("M {} {} Q {} {} {} {}", a.0, a.1, ctrl.0, ctrl.1, b.0, b.1),
        label,
    )
}

fn other_edge(o: &OtherLink, a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> Edge {
    let (path, (lx, ly)) = bowed(a, b, c);
    Edge {
        id: format!("{}|{}", o.a, o.b),
        a: o.a.clone(),
        b: o.b.clone(),
        mine: false,
        how: o.how.clone(),
        tone: tone(&o.how).into(),
        dash: dash(&o.how),
        path,
        label_x: lx,
        label_y: ly,
        label_key: label_key(&o.how),
        latency: latency(o.rtt_ms),
        nearby: false,
        why: Vec::new(),
    }
}

/// Lay out `map` in a `w` × `h` picture.
pub fn layout(map: LinksMap, w: f64, h: f64) -> MapView {
    let me = Spot {
        node_id: map.node_id.clone(),
        x: r1(w / 2.0),
        y: r1(h / 2.0),
        connected: true,
    };
    let places = ring(map.devices.len(), w, h);
    let spots: Vec<Spot> = map
        .devices
        .iter()
        .zip(&places)
        .map(|(d, &(x, y))| Spot {
            node_id: d.node_id.clone(),
            x,
            y,
            connected: d.link.is_some(),
        })
        .collect();
    let mut edges: Vec<Edge> = map
        .devices
        .iter()
        .zip(&places)
        .map(|(d, &at)| my_edge(&me, d, at))
        .collect();
    let at = |node: &str| spots.iter().find(|s| s.node_id == node).map(|s| (s.x, s.y));
    for o in &map.others {
        if let (Some(a), Some(b)) = (at(&o.a), at(&o.b)) {
            edges.push(other_edge(o, a, b, (me.x, me.y)));
        }
    }
    MapView {
        width: w,
        height: h,
        me,
        spots,
        edges,
        map,
    }
}

/// The map, laid out (quick: nothing is measured).
#[tauri::command]
#[specta::specta]
pub fn links_map(engine: State<Eng>) -> MapView {
    layout(engine.links_map(), WIDTH, HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdcore::features::links_map::{MapLink, PathTry};
    use mdcore::model::Source;
    use mdcore::transport::TransportKind;

    fn device(id: &str, how: Option<&str>, rtt: Option<u32>) -> MapDevice {
        MapDevice {
            node_id: id.into(),
            name: id.into(),
            source: Source::Registry,
            link: how.map(|h| MapLink {
                how: h.into(),
                transport: TransportKind::Quic,
                remote: None,
                addr_kind: None,
                rtt_ms: rtt,
                loss_pct: None,
                since_s: Some(5),
                heard_ago_s: 0,
                nearby_media: h == "lan",
                media_latency_ms: None,
            }),
            state: how.unwrap_or("not_connected").into(),
            reason: None,
            last_seen_ago_s: None,
            shares_links: true,
            tries: Vec::new(),
            problem: None,
        }
    }

    #[test]
    fn every_reason_has_words_and_a_hint() {
        for (path, list) in [
            ("lan", &LAN_REASONS[..]),
            ("internet", &INTERNET_REASONS[..]),
            ("relay", &RELAY_REASONS[..]),
            ("ble", &BLE_REASONS[..]),
            ("device", &DEVICE_PROBLEMS[..]),
        ] {
            for r in list {
                let (p, what, fix) = why_keys(path, r);
                assert_eq!(p, format!("map.via.{path}"));
                assert_eq!(what, format!("map.why.{path}.{r}"));
                assert_eq!(fix, format!("map.fix.{path}.{r}"));
            }
        }
        assert_eq!(
            why_keys("lan", "gremlins"),
            (
                "map.via.lan".into(),
                "map.why.unknown".into(),
                "map.fix.unknown".into()
            )
        );
        assert_eq!(why_keys("carrier pigeon", "lost").0, "map.path.unknown");
    }

    #[test]
    fn reasons_show_unless_the_link_is_direct() {
        let tries = vec![
            PathTry {
                path: "lan".into(),
                reason: "blocked".into(),
                ago_s: 9,
            },
            PathTry {
                path: "internet".into(),
                reason: "symmetric_nat".into(),
                ago_s: 8,
            },
        ];
        let mut d = device("phone", None, None);
        d.tries = tries.clone();
        d.problem = Some("bad_key".into());
        let w = whys(&d);
        assert_eq!(
            w.iter()
                .map(|x| (x.path.as_str(), x.reason.as_str()))
                .collect::<Vec<_>>(),
            [
                ("lan", "blocked"),
                ("internet", "symmetric_nat"),
                ("device", "bad_key")
            ]
        );
        assert_eq!(w[0].what_key, "map.why.lan.blocked");
        assert_eq!(w[1].fix_key, "map.fix.internet.symmetric_nat");
        assert_eq!((w[0].ago_s, w[2].ago_s), (Some(9), None));

        // Through the relay: still worth knowing why not directly.
        let mut relayed = device("tablet", Some("relay"), None);
        relayed.tries = tries.clone();
        assert_eq!(whys(&relayed).len(), 2);
        // Directly: nothing to explain.
        let mut direct = device("tv", Some("lan"), Some(2));
        direct.tries = tries;
        assert!(whys(&direct).is_empty());
    }

    fn inside(p: (f64, f64)) -> bool {
        p.0 >= 0.0 && p.0 <= WIDTH && p.1 >= 0.0 && p.1 <= HEIGHT
    }

    #[test]
    fn devices_sit_around_the_middle() {
        assert!(ring(0, WIDTH, HEIGHT).is_empty());
        let one = ring(1, WIDTH, HEIGHT);
        assert_eq!(one[0].0, WIDTH / 2.0, "the first at the top");
        assert!(one[0].1 < HEIGHT / 2.0);
        let two = ring(2, WIDTH, HEIGHT);
        assert!(
            two[0].0 < WIDTH / 2.0 && two[1].0 > WIDTH / 2.0,
            "left and right"
        );
        assert_eq!(two[0].1, two[1].1);
        for n in 1..=12 {
            let pts = ring(n, WIDTH, HEIGHT);
            assert_eq!(pts.len(), n);
            assert!(pts.iter().copied().all(inside), "{n}: {pts:?}");
            // Names fit: nothing hugs the edge.
            assert!(pts
                .iter()
                .all(|p| p.0 >= MARGIN_X - 0.1 && p.1 >= MARGIN_Y - 0.1));
            for (i, a) in pts.iter().enumerate() {
                for b in &pts[i + 1..] {
                    let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
                    assert!(d > 40.0, "{n}: {a:?} and {b:?} overlap");
                }
            }
        }
    }

    #[test]
    fn links_are_worded_and_drawn_by_how() {
        assert_eq!(label_key("lan"), "map.how.lan");
        assert_eq!(label_key("relay"), "map.how.relay");
        assert_eq!(label_key("unreachable"), "map.how.none");
        assert_eq!(label_key("not_connected"), "map.how.none");
        assert_eq!(tone("punched"), "direct");
        assert_eq!(tone("webrtc"), "direct");
        assert_eq!(tone("ble"), "ble");
        assert_eq!(tone("whatever"), "none");
        assert_eq!(dash("lan"), None);
        assert_eq!(dash("relay"), None);
        assert_eq!(dash("ble").as_deref(), Some("2 5"));
        assert_eq!(dash("none").as_deref(), Some("6 6"));
        assert_eq!(latency(None), None);
        assert_eq!(latency(Some(0)).as_deref(), Some("<1 ms"));
        assert_eq!(latency(Some(42)).as_deref(), Some("42 ms"));
    }

    #[test]
    fn the_map_has_a_line_per_device_and_the_reported_ones() {
        let map = LinksMap {
            node_id: "me".into(),
            name: "Mac".into(),
            devices: vec![
                device("laptop", None, None),
                device("phone", Some("lan"), Some(3)),
                device("tablet", Some("relay"), None),
                device("tv", Some("direct"), Some(80)),
            ],
            others: vec![
                OtherLink {
                    a: "laptop".into(),
                    b: "phone".into(),
                    how: "ble".into(),
                    rtt_ms: None,
                    reported_by: "phone".into(),
                    reported_ago_s: 2,
                },
                // Opposite each other: the line would run through the middle.
                OtherLink {
                    a: "phone".into(),
                    b: "tv".into(),
                    how: "direct".into(),
                    rtt_ms: Some(20),
                    reported_by: "phone".into(),
                    reported_ago_s: 2,
                },
                // A device the map doesn't show: left out.
                OtherLink {
                    a: "ghost".into(),
                    b: "phone".into(),
                    how: "lan".into(),
                    rtt_ms: None,
                    reported_by: "phone".into(),
                    reported_ago_s: 2,
                },
            ],
        };
        let v = layout(map, WIDTH, HEIGHT);
        assert_eq!((v.me.x, v.me.y), (WIDTH / 2.0, HEIGHT / 2.0));
        assert_eq!(v.spots.len(), 4);
        assert!(!v.spots[0].connected && v.spots[1].connected);
        assert_eq!(v.edges.len(), 6);

        let mine: Vec<&Edge> = v.edges.iter().filter(|e| e.mine).collect();
        assert_eq!(mine.len(), 4);
        let laptop = mine.iter().find(|e| e.b == "laptop").unwrap();
        assert_eq!(
            (
                laptop.how.as_str(),
                laptop.label_key.as_str(),
                laptop.dash.as_deref()
            ),
            ("none", "map.how.none", Some("6 6"))
        );
        let phone = mine.iter().find(|e| e.b == "phone").unwrap();
        assert_eq!(phone.id, "me:phone");
        assert_eq!(phone.latency.as_deref(), Some("3 ms"));
        assert!(phone
            .path
            .starts_with(&format!("M {} {} L ", v.me.x, v.me.y)));
        // The label is between the two ends.
        let s = &v.spots[1];
        assert!((phone.label_x - (v.me.x + (s.x - v.me.x) * LABEL_AT)).abs() < 0.2);

        let tv = v.edges.iter().find(|e| e.id == "phone|tv").unwrap();
        assert!(!tv.mine);
        assert!(tv.path.contains(" Q "));
        let off = ((tv.label_x - v.me.x).powi(2) + (tv.label_y - v.me.y).powi(2)).sqrt();
        assert!(off > 30.0, "its label clears this computer: {off}");
        assert!(inside((tv.label_x, tv.label_y)));
        let ble = v.edges.iter().find(|e| e.id == "laptop|phone").unwrap();
        assert_eq!(
            (ble.tone.as_str(), ble.dash.as_deref()),
            ("ble", Some("2 5"))
        );
        // Bowed outwards: its label is further out than the straight line's middle.
        let (a, b) = (&v.spots[0], &v.spots[1]);
        let mid = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let d = |p: (f64, f64)| ((p.0 - v.me.x).powi(2) + (p.1 - v.me.y).powi(2)).sqrt();
        assert!(d((ble.label_x, ble.label_y)) > d(mid));
    }
}
