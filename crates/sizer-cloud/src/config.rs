use std::net::SocketAddr;

/// All cloud config comes from the environment, never hardcoded -- see
/// docs/ARCHITECTURE.md's M6 decisions note: the code is provider-agnostic
/// (standard Postgres/Redis/S3 clients), so only connection strings and
/// credentials differ between local dev (native Postgres + Memurai + MinIO)
/// and the real deploy target (Neon/Supabase + Upstash + Cloudflare R2).
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,

    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    /// MinIO (and most self-hosted S3-compatible servers) need path-style
    /// addressing (`endpoint/bucket/key`); R2 and AWS itself use
    /// virtual-hosted style (`bucket.endpoint/key`). Configurable per
    /// deploy target rather than assumed.
    pub s3_force_path_style: bool,

    pub listen_addr: SocketAddr,

    /// Hard cap on upload size, enforced by checking the S3 object's
    /// reported size before a job is queued (uploads go straight to
    /// storage via a presigned URL, so the API process never buffers the
    /// bytes itself to check this).
    pub max_upload_bytes: u64,
    /// Passed through to `CompressOptions::max_decompressed_bytes` for
    /// every job -- decompression-bomb defense, see
    /// docs/ARCHITECTURE.md "Decompression-bomb defense".
    pub max_decompressed_bytes: u64,
    /// How long a completed (or abandoned) job's input/output objects and
    /// DB row are kept before the worker's retention sweep deletes them.
    pub job_retention_secs: i64,
    /// Wall-clock ceiling on a single job's codec run. Not a substitute
    /// for real per-job CPU/memory quotas and process sandboxing (seccomp
    /// and cgroups) -- those are container/orchestrator-level concerns for
    /// the real deploy target and are not implemented by this binary; see
    /// docs/ARCHITECTURE.md "Decompression-bomb defense" for what's still
    /// open.
    pub job_wall_clock_timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        fn required(name: &str) -> anyhow::Result<String> {
            std::env::var(name).map_err(|_| anyhow::anyhow!("missing required env var {name}"))
        }
        fn optional(name: &str, default: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| default.to_string())
        }

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,

            s3_endpoint: required("S3_ENDPOINT")?,
            s3_region: optional("S3_REGION", "us-east-1"),
            s3_bucket: required("S3_BUCKET")?,
            s3_access_key: required("S3_ACCESS_KEY")?,
            s3_secret_key: required("S3_SECRET_KEY")?,
            s3_force_path_style: optional("S3_FORCE_PATH_STYLE", "true").parse().unwrap_or(true),

            listen_addr: optional("LISTEN_ADDR", "127.0.0.1:8080").parse()?,

            max_upload_bytes: optional("MAX_UPLOAD_BYTES", "5368709120").parse()?, // 5 GiB
            max_decompressed_bytes: optional("MAX_DECOMPRESSED_BYTES", "21474836480").parse()?, // 20 GiB
            job_retention_secs: optional("JOB_RETENTION_SECS", "86400").parse()?, // 24h
            job_wall_clock_timeout_secs: optional("JOB_WALL_CLOCK_TIMEOUT_SECS", "1800").parse()?, // 30 min
        })
    }
}
