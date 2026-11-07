//! Registry (HTTP) client. The registry is a directory only: it maps a user's
//! devices to their endpoints. All calls return `Result<_, String>` so adapters
//! can surface the message verbatim.

use std::time::Duration;

use serde::Deserialize;

use crate::model::Identity;

#[derive(Deserialize)]
pub struct TokenResp {
    pub token: String,
    pub username: String,
}

#[derive(Deserialize)]
pub struct RegistryDevice {
    pub node_id: String,
    pub name: String,
    pub ip: String,
    pub tcp_port: u16,
    pub udp_port: u16,
    #[serde(default)]
    pub ws_port: u16,
    #[serde(default)]
    pub quic_port: u16,
    // Peer-visible reachability state (server `DeviceOut`); defaulted so older
    // server responses still deserialize.
    #[serde(default)]
    pub ws_open: bool,
    #[serde(default)]
    pub inbound_blocked: bool,
    #[serde(default)]
    pub reflexive_ip: Option<String>,
    #[serde(default)]
    pub reflexive_udp_port: Option<u16>,
    /// "easy" | "symmetric" | "unknown"; absent from older servers.
    #[serde(default)]
    pub nat: Option<String>,
    #[serde(default)]
    pub ipv6: Option<String>,
    #[serde(default)]
    pub mapped_quic: Option<String>,
    /// What kind of device it is (`features::device_kind`); absent from
    /// older servers and devices.
    #[serde(default)]
    pub kind: Option<serde_json::Value>,
}

/// Normalize a user-entered server URL to just `scheme://host[:port]`, dropping
/// any path/query so pasting a full endpoint (e.g. `.../auth/login`) doesn't get
/// the path appended twice.
pub fn base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    match u.find("://") {
        Some(i) => {
            let host_start = i + 3;
            let host_end = u[host_start..]
                .find('/')
                .map(|j| host_start + j)
                .unwrap_or(u.len());
            u[..host_end].to_string()
        }
        None => u.to_string(),
    }
}

pub fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => r
            .into_json::<serde_json::Value>()
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
            .unwrap_or_else(|| format!("server returned HTTP {code}")),
        other => other.to_string(),
    }
}

// --- a session the server no longer takes ("signed out elsewhere") ---

/// Told about a token the server answered 401 to; returns false once it no
/// longer cares (its engine is gone) and is dropped.
pub(crate) type RefusedHook = std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>;

fn refused_hooks() -> &'static std::sync::Mutex<Vec<RefusedHook>> {
    static HOOKS: std::sync::OnceLock<std::sync::Mutex<Vec<RefusedHook>>> =
        std::sync::OnceLock::new();
    HOOKS.get_or_init(Default::default)
}

/// Call `hook` whenever the server refuses a session token (each engine
/// checks whether it's its own).
pub(crate) fn on_token_refused(hook: RefusedHook) {
    if let Ok(mut h) = refused_hooks().lock() {
        h.push(hook);
    }
}

/// The server answered 401 to `token` (on a call that sends it, or the
/// relay's sign-in). The hooks run on a thread of their own: the caller may
/// hold locks.
pub(crate) fn token_refused(token: &str) {
    let token = token.to_string();
    std::thread::spawn(move || {
        let hooks: Vec<RefusedHook> = refused_hooks()
            .lock()
            .map(|h| h.clone())
            .unwrap_or_default();
        let gone: Vec<RefusedHook> = hooks.into_iter().filter(|h| !h(&token)).collect();
        if !gone.is_empty() {
            if let Ok(mut h) = refused_hooks().lock() {
                h.retain(|x| !gone.iter().any(|g| std::sync::Arc::ptr_eq(g, x)));
            }
        }
    });
}

/// `http_err` for a call made with `token`: a 401 means the session is
/// signed out (`token_refused`).
fn authed_err(token: &str) -> impl FnOnce(ureq::Error) -> String + '_ {
    move |e| {
        if matches!(e, ureq::Error::Status(401, _)) {
            token_refused(token);
        }
        http_err(e)
    }
}

/// What `change_password_call` says when the current password is wrong.
pub const WRONG_PASSWORD: &str = "incorrect password";

/// Change the account password (`/auth/password`); the new session token.
/// Every other session is signed out by the server.
pub fn change_password_call(
    url: &str,
    token: &str,
    password: &str,
    new_password: &str,
) -> Result<TokenResp, String> {
    match crate::tls::http()
        .post(&format!("{}/auth/password", base(url)))
        .timeout(Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "password": password, "new_password": new_password }))
    {
        Ok(r) => r.into_json::<TokenResp>().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(403, _)) => Err(WRONG_PASSWORD.into()),
        // An older server says 401 for a wrong password too: whether the
        // session is still good is asked separately (a 401 there is the
        // sign-out).
        Err(ureq::Error::Status(401, _)) => match registry_fetch(url, token, "") {
            Ok(_) => Err(WRONG_PASSWORD.into()),
            Err(e) => Err(e),
        },
        Err(e) => Err(http_err(e)),
    }
}

