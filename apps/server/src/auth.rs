// Authentication: Argon2id password hashing, the `AuthUser` extractor that
// guards device endpoints, and the /auth/* handlers. Tokens and sessions
// (issuing, checking, signing out) are in `session.rs`.

use std::net::SocketAddr;
use std::time::Instant;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    async_trait,
    extract::{ConnectInfo, FromRequestParts, State},
    http::{header::AUTHORIZATION, request::Parts, HeaderMap, StatusCode},
    Json,
};
use mongodb::bson::{doc, oid::ObjectId};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::models::User;
use crate::signup::{invite_hash, Mode};
use crate::state::AppState;

pub use crate::session::AuthUser;

/// Wrong passwords allowed per username in `GUESS_WINDOW` (login and
/// verify together), so a password can't be guessed online.
const GUESSES_PER_USER: u32 = 10;
const GUESS_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Most usernames tracked at once (older windows are dropped first).
const MAX_TRACKED: usize = 50_000;
/// Longest username / password looked at (Argon2 over megabytes is a CPU sink).
const MAX_USERNAME: usize = 64;
const MAX_PASSWORD: usize = 256;
/// Shortest password accepted for a new account or a new password.
const MIN_PASSWORD: usize = 6;

/// Wrong-password counts per username: (window start, count).
#[derive(Default)]
pub(crate) struct Guesses {
    map: std::collections::HashMap<String, (std::time::Instant, u32)>,
}

impl Guesses {
    /// Whether `user` may try a password now.
    pub(crate) fn allowed(&mut self, user: &str, now: std::time::Instant) -> bool {
        match self.map.get(user) {
            Some((since, n)) if now.saturating_duration_since(*since) < GUESS_WINDOW => {
                *n < GUESSES_PER_USER
            }
            _ => true,
        }
    }

    /// Count a wrong password for `user`.
    pub(crate) fn failed(&mut self, user: &str, now: std::time::Instant) {
        if self.map.len() >= MAX_TRACKED {
            self.map
                .retain(|_, (since, _)| now.saturating_duration_since(*since) < GUESS_WINDOW);
            if self.map.len() >= MAX_TRACKED {
                self.map.clear();
            }
        }
        let e = self.map.entry(user.to_string()).or_insert((now, 0));
        if now.saturating_duration_since(e.0) >= GUESS_WINDOW {
            *e = (now, 0);
        }
        e.1 += 1;
    }
}

fn guesses() -> &'static std::sync::Mutex<Guesses> {
    static G: std::sync::OnceLock<std::sync::Mutex<Guesses>> = std::sync::OnceLock::new();
    G.get_or_init(Default::default)
}

/// Check `pw` against `user`'s stored hash, limited per username, with the
/// Argon2 work off the async workers. A missing user costs the same work.
async fn check_password(username: &str, pw: &str, hash: Option<String>) -> Result<(), AppError> {
    let now = std::time::Instant::now();
    if !guesses().lock().unwrap().allowed(username, now) {
        return Err(AppError::TooMany);
    }
    let pw = pw.to_string();
    let ok = tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password(&pw, &h),
        None => {
            // Same cost as a real check, so timing doesn't say who exists.
            let _ = hash_password(&pw);
            false
        }
    })
    .await
    .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        guesses().lock().unwrap().failed(username, now);
        Err(AppError::Unauthorized)
    }
}

fn too_long(username: &str, pw: &str) -> bool {
    username.len() > MAX_USERNAME || pw.len() > MAX_PASSWORD
}

