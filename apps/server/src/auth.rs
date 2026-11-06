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
/// Wrong passwords allowed per username in `GUESS_WINDOW` (login and
/// verify together), so a password can't be guessed online.
const GUESSES_PER_USER: u32 = 10;
const GUESS_WINDOW: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// Most usernames tracked at once (older windows are dropped first).
const MAX_TRACKED: usize = 50_000;
/// Longest username / password looked at (Argon2 over megabytes is a CPU sink).
const MAX_USERNAME: usize = 64;
const MAX_PASSWORD: usize = 256;

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
    if username.is_empty() || req.password.len() < 6 || too_long(&username, &req.password) {
        return Err(AppError::BadRequest(
            "username required (at most 64 characters) and password must be 6-256 characters"
                .into(),
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
    let pw = req.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || hash_password(&pw))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;
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
    let token = make_token(&s.jwt_secret, &uid, &username)?;
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
