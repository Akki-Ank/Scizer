//! S3-compatible object storage: MinIO locally, Cloudflare R2 in
//! production, plain `aws-sdk-s3` either way (see docs/ARCHITECTURE.md's
//! M6 decisions -- both speak the standard S3 API, so only endpoint/creds
//! change between them).
//!
//! File bytes never round-trip through the API process: callers upload
//! and download directly against presigned URLs. The worker, which does
//! need real bytes to run a codec, downloads/uploads through a local temp
//! file rather than wiring S3's `ByteStream` into `sizer_core::Codec`'s
//! `AsyncRead`/`AsyncWrite` directly -- simpler and still bounded, single-
//! file-at-a-time memory (never the whole object in RAM), matching the
//! "never load 100GB into memory" constraint without the extra complexity
//! of a true streaming S3 bridge.

use std::path::Path;
use std::time::Duration;

use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use tokio::io::AsyncWriteExt;

use crate::config::Config;

pub struct Storage {
    client: Client,
    bucket: String,
}

impl Storage {
    pub fn new(config: &Config) -> Self {
        let credentials = Credentials::new(
            &config.s3_access_key,
            &config.s3_secret_key,
            None,
            None,
            "sizer-cloud",
        );
        let s3_config = aws_sdk_s3::Config::builder()
            .region(Region::new(config.s3_region.clone()))
            .endpoint_url(&config.s3_endpoint)
            .credentials_provider(credentials)
            .force_path_style(config.s3_force_path_style)
            .behavior_version(aws_sdk_s3::config::BehaviorVersion::latest())
            .build();
        Self {
            client: Client::from_conf(s3_config),
            bucket: config.s3_bucket.clone(),
        }
    }

    pub async fn presign_put(&self, key: &str, ttl: Duration) -> anyhow::Result<String> {
        let presigning = PresigningConfig::expires_in(ttl)?;
        let req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await?;
        Ok(req.uri().to_string())
    }

    pub async fn presign_get(&self, key: &str, ttl: Duration) -> anyhow::Result<String> {
        let presigning = PresigningConfig::expires_in(ttl)?;
        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presigning)
            .await?;
        Ok(req.uri().to_string())
    }

    /// Object size in bytes, used to confirm an upload landed (and check
    /// it against `max_upload_bytes`) before a job is queued.
    pub async fn head_object_size(&self, key: &str) -> anyhow::Result<u64> {
        let resp = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(resp.content_length().unwrap_or(0).max(0) as u64)
    }

    pub async fn download_to_file(&self, key: &str, dest: &Path) -> anyhow::Result<()> {
        let mut obj = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        let mut file = tokio::fs::File::create(dest).await?;
        while let Some(chunk) = obj.body.try_next().await? {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }

    /// Streams `src` off disk (`ByteStream::from_path` reads it in chunks,
    /// not all at once), never buffering the whole file in memory.
    pub async fn upload_from_file(&self, key: &str, src: &Path) -> anyhow::Result<()> {
        let body = ByteStream::from_path(src).await?;
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(body)
            .send()
            .await?;
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(())
    }
}
