//! Test helpers for my-domain.
//!
//! The core is generic over [`Platform`] and reports everything through an
//! [`EventSink`]. That makes it testable without a real OS: give it a
//! [`MockPlatform`] (in-memory clipboard, key-value store and notifications,
//! everything on 127.0.0.1) and a [`RecordingSink`] that stores every event so
//! a test can wait for one.
//!
//! ```no_run
//! use md_testkit::TestNode;
//! let a = TestNode::start("alice-laptop");
//! let b = TestNode::start("alice-phone");
//! TestNode::pair(&a, &b);
//! a.engine.send(&b.node_id(), "hi").unwrap();
//! b.sink.wait_for_message("hi");
//! ```
//!
//! Node.js analogy: this is the `jest.mock(...)` + `EventEmitter` spy layer,
//! except the "mocks" are real trait implementations, checked by the compiler.

// Test-only crate: panicking on a poisoned lock or a timeout IS the failure
// report, so unwrap/expect are fine here.
#![allow(clippy::unwrap_used)]

mod portmap;
mod registry;
mod relay;
mod stun;
pub use portmap::FakeRouter;
pub use registry::FakeRegistry;
pub use relay::FakeRelay;
pub use stun::FakeStun;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use mdcore::capabilities::{Clipboard, Notifier};
use mdcore::model::{IncomingMessage, Peer, Source};
use mdcore::signal::{Signal, Signaler};
use mdcore::{CoreConfig, CoreError, CoreEvent, Engine, EventSink, IfaceMode, Platform};

/// Default time a test waits for an event before failing.
pub const WAIT: Duration = Duration::from_secs(5);

/// The passphrase every [`TestNode`] logs in with, so paired nodes share a key.
pub const TEST_PASSPHRASE: &str = "correct horse battery staple";
pub const TEST_USER: &str = "tester";

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// An in-memory clipboard. `set` from the engine is visible via `get` here.
#[derive(Default)]
pub struct MemClipboard(Mutex<Option<String>>);

impl MemClipboard {
    /// Simulate the user copying something on this device.
    pub fn user_copies(&self, text: &str) {
        *self.0.lock().unwrap() = Some(text.to_string());
    }
    pub fn current(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Clipboard for MemClipboard {
    fn get(&self) -> Option<String> {
        self.current()
    }
    fn set(&self, text: &str) {
        self.user_copies(text);
    }
}

/// Records every OS notification the engine posts.
#[derive(Default)]
pub struct RecordingNotifier(Mutex<Vec<(String, String)>>);

impl RecordingNotifier {
    pub fn posted(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap().clone()
    }
}

impl Notifier for RecordingNotifier {
    fn post(&self, title: &str, body: &str) {
        self.0
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string()));
    }
}

// ---------------------------------------------------------------------------
// Platform
// ---------------------------------------------------------------------------

/// A fake device. Everything is in memory and bound to loopback.
///
/// The capability handles are `Arc`s so a test keeps a second handle and can
/// inspect them after the engine has taken ownership of the platform.
pub struct MockPlatform {
    pub id: String,
    pub name: String,
    pub kv: Arc<Mutex<HashMap<String, String>>>,
    pub clipboard: Option<Arc<MemClipboard>>,
    pub notifier: Arc<RecordingNotifier>,
}

impl MockPlatform {
    pub fn new(id: &str) -> Self {
        MockPlatform {
            id: id.to_string(),
            name: id.to_string(),
            kv: Arc::default(),
            clipboard: Some(Arc::default()),
            notifier: Arc::default(),
        }
    }

    /// A device without a clipboard (e.g. a TV or a headless server).
    pub fn without_clipboard(mut self) -> Self {
        self.clipboard = None;
        self
    }
}

impl Platform for MockPlatform {
    fn device_id(&self) -> String {
        self.id.clone()
    }
    fn device_name(&self) -> String {
        self.name.clone()
    }
    fn platform_kind(&self) -> &'static str {
        "test"
    }
    fn iface_mode(&self) -> IfaceMode {
        IfaceMode::Single(Ipv4Addr::LOCALHOST)
    }
    fn local_ip(&self) -> String {
        Ipv4Addr::LOCALHOST.to_string()
    }
    fn local_ipv6(&self) -> Option<String> {
        None // deterministic tests: never publish the host's real IPv6
    }
    fn kv_get(&self, key: &str) -> Option<String> {
        self.kv.lock().unwrap().get(key).cloned()
    }
    fn kv_set(&self, key: &str, value: &str) {
        self.kv
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
    }
    fn clipboard(&self) -> Option<&dyn Clipboard> {
        self.clipboard.as_deref().map(|c| c as &dyn Clipboard)
    }
    fn notifier(&self) -> Option<&dyn Notifier> {
        Some(&*self.notifier)
    }
}

