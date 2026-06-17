//! armázem — the any-storage→S3 gateway (theory/ARMAZEM.md).
//!
//! M0: a Rust S3 gateway over the `s3s` crate with a filesystem backend. Two
//! concurrent servers share one process and one clean-shutdown signal:
//!   - the S3 data plane on `ARMAZEM_LISTEN` (default `0.0.0.0:9000`)
//!   - the health/metrics sidecar on `ARMAZEM_HEALTH_LISTEN` (default
//!     `0.0.0.0:9001`)
//!
//! A SIGTERM (or Ctrl-C) cancels a shared `CancellationToken`; both servers
//! drain and the process exits cleanly.

mod config;
mod gateway;
mod health;

use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::Config::from_env()?;
    init_tracing(&cfg.log_level);

    tracing::info!(
        listen = %cfg.listen,
        health_listen = %cfg.health_listen,
        backend = ?cfg.backend,
        "starting armazem"
    );

    let service = gateway::build_service(&cfg.backend)?;

    // One token; cancelling it shuts down both servers.
    let shutdown = CancellationToken::new();

    let s3 = tokio::spawn(gateway::serve(cfg.listen, service, shutdown.clone()));
    let health = tokio::spawn(health::serve(cfg.health_listen, shutdown.clone()));
    let signals = tokio::spawn(wait_for_shutdown(shutdown.clone()));

    // If either server exits (error or otherwise) trigger a full shutdown.
    tokio::select! {
        res = s3 => {
            tracing::warn!(?res, "S3 data plane task ended; shutting down");
            shutdown.cancel();
        }
        res = health => {
            tracing::warn!(?res, "health task ended; shutting down");
            shutdown.cancel();
        }
        _ = signals => {
            tracing::info!("shutdown signal received; draining");
        }
    }

    // Give the remaining tasks a moment to drain.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    Ok(())
}

/// Initialize `tracing` at the configured level. The level string is used as the
/// default directive but `RUST_LOG`, if set, takes precedence.
fn init_tracing(level: &str) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// Resolve when SIGTERM or Ctrl-C (SIGINT) arrives, then cancel `shutdown`.
async fn wait_for_shutdown(shutdown: CancellationToken) {
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(%err, "failed to install SIGTERM handler");
            return;
        }
    };

    tokio::select! {
        _ = term.recv() => tracing::info!("received SIGTERM"),
        res = tokio::signal::ctrl_c() => {
            if let Err(err) = res {
                tracing::error!(%err, "failed to listen for Ctrl-C");
            } else {
                tracing::info!("received SIGINT");
            }
        }
    }

    shutdown.cancel();
}
