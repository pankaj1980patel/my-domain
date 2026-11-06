//! Wire protocol + shared data model. These structs are serialized to JSON and
//! must stay byte-compatible across desktop, android, and the registry server.

use serde::{Deserialize, Serialize};

/// This device's own endpoints, advertised to peers and the registry.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Identity {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port. 0 = no QUIC (older builds).
    #[serde(default)]
    pub quic_port: u16,
    /// Global IPv6 address, if this device has one. Often directly reachable
    /// (many mobile networks give phones public IPv6 with no NAT).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<String>,
    /// Public `ip:port` the home router forwards to our QUIC port (UPnP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_quic: Option<String>,
}

/// LAN discovery beacon (UDP datagram body).
#[derive(Serialize, Deserialize)]
pub struct Beacon {
    pub node_id: String,
    pub name: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port; 0 when the peer doesn\'t speak QUIC yet.
    #[serde(default)]
    pub quic_port: u16,
    #[serde(default)]
    pub reply: bool,
    /// `crypto::lan_tag` of the sender's key: beacons with another tag (or
    /// none) are ignored.
    #[serde(default)]
    pub tag: String,
}

/// Where a peer entry came from. Serializes to "registry" | "lan" | "manual"
/// to stay compatible with the existing frontend / Kotlin JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum Source {
    Registry,
    Lan,
    Manual,
}

/// A known peer device. Deserializable too: the peer list is cached on the
/// device (see engine/background.rs `spawn_peer_cache`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub ws_port: u16,
    /// QUIC endpoint (UDP) port; 0 when the peer doesn\'t speak QUIC yet.
    #[serde(default)]
    pub quic_port: u16,
    pub source: Source,
    /// Peer-reported reachability (from the registry; `false`/`None` for
    /// LAN/manual peers we discovered ourselves). Drives the connect ladder.
    #[serde(default)]
    pub ws_open: bool,
    #[serde(default)]
    pub inbound_blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_udp_port: Option<u16>,
    /// The peer's NAT type from the registry (`Unknown` for LAN/manual peers).
    #[serde(default)]
    pub nat: crate::nat::NatKind,
    /// The peer's global IPv6 address (from the registry), dialed in parallel
    /// with `ip`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<String>,
    /// Public `ip:port` the peer's router forwards to its QUIC port (UPnP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapped_quic: Option<String>,
}

impl Peer {
    /// Where to dial this peer's QUIC endpoint, all raced in parallel: its
    /// IPv6 address (no NAT in the way, when it exists), the port its router
    /// forwards for it (UPnP), then its LAN/registry IPv4 address.
    /// Unparseable or duplicate entries are skipped.
    pub fn quic_targets(&self) -> Vec<std::net::SocketAddr> {
        use std::net::{IpAddr, SocketAddr};
        let with_port = |s: &str| {
            s.trim()
                .parse::<IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, self.quic_port))
        };
        let candidates = [
            self.ipv6.as_deref().and_then(with_port),
            self.mapped_quic
                .as_deref()
                .and_then(|s| s.trim().parse().ok()),
            with_port(&self.ip),
        ];
        let mut out: Vec<SocketAddr> = Vec::new();
        for a in candidates.into_iter().flatten() {
            if a.port() != 0 && !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }
}

/// Decrypted message body.
#[derive(Default, Serialize, Deserialize)]
pub struct Plaintext {
    pub from: String,
    pub text: String,
    /// A chat message's id. The receiver acknowledges it (`ack`) and drops
    /// repeats, so the sender can resend until it hears back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Set on an acknowledgement: the id of the message that arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack: Option<String>,
}

/// Encrypted on-the-wire envelope (TCP/UDP body, and WS `msg` frames).
#[derive(Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub nonce: String,
    pub ciphertext: String,
}

