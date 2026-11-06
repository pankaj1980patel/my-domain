//! Countdown timers shared between your devices (features::timers): a
//! timer started on one shows on the others, pause, resume, add time,
//! cancel and dismiss go everywhere, and an app from before never gets a
//! timers message.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::model::Timer;
use mdcore::{CoreConfig, CoreEvent};

const MIN: u64 = 60_000;

/// Wait for a `TimersChanged` on `node` whose list passes `ok`.
fn wait_timers(node: &TestNode, what: &str, ok: impl Fn(&[Timer]) -> bool) -> Vec<Timer> {
    node.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::TimersChanged { timers } if ok(timers) => Some(timers.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no timers event: {what}"))
}

fn timer_events(node: &TestNode) -> usize {
    node.sink
        .all()
        .iter()
        .filter(|e| matches!(e, CoreEvent::TimersChanged { .. }))
        .count()
}

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

fn no_timers_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m) if m.text.contains("timers")
    )));
}

#[test]
fn a_timer_started_on_one_device_shows_and_changes_on_the_other() {
    let phone = TestNode::start("timers-phone");
    let mac = TestNode::start("timers-mac");
    TestNode::pair(&phone, &mac);

    // Started before there's a link: it goes when the link comes up.
    let ring = vec![mac.node_id()];
    let t = phone.engine.start_timer("Pasta", 10 * MIN, &ring).unwrap();
    assert_eq!(t.state, "running");
    assert_eq!(t.by, phone.node_id());
    link(&phone, &mac, "hi");
    let got = wait_timers(&mac, "started", |l| l.iter().any(|x| x.id == t.id));
    let shared = got.iter().find(|x| x.id == t.id).unwrap();
    assert_eq!(shared, &t, "the same timer, the same end time");
    assert_eq!(shared.ring_on, ring);
    assert!(phone.engine.timers_supported(&mac.node_id()));
    assert!(mac.engine.timers_supported(&phone.node_id()));

    // Paused on the other one, over the live link.
    let paused = mac.engine.pause_timer(&t.id).unwrap();
    assert_eq!(paused.state, "paused");
    wait_timers(&phone, "paused", |l| {
        l.iter().any(|x| x.id == t.id && x.state == "paused")
    });
    // Resumed and a minute added on the first.
    phone.engine.resume_timer(&t.id).unwrap();
    let more = phone.engine.add_timer_time(&t.id, MIN).unwrap();
    wait_timers(&mac, "a minute more", |l| {
        l.iter()
            .any(|x| x.id == t.id && x.state == "running" && x.ends_at == more.ends_at)
    });
    // Which devices ring changes too.
    let both = vec![mac.node_id(), phone.node_id()];
    mac.engine.set_timer_ring(&t.id, &both).unwrap();
    wait_timers(&phone, "rings on both", |l| {
        l.iter().any(|x| x.id == t.id && x.ring_on == both)
    });

    // A second timer, then the first cancelled: it goes everywhere.
    let before = timer_events(&mac);
    let tea = mac.engine.start_timer("Tea", 3 * MIN, &[]).unwrap();
    wait_timers(&phone, "tea", |l| l.iter().any(|x| x.id == tea.id));
    std::thread::sleep(Duration::from_millis(300));
    // Starting one here makes no event here, and nothing echoes back.
    assert_eq!(timer_events(&mac), before);
    phone.engine.cancel_timer(&t.id).unwrap();
    wait_timers(&mac, "cancelled", |l| {
        l.iter().all(|x| x.id != t.id) && l.iter().any(|x| x.id == tea.id)
    });
    // Gone: it can't be changed any more, here or there.
    assert!(mac.engine.resume_timer(&t.id).is_err());
    assert!(phone.engine.pause_timer(&t.id).is_err());

    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(phone.engine.timers(), mac.engine.timers());
    assert_eq!(mac.engine.timers().len(), 1);
    no_timers_as_chat(&phone);
    no_timers_as_chat(&mac);
}

#[test]
fn an_ended_timer_is_dismissed_everywhere() {
    let a = TestNode::start("timers-end-a");
    let b = TestNode::start("timers-end-b");
    TestNode::pair(&a, &b);
    link(&a, &b, "hey");
    let rings = vec![a.node_id(), b.node_id()];
    let t = a.engine.start_timer("Eggs", 1_000, &rings).unwrap();
    wait_timers(&b, "eggs", |l| l.iter().any(|x| x.id == t.id));
    std::thread::sleep(Duration::from_millis(1_200));
    // Ended: still shown (it rings) until dismissed.
    let now = b.engine.timers();
    assert!(now.iter().any(|x| x.id == t.id && x.state == "running"));
    b.engine.dismiss_timer(&t.id).unwrap();
    wait_timers(&a, "dismissed", |l| l.iter().all(|x| x.id != t.id));
    assert!(a.engine.timers().is_empty());
    assert!(b.engine.timers().is_empty());
}

#[test]
fn an_app_from_before_never_gets_timers() {
    let new = TestNode::start("timers-new");
    let old = TestNode::start_full(
        MockPlatform::new("timers-old"),
        CoreConfig {
            timers: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    new.engine
        .start_timer("Before the link", 5 * MIN, &[])
        .unwrap();
    link(&new, &old, "hello");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    new.sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("timers").is_none()),
            _ => None,
        })
        .filter(|absent| *absent)
        .expect("a status without timers");
    // A change over the live link doesn't go either.
    new.engine
        .start_timer("Over the link", 2 * MIN, &[old.node_id()])
        .unwrap();
    std::thread::sleep(Duration::from_millis(1_000));

    assert!(!new.engine.timers_supported(&old.node_id()));
    no_timers_as_chat(&old);
    assert_eq!(timer_events(&old), 0);
    assert_eq!(timer_events(&new), 0);
    // The old one can't start one at all.
    assert!(old.engine.start_timer("x", MIN, &[]).is_err());
}

#[test]
fn a_timer_stamped_far_in_the_future_is_left_out() {
    let phone = TestNode::start("timers-future-phone");
    let mac = TestNode::start("timers-future-mac");
    TestNode::pair(&phone, &mac);
    link(&phone, &mac, "hi");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let timer = |id: &str, ends_at: u64, updated_at: u64| {
        serde_json::json!({
            "id": id, "label": "x", "duration_ms": 10 * MIN, "state": "running",
            "ends_at": ends_at, "updated_at": updated_at,
            "by": phone.node_id(), "by_name": "Phone",
        })
    };
    let msg = serde_json::json!({
        "type": "timers",
        "from": phone.node_id(),
        "timers": [
            // Would win every later edit for ever.
            timer("t-forever", now + 10 * MIN, u64::MAX),
            // Would show (and count against the limit) for ever.
            timer("t-neverends", u64::MAX, now),
            timer("t-fine", now + 10 * MIN, now),
        ],
    });
    phone.engine.send(&mac.node_id(), &msg.to_string()).unwrap();
    let got = wait_timers(&mac, "the fine one", |l| l.iter().any(|t| t.id == "t-fine"));
    assert!(got.iter().all(|t| t.id == "t-fine"), "{got:?}");
    assert!(mac.engine.timers().iter().all(|t| t.id == "t-fine"));
}