fn hash_password(pw: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// `hash_password` off the async workers.
async fn hash_password_blocking(pw: &str) -> Result<String, AppError> {
    let pw = pw.to_string();
    tokio::task::spawn_blocking(move || hash_password(&pw))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

fn verify_password(pw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(pw.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

/// The bearer token in `headers`, if any.
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(&parts.headers).ok_or(AppError::Unauthorized)?;
        state.sessions.verify(token).await
    }
}

/// Issue a token (for tests that need a valid session without MongoDB; the
/// test stores take any account at version 0).
#[cfg(test)]
pub fn issue_token(secret: &str, user_id: &str, username: &str) -> Option<String> {
    crate::session::make_token(secret, user_id, username, 0).ok()
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
    /// Needed when `REGISTRATION=invite`; ignored otherwise.
    #[serde(default)]
    pub invite: Option<String>,
}

#[derive(Serialize)]
pub struct TokenResp {
    pub token: String,
    pub username: String,
}

pub async fn register(
    State(s): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<AuthReq>,
) -> Result<Json<TokenResp>, AppError> {
    // Per address (the one Caddy saw), before anything costly.
    let client = crate::limits::client_key(crate::devices::client_ip(peer, &headers));
    let now = Instant::now();
    s.signup.admit(&client, now)?;
    let username = req.username.trim().to_lowercase();
    if username.is_empty()
        || req.password.len() < MIN_PASSWORD
        || too_long(&username, &req.password)
    {
        return Err(AppError::BadRequest(
            "username required (at most 64 characters) and password must be 6-256 characters"
                .into(),
        ));
    }
    // An invite is used up first and given back if the sign-up fails.
    let invite = match s.signup.mode() {
        Mode::Invite => {
            let code = req
                .invite
                .as_deref()
                .filter(|c| !c.trim().is_empty() && c.len() <= 64);
            let Some(code) = code else {
                s.signup.bad_invite();
                return Err(AppError::Forbidden(
                    "an invite code is needed to sign up here".into(),
                ));
            };
            let code = code.to_string();
            let hash = tokio::task::spawn_blocking(move || invite_hash(&code))
                .await
                .map_err(|e| AppError::Internal(e.to_string()))??;
            if !s.sessions.store().take_invite(&hash).await? {
                s.signup.bad_invite();
                return Err(AppError::Forbidden("this invite code is not valid".into()));
            }
            Some(hash)
        }
        _ => None,
    };
    let made = create_user(&s, &username, &req.password).await;
    let uid = match made {
        Ok(uid) => uid,
        Err(e) => {
            if let Some(h) = invite {
                let _ = s.sessions.store().return_invite(&h).await;
            }
            return Err(e);
        }
    };
    s.signup.created(&client, now);
    let token = s.sessions.issue_at(&uid, &username, 0)?;
    Ok(Json(TokenResp { token, username }))
}

/// Insert a new account; its id.
async fn create_user(s: &AppState, username: &str, password: &str) -> Result<String, AppError> {
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    if users
        .find_one(doc! { "username": username })
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("username already taken".into()));
    }
    let password_hash = hash_password_blocking(password).await?;
    let res = users
        .insert_one(User {
            id: None,
            username: username.to_string(),
            password_hash,
            token_ver: 0,
            revoked: Vec::new(),
        })
        .await?;
    res.inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .ok_or(AppError::Internal("no inserted id".into()))
}

pub async fn login(
    State(s): State<AppState>,
    Json(req): Json<AuthReq>,
) -> Result<Json<TokenResp>, AppError> {
    let username = req.username.trim().to_lowercase();
    if too_long(&username, &req.password) {
        return Err(AppError::Unauthorized);
    }
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    let found = users.find_one(doc! { "username": &username }).await?;
    check_password(
        &username,
        &req.password,
        found.as_ref().map(|u| u.password_hash.clone()),
    )
    .await?;
    let user = found.ok_or(AppError::Unauthorized)?;
    let uid = user
        .id
        .map(|o| o.to_hex())
        .ok_or(AppError::Internal("no id".into()))?;
    let token = s.sessions.issue_at(&uid, &username, user.token_ver)?;
    Ok(Json(TokenResp { token, username }))
}

/// Confirm a password without issuing a token (used to gate settings changes).
pub async fn verify(
    State(s): State<AppState>,
    Json(req): Json<AuthReq>,
) -> Result<StatusCode, AppError> {
    let username = req.username.trim().to_lowercase();
    if too_long(&username, &req.password) {
        return Err(AppError::Unauthorized);
    }
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    let found = users.find_one(doc! { "username": &username }).await?;
    check_password(&username, &req.password, found.map(|u| u.password_hash)).await?;
    Ok(StatusCode::OK)
}

