// Device endpoints (all require a valid JWT). Devices are scoped per user, so a
// user only ever sees / updates their own devices.

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::models::{Device, DeviceKind};
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
    #[serde(default)]
    pub quic_port: i32,
    // Signaling / NAT-traversal extras (all optional).
    #[serde(default)]
    pub fcm_token: Option<String>,
    #[serde(default)]
    pub ipv6: Option<String>,
    #[serde(default)]
    pub supports_ipv6: bool,
    #[serde(default)]
    pub platform: Option<String>,
    /// Client-measured NAT type: "easy" | "symmetric" | "unknown".
    #[serde(default)]
    pub nat: Option<String>,
    /// Public `ip:port` the device's router forwards to its QUIC port (UPnP).
    #[serde(default)]
    pub mapped_quic: Option<String>,
    /// What kind of device it is (for its icon); absent from older apps.
    #[serde(default)]
    pub kind: Option<DeviceKind>,
}

// Field caps (security review M20). A registration with a field over its
// cap is refused (400, naming the field); a body over `MAX_BODY` gets 413.
// They're far above what the apps send, which is a few dozen bytes each.
/// Largest request body on the device routes (`/devices/*`).
pub const MAX_BODY: usize = 16 * 1024;
/// Node ids (the relay takes the same).
pub const MAX_NODE_ID: usize = 128;
/// Device names (host names, "Pankaj's Pixel 8").
pub const MAX_NAME: usize = 128;
/// Any address text: an IPv4/IPv6 address, `ip:port`.
pub const MAX_ADDR: usize = 64;
/// FCM registration tokens are about 160-200 characters.
pub const MAX_FCM_TOKEN: usize = 4096;
/// `platform` and `nat`.
pub const MAX_SHORT: usize = 32;
/// Each `kind` field as sent (it's cut to 60 characters when stored).
pub const MAX_KIND_FIELD: usize = 256;
/// Entries in a probe's `check` list, and the length of each.
pub const MAX_CHECKS: usize = 8;
/// Devices one account may register (`MAX_DEVICES_PER_ACCOUNT`).
pub const MAX_DEVICES: u64 = 32;

fn field_ok(v: &str, max: usize) -> bool {
    v.chars().count() <= max && !v.chars().any(char::is_control)
}

fn check_field(name: &str, v: &str, max: usize) -> Result<(), AppError> {
    if field_ok(v, max) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "{name} must be at most {max} characters, without control characters"
        )))
    }
}

fn check_opt(name: &str, v: Option<&String>, max: usize) -> Result<(), AppError> {
    v.map_or(Ok(()), |v| check_field(name, v, max))
}

fn check_node_id(v: &str) -> Result<(), AppError> {
    if v.trim().is_empty() {
        return Err(AppError::BadRequest("node_id is required".into()));
    }
    check_field("node_id", v, MAX_NODE_ID)
}

fn check_port(name: &str, p: i32) -> Result<(), AppError> {
    if (0..=i32::from(u16::MAX)).contains(&p) {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("{name} must be 0-65535")))
    }
}

/// Every field of a registration within its cap.
fn validate(req: &DeviceReq) -> Result<(), AppError> {
    check_node_id(&req.node_id)?;
    check_field("name", &req.name, MAX_NAME)?;
    check_opt("ip", req.ip.as_ref(), MAX_ADDR)?;
    check_opt("ipv6", req.ipv6.as_ref(), MAX_ADDR)?;
    check_opt("mapped_quic", req.mapped_quic.as_ref(), MAX_ADDR)?;
    check_opt("fcm_token", req.fcm_token.as_ref(), MAX_FCM_TOKEN)?;
    check_opt("platform", req.platform.as_ref(), MAX_SHORT)?;
    check_opt("nat", req.nat.as_ref(), MAX_SHORT)?;
    for (n, p) in [
        ("tcp_port", req.tcp_port),
        ("udp_port", req.udp_port),
        ("ws_port", req.ws_port),
        ("quic_port", req.quic_port),
    ] {
        check_port(n, p)?;
    }
    if let Some(k) = &req.kind {
        for (n, v) in [
            ("kind.os", &k.os),
            ("kind.form", &k.form),
            ("kind.model", &k.model),
            ("kind.maker", &k.maker),
        ] {
            check_field(n, v, MAX_KIND_FIELD)?;
        }
    }
    Ok(())
}

