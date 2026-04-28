//! `GET /nonce` — issue an HMAC-signed nonce with a 5-minute TTL.
//!
//! Scaffold only. The nonce string format is `<uuid>.<hex-hmac>` where the
//! HMAC covers `<uuid>|<expires_at_unix>` using `Config::nonce_secret`.
//! `POST /send` re-derives and compares.

use std::sync::Arc;

use axum::{extract::State, Json};
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::{config::Config, errors::AppResult, models::NonceResponse, nonce};

/// Lifetime of a single nonce — mirrored on the POST /send side.
pub const NONCE_TTL_MINUTES: i64 = 5;

pub async fn issue_nonce(State(cfg): State<Arc<Config>>) -> AppResult<Json<NonceResponse>> {
    let nonce_id = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::minutes(NONCE_TTL_MINUTES);
    let signed = nonce::sign(&cfg.nonce_secret, &nonce_id, expires_at);

    Ok(Json(NonceResponse {
        nonce: signed,
        expires_at,
    }))
}
