//! UI-bound events and the sink that delivers them. This unifies the two very
//! different delivery models:
//!   * desktop bridges each event to a Tauri `app.emit(...)` (push),
//!   * android records events into an inbox/snapshot drained by JNI poll calls.
//!
//! The Engine only ever calls `EventSink::emit`; the adapter decides how it
//! reaches the UI.

use serde::Serialize;
use serde_json::Value;

use crate::model::{IncomingMessage, Peer};
use crate::transport::TransportKind;

/// Serializes as an *internally tagged* object: the variant name goes into a
/// `"kind"` field next to the variant's own fields, e.g.
/// `{"kind":"notification","from":"…","title":"…","body":"…","app":null}`.
/// (Node analogy: a discriminated union with `kind` as the discriminant.)
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreEvent {
    /// The peer list changed (discovery, registry refresh, manual add/remove).
    PeersUpdated { peers: Vec<Peer> },
    /// A decrypted (or undecryptable) chat message arrived.
    MessageReceived(IncomingMessage),
    /// A peer acknowledged chat message `id` (it arrived).
    MessageDelivered { node_id: String, id: String },
    /// A WebSocket connection to a peer was established.
    WsConnected { node_id: String },
    /// A WebSocket connection to a peer closed.
    WsDisconnected { node_id: String },
    /// A live peer-to-peer connection came up (over any transport). This is the
    /// single source of truth for the UI "connected" (green) indicator.
    PeerConnected {
        node_id: String,
        transport: TransportKind,
    },
    /// A live peer-to-peer connection went away (closed or went stale).
    PeerDisconnected { node_id: String },
    /// Progress of an in-flight `connect()` ladder, for UI feedback.
    ConnectProgress {
        node_id: String,
        stage: String,
        detail: Option<String>,
    },
    /// A clipboard value was synced from a peer (active mode).
    Clipboard {
        from: String,
        ip: String,
        protocol: String,
        action: String,
    },
    /// A peer shared an OS notification.
    Notification {
        from: String,
        title: String,
        body: String,
        app: Option<String>,
    },
    /// A peer shared an incoming/missed/ended call notification.
    CallNotification {
        from: String,
        caller: String,
        number: Option<String>,
        /// "ringing" | "missed" | "ended" | …
        state: String,
    },
    /// A peer shared call-history entries (opaque JSON payload).
    CallHistory {
        from: String,
        entries: serde_json::Value,
    },
    /// A producer device returned its shareable app list + which of them this
    /// device is currently subscribed to.
    AppsList {
        from: String,
        apps: serde_json::Value,       // [{ "pkg": ..., "label": ... }]
        subscribed: serde_json::Value, // [ "pkg", ... ]
    },
    /// A device answered `request_status` (`from` is its node id). `status`
    /// holds what its host app reported (`battery`: 0-100, `charging`) plus
    /// `platform`.
    DeviceStatus {
        from: String,
        status: serde_json::Value,
    },
    /// Another of your devices asks this one to ring (find my device).
    Ring { from: String },
    /// Another of your devices asks this one to open a web link.
    OpenUrl { from: String, url: String },
    /// A file transfer moved on (`incoming`: to this device). `node_id` is
    /// the other device; `done` of `total` bytes are confirmed.
    FileProgress {
        id: String,
        node_id: String,
        name: String,
        done: u64,
        total: u64,
        incoming: bool,
    },
    /// A file arrived and was saved at `path`.
    FileReceived {
        id: String,
        from: String,
        name: String,
        path: String,
        size: u64,
    },
    /// The other device confirmed it saved our file.
    FileSent {
        id: String,
        node_id: String,
        name: String,
    },
    /// A transfer stopped for good (`reason` is for the user).
    FileFailed {
        id: String,
        node_id: String,
        name: String,
        incoming: bool,
        reason: String,
    },
}