/// The address to store: the one the device gave if it parses as an IP,
/// else the one it was seen from.
fn clean_ip(given: Option<&str>, observed: IpAddr) -> String {
    given
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .unwrap_or(observed)
        .to_string()
}

/// Whether a new device fits: `others` is how many the account has that
/// it won't replace.
fn device_fits(others: u64, max: u64) -> bool {
    others < max
}

/// A public IPv4 `ip:port` (what a UPnP mapping must look like), else `None`.
fn clean_mapped(v: Option<&str>) -> Option<String> {
    let sa: std::net::SocketAddrV4 = v?.trim().parse().ok()?;
    let ip = sa.ip();
    let o = ip.octets();
    let private = ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || (o[0] == 100 && (o[1] & 0xc0) == 64);
    (sa.port() != 0 && !private).then(|| sa.to_string())
}

/// A global unicast IPv6 address (2000::/3) in canonical form, else `None`.
fn clean_ipv6(v: Option<&str>) -> Option<String> {
    let ip: std::net::Ipv6Addr = v?.trim().parse().ok()?;
    ((ip.segments()[0] & 0xe000) == 0x2000).then(|| ip.to_string())
}

/// Only the known NAT values are stored; anything else becomes "unknown".
fn clean_nat(v: Option<&str>) -> &'static str {
    match v.map(str::trim) {
        Some("easy") => "easy",
        Some("symmetric") => "symmetric",
        _ => "unknown",
    }
}

#[derive(Serialize)]
pub struct DeviceOut {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: i32,
    pub udp_port: i32,
    pub ws_port: i32,
    pub quic_port: i32,
    pub last_seen: i64,
    // Peer-visible capability/state (NOT fcm_token).
    pub ws_open: bool,
    pub inbound_blocked: bool,
    pub supports_ipv6: bool,
    pub ipv6: Option<String>,
    pub reflexive_ip: Option<String>,
    pub reflexive_udp_port: Option<i32>,
    pub nat: String,
    pub mapped_quic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<DeviceKind>,
}

/// The caller's real IP. Behind the local reverse proxy (Caddy on :443) every
/// connection comes from loopback, so take the client IP the proxy put in
/// `X-Forwarded-For`. The header is trusted ONLY from loopback: a client
/// talking to :6969 directly can't spoof its address with it.
pub fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if peer.ip().is_loopback() {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            // Caddy replaces untrusted XFF with just the client; if a chain
            // ever arrives, the right-most entry is the one our proxy added.
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip;
        }
    }
    peer.ip()
}

