//! Runtime configuration, loaded from env (via `dotenvy` + `std::env`).
//!
//! Secrets land here at deploy time via the deployer-populated `.env`. See
//! `.env.example` for the canonical list.

use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub cors_origin: String,

    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_pass: String,

    pub mail_from: String,
    pub mail_to: String,

    /// Hex-encoded secret used to HMAC-sign nonces. Rotated out-of-band — a
    /// rotation simply invalidates any still-in-flight nonces.
    pub nonce_secret: String,

    pub daily_cap: u32,
    /// Per-IP token-bucket burst size — the count of submissions allowed in
    /// quick succession before tower_governor starts rejecting with 429.
    pub per_ip_burst: u32,
    /// Per-IP token-bucket replenish period in seconds. One slot replenishes
    /// every N seconds; full bucket recovers in `burst * replenish_seconds`.
    pub per_ip_replenish_seconds: u64,

    pub queue_dir: PathBuf,

    /// Optional CDN-injected shared secret. When `Some`, the
    /// `cloudfront_verify_guard` middleware drops any request whose
    /// `X-Origin-Verify` header does not match (silent 204). When `None`,
    /// the layer is a no-op so local dev works without the secret.
    pub cloudfront_verify_secret: Option<String>,

    /// When true, after a successful inquiry send the handler spawns a
    /// fire-and-forget auto-reply to the visitor. Defaults to `true` if
    /// `AUTO_REPLY_ENABLED` is unset; set to `false` to disable without
    /// removing the template files.
    pub auto_reply_enabled: bool,
    /// `From:` address for the auto-reply (e.g. `noreply@cloud-lord.com`).
    /// Required when `auto_reply_enabled` — fail-loud on startup if missing.
    pub auto_reply_from: String,
    /// On-disk path to the HTML auto-reply template. Loaded once at startup;
    /// editing the file requires a process restart to take effect.
    pub auto_reply_html_path: String,
    /// On-disk path to the plain-text auto-reply template (sibling of HTML).
    pub auto_reply_text_path: String,
}

impl Config {
    /// Load configuration from the process environment.
    ///
    /// Callers should have invoked `dotenvy::dotenv()` beforehand (main does).
    pub fn from_env() -> Result<Self> {
        let bind_addr = env_var("BIND_ADDR")?;
        let bind_addr = SocketAddr::from_str(&bind_addr)
            .with_context(|| format!("BIND_ADDR not a valid socket address: {bind_addr}"))?;

        let smtp_port: u16 = env_var("SMTP_PORT")?
            .parse()
            .context("SMTP_PORT must be a u16")?;
        let daily_cap: u32 = env_var("DAILY_CAP")?
            .parse()
            .context("DAILY_CAP must be a u32")?;
        let per_ip_burst: u32 = env_var("PER_IP_BURST")?
            .parse()
            .context("PER_IP_BURST must be a u32")?;
        let per_ip_replenish_seconds: u64 = env_var("PER_IP_REPLENISH_SECONDS")?
            .parse()
            .context("PER_IP_REPLENISH_SECONDS must be a u64")?;

        let auto_reply_enabled = std::env::var("AUTO_REPLY_ENABLED")
            .ok()
            .map(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"))
            .unwrap_or(true);
        let (auto_reply_from, auto_reply_html_path, auto_reply_text_path) = if auto_reply_enabled {
            (
                env_var("AUTO_REPLY_FROM")?,
                env_var("AUTO_REPLY_HTML_PATH")?,
                env_var("AUTO_REPLY_TEXT_PATH")?,
            )
        } else {
            (String::new(), String::new(), String::new())
        };

        Ok(Config {
            bind_addr,
            cors_origin: env_var("CORS_ORIGIN")?,
            smtp_host: env_var("SMTP_HOST")?,
            smtp_port,
            smtp_user: env_var("SMTP_USER")?,
            smtp_pass: env_var("SMTP_PASS")?,
            mail_from: env_var("MAIL_FROM")?,
            mail_to: env_var("MAIL_TO")?,
            nonce_secret: env_var("NONCE_SECRET")?,
            daily_cap,
            per_ip_burst,
            per_ip_replenish_seconds,
            queue_dir: PathBuf::from(env_var("QUEUE_DIR")?),
            cloudfront_verify_secret: std::env::var("CLOUDFRONT_VERIFY_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            auto_reply_enabled,
            auto_reply_from,
            auto_reply_html_path,
            auto_reply_text_path,
        })
    }
}

fn env_var(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("required env var {key} not set"))
}