/// A message surfaced to the UI.
#[derive(Clone, Debug, Serialize)]
pub struct IncomingMessage {
    /// The sender's display name (its account username).
    pub from: String,
    /// The sending device's node id, when the link knows it (it always does
    /// for QUIC, WebRTC and the relay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub ip: String,
    pub protocol: String,
    pub text: String,
    pub ts: u64,
    pub ok: bool,
    /// The message's id, when the sender gave it one (chat does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// WebSocket frames (legacy protocol v1; v2 framing lives in `wire::frame`).
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    #[serde(rename = "hello")]
    Hello {
        node_id: String,
        name: String,
        /// New every time the engine starts (QUIC only): a peer that sees
        /// a new one knows we restarted and drops our old connection.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        boot: String,
        /// The binary media frame version this end reads (over QUIC, or the
        /// relay when its server forwards binary frames; see
        /// `transport::media`), 0 for none. Older apps leave it out (and
        /// ignore it), so they keep getting media as JSON messages.
        #[serde(default, skip_serializing_if = "is_zero")]
        bin_media: u8,
        /// QUIC only: this connection is a side lane for file transfers
        /// (`"wifi_direct"`, see `features::wifi_direct`), not the peer's
        /// link. Older apps leave it out; they're never dialed for one.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        lane: String,
    },
    #[serde(rename = "msg")]
    Msg { nonce: String, ciphertext: String },
    /// Relay only: the sender closed its relayed link to us on purpose (idle
    /// in the background, or Disconnect). Nothing tells us otherwise: the
    /// relay socket stays up on our side, so without this we'd stay "live"
    /// on a route that leads nowhere and every send would vanish into it.
    #[serde(rename = "bye")]
    Bye,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn peer(ip: &str, ipv6: Option<&str>) -> Peer {
        Peer {
            node_id: "n".into(),
            name: "n".into(),
            ip: ip.into(),
            tcp_port: 1,
            udp_port: 2,
            ws_port: 3,
            quic_port: 4,
            source: Source::Registry,
            ws_open: false,
            inbound_blocked: false,
            reflexive_ip: None,
            reflexive_udp_port: None,
            nat: Default::default(),
            ipv6: ipv6.map(String::from),
            mapped_quic: None,
        }
    }

    fn addrs(v: &[&str]) -> Vec<SocketAddr> {
        v.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn quic_targets_try_ipv6_then_the_router_mapping_then_ipv4() {
        let mut p = peer("192.168.1.5", Some("2409:4040::7"));
        assert_eq!(
            p.quic_targets(),
            addrs(&["[2409:4040::7]:4", "192.168.1.5:4"])
        );
        p.mapped_quic = Some("152.67.7.50:41000".into());
        assert_eq!(
            p.quic_targets(),
            addrs(&["[2409:4040::7]:4", "152.67.7.50:41000", "192.168.1.5:4"])
        );
        assert_eq!(
            peer("192.168.1.5", None).quic_targets(),
            addrs(&["192.168.1.5:4"])
        );
    }

    #[test]
    fn quic_targets_skip_garbage_and_duplicates() {
        assert_eq!(
            peer("not-an-ip", Some(" ::1 ")).quic_targets(),
            addrs(&["[::1]:4"])
        );
        assert_eq!(peer("::1", Some("::1")).quic_targets(), addrs(&["[::1]:4"]));
        let mut p = peer("", None);
        p.mapped_quic = Some("garbage".into());
        assert!(p.quic_targets().is_empty());
        let mut no_quic = peer("10.0.0.1", None);
        no_quic.quic_port = 0;
        assert!(
            no_quic.quic_targets().is_empty(),
            "no QUIC port, nothing to dial"
        );
    }

    #[test]
    fn hello_says_bin_media_only_when_set_and_old_hellos_still_parse() {
        let hello = |bin_media| WsFrame::Hello {
            node_id: "n".into(),
            name: "x".into(),
            boot: String::new(),
            bin_media,
            lane: String::new(),
        };
        let old = serde_json::to_string(&hello(0)).unwrap();
        assert!(!old.contains("bin_media"), "{old}");
        let new = serde_json::to_string(&hello(1)).unwrap();
        assert!(new.contains("\"bin_media\":1"), "{new}");
        // An older app's hello (no flag): it reads none.
        let parsed: WsFrame =
            serde_json::from_str(r#"{"type":"hello","node_id":"n","name":"x"}"#).unwrap();
        assert!(matches!(parsed, WsFrame::Hello { bin_media: 0, .. }));
        // A hello with fields this build doesn't know still parses, as ours
        // does for an older app.
        let parsed: WsFrame = serde_json::from_str(
            r#"{"type":"hello","node_id":"n","name":"x","bin_media":2,"later":true}"#,
        )
        .unwrap();
        assert!(matches!(parsed, WsFrame::Hello { bin_media: 2, .. }));
    }
}

/// One clipboard value in the history: what it was, which device it came
/// from (`None`: copied on this one) and when (unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub ts: u64,
}

/// A folder this device lets your other devices browse and download from
/// (`Engine::set_shared_folders`): shown to them as `name`, read from `path`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct SharedFolder {
    pub name: String,
    pub path: String,
}

