//! Discrete-feature layer: small, reliable control messages that ride inside the
//! encrypted `Plaintext.text` as a JSON object with a `"type"` tag.
//!
//! Each feature lives in its own file and exposes a [`ControlHandler`]. Handlers
//! register into a [`ControlRegistry`] keyed by the `"type"` value(s) they claim;
//! the engine builds the registry once and dispatch is a map lookup. Adding a
//! feature = a new file + one `register(...)` line — nothing else changes.
//!
//! Streaming features (audio/video, remote control) need the binary channel-mux
//! (protocol v2) instead; that seam is documented in `crate::service`.

pub mod addr_update;
pub mod audio_codec;
pub mod audio_jitter;
pub mod audio_share;
pub mod backup;
pub mod battery;
pub mod browse;
pub mod call_history;
pub mod call_notification;
pub mod camera;
pub mod clipboard;
pub mod code;
pub mod device;
pub mod diag;
pub mod dnd;
pub mod extras;
pub mod files;
pub mod folder_sync;
pub mod folders;
pub mod gallery;
pub mod input;
pub mod kvm;
pub mod labels;
pub mod links_map;
pub mod media;
pub mod nearby;
pub mod notes;
pub mod notif_apps;
pub mod notification;
pub mod opus_audio;
pub mod p2p_ping;
pub mod read_later;
pub mod screen;
pub mod share;
pub mod signal_fwd;
pub mod timers;
pub mod wake;
pub mod wifi_direct;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::engine::NetCtx;
use crate::platform::Platform;

/// A parsed control message handed to a [`ControlHandler`]. `from`/`ip`/`protocol`
/// describe the carrying chat envelope; field accessors read the JSON body.
pub struct ControlMsg<'a> {
    pub from: &'a str,
    /// The sending device as the link knows it (`None` if the link can't
    /// tell). Trust this over the message's own `"from"` field.
    pub node: Option<&'a str>,
    pub ip: &'a str,
    pub protocol: &'a str,
    value: Value,
}

impl<'a> ControlMsg<'a> {
    /// The sending device's node id: the link's, else the message's
    /// `"from"` field.
    pub fn sender(&self) -> String {
        match self.node {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => self.str("from"),
        }
    }

    pub fn kind(&self) -> &str {
        self.value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
    }
    /// String field, or "" if absent.
    pub fn str(&self, key: &str) -> String {
        self.value
            .get(key)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    }
    /// Optional string field.
    pub fn opt(&self, key: &str) -> Option<String> {
        self.value
            .get(key)
            .and_then(|x| x.as_str())
            .map(String::from)
    }
    /// Boolean field, false if absent (or not a boolean).
    pub fn flag(&self, key: &str) -> bool {
        self.value
            .get(key)
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
    }
    /// Raw JSON field (for opaque passthrough payloads).
    pub fn json(&self, key: &str) -> Value {
        self.value.get(key).cloned().unwrap_or(Value::Null)
    }
}

/// One feature's inbound handler. A handler may claim several `"type"` values
/// (e.g. clipboard's push/request/response trio).
pub trait ControlHandler<P: Platform>: Send + Sync {
    fn kinds(&self) -> &'static [&'static str];
    fn handle(&self, ctx: &NetCtx<P>, msg: &ControlMsg);
}

/// Map of `"type"` → handler. Built once per engine.
pub struct ControlRegistry<P: Platform> {
    handlers: HashMap<&'static str, Arc<dyn ControlHandler<P>>>,
}

