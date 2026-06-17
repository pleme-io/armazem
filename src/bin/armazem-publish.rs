//! armázem-publish — the *publish* step of the
//! `git → forge → armazem(S3) → hanabi → browser` demo.
//!
//! Tars + gzips the contents of a local directory into an in-memory `.tar.gz`
//! buffer and uploads it to armazem's S3 gateway as a single object, anonymously
//! over plain HTTP (armazem M0 has no SigV4 auth, so no auth headers are sent).
//!
//! Runtime contract (no CLI args — everything comes from the environment, as the
//! `pangea-cloud-publish` Job sets it):
//!   - `ARMAZEM_ENDPOINT` — S3 gateway base URL, e.g.
//!     `http://armazem.armazem-system.svc:9000` (plain HTTP, no TLS).
//!   - `ARMAZEM_DIR`      — local directory whose CONTENTS are packaged.
//!   - `ARMAZEM_BUCKET`   — S3 bucket (path-style), e.g. `webapps`.
//!   - `ARMAZEM_KEY`      — object key, e.g. `pangea-cloud/coming-soon.tar.gz`.
//!
//! The archive contains the directory's entries at the archive root (e.g.
//! `index.html`, NOT `page/index.html`) — that's what hanabi's `WebappS3Source`
//! expects when it extracts the bundle into its `static_dir`.
//!
//! Exit codes: 0 on a successful upload; non-zero (with a logged status + body)
//! on any HTTP error, so the Job's `OnFailure` restart + `backoffLimit` retries
//! while armazem is still coming up.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use flate2::Compression;
use flate2::write::GzEncoder;

/// How many times to retry a transport-level failure (endpoint not yet up).
/// The Job's `backoffLimit` is the real retry budget; this just smooths the
/// common "armazem is still starting" race within a single invocation.
const CONNECT_RETRIES: u32 = 3;
/// Delay between connect retries.
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// Resolved publish parameters, read once from the environment.
struct PublishConfig {
    /// S3 gateway base URL (no trailing slash).
    endpoint: String,
    /// Local directory whose contents get archived.
    dir: PathBuf,
    /// Target S3 bucket (path-style).
    bucket: String,
    /// Target object key.
    key: String,
}

impl PublishConfig {
    /// Read the four required env vars, failing with a clear typed error if any
    /// is missing or empty.
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            endpoint: required_env("ARMAZEM_ENDPOINT")?
                .trim_end_matches('/')
                .to_owned(),
            dir: PathBuf::from(required_env("ARMAZEM_DIR")?),
            bucket: required_env("ARMAZEM_BUCKET")?,
            key: required_env("ARMAZEM_KEY")?,
        })
    }

    /// `{endpoint}/{bucket}` — the path-style CreateBucket URL.
    fn bucket_url(&self) -> String {
        let mut url = String::with_capacity(self.endpoint.len() + self.bucket.len() + 1);
        url.push_str(&self.endpoint);
        url.push('/');
        url.push_str(&self.bucket);
        url
    }

    /// `{endpoint}/{bucket}/{key}` — the path-style PutObject URL.
    fn object_url(&self) -> String {
        let mut url = self.bucket_url();
        url.push('/');
        url.push_str(&self.key);
        url
    }
}

fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = PublishConfig::from_env()?;

    tracing::info!(
        endpoint = %cfg.endpoint,
        dir = %cfg.dir.display(),
        bucket = %cfg.bucket,
        key = %cfg.key,
        "armazem-publish: packaging and uploading bundle"
    );

    let body = tar_gz_dir(&cfg.dir)
        .with_context(|| anyhow::anyhow!("failed to tar+gzip {}", cfg.dir.display()))?;

    tracing::info!(bytes = body.len(), "armazem-publish: built .tar.gz bundle");

    ensure_bucket(&cfg)?;
    put_object(&cfg, &body)?;

    tracing::info!(
        bucket = %cfg.bucket,
        key = %cfg.key,
        bytes = body.len(),
        "armazem-publish: upload complete"
    );

    Ok(())
}

/// Read a required env var; error (clear message) if missing or empty.
fn required_env(key: &str) -> anyhow::Result<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => anyhow::bail!("required environment variable {key} is missing or empty"),
    }
}