/// One item in a folder listing from another device (`FolderListing`):
/// a file (`size` in bytes) or a folder (`dir`), last changed at
/// `modified` (unix seconds, 0 if unknown).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    pub modified: u64,
}

/// One photo or video in another device's gallery (`GalleryListing`),
/// newest first. `id` names it for `Engine::download_gallery_item`;
/// `taken` is unix seconds (0 if unknown), `duration` milliseconds (videos).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GalleryEntry {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub taken: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub video: bool,
}

/// What the host app hands the core for one gallery item
/// (`Engine::gallery_reply`): the entry other devices see, where the file
/// is on this device (never sent), and a small JPEG thumbnail.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GalleryItem {
    pub entry: GalleryEntry,
    pub path: String,
    pub thumb: Option<Vec<u8>>,
}

/// A new photo or video for the backup (`Engine::backup_enqueue`). `key`
/// names it for good (never sent twice); `taken` is unix seconds; `month`
/// ("YYYY-MM", local time) picks the folder it lands in, else it's worked
/// out from `taken` (UTC).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupItem {
    pub key: String,
    pub path: String,
    pub taken: u64,
    #[serde(default)]
    pub month: Option<String>,
}

/// What your devices call one of them, as shared between them
/// (`features::labels`): `Engine::shared_labels`, and the `LabelsChanged`
/// event. A label with no nickname, icon or colour is a cleared one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedLabel {
    /// The device it names (node id).
    pub node: String,
    /// Shown instead of its own name; none: its own name.
    pub nickname: Option<String>,
    /// An icon name (the apps' fixed set); none: the initial of its name.
    pub icon: Option<String>,
    /// An accent colour name (the apps' fixed set); none: the default.
    pub color: Option<String>,
    /// When it was set (unix ms) ...
    pub updated_at: u64,
    /// ... and on which device (node id): the newest wins everywhere.
    pub by: String,
}

impl SharedLabel {
    /// A cleared label: nothing set.
    pub fn is_cleared(&self) -> bool {
        self.nickname.is_none() && self.icon.is_none() && self.color.is_none()
    }
}

/// A note in the notepad shared between your devices (`features::notes`):
/// `Engine::notes`, and the `NotesChanged` event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub title: String,
    /// Plain text, up to 64 KB.
    pub body: String,
    /// Shown first.
    pub pinned: bool,
    /// Draw lines starting with `[ ] ` / `[x] ` as checkboxes (the text
    /// stays plain).
    pub checklist: bool,
    /// An accent colour name (the apps' fixed set); none: the default.
    pub color: Option<String>,
    /// Unix ms.
    pub created_at: u64,
    /// Unix ms, when it last changed ...
    pub updated_at: u64,
    /// ... and on which device (node id) ...
    pub by: String,
    /// ... called what then.
    pub by_name: String,
    /// A conflict copy: the note it split from, when it was changed on two
    /// devices at once and this text lost (its title says so).
    pub conflict_of: Option<String>,
    /// A reminder: when (unix ms). Every device with the note shows it
    /// then (the apps schedule it).
    #[serde(default)]
    pub remind_at: Option<u64>,
    /// The reminder was marked Done (on any device): it shows no more.
    #[serde(default)]
    pub remind_done: bool,
}

