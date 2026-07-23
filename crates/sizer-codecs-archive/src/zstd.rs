use async_compression::tokio::write::{ZstdDecoder, ZstdEncoder};
use async_trait::async_trait;
use sizer_core::{Codec, CodecId, CompressOptions, Error, Format, LimitedWriter, Progress, Result};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{copy_with_progress, effort_to_level};

/// zstd via `libzstd` (through `async-compression`) — Sizer's default
/// archive codec: better ratio and much faster than gzip at equivalent
/// settings. Preferred whenever the recipient's tooling doesn't force
/// gzip/zip compatibility.
#[derive(Debug, Default)]
pub struct ZstdCodec;

const ZSTD_MAX_LEVEL: i32 = 22;

#[async_trait]
impl Codec for ZstdCodec {
    fn id(&self) -> CodecId {
        CodecId("zstd")
    }

    fn supported_formats(&self) -> &'static [Format] {
        &[Format::Zstd]
    }

    async fn compress(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()> {
        let level = effort_to_level(options.effort, ZSTD_MAX_LEVEL);
        let mut encoder =
            ZstdEncoder::with_quality(writer, async_compression::Level::Precise(level));
        copy_with_progress(reader, &mut encoder, None, progress).await?;
        encoder.shutdown().await.map_err(Error::Io)?;
        Ok(())
    }

    async fn decompress(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()> {
        let limited = LimitedWriter::new(writer, options.max_decompressed_bytes);
        let mut decoder = ZstdDecoder::new(limited);
        copy_with_progress(reader, &mut decoder, None, progress)
            .await
            .map_err(Error::unwrap_io)?;
        decoder
            .shutdown()
            .await
            .map_err(|e| Error::unwrap_io(Error::Io(e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sizer_core::NullProgress;
    use std::io::Cursor;

    #[tokio::test]
    async fn round_trip_preserves_content() {
        let codec = ZstdCodec;
        let input = b"the quick brown fox jumps over the lazy dog".repeat(100);

        let mut compressed = Vec::new();
        codec
            .compress(
                &mut Cursor::new(&input),
                &mut compressed,
                &CompressOptions {
                    effort: 50,
                    ..Default::default()
                },
                &NullProgress,
            )
            .await
            .unwrap();
        assert!(compressed.len() < input.len());

        let mut output = Vec::new();
        codec
            .decompress(
                &mut Cursor::new(&compressed),
                &mut output,
                &CompressOptions {
                    max_decompressed_bytes: input.len() as u64 + 1,
                    ..Default::default()
                },
                &NullProgress,
            )
            .await
            .unwrap();

        assert_eq!(output, input);
    }

    #[tokio::test]
    async fn decompression_bomb_guard_trips() {
        let codec = ZstdCodec;
        let input = vec![0u8; 1_000_000];

        let mut compressed = Vec::new();
        codec
            .compress(
                &mut Cursor::new(&input),
                &mut compressed,
                &CompressOptions {
                    effort: 100,
                    ..Default::default()
                },
                &NullProgress,
            )
            .await
            .unwrap();

        let mut output = Vec::new();
        let result = codec
            .decompress(
                &mut Cursor::new(&compressed),
                &mut output,
                &CompressOptions {
                    max_decompressed_bytes: 100,
                    ..Default::default()
                },
                &NullProgress,
            )
            .await;

        assert!(matches!(
            result,
            Err(Error::ExpansionLimitExceeded { limit: 100 })
        ));
    }

    #[tokio::test]
    async fn beats_gzip_ratio_on_repetitive_input() {
        use crate::GzipCodec;

        let input = b"the quick brown fox jumps over the lazy dog".repeat(1000);
        let options = CompressOptions {
            effort: 80,
            ..Default::default()
        };

        let mut zstd_out = Vec::new();
        ZstdCodec
            .compress(
                &mut Cursor::new(&input),
                &mut zstd_out,
                &options,
                &NullProgress,
            )
            .await
            .unwrap();

        let mut gzip_out = Vec::new();
        GzipCodec
            .compress(
                &mut Cursor::new(&input),
                &mut gzip_out,
                &options,
                &NullProgress,
            )
            .await
            .unwrap();

        assert!(
            zstd_out.len() <= gzip_out.len(),
            "zstd ({} bytes) should not lose to gzip ({} bytes) on this input",
            zstd_out.len(),
            gzip_out.len()
        );
    }
}
