//! `GET /health` — trivial liveness probe.
//!
//! Returns `200 OK` with a short body. No auth, no DB. The Docker
//! healthcheck and the nginx upstream both hit this.

use axum::{http::StatusCode, response::IntoResponse};

pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