/// `POST /auth/logout`: sign out the session this token belongs to (a token
/// from before session ids signs out every session of the account).
pub async fn logout(State(s): State<AppState>, user: AuthUser) -> Result<StatusCode, AppError> {
    let all = s.sessions.logout(&user).await?;
    s.relay
        .sign_out(&user.user_id, (!all).then_some(user.sid.as_str()));
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /auth/logout_all`: sign out every session of the account, this
/// one included.
pub async fn logout_all(State(s): State<AppState>, user: AuthUser) -> Result<StatusCode, AppError> {
    s.sessions.logout_all(&user.user_id).await?;
    s.relay.sign_out(&user.user_id, None);
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /auth/sessions`: the account's signed-in sessions (this one first,
/// marked `current`), each with the device that registered with it.
pub async fn sessions(
    State(s): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<crate::session::SessionOut>>, AppError> {
    s.sessions.list(&user).await.map(Json)
}

#[derive(Deserialize)]
pub struct RevokeReq {
    pub id: String,
}

/// `POST /auth/sessions/revoke {"id"}`: sign out one of the account's
/// sessions (its relay socket is closed). 404 if the account has no such
/// session. `signed_out_all`: the account's revoked list was full, so every
/// session (this one too) was signed out instead.
pub async fn revoke_session(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<RevokeReq>,
) -> Result<Json<serde_json::Value>, AppError> {
    if req.id.is_empty() || req.id.len() > 64 {
        return Err(AppError::BadRequest("id must be a session id".into()));
    }
    let all = s
        .sessions
        .revoke_session(&user, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound("no such session".into()))?;
    s.relay
        .sign_out(&user.user_id, (!all).then_some(req.id.as_str()));
    Ok(Json(serde_json::json!({ "signed_out_all": all })))
}

#[derive(Deserialize)]
pub struct PasswordReq {
    pub password: String,
    pub new_password: String,
}

/// `POST /auth/password` (signed in) `{"password", "new_password"}`: change
/// the password. Every session is signed out; the caller gets a new token.
/// A wrong current password is 403 (401 means this session is signed out).
pub async fn change_password(
    State(s): State<AppState>,
    user: AuthUser,
    Json(req): Json<PasswordReq>,
) -> Result<Json<TokenResp>, AppError> {
    if req.new_password.len() < MIN_PASSWORD || too_long(&user.username, &req.new_password) {
        return Err(AppError::BadRequest(
            "new_password must be 6-256 characters".into(),
        ));
    }
    let wrong = || AppError::Forbidden("current password is incorrect".into());
    if too_long(&user.username, &req.password) {
        return Err(wrong());
    }
    let oid = ObjectId::parse_str(&user.user_id).map_err(|_| AppError::Unauthorized)?;
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    let found = users.find_one(doc! { "_id": oid }).await?;
    check_password(
        &user.username,
        &req.password,
        found.map(|u| u.password_hash),
    )
    .await
    .map_err(|e| match e {
        AppError::Unauthorized => wrong(),
        e => e,
    })?;
    let password_hash = hash_password_blocking(&req.new_password).await?;
    users
        .update_one(
            doc! { "_id": oid },
            doc! {
                "$set": { "password_hash": password_hash, "revoked": [], "sessions": [] },
                "$inc": { "token_ver": 1_i64 },
            },
        )
        .await?;
    s.sessions.forget(&user.user_id);
    s.relay.sign_out(&user.user_id, None);
    let token = s.sessions.issue(&user.user_id, &user.username).await?;
    Ok(Json(TokenResp {
        token,
        username: user.username,
    }))
}

#[cfg(test)]
mod guess_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn wrong_passwords_are_limited_per_username_and_the_limit_lifts() {
        let mut g = Guesses::default();
        let t0 = Instant::now();
        for _ in 0..GUESSES_PER_USER {
            assert!(g.allowed("ann", t0));
            g.failed("ann", t0);
        }
        assert!(!g.allowed("ann", t0));
        assert!(g.allowed("bob", t0), "other accounts are not affected");
        assert!(g.allowed("ann", t0 + GUESS_WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn overlong_credentials_are_refused_before_hashing() {
        assert!(too_long(&"u".repeat(65), "secret"));
        assert!(too_long("ann", &"p".repeat(257)));
        assert!(!too_long("ann", "secret"));
    }
}
