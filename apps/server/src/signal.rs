// Signaling relay: a device posts a typed signal addressed to one of its OWN
// sibling devices; the server pushes it as an FCM data message to that device's
// token. Both `from` and `to` must belong to the caller's user (anti-spoof).

use axum::{extract::State, Json};
use mongodb::bson::doc;
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::error::AppError;
use crate::fcm::FcmError;
use crate::models::Device;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SignalReq {
    /// Caller's own node_id (verified to belong to the caller).
    pub from: String,
    /// Target sibling node_id.
    pub to: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Type-specific payload (candidates, session id, timing, …).
    #[serde(default)]
    pub data: serde_json::Value,
}

pub async fn relay(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<SignalReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.from == req.to {
        return Err(AppError::BadRequest("cannot signal self".into()));
    }
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);

    // Anti-spoof: the claimed sender must be one of the caller's devices.
    if coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.from })
        .await?
        .is_none()
    {
        return Err(AppError::BadRequest("unknown 'from' device".into()));
    }
    let target = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.to })
        .await?
        .ok_or(AppError::BadRequest("unknown 'to' device".into()))?;

    let fcm = s
        .fcm
        .as_ref()
        .ok_or(AppError::Internal("FCM not configured on server".into()))?;
    let token = target
        .fcm_token
        .clone()
        .ok_or(AppError::Conflict("target has no FCM token (offline)".into()))?;

    // FCM data payloads are string→string; the typed body is stringified.
    // NB: `from` is an FCM-reserved data key (rejected with INVALID_ARGUMENT),
    // so the sender node id travels as `sender`.
    let data = serde_json::json!({
        "type": req.kind,
        "sender": req.from,
        "to": req.to,
        "payload": req.data.to_string(),
    });

    match fcm.send_data(&token, data).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(FcmError::Unregistered) => {
            let _ = coll
                .update_one(
                    doc! { "user_id": &user.user_id, "node_id": &req.to },
                    doc! { "$unset": { "fcm_token": "" } },
                )
                .await;
            Err(AppError::Conflict("target FCM token invalid (cleared)".into()))
        }
        Err(e) => Err(AppError::Internal(format!("FCM send failed: {e:?}"))),
    }
}

#[derive(Deserialize)]
pub struct SelfPingReq {
    /// Caller's own node_id (verified to belong to the caller).
    pub node_id: String,
    /// Correlation id echoed back in the pushed payload.
    #[serde(default)]
    pub sid: String,
}

/// Round-trip FCM self-test: push a `ping` data message to the caller's OWN
/// device token. Unlike `relay`, self-addressing is allowed here — that's the
/// whole point: the device verifies it can actually receive its own pushes.
pub async fn selfping(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<SelfPingReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    let coll = s.db.collection::<Device>(crate::models::DEVICES_COLL);
    let device = coll
        .find_one(doc! { "user_id": &user.user_id, "node_id": &req.node_id })
        .await?
        .ok_or(AppError::BadRequest("unknown device".into()))?;

    let fcm = s
        .fcm
        .as_ref()
        .ok_or(AppError::Internal("FCM not configured on server".into()))?;
    let token = device
        .fcm_token
        .clone()
        .ok_or(AppError::Conflict("device has no FCM token (offline)".into()))?;

    let data = serde_json::json!({
        "type": "ping",
        "sender": req.node_id,
        "to": req.node_id,
        "payload": format!("selfping:{}", req.sid),
    });

    match fcm.send_data(&token, data).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(FcmError::Unregistered) => {
            let _ = coll
                .update_one(
                    doc! { "user_id": &user.user_id, "node_id": &req.node_id },
                    doc! { "$unset": { "fcm_token": "" } },
                )
                .await;
            Err(AppError::Conflict("FCM token invalid (cleared)".into()))
        }
        Err(e) => Err(AppError::Internal(format!("FCM send failed: {e:?}"))),
    }
}
