//! Shared types used across handlers, queue, and SMTP layer.
//!
//! Kept intentionally light — `SendRequest` is the wire schema coming from
//! the React frontend; `Message` is the internal representation we enqueue /
//! hand to the SMTP backend; `NoncePayload` is the HMAC-signed blob issued
//! by `GET /nonce` and re-verified by `POST /send`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Incoming JSON body for `POST /send`.
///
/// The four decoy fields (`company_address`, `website_url`, `phone_alt`, `fax`)
/// are visually-hidden form inputs — any non-empty value means we silently drop
/// the submission (204). The legacy `honeypot` key from the previous single-field
/// schema is accepted as an alias on `company_address` so in-flight submissions
/// during deploy-skew still resolve. `nonce` is the HMAC-signed token previously
/// issued by `GET /nonce`.
#[derive(Debug, Clone, Deserialize)]
pub struct SendRequest {
    #[serde(default)]
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, alias = "honeypot")]
    pub company_address: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub phone_alt: String,
    #[serde(default)]
    pub fax: String,
    /// HMAC-signed nonce as returned by `GET /nonce`.
    pub nonce: String,
}

/// Internal representation of a message to be sent (or queued).
///
/// Enqueued as pretty-printed JSON under `QUEUE_DIR`. IP + user-agent are
/// captured at receive time so we can attach them to the outbound email
/// body for triage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub received_at: DateTime<Utc>,
    pub name: String,
    pub email: String,
    pub subject: String,
    pub body: String,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
}

/// What a `GET /nonce` response carries.
///
/// The `nonce` string is `<uuid>.<hex-hmac>` where the HMAC covers
/// `<uuid>|<expires_at_unix>` using the server-side `NONCE_SECRET`.
#[derive(Debug, Clone, Serialize)]
pub struct NonceResponse {
    pub nonce: String,
    pub expires_at: DateTime<Utc>,
}

/// Parsed contents of a nonce once the HMAC has been verified.
#[derive(Debug, Clone)]
pub struct NoncePayload {
    pub nonce_id: String,
    pub expires_at: DateTime<Utc>,
}
