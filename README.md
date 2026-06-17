# armázem

**armázem** is the storage substrate of Pangea Cloud: a typed Rust **S3 gateway**
that puts the S3 API in front of any Kubernetes storage backend behind one
control surface (theory/ARMAZEM.md). This repo is the gateway binary. **M0**
ships the `fs` (filesystem) backend — a dedicated PersistentVolume becomes an S3
endpoint — built over the [`s3s`](https://crates.io/crates/s3s) crate and its
official filesystem implementation [`s3s-fs`](https://crates.io/crates/s3s-fs).
The bytes hanabi serves a frontend from and the bytes any pod needs are the same
typed S3 surface. M1+ adds `upstream_s3` (front Garage) / `cnpg_blob` /
`node_disk` / `cas` as new typed backend arms, plus SigV4 on the data plane.

## Run

The binary runs two concurrent servers and shuts both down cleanly on SIGTERM:

- **S3 data plane** on `ARMAZEM_LISTEN` (default `0.0.0.0:9000`), anonymous in
  M0 so `aws --no-sign-request` / `curl` / hanabi's `WebappS3Source` work
  directly. Backed by a filesystem root (`ARMAZEM_BACKEND_ROOT`, default
  `/data`).
- **Health + metrics** on `ARMAZEM_HEALTH_LISTEN` (default `0.0.0.0:9001`):
  `GET /health/live`, `GET /health/ready`, and `GET /metrics` (Prometheus text
  exposing `armazem_up 1`).

```sh
ARMAZEM_BACKEND_ROOT=/tmp/armazem-data cargo run --release
# data plane:    http://127.0.0.1:9000
# health/metrics: http://127.0.0.1:9001/health/live
```

### Configuration

| Env var                 | Default        | Meaning                              |
| ----------------------- | -------------- | ------------------------------------ |
| `ARMAZEM_LISTEN`        | `0.0.0.0:9000` | S3 data-plane listen address         |
| `ARMAZEM_HEALTH_LISTEN` | `0.0.0.0:9001` | health/metrics listen address        |
| `ARMAZEM_BACKEND_TYPE`  | `fs`           | active backend (M0: only `fs`)       |
| `ARMAZEM_BACKEND_ROOT`  | `/data`        | filesystem root for the `fs` backend |
| `ARMAZEM_LOG_LEVEL`     | `info`         | tracing filter (`RUST_LOG` wins)     |

## Build (Nix, no Dockerfile)

```sh
nix build .#dockerImage-amd64   # Nix-built service image → ghcr.io/pleme-io/armazem
```

The image is published on merge by `.github/workflows/auto-release.yml` (★★
AUTO-RELEASE / AUTOBUMP) and deployed via the `pleme-armazem` Helm chart.

## License

MIT — see [LICENSE](./LICENSE).
