use async_compression::tokio::write::{GzipDecoder, GzipEncoder};
use async_trait::async_trait;
use sizer_core::{Codec, CodecId, CompressOptions, Error, Format, LimitedWriter, Progress, Result};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{copy_with_progress, effort_to_level};

/// gzip via `flate2` (through `async-compression`). The universal-fallback
/// archive codec — worse ratio than zstd but decodable by essentially
/// every tool that has ever existed, which matters for a "download and
/// hand this to someone else" workflow.
#[derive(Debug, Default)]
pub struct GzipCodec;

const GZIP_MAX_LEVEL: i32 = 9;

#[async_trait]
impl Codec for GzipCodec {
    fn id(&self) -> CodecId {
        CodecId("gzip")
    }

    fn supported_formats(&self) -> &'static [Format] {
        &[Format::Gzip]
    }

    async fn compress(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()> {
        let level = effort_to_level(options.effort, GZIP_MAX_LEVEL);
        let mut encoder =
            GzipEncoder::with_quality(writer, async_compression::Level::Precise(level));
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
        let mut decoder = GzipDecoder::new(limited);
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
        let codec = GzipCodec;
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
        let codec = GzipCodec;
        let input = vec![0u8; 1_000_000]; // highly compressible

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
                    max_decompressed_bytes: 100, // far below the real 1MB
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
}
