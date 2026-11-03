//! Battery alerts and keep-awake between engines over real links: a
//! device's status crossing the low line (or reaching full) tells the
//! others once; keep-awake reaches the device as its action.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use md_testkit::{MockPlatform, TestNode, WAIT};
use mdcore::model::BatteryAlertSettings;
use mdcore::CoreEvent;

fn status(level: u8, charging: bool) -> String {
    format!(r#"{{"battery":{level},"charging":{charging}}}"#)
}

fn alerts(node: &TestNode) -> Vec<(String, String, u8)> {
    node.sink
        .all()
        .into_iter()
        .filter_map(|e| match e {
            CoreEvent::BatteryAlert { from, alert, level } => Some((from, alert, level)),
            _ => None,
        })
        .collect()
}

/// Poll `f` until it gives `Some` (the sink's own wait holds its lock,
/// so it can't look at all events at once).
fn poll<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if std::time::Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn wait_alerts(node: &TestNode, n: usize) -> Vec<(String, String, u8)> {
    poll(|| {
        let a = alerts(node);
        (a.len() >= n).then_some(a)
    })
    .unwrap_or_else(|| panic!("expected {n} alerts; got {:?}", alerts(node)))
}

#[test]
fn other_devices_hear_low_and_full_once_each() {
    let phone = TestNode::start("batt-phone");
    let (pc1, pc2) = (TestNode::start("batt-pc1"), TestNode::start("batt-pc2"));
    TestNode::pair(&phone, &pc1);
    TestNode::pair(&phone, &pc2);
    assert_eq!(
        phone.engine.battery_alerts(),
        BatteryAlertSettings::default()
    );

    for (level, charging) in [(40, false), (20, false), (16, false)] {
        phone.engine.set_device_status(&status(level, charging));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(alerts(&pc1).is_empty());

    phone.engine.set_device_status(&status(15, false));
    phone.engine.set_device_status(&status(14, false));
    phone.engine.set_device_status(&status(15, false));
    for pc in [&pc1, &pc2] {
        let got = wait_alerts(pc, 1);
        assert_eq!(got, [(phone.node_id(), "low".to_string(), 15)]);
    }

    for level in [50, 99, 100, 100] {
        phone.engine.set_device_status(&status(level, true));
    }
    let got = wait_alerts(&pc1, 2);
    assert_eq!(got[1], (phone.node_id(), "full".to_string(), 100));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(alerts(&pc1).len(), 2, "once per crossing");
    assert_eq!(alerts(&pc2).len(), 2);
    // Not shown as chat.
    assert!(!pc1
        .sink
        .all()
        .iter()
        .any(|e| matches!(e, CoreEvent::MessageReceived(_))));
}

#[test]
fn alerts_can_be_turned_off_or_moved_and_survive_a_restart() {
    let (store, secrets) = (Arc::default(), Arc::default());
    let laptop = || MockPlatform {
        kv: Arc::clone(&store),
        secrets: Arc::clone(&secrets),
        ..MockPlatform::new("batt-laptop")
    };
    let pc = TestNode::start("batt-other");
    let lap = TestNode::start_with(laptop());
    TestNode::pair(&lap, &pc);

    assert!(lap
        .engine
        .set_battery_alerts(BatteryAlertSettings {
            enabled: true,
            low: 0
        })
        .is_err());
    assert!(lap
        .engine
        .set_battery_alerts(BatteryAlertSettings {
            enabled: true,
            low: 100
        })
        .is_err());

    // Off: nothing.
    lap.engine
        .set_battery_alerts(BatteryAlertSettings {
            enabled: false,
            low: 15,
        })
        .unwrap();
    lap.engine.set_device_status(&status(40, false));
    lap.engine.set_device_status(&status(10, false));
    std::thread::sleep(Duration::from_millis(500));
    assert!(alerts(&pc).is_empty());

    // On, at 30%: charged a bit (re-armed), then down past 30.
    lap.engine
        .set_battery_alerts(BatteryAlertSettings {
            enabled: true,
            low: 30,
        })
        .unwrap();
    lap.engine.set_device_status(&status(40, false));
    lap.engine.set_device_status(&status(29, false));
    assert_eq!(wait_alerts(&pc, 1)[0].2, 29);

    // Restarted, still low: not told again; the settings are kept.
    drop(lap);
    let lap = TestNode::start_with(laptop());
    TestNode::pair(&lap, &pc);
    assert_eq!(
        lap.engine.battery_alerts(),
        BatteryAlertSettings {
            enabled: true,
            low: 30
        }
    );
    lap.engine.set_device_status(&status(28, false));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(alerts(&pc).len(), 1);
}

#[test]
fn keep_awake_reaches_the_device_as_an_action() {
    let a = TestNode::start("awake-a");
    let b = TestNode::start("awake-b");
    TestNode::pair(&a, &b);
    assert!(a.engine.keep_awake(&b.node_id(), 24 * 60 + 1).is_err());
    a.engine.keep_awake(&b.node_id(), 30).unwrap();
    a.engine.keep_awake(&b.node_id(), 0).unwrap();
    let got = poll(|| {
        let v: Vec<(String, serde_json::Value)> = b
            .sink
            .all()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::Action { from, name, args } if name == "keep_awake" => {
                    assert_eq!(from, a.node_id());
                    Some((name, args))
                }
                _ => None,
            })
            .collect();
        (v.len() >= 2).then_some(v)
    })
    .expect("no keep_awake actions");
    let minutes: Vec<u64> = got
        .iter()
        .map(|(_, args)| args["minutes"].as_u64().unwrap())
        .collect();
    assert!(minutes.contains(&30) && minutes.contains(&0), "{minutes:?}");
}