/// A countdown timer shared between your devices (`features::timers`):
/// `Engine::timers`, and the `TimersChanged` event. Every device that has
/// it counts down from `ends_at` itself; the devices in `ring_on` ring when
/// it ends.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub id: String,
    /// What it's for ("Pasta"); may be blank.
    #[serde(default)]
    pub label: String,
    /// How long it was set for, in ms.
    pub duration_ms: u64,
    /// "running" or "paused" (and, between devices only, "done" once
    /// dismissed and "cancelled": the apps never get those).
    pub state: String,
    /// Running: when it ends (unix ms; past: it has ended and rings until
    /// dismissed). Paused: 0.
    #[serde(default)]
    pub ends_at: u64,
    /// Paused: what's left, in ms. Running: 0.
    #[serde(default)]
    pub remaining_ms: u64,
    /// The devices (node ids) that ring when it ends.
    #[serde(default)]
    pub ring_on: Vec<String>,
    /// Unix ms.
    #[serde(default)]
    pub created_at: u64,
    /// Unix ms, when it last changed ...
    pub updated_at: u64,
    /// ... on which device (node id) ...
    pub by: String,
    /// ... called what then.
    #[serde(default)]
    pub by_name: String,
}

/// A link in the "Read later" list shared between your devices
/// (`features::read_later`): `Engine::read_later`, and the
/// `ReadLaterChanged` event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadLaterItem {
    /// Made from the link, the same on every device.
    pub id: String,
    /// An `http` or `https` link.
    pub url: String,
    /// The title the share gave, else the link's host (nothing is fetched).
    pub title: String,
    /// The link's host, for showing (`example.com`).
    pub host: String,
    /// Marked read (on any device).
    pub read: bool,
    /// Unix ms, when it was (last) saved: the list's order.
    pub saved_at: u64,
    /// The name of the device it was saved on.
    pub saved_on: String,
    /// Unix ms, when it last changed ...
    pub updated_at: u64,
    /// ... and on which device (node id).
    pub by: String,
}

/// A link downloaded on one of your devices at another's request
/// (`features::remote_download`): `Engine::remote_downloads`, and the
/// `RemoteDownload` event whenever its state changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDownloadItem {
    /// Also its transfer id (the File* events and `Engine::cancel_file`).
    pub id: String,
    /// The other device: the one that asked (`here`), or the one that
    /// downloads it.
    pub node_id: String,
    /// This device downloads it; false: another device does, asked from
    /// here.
    pub here: bool,
    pub url: String,
    /// The link's host, for showing (`example.com`).
    pub host: String,
    /// The file's name (a guess from the link until the server says).
    pub name: String,
    /// Bytes, 0 while not known.
    pub size: u64,
    /// Bytes downloaded so far.
    pub done: u64,
    /// "requested" (sent, no answer yet), "checking", "asking" (waiting
    /// for the user on the downloading device), "queued", "downloading",
    /// then "done", "failed", "cancelled" or "refused".
    pub state: String,
    /// Why it failed or was refused (for the user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Here, once done: where the file was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Unix ms.
    pub started_at: u64,
    pub updated_at: u64,
}

impl RemoteDownloadItem {
    /// It ended (done, failed, cancelled or refused).
    pub fn finished(&self) -> bool {
        matches!(
            self.state.as_str(),
            "done" | "failed" | "cancelled" | "refused"
        )
    }
}

/// Several files sent together, as one (`features::batches`): a folder
/// with everything in it, or files picked at once. `Engine::transfer_batches`
/// and the `TransferBatch` event whenever it moves on. Its files are
/// transfers of their own (`File*` events, with `batch` set to `id`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchItem {
    pub id: String,
    /// The other device.
    pub node_id: String,
    /// To this device.
    pub incoming: bool,
    /// The folder's name, or the first file's name.
    pub name: String,
    /// "folder" or "files".
    pub kind: String,
    /// Files in it.
    pub count: u64,
    /// Files saved there so far, and those that failed.
    pub files_done: u64,
    pub files_failed: u64,
    /// Bytes confirmed so far, of `total`.
    pub done: u64,
    pub total: u64,
    /// "active", "paused", then "done", "failed" (some files didn't go)
    /// or "cancelled".
    pub state: String,
    /// Why it stopped (for the user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Received folder: where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Unix ms.
    pub started_at: u64,
}

impl BatchItem {
    /// It ended (done, failed or cancelled).
    pub fn finished(&self) -> bool {
        matches!(self.state.as_str(), "done" | "failed" | "cancelled")
    }
}

/// What `Engine::send_batch` started.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchStart {
    /// The batch's id (`Engine::cancel_file`, `pause_file` and
    /// `retry_file` take it too).
    pub id: String,
    /// Files in it.
    pub count: u64,
}

