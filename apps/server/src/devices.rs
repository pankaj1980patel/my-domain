// Device endpoints (all require a valid JWT). Devices are scoped per user, so a
// user only ever sees / updates their own devices.

use axum::{
    extract::{ConnectInfo, Query, State},
    http::StatusCode,
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::Device;
use crate::state::AppState;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Deserialize)]
pub struct DeviceReq {
    pub node_id: String,
    pub name: String,
    pub ip: Option<String>,
    pub tcp_port: i32,
    pub udp_port: i32,
    #[serde(default)]
    pub ws_port: i32,
    // Signaling / NAT-traversal extras (all optional).
    #[serde(default)]
    pub fcm_token: Option<String>,
    #[serde(default)]
    pub ipv6: Option<String>,
    #[serde(default)]
    pub supports_ipv6: bool,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Serialize)]
pub struct DeviceOut {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: i32,
    pub udp_port: i32,
    pub ws_port: i32,
    pub last_seen: i64,
    // Peer-visible capability/state (NOT fcm_token).
    pub ws_open: bool,
    pub inbound_blocked: bool,
    pub supports_ipv6: bool,
    pub ipv6: Option<String>,
    pub reflexive_ip: Option<String>,
    pub reflexive_udp_port: Option<i32>,
}

/// Register or update (heartbeat / network-change) the caller's device. Uses a
/// `$set` upsert so a heartbeat that omits e.g. `fcm_token` doesn't clobber the
/// value stored by an earlier call.
pub async fn register(
    State(s): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<DeviceReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ip = req
        .ip
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| addr.ip().to_string());

    let mut set = doc! {
        "user_id": &user.user_id,
        "node_id": &req.node_id,
        "name": &req.name,
        "ip": &ip,
        "tcp_port": req.tcp_port,
        "udp_port": req.udp_port,
        "ws_port": req.ws_port,
        "last_seen": now(),
        "supports_ipv6": req.supports_ipv6,
    };
    if let Some(t) = req.fcm_token.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("fcm_token", t);
    }
    if let Some(v) = req.ipv6.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("ipv6", v);
    }
    if let Some(p) = req.platform.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("platform", p);
    }

    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    coll.update_one(
        doc! { "user_id": &user.user_id, "node_id": &req.node_id },
        doc! { "$set": set },
    )
    .upsert(true)
    .await?;
    // Drop stale duplicates of the same physical device (same user + hostname)
    // left behind by older clients that used a fresh random node_id each launch.
    coll.delete_many(doc! {
        "user_id": &user.user_id,
        "name": &req.name,
        "node_id": { "$ne": &req.node_id },
    })
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct PeersQuery {
    pub exclude: Option<String>,
}

/// List the caller's devices (optionally excluding their own node_id).
pub async fn list(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<PeersQuery>,
) -> Result<Json<Vec<DeviceOut>>, AppError> {
    let mut filter = doc! { "user_id": &user.user_id };
    if let Some(ex) = q.exclude.filter(|v| !v.is_empty()) {
        filter.insert("node_id", doc! { "$ne": ex });
    }
    let devices: Vec<Device> = s
        .db
        .collection::<Device>(crate::models::DEVICES_COLL)
        .find(filter)
        .await?
        .try_collect()
        .await?;
    let out = devices
        .into_iter()
        .map(|d| DeviceOut {
            node_id: d.node_id,
            name: d.name,
            ip: d.ip,
            tcp_port: d.tcp_port,
            udp_port: d.udp_port,
            ws_port: d.ws_port,
            last_seen: d.last_seen,
            ws_open: d.ws_open,
            inbound_blocked: d.inbound_blocked,
            supports_ipv6: d.supports_ipv6,
            ipv6: d.ipv6,
            reflexive_ip: d.reflexive_ip,
            reflexive_udp_port: d.reflexive_udp_port,
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct UnregReq {
    pub node_id: String,
}

pub async fn unregister(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<UnregReq>,
) -> Result<StatusCode, AppError> {
    s.db.collection::<Device>(crate::models::DEVICES_COLL)
        .delete_one(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?;
    Ok(StatusCode::OK)
}

/// Partial live-state update (flip `ws_open`, record STUN mapping, etc.) without
/// a full re-register.
#[derive(Deserialize)]
pub struct StateReq {
    pub node_id: String,
    pub ws_open: Option<bool>,
    pub inbound_blocked: Option<bool>,
    pub reflexive_ip: Option<String>,
    pub reflexive_udp_port: Option<i32>,
}

pub async fn state(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<StateReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut set = doc! { "last_seen": now() };
    if let Some(v) = req.ws_open {
        set.insert("ws_open", v);
    }
    if let Some(v) = req.inbound_blocked {
        set.insert("inbound_blocked", v);
    }
    if let Some(v) = req.reflexive_ip.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("reflexive_ip", v);
    }
    if let Some(v) = req.reflexive_udp_port {
        set.insert("reflexive_udp_port", v);
    }
    let res = s
        .db
        .collection::<Device>(crate::models::DEVICES_COLL)
        .update_one(
            doc! { "user_id": &user.user_id, "node_id": &req.node_id },
            doc! { "$set": set },
        )
        .await?;
    if res.matched_count == 0 {
        return Err(AppError::BadRequest("device not found".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Inbound reachability probe: the server dials back to the device's OBSERVED
/// source IP (never a client-claimed address — SSRF guard) on its advertised
/// ports, and records `inbound_blocked`.
#[derive(Deserialize)]
pub struct ProbeReq {
    pub node_id: String,
    #[serde(default)]
    pub check: Vec<String>,
}

#[derive(Serialize)]
pub struct ProbeOut {
    pub tcp_reachable: bool,
    pub ws_reachable: bool,
    pub udp_reachable: bool,
}

async fn probe_tcp(host: IpAddr, port: i32) -> bool {
    if port <= 0 || port > u16::MAX as i32 {
        return false;
    }
    let addr = SocketAddr::new(host, port as u16);
    matches!(
        tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

pub async fn probe(
    State(s): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(req): Json<ProbeReq>,
) -> Result<Json<ProbeOut>, AppError> {
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    let dev = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?
        .ok_or(AppError::BadRequest("device not found".into()))?;

    let host = addr.ip();
    let want = |p: &str| req.check.is_empty() || req.check.iter().any(|c| c == p);
    let tcp_reachable = want("tcp") && probe_tcp(host, dev.tcp_port).await;
    let ws_reachable = want("ws") && dev.ws_port > 0 && probe_tcp(host, dev.ws_port).await;
    // UDP inbound can't be confirmed by a connect; left to STUN/hole-punch.
    let udp_reachable = false;

    let blocked = !(tcp_reachable || ws_reachable);
    let _ = coll
        .update_one(
            doc! { "user_id": &user.user_id, "node_id": &req.node_id },
            doc! { "$set": { "inbound_blocked": blocked, "last_seen": now() } },
        )
        .await;

    Ok(Json(ProbeOut { tcp_reachable, ws_reachable, udp_reachable }))
}
