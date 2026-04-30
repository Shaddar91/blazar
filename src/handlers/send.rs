//! `POST /send` — honeypot/nonce/rate-limit/daily-cap pipeline, then SMTP.
//!
//! Scaffold only — the individual checks delegate to module-level functions
//! which are themselves stubs today. The whole chain compiles end-to-end so
//! wiring tests are possible before real logic is filled in.

use std::{net::SocketAddr, sync::Arc, sync::OnceLock};

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
};
use chrono::{DateTime, Duration, Utc};
use dashmap::DashMap;
use uuid::Uuid;

use crate::{
    config::Config,
    errors::{AppError, AppResult},
    extractors::JsonOrSilent,
    models::{Message, SendRequest},
    nonce, queue, smtp,
};

//Per-recipient auto-reply throttle: visitor email -> last successful send time.
//In-memory by design (24h window, no DB) — restart resets the cap, which is
//acceptable because the daily-cap and per-IP rate-limits already bound abuse.
fn auto_reply_history() -> &'static DashMap<String, DateTime<Utc>> {
    static HISTORY: OnceLock<DashMap<String, DateTime<Utc>>> = OnceLock::new();
    HISTORY.get_or_init(DashMap::new)
}

fn auto_reply_recently_sent(email: &str, now: DateTime<Utc>) -> bool {
    auto_reply_history()
        .get(email)
        .is_some_and(|prev| now.signed_duration_since(*prev) < Duration::hours(24))
}

pub async fn send(
    State(cfg): State<Arc<Config>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    JsonOrSilent(req): JsonOrSilent<SendRequest>,
) -> AppResult<impl IntoResponse> {
    let client_ip = Some(addr.ip().to_string());
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 1. Decoy fields — silent 204 for bots. No leak of the rejection reason.
    if let Some(field) = triggered_decoy(&req) {
        tracing::info!(email = %req.email, field = field, "decoy field triggered");
        return Ok(StatusCode::NO_CONTENT);
    }

    // 2. Nonce HMAC verification (5-min TTL encoded in payload).
    nonce::verify(&cfg.nonce_secret, &req.nonce)
        .map_err(|e| AppError::Unauthorized(format!("invalid nonce: {e}")))?;

    // 3. Basic sanity checks on request contents. Silent 204 instead of 4xx
    //    so the frontend's `if (res.status === 204) return ok` branch swallows
    //    legitimate-mistake submissions without surfacing a generic error to
    //    the user. `name` is no longer required at this layer — the SMTP
    //    builder falls back to the email's local-part for the subject line.
    if req.email.trim().is_empty() {
        tracing::info!("missing email — silent reject");
        return Ok(StatusCode::NO_CONTENT);
    }

    // 4. Daily cap — `check_and_increment` returns `Ok(true)` if we're within
    //    the global cap, `Ok(false)` if over-cap (caller should enqueue and
    //    return a silent 204 per the "cap = real defense" design).
    let within_cap = queue::check_and_increment(&cfg.queue_dir, cfg.daily_cap)?;

    let msg = Message {
        id: Uuid::new_v4().to_string(),
        received_at: Utc::now(),
        name: req.name,
        email: req.email,
        subject: req.subject,
        body: req.message,
        client_ip,
        user_agent,
    };

    if !within_cap {
        tracing::warn!(
            id = %msg.id,
            "daily cap exceeded — enqueueing for midnight flush"
        );
        queue::enqueue(&cfg.queue_dir, &msg)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    // 5. Send via configured SMTP backend. The backend is built here from
    //    config — a later component may hoist this into application state.
    let backend = smtp::LoopbackSmtpBackend::from_config(&cfg).map_err(AppError::Internal)?;
    smtp::SmtpBackend::send(&backend, &msg)
        .await
        .map_err(AppError::Internal)?;

    // 6. Fire-and-forget auto-reply to the visitor. Errors are logged but
    //    never propagated — a slow SMTP call or invalid visitor address must
    //    not fail the inquiry POST. Per-recipient 24h throttle suppresses
    //    backscatter abuse without needing a DB.
    if backend.auto_reply_enabled {
        let visitor_email = msg.email.clone();
        let visitor_name = msg.name.clone();
        let inquiry_id = msg.id.chars().take(8).collect::<String>();
        let now = Utc::now();
        if auto_reply_recently_sent(&visitor_email, now) {
            tracing::info!(
                inquiry_id = %inquiry_id,
                "auto-reply throttled (recent send within 24h)"
            );
        } else {
            //SendRequest carries no `topics` field today; pass an empty slice
            //so the template renders with the empty-topics branch.
            let topics: Vec<String> = Vec::new();
            tokio::spawn(async move {
                match backend
                    .send_auto_reply(&visitor_email, &visitor_name, &topics, &inquiry_id)
                    .await
                {
                    Ok(()) => {
                        auto_reply_history().insert(visitor_email, Utc::now());
                    }
                    Err(e) => {
                        tracing::warn!(
                            inquiry_id = %inquiry_id,
                            error = ?e,
                            "auto-reply send failed (suppressed)"
                        );
                    }
                }
            });
        }
    }

    Ok(StatusCode::ACCEPTED)
}

/// Returns the name of the first non-empty decoy field, or `None` if all are
/// empty. Field order matches the form rendering order in the frontend.
fn triggered_decoy(req: &SendRequest) -> Option<&'static str> {
    if !req.company_address.is_empty() {
        Some("company_address")
    } else if !req.website_url.is_empty() {
        Some("website_url")
    } else if !req.phone_alt.is_empty() {
        Some("phone_alt")
    } else if !req.fax.is_empty() {
        Some("fax")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> SendRequest {
        SendRequest {
            name: "n".into(),
            email: "e@e".into(),
            subject: "s".into(),
            message: "m".into(),
            company_address: String::new(),
            website_url: String::new(),
            phone_alt: String::new(),
            fax: String::new(),
            nonce: "x".into(),
        }
    }

    #[test]
    fn no_decoy_returns_none() {
        assert_eq!(triggered_decoy(&req()), None);
    }

    #[test]
    fn company_address_triggers() {
        let mut r = req();
        r.company_address = "bot".into();
        assert_eq!(triggered_decoy(&r), Some("company_address"));
    }

    #[test]
    fn website_url_triggers() {
        let mut r = req();
        r.website_url = "http://x".into();
        assert_eq!(triggered_decoy(&r), Some("website_url"));
    }

    #[test]
    fn phone_alt_triggers() {
        let mut r = req();
        r.phone_alt = "555".into();
        assert_eq!(triggered_decoy(&r), Some("phone_alt"));
    }

    #[test]
    fn fax_triggers() {
        let mut r = req();
        r.fax = "555".into();
        assert_eq!(triggered_decoy(&r), Some("fax"));
    }

    #[test]
    fn legacy_honeypot_alias_deserializes_into_company_address() {
        let json = r#"{
            "name": "n", "email": "e@e", "subject": "s", "message": "m",
            "honeypot": "bot", "nonce": "x"
        }"#;
        let parsed: SendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.company_address, "bot");
        assert_eq!(triggered_decoy(&parsed), Some("company_address"));
    }

    #[test]
    fn missing_decoy_fields_default_to_empty() {
        let json = r#"{
            "name": "n", "email": "e@e", "subject": "s", "message": "m",
            "nonce": "x"
        }"#;
        let parsed: SendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(triggered_decoy(&parsed), None);
    }
}