/// Register or update (heartbeat / network-change) the caller's device. Uses a
/// `$set` upsert so a heartbeat that omits e.g. `fcm_token` doesn't clobber the
/// value stored by an earlier call.
pub async fn register(
    State(s): State<AppState>,
    user: AuthUser,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<DeviceReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate(&req)?;
    let ip = clean_ip(req.ip.as_deref(), client_ip(addr, &headers));
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    // A new device must fit under the account's cap; the same device again
    // (or one replacing an old record of the same name) always does.
    let known = coll
        .count_documents(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?
        > 0;
    if !known {
        let others = coll
            .count_documents(doc! { "user_id": &user.user_id, "name": { "$ne": &req.name } })
            .await?;
        if !device_fits(others, s.max_devices) {
            return Err(AppError::Conflict(format!(
                "this account has the most devices allowed ({}); remove one first",
                s.max_devices
            )));
        }
    }

    let mut set = doc! {
        "user_id": &user.user_id,
        "node_id": &req.node_id,
        "name": &req.name,
        "ip": &ip,
        "tcp_port": req.tcp_port,
        "udp_port": req.udp_port,
        "ws_port": req.ws_port,
        "quic_port": req.quic_port,
        "last_seen": now(),
        "supports_ipv6": req.supports_ipv6,
        "nat": clean_nat(req.nat.as_deref()),
        "mapped_quic": clean_mapped(req.mapped_quic.as_deref()),
    };
    if let Some(t) = req.fcm_token.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("fcm_token", t);
    }
    // Only a real global IPv6 address is stored; a device that lost IPv6
    // clears it (otherwise peers would keep dialing a dead address).
    match clean_ipv6(req.ipv6.as_deref()) {
        Some(v) => {
            set.insert("ipv6", v);
        }
        None => {
            set.insert("ipv6", mongodb::bson::Bson::Null);
        }
    }
    if let Some(p) = req.platform.as_ref().filter(|v| !v.trim().is_empty()) {
        set.insert("platform", p);
    }
    if let Some(k) = req.kind.as_ref().and_then(DeviceKind::cleaned) {
        set.insert(
            "kind",
            doc! { "os": k.os, "form": k.form, "model": k.model, "maker": k.maker },
        );
    }

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
    if let Some(ex) = q
        .exclude
        .filter(|v| !v.is_empty() && v.len() <= MAX_NODE_ID)
    {
        filter.insert("node_id", doc! { "$ne": ex });
    }
    let devices: Vec<Device> =
        s.db.collection::<Device>(crate::models::DEVICES_COLL)
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
            quic_port: d.quic_port,
            last_seen: d.last_seen,
            ws_open: d.ws_open,
            inbound_blocked: d.inbound_blocked,
            supports_ipv6: d.supports_ipv6,
            ipv6: d.ipv6,
            reflexive_ip: d.reflexive_ip,
            reflexive_udp_port: d.reflexive_udp_port,
            nat: d.nat.unwrap_or_else(|| "unknown".into()),
            mapped_quic: d.mapped_quic,
            platform: d.platform,
            kind: d.kind,
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
    check_node_id(&req.node_id)?;
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
    check_node_id(&req.node_id)?;
    check_opt("reflexive_ip", req.reflexive_ip.as_ref(), MAX_ADDR)?;
    if let Some(p) = req.reflexive_udp_port {
        check_port("reflexive_udp_port", p)?;
    }
    let mut set = doc! { "last_seen": now() };
    if let Some(v) = req.ws_open {
        set.insert("ws_open", v);
    }
    if let Some(v) = req.inbound_blocked {
        set.insert("inbound_blocked", v);
    }
    // Only a real address is stored.
    if let Some(v) = req
        .reflexive_ip
        .as_deref()
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
    {
        set.insert("reflexive_ip", v.to_string());
    }
    if let Some(v) = req.reflexive_udp_port {
        set.insert("reflexive_udp_port", v);
    }
    let res =
        s.db.collection::<Device>(crate::models::DEVICES_COLL)
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
    headers: HeaderMap,
    Json(req): Json<ProbeReq>,
) -> Result<Json<ProbeOut>, AppError> {
    check_node_id(&req.node_id)?;
    if req.check.len() > MAX_CHECKS || req.check.iter().any(|c| c.len() > MAX_CHECKS) {
        return Err(AppError::BadRequest(format!(
            "check takes at most {MAX_CHECKS} short entries"
        )));
    }
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    let dev = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?
        .ok_or(AppError::BadRequest("device not found".into()))?;

    let host = client_ip(addr, &headers);
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

    Ok(Json(ProbeOut {
        tcp_reachable,
        ws_reachable,
        udp_reachable,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xff(v: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", v.parse().unwrap());
        h
    }

    #[test]
    fn client_ip_trusts_forwarded_for_only_from_the_local_proxy() {
        let proxy: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let direct: SocketAddr = "203.0.113.9:50000".parse().unwrap();
        assert_eq!(
            client_ip(proxy, &xff("198.51.100.7")),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_ip(proxy, &xff("1.1.1.1, 198.51.100.7")),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        // A direct client can't spoof its address.
        assert_eq!(client_ip(direct, &xff("10.0.0.1")), direct.ip());
        // Missing or garbage header: fall back to the socket address.
        assert_eq!(client_ip(proxy, &HeaderMap::new()), proxy.ip());
        assert_eq!(client_ip(proxy, &xff("not-an-ip")), proxy.ip());
    }

    #[test]
    fn only_global_ipv6_is_stored() {
        assert_eq!(
            clean_ipv6(Some(" 2409:4040::1 ")).as_deref(),
            Some("2409:4040::1")
        );
        assert_eq!(clean_ipv6(Some("fe80::1")), None, "link-local");
        assert_eq!(clean_ipv6(Some("fd00::1")), None, "unique-local");
        assert_eq!(clean_ipv6(Some("::1")), None);
        assert_eq!(clean_ipv6(Some("1.2.3.4")), None);
        assert_eq!(clean_ipv6(None), None);
    }

    #[test]
    fn only_public_router_mappings_are_stored() {
        assert_eq!(
            clean_mapped(Some("152.67.7.50:41000")).as_deref(),
            Some("152.67.7.50:41000")
        );
        assert_eq!(clean_mapped(Some("192.168.0.5:41000")), None, "private");
        assert_eq!(
            clean_mapped(Some("100.70.1.2:41000")),
            None,
            "carrier-grade NAT"
        );
        assert_eq!(clean_mapped(Some("152.67.7.50:0")), None, "no port");
        assert_eq!(clean_mapped(Some("garbage")), None);
        assert_eq!(clean_mapped(None), None);
    }

    fn req() -> DeviceReq {
        serde_json::from_value(serde_json::json!({
            "node_id": "n1", "name": "Pixel 8", "ip": "192.168.0.5",
            "tcp_port": 51011, "udp_port": 60539, "quic_port": 41000,
            "fcm_token": "t".repeat(180), "platform": "android", "nat": "easy",
            "mapped_quic": "152.67.7.50:41000", "ipv6": "2409:4040::1",
            "kind": {"os": "Android 15", "form": "phone", "model": "Pixel 8", "maker": "Google"},
        }))
        .unwrap()
    }

    fn refused(r: DeviceReq) -> String {
        match validate(&r) {
            Err(AppError::BadRequest(m)) => m,
            _ => panic!("accepted"),
        }
    }

    #[test]
    fn a_normal_registration_passes_and_every_field_is_capped() {
        assert!(validate(&req()).is_ok());
        let long = |n: usize| "x".repeat(n);
        type Mutate = Box<dyn Fn(&mut DeviceReq)>;
        let cases: Vec<(&str, Mutate)> = vec![
            (
                "node_id",
                Box::new(move |r| r.node_id = long(MAX_NODE_ID + 1)),
            ),
            ("node_id", Box::new(|r| r.node_id = " ".into())),
            ("name", Box::new(move |r| r.name = long(MAX_NAME + 1))),
            ("name", Box::new(|r| r.name = "a\u{0}b".into())),
            ("ip", Box::new(move |r| r.ip = Some(long(MAX_ADDR + 1)))),
            ("ipv6", Box::new(move |r| r.ipv6 = Some(long(MAX_ADDR + 1)))),
            (
                "mapped_quic",
                Box::new(move |r| r.mapped_quic = Some(long(MAX_ADDR + 1))),
            ),
            (
                "fcm_token",
                Box::new(move |r| r.fcm_token = Some(long(MAX_FCM_TOKEN + 1))),
            ),
            (
                "platform",
                Box::new(move |r| r.platform = Some(long(MAX_SHORT + 1))),
            ),
            ("nat", Box::new(move |r| r.nat = Some(long(MAX_SHORT + 1)))),
            ("tcp_port", Box::new(|r| r.tcp_port = 70_000)),
            ("udp_port", Box::new(|r| r.udp_port = -1)),
            ("quic_port", Box::new(|r| r.quic_port = 65_536)),
            (
                "kind.model",
                Box::new(move |r| r.kind.as_mut().unwrap().model = long(MAX_KIND_FIELD + 1)),
            ),
        ];
        for (field, f) in cases {
            let mut r = req();
            f(&mut r);
            let m = refused(r);
            assert!(m.starts_with(field), "{field}: {m}");
        }
        // At the cap is fine; names may be any script.
        let mut r = req();
        r.name = "é".repeat(MAX_NAME);
        assert!(validate(&r).is_ok());
    }

    #[test]
    fn a_bad_ip_falls_back_to_the_observed_one() {
        let seen: IpAddr = "203.0.113.4".parse().unwrap();
        assert_eq!(clean_ip(Some(" 10.0.0.2 "), seen), "10.0.0.2");
        assert_eq!(clean_ip(Some("<b>"), seen), "203.0.113.4");
        assert_eq!(clean_ip(Some(""), seen), "203.0.113.4");
        assert_eq!(clean_ip(None, seen), "203.0.113.4");
    }

    #[test]
    fn an_account_holds_at_most_max_devices() {
        assert!(device_fits(0, MAX_DEVICES));
        assert!(device_fits(MAX_DEVICES - 1, MAX_DEVICES));
        assert!(!device_fits(MAX_DEVICES, MAX_DEVICES));
    }

    #[test]
    fn only_known_nat_values_are_stored() {
        assert_eq!(clean_nat(Some("easy")), "easy");
        assert_eq!(clean_nat(Some(" symmetric ")), "symmetric");
        assert_eq!(clean_nat(Some("<script>")), "unknown");
        assert_eq!(clean_nat(None), "unknown");
    }
}
