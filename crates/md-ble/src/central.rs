//! The computer's side: find phones advertising the my-domain service and
//! connect to the ones we have no other link to.
//!
//! Bluetooth is a fallback (and every connection keeps the phone's radio
//! busy), so a connection is only made while the device has no direct
//! link, and dropped once it has one. Which phone is which is learned from
//! its `Hello` on the first connection and remembered.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use mdcore::engine::{BleSender, Engine};
use mdcore::platform::Platform;

use crate::{split, Joiner, FROM_PHONE, SERVICE, TO_PHONE};

/// Bytes per write. macOS and Android agree on a larger packet size by
/// themselves; writes this big also work when they didn't (long writes).
const PACKET: usize = 180;
/// How often connections are checked (drop when direct, retry the rest).
const CHECK_EVERY: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct State {
    /// Connected (or connecting) phones -> their link id once open.
    links: HashMap<PeripheralId, Option<u64>>,
    /// Which device each phone turned out to be.
    known: HashMap<PeripheralId, String>,
    /// Phones seen advertising the service.
    seen: HashSet<PeripheralId>,
}

/// A handle to the engine: an `Arc<Engine>`, or anything else that leads to
/// it (the desktop app keeps its engine in Tauri's state).
pub trait EngineRef<P: Platform>:
    std::ops::Deref<Target = Engine<P>> + Clone + Send + Sync + 'static
{
}

impl<P: Platform, T> EngineRef<P> for T where
    T: std::ops::Deref<Target = Engine<P>> + Clone + Send + Sync + 'static
{
}

/// Start looking for phones in the background. Returns at once; does
/// nothing if this computer has no Bluetooth (or it's off).
pub fn start<P: Platform + 'static, E: EngineRef<P>>(engine: E) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("md-ble")
            .enable_all()
            .build()
        else {
            return;
        };
        if let Err(e) = rt.block_on(run(engine)) {
            eprintln!("[ble] stopped: {e}");
        }
    });
}

async fn run<P: Platform + 'static, E: EngineRef<P>>(engine: E) -> Result<(), btleplug::Error> {
    let manager = Manager::new().await?;
    let Some(adapter) = manager.adapters().await?.into_iter().next() else {
        return Ok(());
    };
    let mut events = adapter.events().await?;
    adapter
        .start_scan(ScanFilter {
            services: vec![SERVICE],
        })
        .await?;
    let state = Arc::new(Mutex::new(State::default()));
    let mut tick = tokio::time::interval(CHECK_EVERY);
    loop {
        tokio::select! {
            event = events.next() => {
                let Some(event) = event else { return Ok(()) };
                match event {
                    CentralEvent::DeviceDiscovered(id)
                    | CentralEvent::DeviceUpdated(id)
                    | CentralEvent::ServicesAdvertisement { id, .. } => {
                        if advertises_service(&adapter, &id).await {
                            state.lock().seen.insert(id.clone());
                            maybe_connect(&engine, &adapter, &state, id);
                        }
                    }
                    CentralEvent::DeviceDisconnected(id) => {
                        let link = state.lock().links.remove(&id).flatten();
                        if let Some(link) = link {
                            engine.ble_closed(link);
                        }
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => check(&engine, &adapter, &state).await,
        }
    }
}

async fn advertises_service(adapter: &Adapter, id: &PeripheralId) -> bool {
    let Ok(p) = adapter.peripheral(id).await else {
        return false;
    };
    matches!(p.properties().await, Ok(Some(props)) if props.services.contains(&SERVICE))
}

fn is_direct<P: Platform>(engine: &Engine<P>, node: &str) -> bool {
    engine
        .link_stats()
        .iter()
        .any(|l| l.node_id == node && l.direct)
        // A stream that chose Bluetooth only keeps its link.
        && !engine.audio_wants_ble(node)
}

/// Drop connections to devices that are direct now; retry the others.
async fn check<P: Platform + 'static, E: EngineRef<P>>(
    engine: &E,
    adapter: &Adapter,
    state: &Arc<Mutex<State>>,
) {
    let (direct, retry): (Vec<_>, Vec<_>) = {
        let s = state.lock();
        let direct = s
            .links
            .keys()
            .filter(|id| s.known.get(*id).is_some_and(|n| is_direct(engine, n)))
            .cloned()
            .collect();
        let retry = s
            .seen
            .iter()
            .filter(|id| !s.links.contains_key(*id))
            .cloned()
            .collect();
        (direct, retry)
    };
    for id in direct {
        if let Ok(p) = adapter.peripheral(&id).await {
            let _ = p.disconnect().await; // DeviceDisconnected closes the link
        }
    }
    for id in retry {
        maybe_connect(engine, adapter, state, id);
    }
}

fn maybe_connect<P: Platform + 'static, E: EngineRef<P>>(
    engine: &E,
    adapter: &Adapter,
    state: &Arc<Mutex<State>>,
    id: PeripheralId,
) {
    {
        let mut s = state.lock();
        if s.links.contains_key(&id) {
            return;
        }
        if s.known.get(&id).is_some_and(|n| is_direct(engine, n)) {
            return;
        }
        s.links.insert(id.clone(), None);
    }
    let (engine, adapter, state) = (engine.clone(), adapter.clone(), state.clone());
    tokio::spawn(async move {
        let link = match adapter.peripheral(&id).await {
            Ok(p) => open(&engine, &state, &id, p).await,
            Err(_) => None,
        };
        let mut s = state.lock();
        match link {
            Some(link) => {
                s.links.insert(id, Some(link));
            }
            None => {
                s.links.remove(&id);
            }
        }
    });
}

