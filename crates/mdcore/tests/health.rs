//! The health check and first-run setup (features::health, app_info,
//! onboarding): each device's app version and feature flags reach the
//! others, an older app that lacks features is flagged, links show how
//! they're carried, and setup and the device's name are kept per device
//! across restarts.

// Test-only file: helpers unwrap freely, a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::time::Instant;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::features::app_info::PeerApp;
use mdcore::features::health::{HealthCheck, HealthLevel, PermissionFact, PlatformFacts};
use mdcore::features::onboarding::Step;
use mdcore::CoreConfig;

fn link(a: &TestNode, b: &TestNode, text: &str) {
    a.engine.send(&b.node_id(), text).unwrap();
    b.sink.wait_for_message(text);
}

/// Wait until `node` knows `peer`'s app with `version`.
fn wait_app(node: &TestNode, peer: &TestNode, version: Option<&str>) -> PeerApp {
    let start = Instant::now();
    loop {
        if let Some(app) = node.engine.peer_app(&peer.node_id()) {
            if app.version.as_deref() == version {
                return app;
            }
        }
        assert!(
            start.elapsed() < WAIT,
            "no app {version:?} for {}",
            peer.node_id()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn check<'a>(checks: &'a [HealthCheck], id: &str, node: &str) -> &'a HealthCheck {
    checks
        .iter()
        .find(|c| c.id == id && c.node_id.as_deref() == Some(node))
        .unwrap_or_else(|| panic!("no {id} check for {node}: {checks:?}"))
}

#[test]
fn an_older_app_that_lacks_features_is_flagged() {
    let new = TestNode::start("health-new");
    // An older app: no batches, no media controls.
    let old = TestNode::start_full(
        MockPlatform::new("health-old"),
        CoreConfig {
            transfer_batches: false,
            media_ctl: false,
            ..test_config()
        },
    );
    TestNode::pair(&new, &old);
    new.engine.set_app_version("0.3.0");
    old.engine.set_app_version("0.2.1");

    link(&new, &old, "hello");
    let app = wait_app(&new, &old, Some("0.2.1"));
    assert!(!app.flags.contains(&"xfer".to_string()));
    assert!(app.flags.contains(&"links_map".to_string()));
    wait_app(&old, &new, Some("0.3.0"));

    let report = new.engine.health(&PlatformFacts::default(), false);
    let v = check(&report.checks, "version", &old.node_id());
    assert_eq!((v.level, v.reason.as_str()), (HealthLevel::Warn, "older"));
    assert_eq!(
        v.features,
        vec!["xfer".to_string(), "media_ctl".to_string()]
    );
    assert_eq!(v.detail.as_deref(), Some("0.2.1"));
    assert_eq!(v.fix.as_deref(), Some("update_other"));
    assert!(report.warn >= 1);
    // The link is up on this machine's network: direct.
    let l = check(&report.checks, "link", &old.node_id());
    assert_eq!((l.level, l.reason.as_str()), (HealthLevel::Ok, "direct"));

    // The older one sees a newer app there: nothing it can fix there.
    let report = old.engine.health(&PlatformFacts::default(), false);
    let v = check(&report.checks, "version", &new.node_id());
    assert_eq!((v.level, v.reason.as_str()), (HealthLevel::Info, "newer"));

    // A version said later goes over the live link at once.
    old.engine.set_app_version("0.3.0");
    wait_app(&new, &old, Some("0.3.0"));
}

#[test]
fn an_app_from_before_versions_says_none() {
    let new = TestNode::start("health-v-new");
    let before = TestNode::start("health-v-before");
    TestNode::pair(&new, &before);
    new.engine.set_app_version("0.3.0");
    link(&new, &before, "hi");
    // What it said came without a version: kept as an app from before.
    let app = wait_app(&new, &before, None);
    assert!(!app.flags.is_empty());
    // It has every feature this one has: grey, not amber.
    let report = new.engine.health(&PlatformFacts::default(), false);
    let v = check(&report.checks, "version", &before.node_id());
    assert_eq!((v.level, v.reason.as_str()), (HealthLevel::Info, "older"));
    assert!(v.features.is_empty());
}

#[test]
fn permissions_follow_the_features_picked() {
    let node = TestNode::start("health-perms");
    let facts = PlatformFacts {
        permissions: vec![PermissionFact {
            id: "accessibility".into(),
            features: vec!["media".into()],
            state: "denied".into(),
            critical: true,
        }],
        disk_free_mb: Some(400),
        disk_total_mb: Some(100_000),
    };
    // Nothing picked yet: everything counts.
    let r = node.engine.health(&facts, false);
    assert!(r
        .checks
        .iter()
        .any(|c| c.id == "permission" && c.level == HealthLevel::Bad));
    assert!(r
        .checks
        .iter()
        .any(|c| c.id == "disk" && c.level == HealthLevel::Bad));
    assert_eq!(r.worst, HealthLevel::Bad);
    // Media not picked: the permission isn't asked about.
    node.engine
        .set_onboarding_features(&["clipboard".into(), "files".into()]);
    let r = node.engine.health(&facts, false);
    assert!(r.checks.iter().all(|c| c.id != "permission"));
}

#[test]
fn setup_and_the_name_are_kept_across_a_restart() {
    let platform = MockPlatform::new("health-setup");
    let kv = platform.kv.clone();
    let secrets = platform.secrets.clone();
    {
        let node = TestNode::start_with(platform);
        assert_eq!(node.engine.onboarding().step, Step::Name);
        assert_eq!(
            node.engine.set_device_name("  Kitchen   Mac ").unwrap(),
            "Kitchen Mac"
        );
        assert_eq!(node.engine.identity().name, "Kitchen Mac");
        assert!(node.engine.set_device_name("   ").is_err());
        node.engine.onboarding_act("next", None).unwrap();
        node.engine.onboarding_act("skip", None).unwrap();
        node.engine.set_onboarding_features(&["files".into()]);
        assert!(node.engine.onboarding_act("fly", None).is_err());
        let s = node
            .engine
            .onboarding_act("goto", Some("permissions"))
            .unwrap();
        assert_eq!(s.step, Step::Permissions);
    }
    let mut again = MockPlatform::new("health-setup");
    again.kv = kv;
    again.secrets = secrets;
    let node = TestNode::start_with(again);
    let s = node.engine.onboarding();
    assert_eq!(s.step, Step::Permissions);
    assert_eq!(s.skipped, vec![Step::Devices]);
    assert_eq!(s.features, Some(vec!["files".to_string()]));
    assert!(!s.finished);
    assert_eq!(node.engine.identity().name, "Kitchen Mac");
    let s = node.engine.onboarding_act("skip_all", None).unwrap();
    assert!(s.finished && s.skipped_all);
}
