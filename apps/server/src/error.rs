use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Uniform API error → JSON `{ "error": "..." }` with a status code.
pub enum AppError {
    Unauthorized,
    BadRequest(String),
    Conflict(String),
    /// Expired, used, or never there (a join code).
    Gone(String),
    /// Too many failed attempts; try again later.
    TooMany,
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::Gone(m) => (StatusCode::GONE, m),
            AppError::TooMany => (
                StatusCode::TOO_MANY_REQUESTS,
                "too many tries, wait a minute".to_string(),
            ),
            AppError::Internal(m) => {
                // Details (database or push errors) stay in the log.
                tracing::warn!("internal error: {m}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

impl From<mongodb::error::Error> for AppError {
    fn from(e: mongodb::error::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
