//! The S3 data-plane gateway.
//!
//! M0 approach: the gateway wraps `s3s_fs::FileSystem` — the official
//! filesystem-backed implementation of the `s3s::S3` trait — and serves it on
//! the S3 wire protocol via the s3s tower `S3Service` + hyper-util's
//! connection-serving loop. This is the exact serving shape from the upstream
//! `s3s-fs` binary, minus its CLI: `FileSystem` already implements every S3
//! verb the contract needs (ListBuckets / PutObject / GetObject / HeadObject,
//! plus CreateBucket / ListObjects / DeleteObject / multipart), so M0 is a real
//! working subset, not a stub.
//!
//! M0 runs the data plane **anonymously** (no SigV4 auth, no virtual-host
//! domains) so consumers like hanabi's `WebappS3Source` and `aws --no-sign-request`
//! / `curl` can pull bundles directly. Auth (SigV4) is the M1+ concern
//! (theory/ARMAZEM.md — SigV4 on the data plane is orthogonal to saguão on the
//! admin plane).

use std::path::Path;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::server::graceful::GracefulShutdown;
use s3s::service::{S3Service, S3ServiceBuilder};
use s3s_fs::FileSystem;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::config::Backend;

/// Build the s3s tower service for the configured backend.
///
/// M0 only the `fs` arm is reachable; the root dir is created if missing so a
/// fresh PVC works on first boot.
pub fn build_service(backend: &Backend) -> anyhow::Result<S3Service> {
    match backend {
        Backend::Fs { root } => {
            ensure_dir(root)?;
            let fs = FileSystem::new(root)
                .map_err(|e| anyhow::anyhow!("failed to open fs backend at {root:?}: {e:?}"))?;
            let builder = S3ServiceBuilder::new(fs);
            // No auth, no virtual-host domains in M0 — anonymous data plane.
            Ok(builder.build())
        }
    }
}

/// Serve the S3 data plane until `shutdown` is cancelled, then drain in-flight
/// connections (bounded by a 10s grace window).
pub async fn serve(
    listen: std::net::SocketAddr,
    service: S3Service,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!(%local_addr, "armazem S3 data plane listening");

    let http = ConnBuilder::new(TokioExecutor::new());
    let graceful = GracefulShutdown::new();

    loop {
        let (socket, _peer) = tokio::select! {
            res = listener.accept() => match res {
                Ok(conn) => conn,
                Err(err) => {
                    tracing::error!(%err, "error accepting S3 connection");
                    continue;
                }
            },
            () = shutdown.cancelled() => break,
        };

        let conn = http.serve_connection(TokioIo::new(socket), service.clone());
        let conn = graceful.watch(conn.into_owned());
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                tracing::debug!(%err, "S3 connection ended");
            }
        });
    }

    tokio::select! {
        () = graceful.shutdown() => tracing::debug!("S3 data plane drained"),
        () = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
            tracing::warn!("S3 data plane drain timed out after 10s, aborting");
        }
    }

    tracing::info!("armazem S3 data plane stopped");
    Ok(())
}

fn ensure_dir(root: &Path) -> anyhow::Result<()> {
    if !root.exists() {
        std::fs::create_dir_all(root)
            .map_err(|e| anyhow::anyhow!("failed to create backend root {root:?}: {e}"))?;
    }
    Ok(())
}
