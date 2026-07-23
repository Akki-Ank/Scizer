use std::time::Duration;

use axum::extract::{Path, State};
use axum::Extension;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::ApiKeyContext;
use crate::error::ApiError;
use crate::jobs::{self, JobRow};

/// Presigned-URL TTLs: generous enough for a large-file upload/download
/// over a slow link, short enough not to leave a long-lived open door on
/// an object that (per retention policy) won't exist much longer anyway.
const PRESIGN_TTL: Duration = Duration::from_secs(900);

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    /// "archive" | "image" -- video/document domains are a follow-up (see
    /// docs/ARCHITECTURE.md's M6 note): both have a different codec shape
    /// (file-path-based / no-progress-param) that the worker doesn't wire
    /// up yet.
    pub domain: String,
    /// "gzip" | "zstd" for archive, "png" | "jpeg" for image.
    pub codec: String,
    /// "compress" | "decompress" for archive, "recompress" for image.
    pub operation: String,
}

fn validate_request(req: &CreateJobRequest) -> Result<(), ApiError> {
    match (req.domain.as_str(), req.operation.as_str(), req.codec.as_str()) {
        ("archive", "compress" | "decompress", "gzip" | "zstd") => Ok(()),
        ("image", "recompress", "png" | "jpeg" | "jpg") => Ok(()),
        _ => Err(ApiError::BadRequest(format!(
            "unsupported combination domain={:?} operation={:?} codec={:?} \
             (supported: archive/compress|decompress/gzip|zstd, image/recompress/png|jpeg)",
            req.domain, req.operation, req.codec
        ))),
    }
}

#[derive(Debug, Serialize)]
pub struct CreateJobResponse {
    pub job_id: Uuid,
    /// Caller PUTs the file bytes here directly -- the API process never
    /// buffers the upload itself.
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

pub async fn create_job(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    validate_request(&req)?;

    let job_id = Uuid::new_v4();
    let input_key = format!("uploads/{}/{job_id}/input", key.id);
    let expires_at = Utc::now() + chrono::Duration::seconds(state.config.job_retention_secs);

    let job = jobs::create_job(
        &state.db,
        job_id,
        key.id,
        &req.domain,
        &req.codec,
        &req.operation,
        &input_key,
        expires_at,
    )
    .await?;

    let upload_url = state
        .storage
        .presign_put(&input_key, PRESIGN_TTL)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(CreateJobResponse {
        job_id: job.id,
        upload_url,
        expires_at: job.expires_at,
    }))
}

/// Caller calls this once their PUT to `upload_url` has completed. Checks
/// the object actually landed (and its size) before handing the job to the
/// worker -- queuing a job whose input was never uploaded would just fail
/// in the worker with a less useful error.
pub async fn submit_job(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatusResponse>, ApiError> {
    let job = load_owned_job(&state, &key, id).await?;

    if job.status != "pending" {
        return Err(ApiError::BadRequest(format!(
            "job is {} not pending",
            job.status
        )));
    }

    let size = state
        .storage
        .head_object_size(&job.input_key)
        .await
        .map_err(|_| ApiError::BadRequest("input has not been uploaded yet".to_string()))?;
    if size > state.config.max_upload_bytes {
        return Err(ApiError::BadRequest(format!(
            "input is {size} bytes, exceeds max_upload_bytes ({})",
            state.config.max_upload_bytes
        )));
    }

    jobs::mark_queued(&state.db, id, size as i64).await?;
    state.queue.enqueue(id).await.map_err(ApiError::Internal)?;

    let job = jobs::get_job(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(JobStatusResponse::from_row(&job, None, None)))
}

pub async fn get_job(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatusResponse>, ApiError> {
    let job = load_owned_job(&state, &key, id).await?;

    let progress = if job.status == "running" {
        state.queue.get_progress(id).await.unwrap_or(None)
    } else {
        None
    };

    let download_url = match (&job.status[..], &job.output_key) {
        ("succeeded", Some(output_key)) => Some(
            state
                .storage
                .presign_get(output_key, PRESIGN_TTL)
                .await
                .map_err(ApiError::Internal)?,
        ),
        _ => None,
    };

    Ok(Json(JobStatusResponse::from_row(&job, progress, download_url)))
}

pub async fn cancel_job(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatusResponse>, ApiError> {
    let job = load_owned_job(&state, &key, id).await?;

    if matches!(job.status.as_str(), "succeeded" | "failed" | "expired") {
        return Err(ApiError::BadRequest(format!(
            "job already {}",
            job.status
        )));
    }

    state
        .queue
        .request_cancel(id)
        .await
        .map_err(ApiError::Internal)?;
    jobs::mark_failed(&state.db, id, "cancelled by caller").await?;

    let job = jobs::get_job(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    Ok(Json(JobStatusResponse::from_row(&job, None, None)))
}

/// Fetches a job and checks ownership in one place -- a 404 (not a 403)
/// for another caller's job ID, so job IDs don't become an enumeration
/// oracle for what other API keys are doing.
async fn load_owned_job(
    state: &AppState,
    key: &ApiKeyContext,
    id: Uuid,
) -> Result<JobRow, ApiError> {
    let job = jobs::get_job(&state.db, id).await?.ok_or(ApiError::NotFound)?;
    if job.api_key_id != key.id {
        return Err(ApiError::NotFound);
    }
    Ok(job)
}

#[derive(Debug, Serialize)]
pub struct ProgressInfo {
    pub processed: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct JobStatusResponse {
    pub job_id: Uuid,
    pub status: String,
    pub domain: String,
    pub codec: String,
    pub operation: String,
    pub input_bytes: Option<i64>,
    pub output_bytes: Option<i64>,
    pub error: Option<String>,
    pub progress: Option<ProgressInfo>,
    pub download_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl JobStatusResponse {
    fn from_row(
        row: &JobRow,
        progress: Option<(u64, Option<u64>)>,
        download_url: Option<String>,
    ) -> Self {
        Self {
            job_id: row.id,
            status: row.status.clone(),
            domain: row.codec_domain.clone(),
            codec: row.codec_name.clone(),
            operation: row.operation.clone(),
            input_bytes: row.input_bytes,
            output_bytes: row.output_bytes,
            error: row.error_message.clone(),
            progress: progress.map(|(processed, total)| ProgressInfo { processed, total }),
            download_url,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}