// ---------------------------------------------------------------------------
// Event recorder
// ---------------------------------------------------------------------------

/// An [`EventSink`] that keeps every event and lets a test block until a
/// matching one arrives.
///
/// It uses a `Condvar` (condition variable): the engine's threads call
/// `notify_all` after pushing an event, and a waiting test wakes up to re-check
/// instead of sleeping in a loop. Node analogy: awaiting a Promise that the
/// `emit` side resolves.
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<CoreEvent>>,
    changed: Condvar,
}

impl EventSink for RecordingSink {
    fn emit(&self, ev: CoreEvent) {
        self.events.lock().unwrap().push(ev);
        self.changed.notify_all();
    }
}

impl RecordingSink {
    pub fn all(&self) -> Vec<CoreEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Block until an event for which `pick` returns `Some` arrives (or one
    /// already has), and return what `pick` extracted. `None` on timeout.
    pub fn wait_until<T>(
        &self,
        timeout: Duration,
        mut pick: impl FnMut(&CoreEvent) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut events = self.events.lock().unwrap();
        loop {
            if let Some(found) = events.iter().find_map(&mut pick) {
                return Some(found);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            events = self.changed.wait_timeout(events, left).unwrap().0;
        }
    }

    /// Wait for a chat message with exactly this text. Panics on timeout.
    pub fn wait_for_message(&self, text: &str) -> IncomingMessage {
        self.wait_until(WAIT, |e| match e {
            CoreEvent::MessageReceived(m) if m.text == text => Some(m.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no message {text:?} within {WAIT:?}; got {:#?}", self.all()))
    }

    /// Assert that no message with this text arrives within `within`.
    pub fn assert_no_message(&self, text: &str, within: Duration) {
        let got = self.wait_until(within, |e| match e {
            CoreEvent::MessageReceived(m) if m.text == text => Some(()),
            _ => None,
        });
        assert!(got.is_none(), "unexpected message {text:?}");
    }
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// Config for test engines: no STUN (keeps tests off the internet), all
/// ladder rungs on.
pub fn test_config() -> CoreConfig {
    CoreConfig {
        stun_servers: vec![],
        port_mapper: None, // never touch the real router from tests
        ..CoreConfig::default()
    }
}

// ---------------------------------------------------------------------------
// In-memory signaling
// ---------------------------------------------------------------------------

/// Plays the registry server's `/signal` relay (+ FCM push) in memory: a
/// signal sent to a node id is delivered to that engine's `on_signal` on a new
/// thread, like a push arriving asynchronously.
///
/// Holds `Weak` references: each engine holds the hub (as its signaler), so
/// strong references both ways would form a cycle that never gets freed.
#[derive(Default)]
pub struct SignalHub {
    nodes: Mutex<HashMap<String, Weak<Engine<MockPlatform>>>>,
    log: Mutex<Vec<(String, String, String)>>,
}

impl SignalHub {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    fn register(self: &Arc<Self>, engine: &Arc<Engine<MockPlatform>>) {
        let id = engine.identity().node_id;
        self.nodes
            .lock()
            .unwrap()
            .insert(id, Arc::downgrade(engine));
        engine.set_signaler(self.clone());
    }

    /// Every signal relayed so far, as `(from, to, type)`, e.g.
    /// `("a", "b", "punch_offer")`.
    pub fn log(&self) -> Vec<(String, String, String)> {
        self.log.lock().unwrap().clone()
    }

    /// The signal types relayed so far, in order.
    pub fn kinds(&self) -> Vec<String> {
        self.log().into_iter().map(|(_, _, k)| k).collect()
    }
}

impl Signaler for SignalHub {
    fn send(&self, from: &str, to: &str, signal: &Signal) -> mdcore::Result<()> {
        let target = self
            .nodes
            .lock()
            .unwrap()
            .get(to)
            .and_then(Weak::upgrade)
            .ok_or(CoreError::PeerNotFound)?;
        let payload = serde_json::to_string(signal).map_err(|e| CoreError::Other(e.to_string()))?;
        let kind = serde_json::from_str::<serde_json::Value>(&payload)
            .ok()
            .and_then(|v| v["type"].as_str().map(String::from))
            .unwrap_or_default();
        self.log
            .lock()
            .unwrap()
            .push((from.to_string(), to.to_string(), kind));
        let from = from.to_string();
        std::thread::spawn(move || target.on_signal(&from, &payload));
        Ok(())
    }
}

/// One running engine plus handles to everything a test wants to inspect.
pub struct TestNode {
    /// In an `Arc` so a [`SignalHub`] can reach it from other threads.
    pub engine: Arc<Engine<MockPlatform>>,
    pub sink: Arc<RecordingSink>,
    pub clipboard: Option<Arc<MemClipboard>>,
    pub notifier: Arc<RecordingNotifier>,
}

impl TestNode {
    /// Start a node with a clipboard, logged in (offline) with the shared test
    /// passphrase.
    pub fn start(id: &str) -> Self {
        Self::start_with(MockPlatform::new(id))
    }

    pub fn start_with(platform: MockPlatform) -> Self {
        Self::start_full(platform, test_config())
    }

    /// Start a node whose signals go through `hub`, with the given config
    /// (e.g. only some ladder rungs enabled).
    pub fn start_on_hub(id: &str, hub: &Arc<SignalHub>, config: CoreConfig) -> Self {
        let node = Self::start_full(MockPlatform::new(id), config);
        hub.register(&node.engine);
        node
    }

    pub fn start_full(platform: MockPlatform, config: CoreConfig) -> Self {
        let node = Self::start_logged_out_with(platform, config);
        node.engine
            .dev_login(TEST_USER, TEST_PASSPHRASE)
            .expect("offline login");
        node
    }

    /// A node with no username and no key, e.g. to test the login flow
    /// against a [`FakeRegistry`].
    pub fn start_logged_out(id: &str) -> Self {
        Self::start_logged_out_with(MockPlatform::new(id), test_config())
    }

    pub fn start_logged_out_with(platform: MockPlatform, config: CoreConfig) -> Self {
        let clipboard = platform.clipboard.clone();
        let notifier = platform.notifier.clone();
        let sink = Arc::new(RecordingSink::default());
        let engine = Arc::new(
            Engine::start_with_config(platform, sink.clone(), config).expect("engine starts"),
        );
        TestNode {
            engine,
            sink,
            clipboard,
            notifier,
        }
    }

    pub fn node_id(&self) -> String {
        self.engine.identity().node_id
    }

    /// This node as a peer entry, as another device would store it.
    pub fn as_peer(&self) -> Peer {
        let id = self.engine.identity();
        Peer {
            node_id: id.node_id,
            name: id.name,
            ip: Ipv4Addr::LOCALHOST.to_string(),
            tcp_port: id.tcp_port,
            udp_port: id.udp_port,
            ws_port: id.ws_port,
            quic_port: id.quic_port,
            source: Source::Manual,
            ws_open: false,
            inbound_blocked: false,
            reflexive_ip: None,
            reflexive_udp_port: None,
            nat: self.engine.nat_kind(),
            ipv6: None,
            mapped_quic: None,
        }
    }

    /// Wait until this node has classified its NAT as `kind` (STUN runs in
    /// the background at start).
    pub fn wait_nat(&self, kind: mdcore::nat::NatKind, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while self.engine.nat_kind() != kind {
            assert!(
                std::time::Instant::now() < deadline,
                "NAT stayed {:?}, wanted {kind:?}",
                self.engine.nat_kind()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Make two nodes know each other (what discovery or the registry does in
    /// production).
    pub fn pair(a: &TestNode, b: &TestNode) {
        a.engine.upsert_peer(b.as_peer());
        b.engine.upsert_peer(a.as_peer());
    }

    /// Wait until this node reports a live link to `peer` over any transport;
    /// returns the transport kind ("udp", "ws", "punch", "webrtc").
    pub fn wait_connected(&self, peer: &TestNode, timeout: Duration) -> Option<String> {
        let id = peer.node_id();
        self.sink.wait_until(timeout, |e| match e {
            CoreEvent::PeerConnected { node_id, transport } if *node_id == id => {
                serde_json::to_value(transport)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
            }
            _ => None,
        })
    }

    /// The connect-ladder stages this node reported for `peer`, in order.
    pub fn ladder_stages(&self, peer: &TestNode) -> Vec<String> {
        let id = peer.node_id();
        self.sink
            .all()
            .into_iter()
            .filter_map(|e| match e {
                CoreEvent::ConnectProgress { node_id, stage, .. } if node_id == id => Some(stage),
                _ => None,
            })
            .collect()
    }

    pub fn clip(&self) -> &MemClipboard {
        self.clipboard.as_deref().expect("node has a clipboard")
    }
}
