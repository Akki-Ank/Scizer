//! Job metadata (Postgres, source of truth for status/history) as distinct
//! from job progress/queue state (Redis, ephemeral -- see `queue.rs`).
//! Runtime-checked queries (`query_as`/`query`, not the `query!` macro) on
//! purpose: `query!` needs a live DB reachable at *compile* time (or a
//! checked-in `.sqlx` offline cache), which would make `cargo build` here
//! depend on Postgres being up. Runtime checking costs a class of typo
//! bugs that integration tests catch instead.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct JobRow {
    pub id: Uuid,
    pub api_key_id: Uuid,
    pub codec_domain: String,
    pub codec_name: String,
    pub operation: String,
    pub status: String,
    pub input_key: String,
    pub output_key: Option<String>,
    pub input_bytes: Option<i64>,
    pub output_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Takes `id` explicitly rather than letting Postgres's `gen_random_uuid()`
/// default assign it -- the caller needs the job ID *before* the row
/// exists, to build `input_key` (the S3 object path the presigned upload
/// URL points at). Generating a separate UUID in Rust and letting the DB
/// assign its own would leave the row's real `id` and the UUID embedded in
/// its own `input_key` permanently out of sync.
#[allow(clippy::too_many_arguments)]
pub async fn create_job(
    pool: &PgPool,
    id: Uuid,
    api_key_id: Uuid,
    domain: &str,
    codec: &str,
    operation: &str,
    input_key: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<JobRow> {
    sqlx::query_as::<_, JobRow>(
        "INSERT INTO jobs (id, api_key_id, codec_domain, codec_name, operation, input_key, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING *",
    )
    .bind(id)
    .bind(api_key_id)
    .bind(domain)
    .bind(codec)
    .bind(operation)
    .bind(input_key)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

pub async fn get_job(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<JobRow>> {
    sqlx::query_as::<_, JobRow>("SELECT * FROM jobs WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn mark_queued(pool: &PgPool, id: Uuid, input_bytes: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = 'queued', input_bytes = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(input_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_running(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'running', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_succeeded(
    pool: &PgPool,
    id: Uuid,
    output_key: &str,
    output_bytes: i64,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = 'succeeded', output_key = $2, output_bytes = $3, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(output_key)
    .bind(output_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(pool: &PgPool, id: Uuid, error_message: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error_message = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(error_message)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_expired(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE jobs SET status = 'expired', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Jobs whose retention window has passed and haven't been swept yet --
/// picked up by the worker's idle-tick retention sweep (`worker.rs`).
/// Deliberately excludes `queued`/`running` so the sweep never deletes an
/// in-flight job's input out from under it.
pub async fn find_expired(pool: &PgPool, now: DateTime<Utc>) -> sqlx::Result<Vec<JobRow>> {
    sqlx::query_as::<_, JobRow>(
        "SELECT * FROM jobs WHERE expires_at < $1 AND status IN ('pending', 'succeeded', 'failed')",
    )
    .bind(now)
    .fetch_all(pool)
    .await
}