impl<P: Platform> ControlRegistry<P> {
    /// The standard feature set. `has_clipboard` should reflect whether the
    /// platform provides a `Clipboard`: when it doesn't (e.g. android handles
    /// clipboard in its host layer), the clipboard handler is left unregistered
    /// so those control messages pass through to the host instead of being
    /// swallowed here. `folder_sync`: this device syncs folders
    /// (`CoreConfig::folder_sync`); without it the `sync_*` messages aren't
    /// known here, as in an app from before; likewise `labels` and the
    /// `labels` message (`CoreConfig::shared_labels`), `notes` and the
    /// `notes*` messages (`CoreConfig::notes`), `ping` with the ping
    /// and speed-test messages (`CoreConfig::diag`), `wake_relay`
    /// (`CoreConfig::wake_relay`), `camera_ctl` (`CoreConfig::camera`),
    /// `timers` (`CoreConfig::timers`), the `read_later*` messages
    /// (`CoreConfig::read_later`), `links_map`
    /// (`CoreConfig::links_map`), the `nearby_*` clock pings
    /// (`CoreConfig::nearby`), and the `wifi_direct_*` messages
    /// (`CoreConfig::wifi_direct`), and the `kvm_*` messages
    /// (`CoreConfig::kvm`).
    #[allow(clippy::too_many_arguments)]
    pub fn with_defaults(
        has_clipboard: bool,
        folder_sync: bool,
        labels: bool,
        notes: bool,
        diag: bool,
        wake_relay: bool,
        camera: bool,
        timers: bool,
        read_later: bool,
        links_map: bool,
        nearby: bool,
        wifi_direct: bool,
        kvm: bool,
    ) -> Self {
        let mut r = ControlRegistry {
            handlers: HashMap::new(),
        };
        if has_clipboard {
            r.register(Arc::new(clipboard::ClipboardHandler));
        }
        r.register(Arc::new(notification::NotificationHandler));
        r.register(Arc::new(call_notification::CallNotificationHandler));
        r.register(Arc::new(call_history::CallHistoryHandler));
        r.register(Arc::new(notif_apps::NotifAppsHandler));
        r.register(Arc::new(code::CodeHandler));
        r.register(Arc::new(device::DeviceHandler));
        r.register(Arc::new(files::FilesHandler));
        r.register(Arc::new(browse::BrowseHandler));
        r.register(Arc::new(share::ShareHandler));
        r.register(Arc::new(media::MediaHandler));
        r.register(Arc::new(p2p_ping::P2pPingHandler));
        r.register(Arc::new(addr_update::AddrUpdateHandler));
        r.register(Arc::new(signal_fwd::SignalFwdHandler));
        r.register(Arc::new(gallery::GalleryHandler));
        r.register(Arc::new(battery::BatteryHandler));
        r.register(Arc::new(input::InputHandler));
        r.register(Arc::new(dnd::DndHandler));
        r.register(Arc::new(screen::ScreenHandler));
        r.register(Arc::new(audio_share::AudioShareHandler));
        if folder_sync {
            r.register(Arc::new(folder_sync::FolderSyncHandler));
        }
        if labels {
            r.register(Arc::new(labels::LabelsHandler));
        }
        if notes {
            r.register(Arc::new(notes::NotesHandler));
        }
        if diag {
            r.register(Arc::new(diag::DiagHandler));
        }
        if wake_relay {
            r.register(Arc::new(wake::WakeRelayHandler));
        }
        if camera {
            r.register(Arc::new(camera::CameraHandler));
        }
        if timers {
            r.register(Arc::new(timers::TimersHandler));
        }
        if read_later {
            r.register(Arc::new(read_later::ReadLaterHandler));
        }
        if links_map {
            r.register(Arc::new(links_map::LinksMapHandler));
        }
        if nearby {
            r.register(Arc::new(nearby::NearbyHandler));
        }
        if wifi_direct {
            r.register(Arc::new(wifi_direct::WifiDirectHandler));
        }
        if kvm {
            r.register(Arc::new(kvm::KvmHandler));
        }
        r
    }

    pub fn register(&mut self, handler: Arc<dyn ControlHandler<P>>) {
        for kind in handler.kinds() {
            self.handlers.insert(kind, handler.clone());
        }
    }

    /// Dispatch `text` to its handler. Returns `true` if it was a recognized
    /// control message (handled in-band; the caller must NOT show it as chat).
    pub fn dispatch(
        &self,
        ctx: &NetCtx<P>,
        from: &str,
        node: Option<&str>,
        ip: &str,
        protocol: &str,
        text: &str,
    ) -> bool {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        let kind = match value.get("type").and_then(|t| t.as_str()) {
            Some(k) => k.to_string(),
            None => return false,
        };
        let Some(handler) = self.handlers.get(kind.as_str()).cloned() else {
            return false;
        };
        let msg = ControlMsg {
            from,
            node,
            ip,
            protocol,
            value,
        };
        handler.handle(ctx, &msg);
        true
    }
}
