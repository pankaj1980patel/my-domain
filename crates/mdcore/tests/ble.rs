//! Bluetooth LE links (`engine::ble`): the platform carries the frames; here
//! an in-memory pipe plays the radio between two engines.

// Test-only file: a panic IS the failure report.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use md_testkit::{test_config, MockPlatform, TestNode, WAIT};
use mdcore::engine::{BleSender, Engine};
use mdcore::CoreConfig;

/// One direction of the "radio": frames go to the other engine's link.
#[derive(Default)]
struct Pipe(Mutex<Option<(Weak<Engine<MockPlatform>>, u64)>>);

impl BleSender for Pipe {
    fn send(&self, frame: Vec<u8>) {
        let to = self.0.lock().unwrap().clone();
        if let Some((engine, id)) = to {
            if let Some(engine) = engine.upgrade() {
                engine.ble_frame(id, &frame);
            }
        }
    }
}

/// Connect `a` and `b` over Bluetooth; returns their link ids.
fn radio(a: &TestNode, b: &TestNode) -> (u64, u64) {
    let (to_b, to_a) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    let id_a = a.engine.ble_opened(to_b.clone());
    let id_b = b.engine.ble_opened(to_a.clone());
    *to_b.0.lock().unwrap() = Some((Arc::downgrade(&b.engine), id_b));
    *to_a.0.lock().unwrap() = Some((Arc::downgrade(&a.engine), id_a));
    (id_a, id_b)
}

/// No network path between them: no QUIC, no rungs.
fn offline() -> CoreConfig {
    CoreConfig {
        auto_quic: false,
        rungs: mdcore::ConnectRungs::none(),
        ..test_config()
    }
}

#[test]
fn with_no_network_messages_go_over_bluetooth() {
    let a = TestNode::start_full(MockPlatform::new("ble-a"), offline());
    let b = TestNode::start_full(MockPlatform::new("ble-b"), offline());
    TestNode::pair(&a, &b);
    let (id_a, _) = radio(&a, &b);
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("ble"));
    assert_eq!(b.wait_connected(&a, WAIT).as_deref(), Some("ble"));

    assert_eq!(a.engine.send(&b.node_id(), "over the air").unwrap(), "BLE");
    let got = b.sink.wait_for_message("over the air");
    assert_eq!(got.protocol, "BLE");
    assert!(got.ok, "decrypted");
    b.engine.send(&a.node_id(), "and back").unwrap();
    a.sink.wait_for_message("and back");

    // The connection ends: the link goes with it.
    a.engine.ble_closed(id_a);
    a.sink
        .wait_until(WAIT, |e| match e {
            mdcore::CoreEvent::PeerDisconnected { node_id } if *node_id == b.node_id() => Some(()),
            _ => None,
        })
        .expect("link dropped");
}

#[test]
fn a_direct_link_replaces_bluetooth() {
    let a = TestNode::start("ble-up-a");
    let b = TestNode::start("ble-up-b");
    TestNode::pair(&a, &b);
    radio(&a, &b);
    assert_eq!(a.wait_connected(&b, WAIT).as_deref(), Some("ble"));
    // Bluetooth isn't direct: connecting still looks for a better path.
    a.engine.connect(&b.node_id()).unwrap();
    a.sink
        .wait_until(WAIT, |e| match e {
            mdcore::CoreEvent::PeerConnected { node_id, transport }
                if *node_id == b.node_id() && !transport.is_fallback() =>
            {
                Some(())
            }
            _ => None,
        })
        .expect("upgraded to a direct link");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.engine.send(&b.node_id(), "direct now").unwrap(), "QUIC");
    b.sink.wait_for_message("direct now");
}
