//! Staying reachable (`features::reconnect`): links come back after a
//! network change, even one that dropped them while there was no network;
//! each comeback is timed; a device that closed its link to save battery,
//! or didn't answer a push, shows as asleep, and automatic connects stop
//! pushing it.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, SignalHub, TestNode, WAIT};
use mdcore::config::{ConnectRungs, CoreConfig};
use mdcore::features::reconnect::{DeviceReach, ReconnectTrigger};
use mdcore::CoreEvent;

fn wait_for(what: &str, within: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn linked(n: &TestNode, peer: &TestNode) -> bool {
    n.engine.connected_peers().contains(&peer.node_id())
}

fn stage_seen(n: &TestNode, peer: &TestNode, stage: &str) -> bool {
    let id = peer.node_id();
    n.sink.all().iter().any(|e| {
        matches!(e, CoreEvent::ConnectProgress { node_id, stage: s, .. } if *node_id == id && s == stage)
    })
}

#[test]
fn a_link_dropped_while_offline_comes_back_when_a_network_does() {
    let (a, b) = (TestNode::start("rc-net-a"), TestNode::start("rc-net-b"));
    TestNode::pair(&a, &b);
    b.engine.send(&a.node_id(), "b dials a").unwrap(); // a accepts this link
    a.sink.wait_for_message("b dials a");
    wait_for("the link", WAIT, || linked(&a, &b));

    // Wi-Fi gone, nothing else: the link on it goes, nothing is dialled.
    a.engine.network_changed_on(Some("127.0.0.2"), "offline");
    wait_for("the link to drop", WAIT, || !linked(&a, &b));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!linked(&a, &b), "nothing dialled while offline");

    // A network again (same address as before: nothing "moved"), and the
    // device whose link went is dialled again.
    a.engine.network_changed_on(Some("127.0.0.2"), "wifi");
    wait_for("the redial", WAIT, || linked(&a, &b));
    a.engine.send(&b.node_id(), "back").unwrap();
    b.sink.wait_for_message("back");

    let r = a.engine.diagnose(&b.node_id()).unwrap().reconnect;
    assert_eq!(r.last_trigger, Some(ReconnectTrigger::Network));
    assert!(r.samples >= 1);
    assert!(r.last_ms.unwrap() < 15_000, "{r:?}");
    assert_eq!(r.last_drop.as_deref(), Some("network_change"));
    assert!(r.drops.iter().any(|d| d.reason == "network_change"));
    assert_eq!(r.reach, Some(DeviceReach::Connected));
    assert!(r.up_for_s.is_some());
    // The other end heard why.
    let theirs = b.engine.diagnose(&a.node_id()).unwrap().reconnect;
    assert!(
        theirs.drops.iter().any(|d| d.reason == "network_change"),
        "{theirs:?}"
    );
}

#[test]
fn links_made_on_mobile_data_work() {
    let (a, b) = (TestNode::start("rc-mob-a"), TestNode::start("rc-mob-b"));
    TestNode::pair(&a, &b);
    let ip = a.engine.identity().ip;
    // Shorter keep-alive for links made from now on.
    a.engine.network_changed_on(Some(&ip), "mobile");
    a.engine.send(&b.node_id(), "over mobile").unwrap();
    b.sink.wait_for_message("over mobile");
    b.engine.send(&a.node_id(), "and back").unwrap();
    a.sink.wait_for_message("and back");
}

#[test]
fn a_push_is_timed_to_the_link_it_brings() {
    let (a, b) = (TestNode::start("rc-push-a"), TestNode::start("rc-push-b"));
    TestNode::pair(&a, &b);
    // b's push arrived 300 ms ago (its app had to start first).
    b.engine.push_received(&a.node_id(), 300);
    a.engine.send(&b.node_id(), "hi").unwrap();
    b.sink.wait_for_message("hi");
    wait_for("b to see the link", WAIT, || linked(&b, &a));
    let r = b.engine.diagnose(&a.node_id()).unwrap().reconnect;
    assert_eq!(r.last_trigger, Some(ReconnectTrigger::Push), "{r:?}");
    assert!(r.push_avg_ms.unwrap() >= 300, "{r:?}");
}

#[test]
fn a_device_that_closes_its_idle_link_is_asleep_not_gone() {
    let cfg = || CoreConfig {
        idle_close_after: Some(Duration::from_secs(1)),
        ..test_config()
    };
    let a = TestNode::start_full(MockPlatform::new("rc-idle-a"), cfg());
    let b = TestNode::start_full(MockPlatform::new("rc-idle-b"), cfg());
    TestNode::pair(&a, &b);
    a.engine.send(&b.node_id(), "hi").unwrap();
    b.sink.wait_for_message("hi");
    // The phone (b) goes to the background and closes the unused link.
    b.engine.set_background(true);
    wait_for("the link to close", WAIT, || !linked(&a, &b));
    wait_for("asleep", WAIT, || stage_seen(&a, &b, "asleep"));
    let r = a.engine.diagnose(&b.node_id()).unwrap().reconnect;
    assert_eq!(r.reach, Some(DeviceReach::Asleep));
    assert_eq!(r.last_drop.as_deref(), Some("peer_idle"));
    assert!(r.lifetime_last_s.is_some());
    // b closed it itself.
    let mine = b.engine.diagnose(&a.node_id()).unwrap().reconnect;
    assert_eq!(mine.last_drop.as_deref(), Some("idle"));
}

#[test]
fn a_device_that_doesnt_answer_its_push_isnt_pushed_again_by_itself() {
    let rungs = ConnectRungs {
        quic: false,
        signaled_quic: true,
        punch: true,
        webrtc: true,
        relay: false,
    };
    let cfg = || CoreConfig {
        rungs: rungs.clone(),
        ..test_config()
    };
    let hub = SignalHub::new();
    let a = TestNode::start_on_hub("rc-sleep-a", &hub, cfg());
    let b = TestNode::start_on_hub("rc-sleep-b", &hub, cfg());
    TestNode::pair(&a, &b);
    hub.sleep(&b.node_id()); // switched off: pushes go nowhere

    a.engine.connect(&b.node_id()).unwrap();
    wait_for("asleep", Duration::from_secs(25), || {
        stage_seen(&a, &b, "asleep")
    });
    assert!(stage_seen(&a, &b, "waking"));
    assert!(!stage_seen(&a, &b, "failed"));
    // One push: punching and WebRTC would only have pushed it twice more.
    assert_eq!(hub.kinds(), vec!["quic_offer".to_string()]);
    let r = a.engine.diagnose(&b.node_id()).unwrap().reconnect;
    assert_eq!(r.reach, Some(DeviceReach::Asleep));
    assert!(r.quiet_for_s.unwrap() > 0);

    // Something sent by itself (a code shared to every device) waits for a
    // link instead of pushing the sleeping device again ...
    a.engine.share_code("482913", None).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(hub.kinds().len(), 1, "{:?}", hub.kinds());

    // ... while the user asking to connect still does.
    a.engine.connect(&b.node_id()).unwrap();
    wait_for("a second push", WAIT, || hub.kinds().len() == 2);
}
