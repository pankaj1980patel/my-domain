//! "How your devices are connected" (features::links_map): the map shows
//! this device's links, the devices it isn't linked to (with when they were
//! last seen), and the links between the OTHER devices that they reported;
//! an app from before never gets a `links_map` message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::features::links_map::LinksMap;
use mdcore::{CoreConfig, CoreEvent};

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Poll `node`'s map until `ok` holds.
fn wait_map(node: &TestNode, what: &str, ok: impl Fn(&LinksMap) -> bool) -> LinksMap {
    let end = Instant::now() + WAIT;
    loop {
        let m = node.engine.links_map();
        if ok(&m) {
            return m;
        }
        assert!(Instant::now() < end, "map never showed {what}: {m:#?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn no_links_map_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("links_map")
    )));
}

#[test]
fn the_map_shows_my_links_and_the_ones_between_my_other_devices() {
    let mac = TestNode::start("map-mac");
    let phone = TestNode::start("map-phone");
    let laptop = TestNode::start("map-laptop");
    TestNode::pair(&mac, &phone);
    TestNode::pair(&mac, &laptop);
    TestNode::pair(&phone, &laptop);
    let (m, p, l) = (mac.node_id(), phone.node_id(), laptop.node_id());

    // Nothing linked yet: both are there, not connected, never seen.
    let map = mac.engine.links_map();
    assert_eq!(map.node_id, m);
    assert_eq!(map.devices.len(), 2);
    assert!(map
        .devices
        .iter()
        .all(|d| d.link.is_none() && d.state == "not_connected" && d.last_seen_ago_s.is_none()));
    assert!(map.others.is_empty());

    // The Mac and the phone: a link on this network, both share their links.
    link(&mac, &phone, "hi phone");
    let map = wait_map(&mac, "the phone sharing", |m| {
        m.devices
            .iter()
            .any(|d| d.node_id == p && d.shares_links && d.link.is_some())
    });
    let d = map.devices.iter().find(|d| d.node_id == p).unwrap();
    let ln = d.link.as_ref().unwrap();
    assert_eq!(ln.how, "lan");
    assert_eq!(d.state, "lan");
    assert_eq!(ln.addr_kind.as_deref(), Some("local"));
    assert!(ln.since_s.is_some());
    assert!(mac.engine.links_map_supported(&p));

    // The phone and the laptop: the Mac hears of it from the phone.
    link(&phone, &laptop, "hi laptop");
    let map = wait_map(&mac, "the phone and the laptop", |m| {
        m.others
            .iter()
            .any(|o| [&o.a, &o.b].contains(&&p) && [&o.a, &o.b].contains(&&l))
    });
    let o = &map.others[0];
    assert_eq!(o.how, "lan");
    assert!(o.a < o.b);
    assert!(o.reported_by == p || o.reported_by == l);
    // Still not linked to the Mac itself.
    let lap = map.devices.iter().find(|d| d.node_id == l).unwrap();
    assert!(lap.link.is_none());
    assert!(mac.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::LinksMapChanged { from } if *from == p
    )));

    // The phone drops the laptop: the Mac's map loses that link, and the
    // phone remembers when it last saw the laptop.
    phone.engine.disconnect(&l).unwrap();
    wait_map(&mac, "the phone-laptop link gone", |m| m.others.is_empty());
    let pm = wait_map(&phone, "the laptop last seen", |m| {
        m.devices
            .iter()
            .any(|d| d.node_id == l && d.link.is_none() && d.last_seen_ago_s.is_some())
    });
    let gone = pm.devices.iter().find(|d| d.node_id == l).unwrap();
    assert_eq!(gone.state, "unreachable");
    assert_eq!(gone.reason.as_deref(), Some("dropped"));

    // The Mac drops the phone: what the phone said no longer counts.
    link(&phone, &laptop, "back again");
    wait_map(&mac, "the link again", |m| !m.others.is_empty());
    mac.engine.disconnect(&p).unwrap();
    wait_map(&mac, "a stale list dropped", |m| m.others.is_empty());

    for n in [&mac, &phone, &laptop] {
        no_links_map_as_chat(n);
    }
}

#[test]
fn a_device_that_cannot_be_reached_says_why_for_each_path() {
    use mdcore::features::links_map::{INTERNET_REASONS, LAN_REASONS};
    let mac = TestNode::start("map-why-mac");
    // A device whose app isn't running: its address answers nothing.
    let gone = {
        let phone = TestNode::start("map-why-phone");
        let mut p = phone.as_peer();
        p.quic_port = 1; // nothing listens there
        p
    };
    let id = gone.node_id.clone();
    mac.engine.upsert_peer(gone);
    mac.engine.set_ble_state(Some("off"));
    mac.engine.connect(&id).unwrap();

    let end = Instant::now() + Duration::from_secs(40);
    let d = loop {
        let m = mac.engine.links_map();
        let d = m.devices.into_iter().find(|d| d.node_id == id).unwrap();
        if d.tries.iter().any(|t| t.path == "internet") {
            break d;
        }
        assert!(Instant::now() < end, "no reasons: {d:#?}");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(d.link.is_none());
    let reason = |path: &str| {
        d.tries
            .iter()
            .find(|t| t.path == path)
            .map(|t| t.reason.clone())
            .unwrap_or_else(|| panic!("no {path} reason: {:?}", d.tries))
    };
    assert!(LAN_REASONS.contains(&reason("lan").as_str()));
    assert!(INTERNET_REASONS.contains(&reason("internet").as_str()));
    // Not signed in to a server in tests: no relay.
    assert_eq!(reason("relay"), "signed_out");
    assert_eq!(reason("ble"), "off");
    let order: Vec<&str> = d.tries.iter().map(|t| t.path.as_str()).collect();
    assert_eq!(order, ["lan", "internet", "relay", "ble"]);
    assert_eq!(d.problem, None);

    // Bluetooth back: it no longer shows.
    mac.engine.set_ble_state(None);
    let m = mac.engine.links_map();
    let d = m.devices.iter().find(|d| d.node_id == id).unwrap();
    assert!(d.tries.iter().all(|t| t.path != "ble"));
}

#[test]
fn an_app_from_before_never_gets_the_list() {
    let new = TestNode::start("map-new");
    let old = TestNode::start_full(
        MockPlatform::new("map-old"),
        CoreConfig {
            links_map: false,
            ..test_config()
        },
    );
    let other = TestNode::start("map-other");
    TestNode::pair(&new, &old);
    TestNode::pair(&new, &other);
    TestNode::pair(&old, &other);
    link(&new, &old, "hello");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("links_map").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without links_map");
    // The new one's links change: still nothing goes to the old one.
    link(&new, &other, "hey");
    std::thread::sleep(Duration::from_millis(1_000));
    // The old one is linked to the other: the new one never hears of it.
    link(&old, &other, "and you");
    std::thread::sleep(Duration::from_millis(800));

    assert!(!new.engine.links_map_supported(&old.node_id()));
    let map = new.engine.links_map();
    let o = map
        .devices
        .iter()
        .find(|d| d.node_id == old.node_id())
        .unwrap();
    assert!(!o.shares_links);
    assert!(o.link.is_some(), "its link still shows");
    // The other one reported old-other (it is new enough).
    wait_map(&new, "old-other from the other", |m| m.others.len() == 1);
    no_links_map_as_chat(&old);
    assert!(!old
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::LinksMapChanged { .. })));
    assert!(old.engine.links_map().others.is_empty());
}