impl CoreEvent {
    /// The tagged JSON form (`{"kind": ..., ...}`). This is what the Android
    /// app consumes.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Event name the desktop UI listens for (`listen("peers-updated", …)`).
    pub fn name(&self) -> &'static str {
        match self {
            CoreEvent::PeersUpdated { .. } => "peers-updated",
            CoreEvent::MessageReceived(_) => "message-received",
            CoreEvent::MessageDelivered { .. } => "message-delivered",
            CoreEvent::WsConnected { .. } => "ws-connected",
            CoreEvent::WsDisconnected { .. } => "ws-disconnected",
            CoreEvent::PeerConnected { .. } => "peer-connected",
            CoreEvent::PeerDisconnected { .. } => "peer-disconnected",
            CoreEvent::ConnectProgress { .. } => "connect-progress",
            CoreEvent::Clipboard { .. } => "clipboard-event",
            CoreEvent::Notification { .. } => "notification-event",
            CoreEvent::CallNotification { .. } => "call-notification-event",
            CoreEvent::CallHistory { .. } => "call-history-event",
            CoreEvent::AppsList { .. } => "apps-list-event",
            CoreEvent::DeviceStatus { .. } => "device-status-event",
            CoreEvent::Ring { .. } => "ring-event",
            CoreEvent::OpenUrl { .. } => "open-url-event",
            CoreEvent::FileProgress { .. } => "file-progress",
            CoreEvent::FileReceived { .. } => "file-received",
            CoreEvent::FileSent { .. } => "file-sent",
            CoreEvent::FileFailed { .. } => "file-failed",
        }
    }

    /// Payload shape the desktop UI expects for `name()`. Mostly the variant's
    /// fields as an object (no `kind`), except a few events that have always
    /// carried a bare value: the peer list, the message, or a node id string.
    pub fn ui_payload(&self) -> Value {
        match self {
            CoreEvent::PeersUpdated { peers } => serde_json::to_value(peers).unwrap_or(Value::Null),
            CoreEvent::MessageReceived(m) => serde_json::to_value(m).unwrap_or(Value::Null),
            CoreEvent::WsConnected { node_id }
            | CoreEvent::WsDisconnected { node_id }
            | CoreEvent::PeerDisconnected { node_id } => Value::String(node_id.clone()),
            _ => {
                let mut v = self.to_json();
                if let Value::Object(fields) = &mut v {
                    fields.remove("kind");
                }
                v
            }
        }
    }
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, ev: CoreEvent);
}

#[cfg(test)]
mod tests {
    //! These JSON shapes are a contract with the desktop JS (`listen(...)`) and
    //! the Android Kotlin (`o.optString("kind")`). Change them only together
    //! with those apps.
    use super::*;
    use serde_json::json;

    fn notification() -> CoreEvent {
        CoreEvent::Notification {
            from: "phone".into(),
            title: "Hi".into(),
            body: "there".into(),
            app: None,
        }
    }

    #[test]
    fn android_gets_a_kind_tagged_object() {
        assert_eq!(
            notification().to_json(),
            json!({"kind": "notification", "from": "phone", "title": "Hi", "body": "there", "app": null})
        );
        let call = CoreEvent::CallNotification {
            from: "p".into(),
            caller: "Mom".into(),
            number: Some("+1".into()),
            state: "ringing".into(),
        };
        assert_eq!(call.to_json()["kind"], "call_notification");
    }

    #[test]
    fn desktop_gets_the_same_fields_without_kind() {
        assert_eq!(notification().name(), "notification-event");
        assert_eq!(
            notification().ui_payload(),
            json!({"from": "phone", "title": "Hi", "body": "there", "app": null})
        );
    }

    #[test]
    fn desktop_bare_value_events_keep_their_shape() {
        let ev = CoreEvent::WsConnected {
            node_id: "n1".into(),
        };
        assert_eq!((ev.name(), ev.ui_payload()), ("ws-connected", json!("n1")));
        let ev = CoreEvent::PeersUpdated { peers: vec![] };
        assert_eq!((ev.name(), ev.ui_payload()), ("peers-updated", json!([])));
    }

    #[test]
    fn peer_connected_carries_a_lowercase_transport() {
        let ev = CoreEvent::PeerConnected {
            node_id: "n1".into(),
            transport: TransportKind::WebRtc,
        };
        assert_eq!(
            ev.ui_payload(),
            json!({"node_id": "n1", "transport": "webrtc"})
        );
    }
}