/// Connect, subscribe, and hand the connection to the core.
async fn open<P: Platform + 'static, E: EngineRef<P>>(
    engine: &E,
    state: &Arc<Mutex<State>>,
    id: &PeripheralId,
    p: Peripheral,
) -> Option<u64> {
    tokio::time::timeout(CONNECT_TIMEOUT, p.connect())
        .await
        .ok()?
        .ok()?;
    p.discover_services().await.ok()?;
    let chars = p.characteristics();
    let find = |u| {
        chars
            .iter()
            .find(|c: &&Characteristic| c.uuid == u)
            .cloned()
    };
    let (Some(to_phone), Some(from_phone)) = (find(TO_PHONE), find(FROM_PHONE)) else {
        let _ = p.disconnect().await;
        return None;
    };
    p.subscribe(&from_phone).await.ok()?;
    let mut notes = p.notifications().await.ok()?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let waiting = Arc::new(AtomicUsize::new(0));
    let link = engine.ble_opened(Arc::new(Queue(tx, waiting.clone())));

    // Writer: the core's frames, chunk by chunk, in order.
    let writer = p.clone();
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            for chunk in split(&frame, PACKET) {
                if writer
                    .write(&to_phone, &chunk, WriteType::WithResponse)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            waiting.fetch_sub(1, Ordering::Relaxed);
        }
    });

    // Reader: whole frames to the core; the phone's `Hello` says who it is.
    let (engine, state, id) = (engine.clone(), state.clone(), id.clone());
    tokio::spawn(async move {
        let mut joiner = Joiner::default();
        while let Some(note) = notes.next().await {
            if note.uuid != FROM_PHONE {
                continue;
            }
            let Some(frame) = joiner.push(&note.value) else {
                continue;
            };
            if let Some(node) = hello_from(&frame) {
                state.lock().known.insert(id.clone(), node);
            }
            let engine = engine.clone();
            tokio::task::block_in_place(|| engine.ble_frame(link, &frame));
        }
    });
    Some(link)
}

/// The node id in a `Hello` frame.
fn hello_from(frame: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(frame).ok()?;
    (v["type"] == "hello").then(|| v["node_id"].as_str().map(String::from))?
}

/// The core's outgoing frames, queued for the writer task, and how many
/// wait (the core holds back audio when the radio is behind).
struct Queue(
    tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    Arc<AtomicUsize>,
);

impl BleSender for Queue {
    fn send(&self, frame: Vec<u8>) {
        self.1.fetch_add(1, Ordering::Relaxed);
        if self.0.send(frame).is_err() {
            self.1.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn pending(&self) -> usize {
        self.1.load(Ordering::Relaxed)
    }
}
