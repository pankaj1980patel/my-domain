//! Operator routes, only when `ADMIN_TOKEN` is set (at least 32 bytes);
//! otherwise they answer 404 like any unknown path. The token goes in
//! `Authorization: Bearer <ADMIN_TOKEN>`.
//!
//!   GET  /admin/limits  -> counts from the limiters (no addresses, names or ids)
//!   POST /admin/invites {"uses": 1, "days": 7} -> {"code": "...."}
//!        (shown once; only its hash is stored)

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;
use crate::signup::{invite_hash, new_invite_code};
use crate::state::AppState;

/// Shortest `ADMIN_TOKEN` accepted.
pub const MIN_ADMIN_TOKEN: usize = 32;
/// Most uses and days one invite may have.
const MAX_USES: u32 = 1000;
const MAX_DAYS: u32 = 90;

/// `ADMIN_TOKEN` from the environment, if set and long enough.
pub fn token_from_env() -> Option<std::sync::Arc<str>> {
    let t = std::env::var("ADMIN_TOKEN").ok()?;
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    if t.len() < MIN_ADMIN_TOKEN {
        tracing::warn!(
            "ADMIN_TOKEN is under {MIN_ADMIN_TOKEN} bytes; /admin is off (openssl rand -hex 32)"
        );
        return None;
    }
    Some(t.into())
}

/// Equal without saying where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// `Ok` for the operator; 404 when /admin is off, 401 for a wrong token.
fn operator(s: &AppState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let Some(want) = s.admin_token.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    match crate::auth::bearer(headers) {
        Some(got) if same(got.as_bytes(), want.as_bytes()) => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn limits(State(s): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = operator(&s, &headers) {
        return r.into_response();
    }
    let invites = s.sessions.store().invites_left().await.ok();
    Json(json!({
        "signup": s.signup.stats(),
        "invites_with_uses_left": invites,
        "pair": s.pair.stats(),
        "push": { "accounts_counted_today": s.push.accounts_counted_today() },
        "sessions": { "accounts_cached": s.sessions.cached() },
        "relay": { "connections": s.relay.connections() },
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
pub struct InviteReq {
    #[serde(default)]
    uses: Option<u32>,
    #[serde(default)]
    days: Option<u32>,
}

pub async fn invites(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<InviteReq>>,
) -> Response {
    if let Err(r) = operator(&s, &headers) {
        return r.into_response();
    }
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let uses = req.uses.unwrap_or(1).clamp(1, MAX_USES);
    let days = req.days.unwrap_or(7).clamp(1, MAX_DAYS);
    let code = new_invite_code();
    let made = async {
        let c = code.clone();
        let hash = tokio::task::spawn_blocking(move || invite_hash(&c))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))??;
        let expires = crate::limits::wall_ms() + i64::from(days) * 24 * 3600 * 1000;
        s.sessions.store().add_invite(&hash, uses, expires).await
    }
    .await;
    match made {
        Ok(()) => Json(json!({ "code": code, "uses": uses, "days": days })).into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_compared_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
        assert!(!same(b"", b"a"));
    }
}
