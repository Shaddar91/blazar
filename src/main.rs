//! Blazar entrypoint.
//CI5: end-to-end auto-deploy verification (push→ECR→server-pull).

use std::{net::SocketAddr, sync::Arc, time::Duration as StdDuration};

use anyhow::Result;
use axum::{
    routing::{get, post},
    Router,
};
use chrono::{Duration, Utc};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod config;
mod errors;
mod extractors;
mod handlers;
mod middleware;
mod models;
mod nonce;
mod queue;
mod smtp;

use crate::{
    config::Config,
    handlers::{health::health, nonce::issue_nonce, send::send},
    smtp::{LoopbackSmtpBackend, SmtpBackend},
};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let _ = dotenvy::dotenv();

    let cfg = Arc::new(Config::from_env()?);
    tracing::info!(bind_addr = %cfg.bind_addr, cors_origin = %cfg.cors_origin, "blazar starting");

    spawn_midnight_flush(cfg.clone());

    let app = build_router(cfg.clone());

    let listener = TcpListener::bind(cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, "listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn build_router(cfg: Arc<Config>) -> Router {
    let cors = middleware::cors_layer(&cfg.cors_origin);

    // Per-IP token bucket: PER_IP_BURST is the bucket size, PER_IP_REPLENISH_SECONDS
    // is how often one slot is added back. With burst=3 + replenish=6000s, an IP
    // gets 3 quick submissions then must wait 100min per slot — full recovery in 5h.
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(cfg.per_ip_replenish_seconds.max(1))
        .burst_size(cfg.per_ip_burst.max(1))
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("valid governor config");

    let send_limited =
        post(send).layer(ServiceBuilder::new().layer(GovernorLayer::new(governor_conf)));

    Router::new()
        .route("/health", get(health))
        .route("/nonce", get(issue_nonce))
        .route("/send", send_limited)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn_with_state(
            cfg.clone(),
            middleware::cloudfront_verify_guard,
        ))
        .with_state(cfg)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("blazar=info,tower_http=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json())
        .init();
}

/// Spawn the midnight-UTC queue flusher. Computes the duration to the next 00:00 UTC,
/// sleeps, then drains the queue through the SMTP backend, loops.
fn spawn_midnight_flush(cfg: Arc<Config>) {
    tokio::spawn(async move {
        tracing::info!(
            queue_dir = %cfg.queue_dir.display(),
            "midnight-flush task spawned"
        );
        loop {
            let now = Utc::now();
            let tomorrow = (now + Duration::days(1)).date_naive();
            let next_midnight = tomorrow.and_hms_opt(0, 0, 0).unwrap().and_utc();
            let sleep_for = (next_midnight - now).to_std().unwrap_or(StdDuration::from_secs(60));
            tokio::time::sleep(sleep_for).await;

            let drained = match queue::flush_all(&cfg.queue_dir) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(error = ?e, "flush_all failed");
                    continue;
                }
            };
            tracing::info!(count = drained.len(), "midnight flush start");
            let backend = LoopbackSmtpBackend::from_config(&cfg);
            for msg in drained {
                if let Err(e) = SmtpBackend::send(&backend, &msg).await {
                    tracing::error!(id = %msg.id, error = ?e, "midnight send failed — re-enqueueing");
                    let _ = queue::enqueue(&cfg.queue_dir, &msg);
                }
            }
        }
    });
}
