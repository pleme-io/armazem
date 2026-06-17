//! The health + metrics sidecar server.
//!
//! Serves on the health port (`ARMAZEM_HEALTH_LISTEN`, default `0.0.0.0:9001`),
//! exactly the three endpoints the `pleme-armazem` chart's probes +
//! ServiceMonitor expect:
//!   - `GET /health/live`  → 200 (liveness)
//!   - `GET /health/ready` → 200 (readiness)
//!   - `GET /metrics`      → Prometheus text exposing `armazem_up 1`

use std::net::SocketAddr;

use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio_util::sync::CancellationToken;

/// Build the health/metrics router.
pub fn router() -> Router {
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/metrics", get(metrics))
}

/// Serve the health server until `shutdown` is cancelled.
pub async fn serve(listen: SocketAddr, shutdown: CancellationToken) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!(%local_addr, "armazem health/metrics listening");

    axum::serve(listener, router())
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;

    tracing::info!("armazem health/metrics stopped");
    Ok(())
}

async fn live() -> impl IntoResponse {
    "ok"
}

async fn ready() -> impl IntoResponse {
    "ok"
}

/// Minimal Prometheus exposition. M0 exports a single `armazem_up` gauge so the
/// chart's ServiceMonitor scrape lands a real series; richer S3-op metrics are
/// an M1 concern.
async fn metrics() -> impl IntoResponse {
    let body = "# HELP armazem_up 1 if the armazem gateway process is up.\n\
                # TYPE armazem_up gauge\n\
                armazem_up 1\n";
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}
