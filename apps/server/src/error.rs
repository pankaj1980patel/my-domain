use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Uniform API error → JSON `{ "error": "..." }` with a status code.
#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    /// Not allowed here at all (sign-up closed, a bad invite code).
    Forbidden(String),
    BadRequest(String),
    Conflict(String),
    /// Expired, used, or never there (a join code).
    Gone(String),
    /// Too many failed attempts; try again later.
    TooMany,
    /// Over a rate limit: 429 with `Retry-After` (whole seconds, at least 1).
    RateLimited(std::time::Duration, String),
    /// A body or field over its size cap.
    TooLarge(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::RateLimited(wait, m) => {
                let secs = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
                let mut r =
                    (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": m }))).into_response();
                r.headers_mut().insert(
                    axum::http::header::RETRY_AFTER,
                    axum::http::HeaderValue::from(secs.max(1)),
                );
                return r;
            }
            AppError::TooLarge(m) => (StatusCode::PAYLOAD_TOO_LARGE, m),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rate_limited_answers_429_with_retry_after_rounded_up() {
        let r =
            AppError::RateLimited(Duration::from_millis(1500), "slow down".into()).into_response();
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(r.headers()["retry-after"], "2");
        let r = AppError::RateLimited(Duration::ZERO, "x".into()).into_response();
        assert_eq!(r.headers()["retry-after"], "1");
        let r = AppError::TooLarge("big".into()).into_response();
        assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
