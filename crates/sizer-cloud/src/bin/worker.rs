use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use sizer_cloud::config::Config;
use sizer_cloud::jobs::{self, JobRow};
use sizer_cloud::progress::RedisProgress;
use sizer_cloud::db;
use sizer_cloud::queue::Queue;
use sizer_cloud::storage::Storage;
use sizer_core::{CompressOptions, Error as CoreError};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let config = Arc::new(Config::from_env()?);
    let db_pool = db::connect(&config.database_url).await?;
    let queue = Queue::connect(&config.redis_url).await?;
    let storage = Arc::new(Storage::new(&config));

    tracing::info!("sizer-cloud-worker started");

    loop {
        match queue.dequeue_blocking(Duration::from_secs(5)).await {
            Ok(Some(job_id)) => {
                tracing::info!(%job_id, "dequeued job");
                if let Err(err) =
                    process_job(&db_pool, &queue, &storage, &config, job_id).await
                {
                    tracing::error!(%job_id, error = %err, "job failed");
                    if let Err(db_err) =
                        jobs::mark_failed(&db_pool, job_id, &err.to_string()).await
                    {
                        tracing::error!(%job_id, error = %db_err, "failed to record job failure");
                    }
                }
            }
            Ok(None) => {
                // Idle tick: no job showed up within the BRPOP timeout --
                // use the gap to sweep expired jobs instead of a separate
                // cron-like process.
                if let Err(err) = sweep_expired(&db_pool, &storage, config.job_retention_secs).await
                {
                    tracing::warn!(error = %err, "retention sweep failed");
                }
            }
            Err(err) => {
                tracing::error!(error = %err, "dequeue failed, retrying after backoff");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn process_job(
    db_pool: &PgPool,
    queue: &Queue,
    storage: &Storage,
    config: &Config,
    job_id: Uuid,
) -> anyhow::Result<()> {
    let job = jobs::get_job(db_pool, job_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("job {job_id} not found"))?;

    jobs::mark_running(db_pool, job_id).await?;

    let tmp_dir = std::env::temp_dir().join(format!("sizer-cloud-job-{job_id}"));
    tokio::fs::create_dir_all(&tmp_dir).await?;
    let input_path = tmp_dir.join("input");
    let output_path = tmp_dir.join("output");

    // Always clean up the temp dir, whether the job succeeded, failed, or
    // timed out -- a guard rather than duplicating the remove_dir_all call
    // at every early-return site.
    let cleanup = TempDirGuard(tmp_dir.clone());

    let result = run_job(&job, storage, queue, config, &input_path, &output_path).await;

    let output_bytes = match &result {
        Ok(()) => Some(tokio::fs::metadata(&output_path).await?.len()),
        Err(_) => None,
    };

    if let (Ok(()), Some(output_bytes)) = (&result, output_bytes) {
        let output_key = format!("results/{}/{job_id}/output", job.api_key_id);
        storage.upload_from_file(&output_key, &output_path).await?;
        jobs::mark_succeeded(db_pool, job_id, &output_key, output_bytes as i64).await?;
        // Input is no longer needed once the output is durably stored.
        let _ = storage.delete(&job.input_key).await;
        tracing::info!(%job_id, output_bytes, "job succeeded");
    }

    drop(cleanup);
    result
}

struct TempDirGuard(std::path::PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let path = self.0.clone();
        // Drop can't be async; spawn the removal as a detached task rather
        // than blocking this thread synchronously, same "never stall the
        // runtime" discipline RedisProgress's channel hand-off follows.
        tokio::spawn(async move {
            if let Err(err) = tokio::fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), error = %err, "temp dir cleanup failed");
            }
        });
    }
}

async fn run_job(
    job: &JobRow,
    storage: &Storage,
    queue: &Queue,
    config: &Config,
    input_path: &Path,
    output_path: &Path,
) -> anyhow::Result<()> {
    storage.download_to_file(&job.input_key, input_path).await?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let progress = RedisProgress::spawn(queue.clone(), job.id, cancelled.clone());
    let cancel_poller = spawn_cancel_poller(queue.clone(), job.id, cancelled);

    let options = CompressOptions {
        target_bytes: None,
        effort: 50,
        max_decompressed_bytes: config.max_decompressed_bytes,
    };

    let run = run_codec(job, input_path, output_path, &options, &progress);
    let timeout = Duration::from_secs(config.job_wall_clock_timeout_secs);
    let outcome = tokio::time::timeout(timeout, run).await;

    cancel_poller.abort();

    match outcome {
        Ok(inner) => inner,
        Err(_) => Err(anyhow::anyhow!(
            "job exceeded wall-clock limit of {}s",
            config.job_wall_clock_timeout_secs
        )),
    }
}

/// Dispatches to the right codec shape for the job's domain. Only
/// `archive` (streaming `Codec`) and `image` (in-memory `ImageCodec`) are
/// wired up -- `video`/`document` use different codec shapes (file-path,
/// no-progress-param) not yet plumbed through the cloud worker; see
/// docs/ARCHITECTURE.md's M6 note.
async fn run_codec(
    job: &JobRow,
    input_path: &Path,
    output_path: &Path,
    options: &CompressOptions,
    progress: &RedisProgress,
) -> anyhow::Result<()> {
    match job.codec_domain.as_str() {
        "archive" => {
            let codec = sizer_registry::codec_by_name(&job.codec_name)?;
            let mut reader = tokio::fs::File::open(input_path).await?;
            let mut writer = tokio::fs::File::create(output_path).await?;
            match job.operation.as_str() {
                "compress" => codec
                    .compress(&mut reader, &mut writer, options, progress)
                    .await
                    .map_err(CoreError::unwrap_io)?,
                "decompress" => codec
                    .decompress(&mut reader, &mut writer, options, progress)
                    .await
                    .map_err(CoreError::unwrap_io)?,
                other => anyhow::bail!("unsupported operation {other:?} for archive domain"),
            }
            writer.flush().await?;
        }
        "image" => {
            let codec = sizer_registry::image_codec_by_name(&job.codec_name)?;
            let input_bytes = tokio::fs::read(input_path).await?;
            let output_bytes = codec.recompress(input_bytes, options).await?;
            tokio::fs::write(output_path, output_bytes).await?;
        }
        other => anyhow::bail!(
            "unsupported codec domain {other:?} (supported: archive, image)"
        ),
    }
    Ok(())
}

fn spawn_cancel_poller(
    queue: Queue,
    job_id: Uuid,
    flag: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match queue.is_cancel_requested(job_id).await {
                Ok(true) => {
                    flag.store(true, Ordering::Relaxed);
                    break;
                }
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(%job_id, error = %err, "cancel poll failed");
                }
            }
        }
    })
}

async fn sweep_expired(
    db_pool: &PgPool,
    storage: &Storage,
    _retention_secs: i64,
) -> anyhow::Result<()> {
    let expired = jobs::find_expired(db_pool, Utc::now()).await?;
    for row in expired {
        let _ = storage.delete(&row.input_key).await;
        if let Some(output_key) = &row.output_key {
            let _ = storage.delete(output_key).await;
        }
        jobs::mark_expired(db_pool, row.id).await?;
        tracing::info!(job_id = %row.id, "swept expired job");
    }
    Ok(())
}