/// Tar + gzip the CONTENTS of `dir` into an in-memory `.tar.gz` byte buffer.
///
/// Entries are stored relative to `dir` (so the archive contains `index.html`,
/// not `<dir>/index.html`). Regular files are followed (the configMap mount is
/// plain files); `tar::Builder::append_dir_all` walks the tree recursively with
/// `""` as the in-archive prefix, which yields exactly that root-relative shape.
fn tar_gz_dir(dir: &Path) -> anyhow::Result<Vec<u8>> {
    if !dir.is_dir() {
        anyhow::bail!("{} is not a directory", dir.display());
    }

    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    // Follow symlinks as the files they point at (configMap projection mounts).
    builder.follow_symlinks(true);

    // "" prefix → entries live at the archive root, relative to `dir`.
    builder
        .append_dir_all("", dir)
        .with_context(|| anyhow::anyhow!("failed to add {} to the archive", dir.display()))?;

    // Finish the tar stream, then finish the gzip stream to flush all bytes.
    let encoder = builder
        .into_inner()
        .context("failed to finalize tar stream")?;
    let buf = encoder.finish().context("failed to finalize gzip stream")?;
    Ok(buf)
}

/// PUT `{endpoint}/{bucket}` to create the bucket. Idempotent: an already-exists
/// response (2xx, or 409 BucketAlreadyOwnedByYou / BucketAlreadyExists) counts as
/// success.
fn ensure_bucket(cfg: &PublishConfig) -> anyhow::Result<()> {
    let url = cfg.bucket_url();
    tracing::info!(%url, "armazem-publish: ensuring bucket exists");

    // An empty body PUT — S3 path-style CreateBucket.
    let outcome = put_with_retries(&url, b"", "application/xml")?;
    match outcome {
        HttpOutcome::Success(status) => {
            tracing::info!(%url, status, "armazem-publish: bucket ready");
            Ok(())
        }
        HttpOutcome::Status(status, body) => {
            // 409 ⇒ bucket already exists / already owned by us — treat as OK.
            if status == 409 {
                tracing::info!(%url, status, "armazem-publish: bucket already exists (idempotent)");
                Ok(())
            } else {
                anyhow::bail!("CreateBucket {url} failed: HTTP {status}: {body}")
            }
        }
    }
}

/// PUT the gzipped tar bytes to `{endpoint}/{bucket}/{key}` as `application/gzip`.
fn put_object(cfg: &PublishConfig, body: &[u8]) -> anyhow::Result<()> {
    let url = cfg.object_url();
    tracing::info!(%url, bytes = body.len(), "armazem-publish: uploading object");

    match put_with_retries(&url, body, "application/gzip")? {
        HttpOutcome::Success(status) => {
            tracing::info!(%url, status, "armazem-publish: object stored");
            Ok(())
        }
        HttpOutcome::Status(status, resp_body) => {
            anyhow::bail!("PutObject {url} failed: HTTP {status}: {resp_body}")
        }
    }
}

/// The shape of a completed HTTP PUT we care about.
enum HttpOutcome {
    /// A 2xx response with this status code.
    Success(u16),
    /// A non-2xx response with this status code and (decoded) body text.
    Status(u16, String),
}

/// PUT `body` to `url` with `Content-Type: content_type`, anonymously (no auth
/// headers). Retries only on transport-level errors (endpoint not yet up); an
/// HTTP status response is returned to the caller to classify.
fn put_with_retries(url: &str, body: &[u8], content_type: &str) -> anyhow::Result<HttpOutcome> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match ureq::put(url)
            .set("Content-Type", content_type)
            .send_bytes(body)
        {
            Ok(resp) => {
                let status = resp.status();
                return Ok(HttpOutcome::Success(status));
            }
            // ureq surfaces a non-2xx HTTP response as Err(Status(..)).
            Err(ureq::Error::Status(status, resp)) => {
                let text = resp
                    .into_string()
                    .unwrap_or_else(|_| "<unreadable body>".to_owned());
                return Ok(HttpOutcome::Status(status, text));
            }
            // Transport error (connection refused, DNS, timeout) — retry a few
            // times for the endpoint coming up, then give up (the Job retries).
            Err(ureq::Error::Transport(transport)) => {
                if attempt >= CONNECT_RETRIES {
                    return Err(anyhow::anyhow!(
                        "PUT {url} failed after {attempt} attempts: {transport}"
                    ));
                }
                tracing::warn!(
                    %url,
                    attempt,
                    %transport,
                    "armazem-publish: transport error, retrying"
                );
                std::thread::sleep(RETRY_DELAY);
            }
        }
    }
}

/// Minimal `tracing` setup. `RUST_LOG` (if set) wins; default is `info`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
