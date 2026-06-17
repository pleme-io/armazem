//! Typed runtime configuration, read from the environment.
//!
//! M0 reads the five env vars the `pleme-armazem` Helm chart sets
//! (`helmworks/charts/pleme-armazem/values.yaml` → container env). Every field
//! has a default that matches the chart's default values so the binary runs
//! standalone with no env at all.

use std::net::SocketAddr;
use std::path::PathBuf;

/// The active storage backend. M0 ships exactly the `fs` arm; M1 adds
/// `upstream_s3` / `cnpg_blob` / `node_disk` / `cas` as new arms here
/// (theory/ARMAZEM.md §IV — per-backend capability tier).
#[derive(Debug, Clone)]
pub enum Backend {
    /// Filesystem-rooted S3 store (a PVC mounted at `root`).
    Fs { root: PathBuf },
}

/// Resolved gateway configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// S3 data-plane listen address (`ARMAZEM_LISTEN`, default `0.0.0.0:9000`).
    pub listen: SocketAddr,
    /// Health/metrics listen address (`ARMAZEM_HEALTH_LISTEN`, default
    /// `0.0.0.0:9001`).
    pub health_listen: SocketAddr,
    /// The selected backend (`ARMAZEM_BACKEND_TYPE` + its params).
    pub backend: Backend,
    /// Log filter (`ARMAZEM_LOG_LEVEL`, default `info`).
    pub log_level: String,
}

impl Config {
    /// Read configuration from the environment, applying chart-matching
    /// defaults for any unset key. Returns an error only for a value that is
    /// present but unparseable (a bad `ARMAZEM_LISTEN`, an unknown backend
    /// type) — never for an absent key.
    pub fn from_env() -> anyhow::Result<Self> {
        let listen = parse_addr("ARMAZEM_LISTEN", "0.0.0.0:9000")?;
        let health_listen = parse_addr("ARMAZEM_HEALTH_LISTEN", "0.0.0.0:9001")?;
        let log_level = env_or("ARMAZEM_LOG_LEVEL", "info");

        let backend_type = env_or("ARMAZEM_BACKEND_TYPE", "fs");
        let backend = match backend_type.as_str() {
            "fs" => {
                let root = PathBuf::from(env_or("ARMAZEM_BACKEND_ROOT", "/data"));
                Backend::Fs { root }
            }
            other => {
                anyhow::bail!(
                    "unsupported ARMAZEM_BACKEND_TYPE {other:?} (M0 supports only \"fs\")"
                )
            }
        };

        Ok(Self {
            listen,
            health_listen,
            backend,
            log_level,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => default.to_owned(),
    }
}

fn parse_addr(key: &str, default: &str) -> anyhow::Result<SocketAddr> {
    let raw = env_or(key, default);
    raw.parse::<SocketAddr>()
        .map_err(|e| anyhow::anyhow!("invalid socket address in {key} ({raw:?}): {e}"))
}