/// Sign out every session of the account (`/auth/logout_all`), this one too.
pub fn logout_all_call(url: &str, token: &str) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/auth/logout_all", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({}))
        .map_err(authed_err(token))?;
    Ok(())
}

/// One signed-in session of the account (`GET /auth/sessions`).
#[derive(Deserialize, serde::Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AccountSession {
    pub id: String,
    /// The device that signed in with it; empty until it registered.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub node_id: String,
    /// Seconds since the epoch.
    #[serde(default)]
    pub created: f64,
    #[serde(default)]
    pub last_seen: f64,
    /// This device's own session.
    #[serde(default)]
    pub current: bool,
}

/// What an older server without `/auth/sessions` gets turned into.
pub const SESSIONS_OLD_SERVER: &str = "update the server to list sessions";

/// The account's signed-in sessions, this one first.
pub fn sessions_call(url: &str, token: &str) -> Result<Vec<AccountSession>, String> {
    match crate::tls::http()
        .get(&format!("{}/auth/sessions", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
    {
        Ok(r) => r
            .into_json::<Vec<AccountSession>>()
            .map_err(|e| e.to_string()),
        Err(ureq::Error::Status(404 | 405, _)) => Err(SESSIONS_OLD_SERVER.into()),
        Err(e) => Err(authed_err(token)(e)),
    }
}

/// Sign out session `id`; true when the server signed out every session
/// instead (this one too).
pub fn revoke_session_call(url: &str, token: &str, id: &str) -> Result<bool, String> {
    let v = crate::tls::http()
        .post(&format!("{}/auth/sessions/revoke", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "id": id }))
        .map_err(authed_err(token))?
        .into_json::<serde_json::Value>()
        .map_err(|e| e.to_string())?;
    Ok(v["signed_out_all"].as_bool().unwrap_or(false))
}

pub fn auth_call(
    url: &str,
    path: &str,
    username: &str,
    password: &str,
) -> Result<TokenResp, String> {
    crate::tls::http()
        .post(&format!("{}{}", base(url), path))
        .timeout(Duration::from_secs(10))
        .send_json(serde_json::json!({ "username": username, "password": password }))
        .map_err(http_err)?
        .into_json::<TokenResp>()
        .map_err(|e| e.to_string())
}

pub fn verify_password_call(url: &str, username: &str, password: &str) -> Result<bool, String> {
    match crate::tls::http()
        .post(&format!("{}/auth/verify", base(url)))
        .timeout(Duration::from_secs(10))
        .send_json(serde_json::json!({ "username": username, "password": password }))
    {
        Ok(_) => Ok(true),
        Err(ureq::Error::Status(401, _)) => Ok(false),
        Err(e) => Err(http_err(e)),
    }
}

/// Sign this session's token out on the server (`/auth/logout`; an older
/// server answers 404, which changes nothing).
pub fn auth_logout(url: &str, token: &str) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/auth/logout", base(url)))
        .timeout(Duration::from_secs(5))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({}))
        .map_err(http_err)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn registry_register(
    url: &str,
    token: &str,
    id: &Identity,
    fcm_token: Option<&str>,
    platform: &str,
    supports_ipv6: bool,
    nat: crate::nat::NatKind,
    kind: Option<serde_json::Value>,
) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/devices/register", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({
            "node_id": id.node_id,
            "name": id.name,
            "ip": id.ip,
            "tcp_port": id.tcp_port,
            "udp_port": id.udp_port,
            "ws_port": id.ws_port,
            "quic_port": id.quic_port,
            "fcm_token": fcm_token,
            "platform": platform,
            "supports_ipv6": supports_ipv6,
            "ipv6": id.ipv6,
            "mapped_quic": id.mapped_quic,
            "nat": nat.as_str(),
            "kind": kind,
        }))
        .map_err(authed_err(token))?;
    Ok(())
}

/// Relay a typed signal to one of the user's other devices (server pushes it via
/// FCM). `data` is the typed payload.
pub fn send_signal(
    url: &str,
    token: &str,
    from: &str,
    to: &str,
    kind: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/signal", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "from": from, "to": to, "type": kind, "data": data }))
        .map_err(authed_err(token))?;
    Ok(())
}

/// Remove a device of ours from the registry.
pub fn registry_unregister(url: &str, token: &str, node_id: &str) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/devices/unregister", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "node_id": node_id }))
        .map_err(authed_err(token))?;
    Ok(())
}

/// Round-trip FCM self-test: ask the server to push a `ping` to our OWN device
/// token. The pong is delivered back through the platform's FCM receiver. `sid`
/// correlates the request with the received pong.
pub fn send_selfping(url: &str, token: &str, node_id: &str, sid: &str) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/signal/selfping", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "node_id": node_id, "sid": sid }))
        .map_err(authed_err(token))?;
    Ok(())
}