/// What this device does when another of yours asks it to download a link
/// (`features::remote_download`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDownloadSettings {
    /// "ask" (the first time for each device), "always" or "never".
    pub policy: String,
    /// Links to this device or the local network are allowed too.
    #[serde(default)]
    pub allow_lan: bool,
    /// The biggest download, in MB (0: no limit).
    #[serde(default)]
    pub max_mb: u64,
    /// Devices (node ids) allowed without asking.
    #[serde(default)]
    pub allowed: Vec<String>,
}

impl Default for RemoteDownloadSettings {
    fn default() -> Self {
        RemoteDownloadSettings {
            policy: "ask".into(),
            allow_lan: false,
            max_mb: 4096,
            allowed: Vec::new(),
        }
    }
}

/// A synced folder: one folder kept the same on this device and another
/// (`features::folder_sync`). `Engine::sync_pairs`, and the `SyncPairs`
/// event whenever any of them changes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncPair {
    /// The pair's id: the same on both devices.
    pub id: String,
    /// The other device (node id).
    pub peer: String,
    /// The folder's name (the creator's folder name).
    pub name: String,
    /// The folder on this device ("" while an invitation waits for an
    /// answer here).
    pub path: String,
    /// This device's part: "both", "send" (only sends its changes) or
    /// "receive" (only takes the other's).
    pub mode: String,
    pub paused: bool,
    /// "invited" (the other device asks; accept or decline here),
    /// "pending" (waiting for the other device to accept), "paused",
    /// "error" (see `error`), "syncing", "in_sync", or "waiting" (not
    /// compared with the other device yet: no link).
    pub state: String,
    /// Files on their way, either way (or still different).
    pub syncing: u32,
    /// Files in the folder that are synced.
    pub files: u32,
    /// Conflict copies in the folder ("name (conflict from ...)").
    pub conflicts: u32,
    /// Files left out: bigger than the size limit, or unreadable.
    pub skipped: u32,
    /// The other device's files kept here under another name, because
    /// this device's disk can't tell their names from another file's
    /// (they differ only by letter case: "Notes.txt" and "notes.txt").
    pub renamed: u32,
    /// When both sides last held the same (unix seconds, 0 = never).
    pub last_synced: u64,
    pub error: Option<String>,
}

/// Where the photo backup stands (`Engine::backup_state`, and the
/// `BackupState` event whenever it changes).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupStatus {
    pub enabled: bool,
    /// The computer that gets the photos ("" = none chosen).
    pub target: String,
    /// Only while the phone is charging (it always waits for unmetered
    /// Wi-Fi).
    pub charging_only: bool,
    /// Photos added from this time on (unix seconds) are backed up.
    pub since: u64,
    /// Waiting to be sent.
    pub queued: u32,
    /// Sent so far, all time.
    pub sent: u32,
    /// What it waits for: "off", "target", "battery" (held back by the
    /// phone's battery saver), "wifi", "charging", "retry" (the last try
    /// failed; tries again soon), or "" (sending, or all done).
    pub waiting_for: String,
    /// The file going now, with bytes confirmed of its size.
    pub current: Option<String>,
    pub done: u64,
    pub total: u64,
    /// The last failure, for the user.
    pub last_error: Option<String>,
    /// When the last file was sent (unix seconds, 0 = never).
    pub last_sent_at: u64,
}

/// Battery alerts this device sends your other devices
/// (`Engine::set_battery_alerts`): once when it drops to `low` percent
/// while not charging, once per charge when charging reaches `charged`
/// percent (0 = off: then once when it reaches 100%).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct BatteryAlertSettings {
    pub enabled: bool,
    pub low: u8,
    /// Tell the others when charging reaches this level (50-100; the apps
    /// offer 80, 90 and 100), or 0 for off. Missing in settings saved
    /// before it existed, so it defaults to off.
    #[serde(default)]
    pub charged: u8,
}

impl Default for BatteryAlertSettings {
    fn default() -> Self {
        BatteryAlertSettings {
            enabled: true,
            low: 15,
            charged: 0,
        }
    }
}

/// A mouse button the phone's touchpad can click.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// A key the phone's keyboard can press (typed text goes as `Text`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum InputKey {
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Esc,
}

