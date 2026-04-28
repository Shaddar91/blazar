//! Unified error type + `IntoResponse` impl so handlers can return
//! `Result<Json<T>, AppError>` and get sensible HTTP status codes for free.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    /// 400 — request payload malformed / missing fields.
    BadRequest(String),
    /// 401 — nonce HMAC verification failed or nonce expired.
    Unauthorized(String),
    /// 403 — origin/referer check failed.
    Forbidden(String),
    /// 429 — per-IP rate limit exceeded (handled mostly by tower_governor,
    /// but surfaced here for the application-layer daily cap signalling).
    TooManyRequests(String),
    /// 500 — anything else (SMTP failure, queue write failure, …).
    Internal(anyhow::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::BadRequest(m) => write!(f, "bad request: {m}"),
            AppError::Unauthorized(m) => write!(f, "unauthorized: {m}"),
            AppError::Forbidden(m) => write!(f, "forbidden: {m}"),
            AppError::TooManyRequests(m) => write!(f, "too many requests: {m}"),
            AppError::Internal(e) => write!(f, "internal error: {e}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Internal(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.clone()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            AppError::TooManyRequests(m) => (StatusCode::TOO_MANY_REQUESTS, m.clone()),
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
