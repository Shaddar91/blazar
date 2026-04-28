//! HTTP middleware: strict CORS layer + an Origin/Referer validator stub.
//!
//! Later components fill in the bodies; this file pins down the signatures
//! so handler wiring in `main.rs` compiles against something stable.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;
use tower_http::cors::CorsLayer;

use crate::config::Config;

/// Build the CORS layer allowed only for the configured origin.
///
/// We intentionally do NOT allow credentials — the form is stateless; no
/// cookies, no auth headers.
pub fn cors_layer(cors_origin: &str) -> CorsLayer {
    let origin = cors_origin
        .parse::<HeaderValue>()
        .expect("CORS_ORIGIN must be a valid HTTP header value");

    CorsLayer::new()
        .allow_origin(origin)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE])
}

/// Origin + Referer defensive check.
///
/// Stub — to be implemented as an `axum::middleware::from_fn` wrapping
/// `/send`. Rejects with 403 when neither header points at `cors_origin`.
pub async fn origin_referer_guard() {
    // todo(E4): validate req.headers().get("origin") / "referer" against cfg
}

/// Upstream-CDN-injected `X-Origin-Verify` header guard.
///
/// When `Config::cloudfront_verify_secret` is `Some`, every inbound request
/// must carry an `X-Origin-Verify` header whose value matches the secret.
/// Mismatches and absent headers are dropped silently with 204 — clients
/// that bypass the CDN and hit the origin directly get no signal that
/// anything is wrong. When the secret is `None`, this is a no-op so local
/// dev without the secret still works.
///
/// Comparison is constant-time via `subtle::ConstantTimeEq`.
pub async fn cloudfront_verify_guard(
    State(cfg): State<Arc<Config>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(expected) = cfg.cloudfront_verify_secret.as_deref() else {
        return next.run(request).await;
    };

    let header_ok = request
        .headers()
        .get("x-origin-verify")
        .and_then(|v| v.to_str().ok())
        .map(|got| {
            let got = got.as_bytes();
            let expected = expected.as_bytes();
            got.len() == expected.len() && bool::from(got.ct_eq(expected))
        })
        .unwrap_or(false);

    if !header_ok {
        return StatusCode::NO_CONTENT.into_response();
    }

    next.run(request).await
}
