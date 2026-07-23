# sizer-cloud

Cloud API + worker (M6): the fourth surface over `sizer-core`, alongside
the CLI, Tauri desktop app, and WASM browser build. Off by default, opt-in
-- see `docs/ARCHITECTURE.md`'s "Cloud mode is opt-in, not core".

Two binaries, one process each:

- `sizer-cloud-api` -- Axum HTTP server. Auth, job bookkeeping, issues
  presigned upload/download URLs. Never touches file bytes itself.
- `sizer-cloud-worker` -- dequeues jobs from Redis, downloads the input,
  runs the codec, uploads the result. Run one or more of these; they're
  stateless and safe to scale horizontally.

## Local dev services

No Docker/WSL2 dependency -- three native Windows services instead:

| Service | Local dev | Production target |
|---|---|---|
| Postgres | PostgreSQL 17 (winget `PostgreSQL.PostgreSQL.17`) | Neon / Supabase |
| Redis | Memurai Developer (winget `Memurai.MemuraiDeveloper`) | Upstash |
| S3-compatible storage | MinIO Server (winget `MinIO.Server`) | Cloudflare R2 |

All three are accessed through the standard Postgres/Redis/S3 client
protocols -- nothing in this crate is provider-specific. Swapping in the
production services is a config change, not a code change.

### One-time setup

```powershell
# Postgres: create the database sizer-cloud will connect to
& "C:\Program Files\PostgreSQL\17\bin\createdb.exe" -U postgres sizer_cloud

# MinIO: start the server (defaults to :9000 API / :9001 console),
# then create the bucket sizer-cloud writes to
minio server C:\minio-data
# in another shell, using the MinIO client (winget install MinIO.Client):
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/sizer-cloud

# Memurai installs and starts as a Windows service automatically; confirm:
Get-Service Memurai
```

### `.env` (copy to `crates/sizer-cloud/.env`, loaded via `dotenvy`)

```
DATABASE_URL=postgres://postgres:<password>@localhost:5432/sizer_cloud
REDIS_URL=redis://127.0.0.1:6379
S3_ENDPOINT=http://127.0.0.1:9000
S3_REGION=us-east-1
S3_BUCKET=sizer-cloud
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
S3_FORCE_PATH_STYLE=true
LISTEN_ADDR=127.0.0.1:8080
MAX_UPLOAD_BYTES=5368709120
MAX_DECOMPRESSED_BYTES=21474836480
JOB_RETENTION_SECS=86400
JOB_WALL_CLOCK_TIMEOUT_SECS=1800
```

For Cloudflare R2 in production: `S3_ENDPOINT` is the account's R2 S3 API
endpoint, `S3_FORCE_PATH_STYLE=false`, credentials are an R2 API token.

### Running it

```powershell
# Migrations run automatically on startup (sqlx::migrate!), but keygen
# also runs them so it works standalone against a fresh database.
cargo run -p sizer-cloud --bin sizer-cloud-keygen -- "local-dev" 5368709120
# -> prints a raw API key once; save it, it's not retrievable again

cargo run -p sizer-cloud --bin sizer-cloud-api
cargo run -p sizer-cloud --bin sizer-cloud-worker   # separate terminal
```

### Example job (archive/gzip)

```bash
API_KEY=sk_...
BASE=http://127.0.0.1:8080

# 1. Create the job, get a presigned upload URL
create=$(curl -s -X POST "$BASE/v1/jobs" \
  -H "X-API-Key: $API_KEY" -H "Content-Type: application/json" \
  -d '{"domain":"archive","codec":"gzip","operation":"compress"}')
job_id=$(echo "$create" | jq -r .job_id)
upload_url=$(echo "$create" | jq -r .upload_url)

# 2. Upload the file bytes directly to storage
curl -s -X PUT "$upload_url" --data-binary @some-file.tar

# 3. Tell the API the upload landed -- this queues the job
curl -s -X POST "$BASE/v1/jobs/$job_id/submit" -H "X-API-Key: $API_KEY"

# 4. Poll status; once "succeeded", download_url is a presigned GET
curl -s "$BASE/v1/jobs/$job_id" -H "X-API-Key: $API_KEY"
```

## What's implemented vs. deferred

Implemented: API-key auth, presigned-URL upload/download (API never
buffers file bytes), Postgres job metadata, Redis job queue +
progress/cancel state, per-job wall-clock timeout, decompression-bomb
guard (`CompressOptions::max_decompressed_bytes`, already enforced in
`sizer-core`), retention sweep (deletes expired input/output objects and
marks the job row `expired`).

Deferred, not implemented by this crate:

- **`video`/`document` job domains.** `VideoCodec` and `DocumentCodec`
  have different shapes (file-path-based, or no progress callback) than
  what the worker currently wires up (`archive`'s streaming `Codec` and
  `image`'s in-memory `ImageCodec`). Adding them is a worker-side
  dispatch change, not an architecture change.
- **Per-caller quota enforcement.** `api_keys.quota_bytes_per_day` exists
  in the schema and is loaded into `ApiKeyContext`, but nothing currently
  checks a caller's rolling usage against it before accepting a job.
- **Process-level sandboxing (seccomp + cgroups) and real per-job
  CPU/memory limits.** These are container/orchestrator-level concerns for
  whatever actually runs `sizer-cloud-worker` in production (e.g. resource
  limits on the container, not something this Rust binary self-enforces)
  and aren't meaningful to set up against a bare Windows dev process. The
  wall-clock timeout here is the only enforced limit today.
- **Nesting-depth limits on archive-within-archive extraction.** Not yet
  implemented anywhere in `sizer-core`/`sizer-codecs-archive`; a
  decompression bomb built from nested archives would only be caught by
  the existing byte-expansion guard on the outermost layer today.
