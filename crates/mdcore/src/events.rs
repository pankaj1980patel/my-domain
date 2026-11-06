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
    /// The clipboard history changed (newest first; see
    /// `Engine::clipboard_history`).
    ClipboardHistory {
        entries: Vec<crate::model::ClipEntry>,
    },
    /// A verification code from another device was put on this device's
    /// clipboard.
    CodeCopied {
        from: String,
        code: String,
        app: Option<String>,
    },
    /// A peer shared an OS notification.
    Notification {
        from: String,
        title: String,
        body: String,
        app: Option<String>,
        /// The notification's key on the phone, when it can be acted on
        /// (`Engine::notification_action`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        /// Its buttons, `[{"title": "Reply", "reply": true}, ...]`, in order;
        /// `reply` ones take text.
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        actions: serde_json::Value,
    },
    /// Another device asks this one to press button `index` of its
    /// notification `key` (with `text` for a reply), or to dismiss it
    /// (`index` -1).
    NotificationAction {
        from: String,
        key: String,
        index: i64,
        text: Option<String>,
    },
    /// A notification `from` shared (with `key`) went away there: swiped,
    /// opened, or cancelled by its app.
    NotificationRemoved { from: String, key: String },
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
    /// Another of your devices asks this one to open a web link, in app
    /// `app` (an Android package) if it's given and installed.
    OpenUrl {
        from: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
    },
    /// Another of your devices asks this one to run action `name` (e.g.
    /// "flashlight" with `{"on": true}`); the host app carries it out.
    Action {
        from: String,
        name: String,
        args: serde_json::Value,
    },
    /// A file transfer moved on (`incoming`: to this device). `node_id` is
    /// the other device; `done` of `total` bytes are confirmed. `resumed`:
    /// it carried on from a partial file (after a dropped link or a
    /// restart), so `done` starts there rather than at 0 (only sent when
    /// true).
    FileProgress {
        id: String,
        node_id: String,
        name: String,
        done: u64,
        total: u64,
        incoming: bool,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        resumed: bool,
    },
    /// A file arrived and was saved at `path`.
    FileReceived {
        id: String,
        from: String,
        name: String,
        path: String,
        size: u64,
        /// A copied picture from the other device's clipboard: put it on
        /// this one's clipboard too.
        clipboard: bool,
        /// A file of a synced folder (`features::folder_sync`), already in
        /// its place there: no notice, and the host leaves it where it is.
        sync: bool,
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
    /// A device answered `list_folder` (request `req`): one page of folder
    /// `path` in its shared folder `root` (`root` "" = the shared folders
    /// themselves). `next` is the offset of the next page, if there is
    /// one; `error` says why there's no listing.
    FolderListing {
        from: String,
        req: String,
        root: String,
        path: String,
        entries: Vec<crate::model::RemoteEntry>,
        next: Option<u64>,
        error: Option<String>,
    },
    /// What's playing on device `from` changed (it pushes this to devices
    /// that asked for its status lately): `media` is `{title, artist?,
    /// album?, app?, state}` or `null` when nothing is playing.
    NowPlaying {
        from: String,
        media: serde_json::Value,
    },
    /// Another of your devices sent text to this one (`Engine::send_text`):
    /// `mode` "clipboard" (already on the clipboard if the core has one
    /// here; not in the core's history if `sensitive`) or "notify" (show it).
    TextShared {
        from: String,
        text: String,
        mode: String,
        /// The sender marked it sensitive (a password copied from a
        /// password manager): put it on the clipboard marked so (Android:
        /// `ClipDescription.EXTRA_IS_SENSITIVE`), keep it out of the
        /// history, and don't show it in a notice.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        sensitive: bool,
    },
    /// Another device wants a page of this device's gallery (`gallery_list`):
    /// the host lists `limit` photos/videos from `offset` (newest first), with
    /// thumbnails if `thumbs`, and answers with `Engine::gallery_reply`.
    GalleryRequest {
        from: String,
        req: String,
        offset: u64,
        limit: u32,
        thumbs: bool,
    },
    /// A device answered `list_gallery` (request `req`): one page of its
    /// photos and videos, newest first. `next` is the offset of the next
    /// page; `error` says why there's none (e.g. photo access not allowed).
    GalleryListing {
        from: String,
        req: String,
        items: Vec<crate::model::GalleryEntry>,
        next: Option<u64>,
        error: Option<String>,
    },
    /// The thumbnail of gallery item `id` (from request `req`): a small JPEG,
    /// base64 (`data:image/jpeg;base64,` + `jpeg` shows it).
    GalleryThumb {
        from: String,
        req: String,
        id: String,
        jpeg: String,
    },
    /// Where this device's photo backup stands (sent when it changes).
    BackupState(crate::model::BackupStatus),
    /// The synced folders, after any of them changed (state, counts, a new
    /// invitation, one removed).
    SyncPairs { pairs: Vec<crate::model::SyncPair> },
    /// Another device's battery is low (`alert` "low"), finished charging
    /// ("full") or reached the level chosen for it ("charged"), at `level`
    /// percent.
    BatteryAlert {
        from: String,
        alert: String,
        level: u8,
    },
    /// Another of your devices (a phone used as a touchpad and keyboard)
    /// sent pointer, click, scroll, text or key events for this computer
    /// to play back, in order. Only sent while remote input is allowed.
    Input {
        from: String,
        events: Vec<crate::model::InputEvent>,
    },
    /// Another device went into (`on`) or out of Do Not Disturb.
    Dnd { from: String, on: bool },
    /// Labels your other devices set changed what devices are called here
    /// (`features::labels`): only the changed ones; a cleared one
    /// (`SharedLabel::is_cleared`) means "forget its label". Apply them to
    /// the app's own labels (that isn't a change to send back).
    LabelsChanged {
        labels: Vec<crate::model::SharedLabel>,
    },
    /// Notes changed by your other devices (`features::notes`): the
    /// changed or new ones (conflict copies included) and the ids of ones
    /// deleted. Saving a note here makes no event.
    NotesChanged {
        notes: Vec<crate::model::Note>,
        removed: Vec<String>,
    },
    /// Timers started or changed on your other devices
    /// (`features::timers`): every timer shown now (a timer missing from
    /// it was cancelled or dismissed). Starting or changing one here makes
    /// no event.
    TimersChanged { timers: Vec<crate::model::Timer> },
    /// The "Read later" list changed on your other devices
    /// (`features::read_later`): the changed or new links and the ids of
    /// ones deleted. Saving here makes no event.
    ReadLaterChanged {
        items: Vec<crate::model::ReadLaterItem>,
        removed: Vec<String>,
    },
    /// A side route for a big transfer (`features::wifi_direct`), `method`
    /// "wifi_direct" or "hotspot": the platform is asked to `op` "consent"
    /// (ask the user to leave this computer's Wi-Fi for the phone's
    /// hotspot; answer `Engine::wifi_direct_consent`), "host" a group (with
    /// the suggested `ssid` and `passphrase`, none for a hotspot; answer
    /// `Engine::wifi_direct_hosted` with what it used), "join" the group
    /// `ssid` with `passphrase` (`host_ip`: the host's address there, when
    /// it said; answer `Engine::wifi_direct_joined` with the host's
    /// address), or "leave" it (and go back to the Wi-Fi it was on). Either
    /// fails with `Engine::wifi_direct_failed`. Carries a passphrase: never
    /// log it.
    WifiDirectRequest {
        sid: String,
        node_id: String,
        method: String,
        op: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        ssid: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        passphrase: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        host_ip: Option<String>,
    },
    /// Transfers with `node_id` go over a side route now (`active`), or no
    /// longer do; `via`: "wifi_direct" or "hotspot".
    WifiDirect {
        node_id: String,
        active: bool,
        via: String,
    },
    /// Device `from` sent its list of links (`features::links_map`): the
    /// map (`Engine::links_map`) may have changed. This device's own link
    /// changes come as `PeerConnected` / `PeerDisconnected`.
    LinksMapChanged { from: String },
    /// How a running media stream with a device stands
    /// (`features::nearby`): whether it's near, on a local low-latency link
    /// (the "Nearby · low latency" badge), and the end-to-end latency now
    /// and before the last move. When it changes, and every couple of
    /// seconds while a stream runs.
    NearbyMedia(crate::features::nearby::NearbyStatus),
    /// Device `node_id` came with another key than the one this device
    /// remembers (reinstalled, data cleared, or someone else using its id)
    /// and its direct connections are refused until the user trusts it
    /// (`Engine::trust_new_key`). Once per new key. Short fingerprints of
    /// the key it had and the one it came with.
    KeyChanged {
        node_id: String,
        name: String,
        old_fingerprint: String,
        new_fingerprint: String,
    },
    /// How a speed test started with `Engine::speed_test` is going
    /// (`features::diag`). `phase`: "ping", "upload", "download" (with
    /// `done` of `total` bytes), then "done" (with `result`), "failed"
    /// (with `error`) or "cancelled".
    SpeedTest {
        node_id: String,
        id: String,
        phase: String,
        done: u64,
        total: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<crate::features::diag::SpeedResult>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Another of your devices started (`on`) or stopped watching this
    /// screen: while any does (`Engine::screen_watchers`), send snapshots
    /// with `Engine::screen_frame`.
    ScreenWatch { from: String, on: bool },
    /// A snapshot of another device's screen you're watching: a JPEG,
    /// base64 (`data:image/jpeg;base64,` + `jpeg` shows it). `seq` counts up.
    ScreenFrame {
        from: String,
        seq: u64,
        jpeg: String,
    },
    /// A phone started (`on`) sharing its screen with this computer, its
    /// owner's choice: watch it (`Engine::screen_watch`) to see it. Or it
    /// stopped (`!on`).
    ScreenOffer { from: String, on: bool },
    /// A computer started (`on`) or stopped asking to see this phone's
    /// camera (`features::camera`): ask the owner, then send its frames
    /// with `Engine::camera_frame_to` while it watches
    /// (`Engine::camera_watchers`). A renewal is not reported.
    CameraWatch { from: String, on: bool },
    /// A picture from a phone's camera this computer asked to see: a JPEG,
    /// base64, as `ScreenFrame`.
    CameraFrame {
        from: String,
        seq: u64,
        jpeg: String,
    },
    /// A phone's camera is on for this computer (`on`; its owner said yes),
    /// with which camera and the torch (`None`: it has none), or it's off
    /// (`reason`: "declined", "stopped", "locked", ...).
    CameraOffer {
        from: String,
        on: bool,
        front: bool,
        torch: Option<bool>,
        reason: Option<String>,
    },
    /// The computer watching this camera asks it to `op`: "switch" the
    /// camera, "torch" (`on`), or take a "photo" (sent as a file).
    CameraControl { from: String, op: String, on: bool },
    /// A device started (`on`) or stopped listening to this device's
    /// audio (`features::audio_share`; a computer to a phone's, or a phone
    /// to a computer's): while any does (`Engine::audio_listeners`),
    /// capture and send with `Engine::audio_frame_to`. A renewal is not
    /// reported; a stop always is (it's also the answer "no" to an offer).
    AudioListen { from: String, on: bool },
    /// A device started (`on`) playing its audio to this one, its owner's
    /// choice (listen with `Engine::audio_listen` to hear it), or it
    /// stopped (`!on`; also its answer when its owner said no). `rate` is
    /// what it captures at (0 when stopping).
    AudioOffer { from: String, on: bool, rate: u32 },
    /// Audio from a device this one listens to: `pcm` is base64 of s16le
    /// samples, `channels` interleaved, at `rate` Hz. `seq` counts up.
    /// Many a second: the hosts play these themselves (the desktop never
    /// hands them to the window, Android keeps them out of its logs).
    AudioFrame {
        from: String,
        seq: u64,
        rate: u32,
        channels: u8,
        pcm: String,
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
            CoreEvent::ClipboardHistory { .. } => "clipboard-history",
            CoreEvent::CodeCopied { .. } => "code-copied",
            CoreEvent::Notification { .. } => "notification-event",
            CoreEvent::NotificationAction { .. } => "notification-action-event",
            CoreEvent::NotificationRemoved { .. } => "notification-removed-event",
            CoreEvent::CallNotification { .. } => "call-notification-event",
            CoreEvent::CallHistory { .. } => "call-history-event",
            CoreEvent::AppsList { .. } => "apps-list-event",
            CoreEvent::DeviceStatus { .. } => "device-status-event",
            CoreEvent::Ring { .. } => "ring-event",
            CoreEvent::OpenUrl { .. } => "open-url-event",
            CoreEvent::Action { .. } => "action-event",
            CoreEvent::FileProgress { .. } => "file-progress",
            CoreEvent::FileReceived { .. } => "file-received",
            CoreEvent::FileSent { .. } => "file-sent",
            CoreEvent::FileFailed { .. } => "file-failed",
            CoreEvent::FolderListing { .. } => "folder-listing",
            CoreEvent::TextShared { .. } => "text-shared",
            CoreEvent::NowPlaying { .. } => "now-playing",
            CoreEvent::GalleryRequest { .. } => "gallery-request",
            CoreEvent::GalleryListing { .. } => "gallery-listing",
            CoreEvent::GalleryThumb { .. } => "gallery-thumb",
            CoreEvent::BackupState(_) => "backup-state",
            CoreEvent::SyncPairs { .. } => "sync-pairs",
            CoreEvent::BatteryAlert { .. } => "battery-alert",
            CoreEvent::Input { .. } => "input-event",
            CoreEvent::Dnd { .. } => "dnd-changed",
            CoreEvent::LabelsChanged { .. } => "labels-changed",
            CoreEvent::NotesChanged { .. } => "notes-changed",
            CoreEvent::TimersChanged { .. } => "timers-changed",
            CoreEvent::ReadLaterChanged { .. } => "read-later-changed",
            CoreEvent::LinksMapChanged { .. } => "links-map-changed",
            CoreEvent::NearbyMedia(_) => "nearby-media",
            CoreEvent::WifiDirectRequest { .. } => "wifi-direct-request",
            CoreEvent::WifiDirect { .. } => "wifi-direct-changed",
            CoreEvent::KeyChanged { .. } => "key-changed",
            CoreEvent::SpeedTest { .. } => "speed-test",
            CoreEvent::ScreenWatch { .. } => "screen-watch",
            CoreEvent::ScreenFrame { .. } => "screen-frame",
            CoreEvent::ScreenOffer { .. } => "screen-offer",
            CoreEvent::CameraWatch { .. } => "camera-watch",
            CoreEvent::CameraFrame { .. } => "camera-frame",
            CoreEvent::CameraOffer { .. } => "camera-offer",
            CoreEvent::CameraControl { .. } => "camera-control",
            CoreEvent::AudioListen { .. } => "audio-listen",
            CoreEvent::AudioOffer { .. } => "audio-offer",
            CoreEvent::AudioFrame { .. } => "audio-frame",
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
            key: None,
            actions: serde_json::Value::Null,
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
    fn notification_removed_keeps_its_shape() {
        let ev = CoreEvent::NotificationRemoved {
            from: "phone".into(),
            key: "0|com.app|1|null|10".into(),
        };
        assert_eq!(ev.to_json()["kind"], "notification_removed");
        assert_eq!(ev.name(), "notification-removed-event");
        assert_eq!(
            ev.ui_payload(),
            json!({"from": "phone", "key": "0|com.app|1|null|10"})
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
    fn input_and_dnd_events_keep_their_shape() {
        use crate::model::{InputEvent, MouseButton};
        let ev = CoreEvent::Input {
            from: "phone".into(),
            events: vec![
                InputEvent::Move { dx: 3, dy: -2 },
                InputEvent::Click {
                    button: MouseButton::Left,
                },
            ],
        };
        assert_eq!(ev.name(), "input-event");
        assert_eq!(
            ev.ui_payload(),
            json!({"from": "phone", "events": [
                {"kind": "move", "dx": 3, "dy": -2},
                {"kind": "click", "button": "left"},
            ]})
        );
        let ev = CoreEvent::Dnd {
            from: "phone".into(),
            on: true,
        };
        assert_eq!(
            (ev.name(), ev.to_json()),
            (
                "dnd-changed",
                json!({"kind": "dnd", "from": "phone", "on": true})
            )
        );
    }

    #[test]
    fn read_later_changes_keep_their_shape() {
        let ev = CoreEvent::ReadLaterChanged {
            items: vec![crate::model::ReadLaterItem {
                id: "u1".into(),
                url: "https://x.io/a".into(),
                title: "x.io".into(),
                host: "x.io".into(),
                read: true,
                saved_at: 5,
                saved_on: "Mac".into(),
                updated_at: 6,
                by: "node".into(),
            }],
            removed: vec!["u2".into()],
        };
        assert_eq!(ev.name(), "read-later-changed");
        assert_eq!(
            ev.to_json(),
            json!({"kind": "read_later_changed", "items": [{
                "id": "u1", "url": "https://x.io/a", "title": "x.io", "host": "x.io",
                "read": true, "saved_at": 5, "saved_on": "Mac", "updated_at": 6, "by": "node",
            }], "removed": ["u2"]})
        );
    }

    #[test]
    fn wifi_direct_events_keep_their_shape() {
        let ev = CoreEvent::WifiDirectRequest {
            sid: "s1".into(),
            node_id: "n1".into(),
            method: "hotspot".into(),
            op: "join".into(),
            ssid: Some("DIRECT-md-x".into()),
            passphrase: Some("secret123".into()),
            host_ip: None,
        };
        assert_eq!(ev.name(), "wifi-direct-request");
        assert_eq!(
            ev.to_json(),
            json!({"kind": "wifi_direct_request", "sid": "s1", "node_id": "n1",
                   "method": "hotspot", "op": "join", "ssid": "DIRECT-md-x",
                   "passphrase": "secret123"})
        );
        let ev = CoreEvent::WifiDirect {
            node_id: "n1".into(),
            active: true,
            via: "hotspot".into(),
        };
        assert_eq!(ev.name(), "wifi-direct-changed");
        assert_eq!(
            ev.ui_payload(),
            json!({"node_id": "n1", "active": true, "via": "hotspot"})
        );
    }

    #[test]
    fn links_map_changes_keep_their_shape() {
        let ev = CoreEvent::LinksMapChanged { from: "n1".into() };
        assert_eq!(ev.name(), "links-map-changed");
        assert_eq!(
            ev.to_json(),
            json!({"kind": "links_map_changed", "from": "n1"})
        );
        assert_eq!(ev.ui_payload(), json!({"from": "n1"}));
    }

    #[test]
    fn nearby_media_keeps_its_shape() {
        let ev = CoreEvent::NearbyMedia(crate::features::nearby::NearbyStatus {
            node_id: "n1".into(),
            media: true,
            near: Some("lan".into()),
            low_latency: true,
            path: "lan".into(),
            latency_ms: Some(12),
            before_ms: Some(140),
            rtt_ms: Some(3),
        });
        assert_eq!(ev.name(), "nearby-media");
        assert_eq!(
            ev.to_json(),
            json!({"kind": "nearby_media", "node_id": "n1", "media": true, "near": "lan",
                "low_latency": true, "path": "lan", "latency_ms": 12, "before_ms": 140, "rtt_ms": 3})
        );
        assert_eq!(ev.ui_payload()["low_latency"], json!(true));
    }

    #[test]
    fn camera_events_keep_their_shape() {
        let ev = CoreEvent::CameraOffer {
            from: "phone".into(),
            on: true,
            front: false,
            torch: None,
            reason: None,
        };
        assert_eq!(
            (ev.name(), ev.ui_payload()),
            (
                "camera-offer",
                json!({"from": "phone", "on": true, "front": false, "torch": null, "reason": null})
            )
        );
        let ev = CoreEvent::CameraControl {
            from: "pc".into(),
            op: "torch".into(),
            on: true,
        };
        // Android reads the tag.
        assert_eq!(
            ev.to_json(),
            json!({"kind": "camera_control", "from": "pc", "op": "torch", "on": true})
        );
        let ev = CoreEvent::CameraWatch {
            from: "pc".into(),
            on: true,
        };
        assert_eq!(ev.to_json()["kind"], "camera_watch");
    }

    #[test]
    fn screen_offers_and_phone_touches_keep_their_shape() {
        use crate::model::InputEvent;
        let ev = CoreEvent::ScreenOffer {
            from: "phone".into(),
            on: true,
        };
        assert_eq!(
            (ev.name(), ev.ui_payload()),
            ("screen-offer", json!({"from": "phone", "on": true}))
        );
        // Android reads "screen_offer" (it never gets one, but the tag is
        // what NetService would match).
        assert_eq!(ev.to_json()["kind"], "screen_offer");
        let ev = CoreEvent::Input {
            from: "pc".into(),
            events: vec![
                InputEvent::Tap { x: 5000, y: 100 },
                InputEvent::Swipe {
                    x: 1,
                    y: 2,
                    x2: 3,
                    y2: 4,
                    ms: 300,
                },
            ],
        };
        assert_eq!(
            ev.to_json(),
            json!({"kind": "input", "from": "pc", "events": [
                {"kind": "tap", "x": 5000, "y": 100},
                {"kind": "swipe", "x": 1, "y": 2, "x2": 3, "y2": 4, "ms": 300},
            ]})
        );
    }

    #[test]
    fn phone_audio_events_keep_their_shape() {
        // Android matches "audio_listen"; the desktop listens to
        // "audio-offer".
        let ev = CoreEvent::AudioListen {
            from: "pc".into(),
            on: true,
        };
        assert_eq!(
            ev.to_json(),
            json!({"kind": "audio_listen", "from": "pc", "on": true})
        );
        let ev = CoreEvent::AudioOffer {
            from: "phone".into(),
            on: false,
            rate: 0,
        };
        assert_eq!(
            (ev.name(), ev.ui_payload()),
            (
                "audio-offer",
                json!({"from": "phone", "on": false, "rate": 0})
            )
        );
    }

    #[test]
    fn key_changes_keep_their_shape() {
        let ev = CoreEvent::KeyChanged {
            node_id: "n1".into(),
            name: "Pixel".into(),
            old_fingerprint: "AAAA BBBB CCCC DDDD".into(),
            new_fingerprint: "1111 2222 3333 4444".into(),
        };
        let fields = json!({
            "node_id": "n1",
            "name": "Pixel",
            "old_fingerprint": "AAAA BBBB CCCC DDDD",
            "new_fingerprint": "1111 2222 3333 4444",
        });
        assert_eq!((ev.name(), ev.ui_payload()), ("key-changed", fields));
        // Android matches "key_changed".
        assert_eq!(ev.to_json()["kind"], "key_changed");
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
