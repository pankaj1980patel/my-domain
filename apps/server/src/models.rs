use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

// Dedicated collection names. The `device_registry` DB is shared with another
// project that uses `users`/`devices` with a different schema, so we namespace
// ours to avoid colliding with its documents and indexes.
pub const USERS_COLL: &str = "md_users";
pub const DEVICES_COLL: &str = "md_devices";

/// A user account. Password is stored only as an Argon2id hash.
#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub username: String,
    pub password_hash: String,
}

/// A device owned by a user. `node_id` is the per-session id the client uses;
/// `ip`/`tcp_port`/`udp_port` are exactly what that client listens on, so peers
/// can connect directly once they look it up.
///
/// The fields below `last_seen` support the signaling / NAT-traversal layer.
/// All are `#[serde(default)]` so documents written by older clients still load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub user_id: String,
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: i32,
    pub udp_port: i32,
    #[serde(default)]
    pub ws_port: i32,
    pub last_seen: i64,

    /// FCM registration token — SERVER SECRET. Used only to relay signals; never
    /// returned in `/devices`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fcm_token: Option<String>,
    /// Global-scope IPv6 address, if the device has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<String>,
    #[serde(default)]
    pub supports_ipv6: bool,
    /// Live: is the device's WebSocket server currently listening & reachable?
    #[serde(default)]
    pub ws_open: bool,
    /// From the firewall check: inbound connections are blocked.
    #[serde(default)]
    pub inbound_blocked: bool,
    /// STUN-observed public mapping (for hole punching).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflexive_udp_port: Option<i32>,
    /// "android" | "desktop" | "ios" | "mac"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}
