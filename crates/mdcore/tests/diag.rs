//! "Check connection" (features::diag): the report, pings, the speed test,
//! and that an app from before never gets any of it.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use md_testkit::{test_config, MockPlatform, SignalHub, TestNode, WAIT};
use mdcore::features::diag::SpeedResult;
use mdcore::{ConnectRungs, CoreConfig, CoreEvent};

/// Bring up a link from `a` to `b` (a chat message), and wait for it.
fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Wait until `a` has heard that `b` answers pings (its `addr_update`).
fn wait_ping_support(a: &TestNode, b: &TestNode) {
    let deadline = Instant::now() + WAIT;
    while !a
        .engine
        .diagnose(&b.node_id())
        .is_ok_and(|d| d.ping_supported)
    {
        assert!(Instant::now() < deadline, "never heard it answers pings");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// No diag message ever showed up as chat on `node`.
fn nothing_as_chat(node: &TestNode) {
    assert!(!node.sink.all().iter().any(|e| matches!(
        e,
        CoreEvent::MessageReceived(m)
            if m.text.contains("diag_") || m.text.contains("speed_")
    )));
}

/// Wait for test `id` to end on `node`: its last phase and result.
fn wait_test_end(node: &TestNode, id: &str, within: Duration) -> (String, Option<SpeedResult>) {
    node.sink
        .wait_until(within, |e| match e {
            CoreEvent::SpeedTest {
                id: i,
                phase,
                result,
                ..
            } if i == id && matches!(phase.as_str(), "done" | "failed" | "cancelled") => {
                Some((phase.clone(), result.clone()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("speed test {id} never ended"))
}

#[test]
fn a_report_measures_the_round_trip_over_the_live_link() {
    let mac = TestNode::start("diag-mac");
    let phone = TestNode::start("diag-phone");
    TestNode::pair(&mac, &phone);

    // Never connected, never tried.
    let before = mac.engine.diagnose(&phone.node_id()).unwrap();
    assert_eq!(before.verdict, "not_connected");
    assert!(before.link.is_none() && before.rtt.is_none());
    assert!(!before.relay.signed_in, "no server in tests");

    link(&mac, &phone, "hi");
    wait_ping_support(&mac, &phone);
    let d = mac.engine.diagnose(&phone.node_id()).unwrap();
    // Loopback is the local network.
    assert_eq!(d.verdict, "lan", "{d:#?}");
    let l = d.link.as_ref().unwrap();
    assert!(l.direct && l.lan);
    assert!(l.remote.as_deref().unwrap().starts_with("127.0.0.1:"));
    let rtt = d.rtt.as_ref().unwrap();
    assert_eq!((rtt.samples, rtt.lost), (3, 0), "{rtt:?}");
    let avg = rtt.avg_ms.unwrap();
    assert!(avg > 0.0 && avg < 1000.0, "{avg}");
    assert!(rtt.min_ms.unwrap() <= avg && avg <= rtt.max_ms.unwrap());
    assert_eq!(d.peer.quic_port, phone.engine.identity().quic_port);
    assert!(d.attempts.iter().any(|a| a.stage == "link_up"));
    // The other way too, and one ping on its own.
    wait_ping_support(&phone, &mac);
    assert!(phone.engine.ping(&mac.node_id()).is_some());

    // Link gone: no pings, and the report says it dropped.
    mac.engine.disconnect(&phone.node_id()).unwrap();
    let deadline = Instant::now() + WAIT;
    while !mac.engine.connected_peers().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    let after = mac.engine.diagnose(&phone.node_id()).unwrap();
    assert_eq!(after.verdict, "unreachable");
    assert_eq!(after.reason.as_deref(), Some("dropped"));
    assert!(after.rtt.is_none());
    assert!(after.last_try_ago_s.is_some());
    nothing_as_chat(&mac);
    nothing_as_chat(&phone);
}

#[test]
fn an_app_from_before_never_gets_a_ping() {
    let new = TestNode::start("diag-new");
    let old = TestNode::start_full(
        MockPlatform::new("diag-old"),
        CoreConfig {
            diag: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    link(&new, &old, "hello");
    link(&old, &new, "hello back");
    // Its status doesn't say it either.
    new.engine.request_status(&old.node_id()).unwrap();
    let says = new
        .sink
        .wait_until(WAIT, |e| match e {
            CoreEvent::DeviceStatus { status, .. } => Some(status.get("ping").is_some()),
            _ => None,
        })
        .unwrap();
    assert!(!says);

    let d = new.engine.diagnose(&old.node_id()).unwrap();
    assert!(!d.ping_supported);
    assert!(d.rtt.is_none());
    assert!(d.link.is_some(), "the link itself is fine");
    assert!(new.engine.ping(&old.node_id()).is_none());
    assert!(new.engine.speed_test(&old.node_id(), 1 << 20).is_err());
    std::thread::sleep(Duration::from_millis(500));
    nothing_as_chat(&old);
    // The old one never pings the new one either.
    assert!(!old.engine.diagnose(&new.node_id()).unwrap().ping_supported);
}

#[test]
fn a_speed_test_runs_both_ways_on_loopback() {
    let a = TestNode::start("speed-a");
    let b = TestNode::start("speed-b");
    TestNode::pair(&a, &b);
    // No link: refused at once.
    assert!(a.engine.speed_test(&b.node_id(), 1 << 20).is_err());

    link(&a, &b, "hi");
    wait_ping_support(&a, &b);
    let bytes = 4 << 20;
    let id = a.engine.speed_test(&b.node_id(), bytes).unwrap();
    let (phase, result) = wait_test_end(&a, &id, Duration::from_secs(60));
    assert_eq!(phase, "done");
    let r = result.unwrap();
    assert_eq!(r.bytes, bytes);
    // Loopback: well above a slow network, well below the impossible.
    for mbps in [r.upload_mbps, r.download_mbps] {
        assert!(mbps > 1.0 && mbps < 1_000_000.0, "{r:?}");
    }
    assert!(r.upload_ms > 0 && r.download_ms > 0, "{r:?}");
    assert_eq!(r.rtt.samples, 5, "{r:?}");
    assert_eq!(
        serde_json::to_value(r.transport).unwrap(),
        serde_json::json!("quic")
    );
    // It went through its phases in order, with progress.
    let phases: Vec<String> = a
        .sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::SpeedTest { id: i, phase, .. } if i == id => Some(phase),
            _ => None,
        })
        .collect();
    let first = |p: &str| phases.iter().position(|x| x == p).unwrap();
    assert!(first("ping") < first("upload") && first("upload") < first("download"));
    assert_eq!(phases.last().map(String::as_str), Some("done"));
    // Too big is cut to the maximum (and still allowed to start).
    let big = a.engine.speed_test(&b.node_id(), u32::MAX).unwrap();
    assert!(a.engine.cancel_speed_test(&big));
    let (phase, _) = wait_test_end(&a, &big, WAIT);
    assert_eq!(phase, "cancelled");
    assert!(!a.engine.cancel_speed_test(&big), "already over");
    // The link still carries messages after all that.
    link(&b, &a, "still here");
    nothing_as_chat(&a);
    nothing_as_chat(&b);
}

#[test]
fn a_device_that_never_answers_its_push_is_reported() {
    let hub = SignalHub::new();
    let rungs = ConnectRungs {
        signaled_quic: true,
        ..ConnectRungs::NONE
    };
    let config = CoreConfig {
        rungs,
        auto_quic: false,
        ..test_config()
    };
    let a = TestNode::start_on_hub("push-a", &hub, config.clone());
    let b = TestNode::start_on_hub("push-b", &hub, config);
    TestNode::pair(&a, &b);
    hub.sleep(&b.node_id());
    a.engine.connect(&b.node_id()).unwrap();
    let ended = a.sink.wait_until(Duration::from_secs(30), |e| match e {
        CoreEvent::ConnectProgress { stage, .. } if stage == "failed" => Some(()),
        _ => None,
    });
    assert!(ended.is_some(), "the ladder gave up");
    let d = a.engine.diagnose(&b.node_id()).unwrap();
    assert_eq!(d.verdict, "unreachable");
    // The last thing that explains it: the push went, nothing came back.
    assert!(
        d.attempts.iter().any(|x| x.stage == "no_answer"),
        "{:#?}",
        d.attempts
    );
    assert_eq!(d.reason.as_deref(), Some("no_answer"));
    assert!(d.last_try_ago_s.unwrap() < 60);
}