/// Which way a multi-finger swipe went (the way the fingers moved).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SwipeDir {
    Left,
    Right,
    Up,
    Down,
}

/// One thing the phone's touchpad or keyboard did, for a computer to play
/// back (`features::input`). Moves and scrolls are relative, in pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    Move {
        dx: i32,
        dy: i32,
    },
    Click {
        button: MouseButton,
    },
    Scroll {
        dx: i32,
        dy: i32,
    },
    Text {
        text: String,
    },
    Key {
        key: InputKey,
    },
    /// A three- or four-finger swipe; the computer turns it into its own
    /// shortcut (switch apps, switch desktops, show all windows, show the
    /// desktop).
    Gesture {
        fingers: u8,
        dir: SwipeDir,
    },
    /// Put the pointer at a spot on the main screen, in ten-thousandths of
    /// its width (`x`) and height (`y`) from the top left: where you
    /// tapped on the phone's picture of the screen.
    Point {
        x: u16,
        y: u16,
    },
    /// A tap on a phone's screen, at a spot in ten-thousandths of its width
    /// and height (as `Point`): from a computer showing the phone's screen.
    /// The phone plays it with its Accessibility helper, only while it
    /// shares its screen with that computer and its owner allows control.
    /// Older devices drop it (an event kind they don't know).
    Tap {
        x: u16,
        y: u16,
    },
    /// A drag on a phone's screen from (`x`, `y`) to (`x2`, `y2`) over `ms`
    /// milliseconds (same spots as `Tap`); the same spot held long is a
    /// long press. Played like `Tap`.
    Swipe {
        x: u16,
        y: u16,
        x2: u16,
        y2: u16,
        ms: u16,
    },
}

impl MouseButton {
    /// "left", "right" or "middle".
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

impl SwipeDir {
    /// "left", "right", "up" or "down".
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

impl InputKey {
    /// "enter", "backspace", "left", "right", "up", "down" or "esc".
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

/// What waking a sleeping computer did (`Engine::wake_device`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct WakeReport {
    /// Where this device sent the magic packet (`ip:port`, broadcasts).
    pub sent: Vec<String>,
    /// The other devices of yours on the computer's network asked to send
    /// it too (node ids).
    pub relays: Vec<String>,
}

/// One thing a computer's mouse or keyboard did, for the computer it
/// controls to play back (`features::kvm`, "Share mouse and keyboard").
/// Moves are relative, in pixels; wheel turns in steps (`dy` positive
/// scrolls down, `dx` positive right); keys are USB HID usage codes
/// (keyboard page), the same on every OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KvmEvent {
    Move {
        dx: i32,
        dy: i32,
    },
    /// `button` 1 left, 2 right, 3 middle, 4 back, 5 forward.
    Button {
        button: u8,
        down: bool,
    },
    Wheel {
        dx: i32,
        dy: i32,
    },
    Key {
        code: u16,
        down: bool,
    },
}

/// Who this computer's mouse and keyboard are with (`features::kvm`):
/// `Engine::kvm_state` and the `KvmChanged` event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KvmState {
    /// "idle", "controlling" (this computer's mouse and keyboard drive
    /// `node_id`) or "controlled" (`node_id`'s drive this one).
    pub role: String,
    #[serde(default)]
    pub node_id: Option<String>,
    /// The other computer's name.
    #[serde(default)]
    pub name: Option<String>,
    /// Controlled: the edge of this screen the pointer came in at (and
    /// leaves by). Controlling: the edge of this screen it went out at.
    /// Idle after a return: the edge of this screen it comes back at.
    /// "left", "right", "top" or "bottom".
    #[serde(default)]
    pub edge: Option<String>,
    /// Where along `edge`, in ten-thousandths of its length from the top
    /// or left.
    #[serde(default)]
    pub pos: Option<u16>,
    /// Why it became idle: "returned" (the pointer came back), "stopped"
    /// (the hotkey or Stop), "link_lost", "timeout", "off" (that computer
    /// doesn't allow being controlled), "busy", "ended".
    #[serde(default)]
    pub reason: Option<String>,
}

/// A computer placed next to this one for "Share mouse and keyboard":
/// `side` is "left", "right", "top" or "bottom" of this screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct KvmNeighbor {
    pub node_id: String,
    pub side: String,
}