/// Partial live-state update (e.g. flip `ws_open`, record STUN mapping).
pub fn update_device_state(
    url: &str,
    token: &str,
    node_id: &str,
    ws_open: Option<bool>,
    inbound_blocked: Option<bool>,
    reflexive_ip: Option<&str>,
    reflexive_udp_port: Option<u16>,
) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/devices/state", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({
            "node_id": node_id,
            "ws_open": ws_open,
            "inbound_blocked": inbound_blocked,
            "reflexive_ip": reflexive_ip,
            "reflexive_udp_port": reflexive_udp_port,
        }))
        .map_err(authed_err(token))?;
    Ok(())
}

#[derive(Deserialize, Clone, Copy)]
pub struct ProbeResult {
    pub tcp_reachable: bool,
    pub ws_reachable: bool,
    pub udp_reachable: bool,
}

/// Ask the server to dial back to our advertised ports (inbound firewall check).
pub fn probe_inbound(
    url: &str,
    token: &str,
    node_id: &str,
    checks: &[&str],
) -> Result<ProbeResult, String> {
    crate::tls::http()
        .post(&format!("{}/devices/probe", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "node_id": node_id, "check": checks }))
        .map_err(authed_err(token))?
        .into_json::<ProbeResult>()
        .map_err(|e| e.to_string())
}

/// Outbound reachability: can we reach the server at all?
pub fn health_ok(url: &str) -> bool {
    ureq::get(&format!("{}/health", base(url)))
        .timeout(Duration::from_secs(5))
        .call()
        .is_ok()
}

// --- adding a device by QR code / 6-digit code (server `/pair/*`) ---

/// What an older server without `/pair/*` gets turned into (the apps show
/// "Update the server to use QR join").
pub const PAIR_OLD_SERVER: &str = "update the server to use qr join";

fn pair_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(404 | 405, _) => PAIR_OLD_SERVER.to_string(),
        other => http_err(other),
    }
}

/// `pair_err` for a call made with `token` (a 401 is a sign-out).
fn pair_err_authed(token: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(404 | 405, _) => PAIR_OLD_SERVER.to_string(),
        other => authed_err(token)(other),
    }
}

#[derive(Deserialize)]
pub struct PairCreated {
    pub code: String,
    pub expires_in: u64,
}

/// Start a pairing: `claim` is the join link's [`crate::join::claim_id`],
/// `code` the 6-digit code and `code_box` the key sealed under it.
/// `Ok(None)` when the server says the code is taken (pick another).
pub fn pair_create(
    url: &str,
    token: &str,
    claim: &str,
    code: &str,
    code_box: &str,
) -> Result<Option<PairCreated>, String> {
    match crate::tls::http()
        .post(&format!("{}/pair/create", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "claim": claim, "code": code, "code_box": code_box }))
    {
        Ok(r) => r
            .into_json::<PairCreated>()
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(ureq::Error::Status(409, _)) => Ok(None),
        Err(e) => Err(pair_err_authed(token, e)),
    }
}

#[derive(Deserialize)]
pub struct PairClaimed {
    pub token: String,
    pub username: String,
    #[serde(default)]
    pub code_box: Option<String>,
}

/// Trade a join link's claim, or a typed code, for a session.
pub fn pair_claim(
    url: &str,
    claim: Option<&str>,
    code: Option<&str>,
    name: &str,
) -> Result<PairClaimed, String> {
    crate::tls::http()
        .post(&format!("{}/pair/claim", base(url)))
        .timeout(Duration::from_secs(10))
        .send_json(serde_json::json!({ "claim": claim, "code": code, "name": name }))
        .map_err(pair_err)?
        .into_json::<PairClaimed>()
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct PairStatusResp {
    /// "none", "waiting" or "joined".
    pub state: String,
    #[serde(default)]
    pub name: Option<String>,
}

pub fn pair_status(url: &str, token: &str) -> Result<PairStatusResp, String> {
    crate::tls::http()
        .get(&format!("{}/pair/status", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| pair_err_authed(token, e))?
        .into_json::<PairStatusResp>()
        .map_err(|e| e.to_string())
}

pub fn pair_cancel(url: &str, token: &str) -> Result<(), String> {
    crate::tls::http()
        .post(&format!("{}/pair/cancel", base(url)))
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({}))
        .map_err(|e| pair_err_authed(token, e))?;
    Ok(())
}

pub fn registry_fetch(
    url: &str,
    token: &str,
    exclude: &str,
) -> Result<Vec<RegistryDevice>, String> {
    // `.query` percent-encodes the value; node ids are opaque strings and
    // were previously pasted into the URL raw.
    ureq::get(&format!("{}/devices", base(url)))
        .query("exclude", exclude)
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(authed_err(token))?
        .into_json::<Vec<RegistryDevice>>()
        .map_err(|e| e.to_string())
}
