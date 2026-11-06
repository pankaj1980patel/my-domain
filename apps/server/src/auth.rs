// Authentication: Argon2id password hashing, JWT issuance/verification, the
// `AuthUser` extractor that guards device endpoints, and the /auth/* handlers.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{header::AUTHORIZATION, request::Parts, StatusCode},
    Json,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use mongodb::bson::doc;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::AppError;
use crate::models::User;
use crate::state::AppState;

const TOKEN_TTL_SECS: u64 = 7 * 24 * 3600;

#[derive(Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user id (hex)
    pub username: String,
    pub exp: usize,
}

fn hash_password(pw: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(e.to_string()))
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

pub(crate) fn make_token(secret: &str, user_id: &str, username: &str) -> Result<String, AppError> {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as usize
        + TOKEN_TTL_SECS as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        username: username.to_string(),
        exp,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

/// Extractor: requires a valid `Authorization: Bearer <jwt>` header.
pub struct AuthUser {
    pub user_id: String,
    pub username: String,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .ok_or(AppError::Unauthorized)?;
        verify_token(&state.jwt_secret, token).ok_or(AppError::Unauthorized)
    }
}

/// Check a session token (signature + expiry) and return who it belongs to.
/// Shared by the HTTP `Authorization` extractor and the WebSocket relay, where
/// the token arrives in the first frame instead of a header.
pub fn verify_token(secret: &str, token: &str) -> Option<AuthUser> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    Some(AuthUser {
        user_id: data.claims.sub,
        username: data.claims.username,
    })
}

/// Issue a token (for tests that need a valid session without MongoDB).
#[cfg(test)]
pub fn issue_token(secret: &str, user_id: &str, username: &str) -> Option<String> {
    make_token(secret, user_id, username).ok()
}

#[derive(Deserialize)]
pub struct AuthReq {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct TokenResp {
    pub token: String,
    pub username: String,
}

pub async fn register(
    State(s): State<AppState>,
    Json(req): Json<AuthReq>,
) -> Result<Json<TokenResp>, AppError> {
    let username = req.username.trim().to_lowercase();
    if username.is_empty() || req.password.len() < 6 {
        return Err(AppError::BadRequest(
            "username required and password must be at least 6 characters".into(),
        ));
    }
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    if users
        .find_one(doc! { "username": &username })
        .await?
        .is_some()
    {
        return Err(AppError::Conflict("username already taken".into()));
    }
    let password_hash = hash_password(&req.password)?;
    let res = users
        .insert_one(User {
            id: None,
            username: username.clone(),
            password_hash,
        })
        .await?;
    let uid = res
        .inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .ok_or(AppError::Internal("no inserted id".into()))?;
    let token = make_token(&s.jwt_secret, &uid, &username)?;
    Ok(Json(TokenResp { token, username }))
}

pub async fn login(
    State(s): State<AppState>,
    Json(req): Json<AuthReq>,
) -> Result<Json<TokenResp>, AppError> {
    let username = req.username.trim().to_lowercase();
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    let user = users
        .find_one(doc! { "username": &username })
        .await?
        .ok_or(AppError::Unauthorized)?;
    if !verify_password(&req.password, &user.password_hash) {
        return Err(AppError::Unauthorized);
    }
    let uid = user
        .id
        .map(|o| o.to_hex())
        .ok_or(AppError::Internal("no id".into()))?;
    let token = make_token(&s.jwt_secret, &uid, &username)?;
    Ok(Json(TokenResp { token, username }))
}

/// Confirm a password without issuing a token (used to gate settings changes).
pub async fn verify(
    State(s): State<AppState>,
    Json(req): Json<AuthReq>,
) -> Result<StatusCode, AppError> {
    let username = req.username.trim().to_lowercase();
    let users = s.db.collection::<User>(crate::models::USERS_COLL);
    let user = users
        .find_one(doc! { "username": &username })
        .await?
        .ok_or(AppError::Unauthorized)?;
    if verify_password(&req.password, &user.password_hash) {
        Ok(StatusCode::OK)
    } else {
        Err(AppError::Unauthorized)
    }
}
