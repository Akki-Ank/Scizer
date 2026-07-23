use async_trait::async_trait;
use jpeg_encoder::{ColorType, Encoder};
use sizer_core::{CompressOptions, Error, Result};

use crate::{run_blocking, ImageCodec};

/// Lossy JPEG re-encode via `jpeg-encoder` (pure Rust). Decodes the input
/// (any format the `image` crate understands, not just JPEG — recompress
/// is really "re-encode as JPEG at this quality") and writes it back out
/// at a chosen quality.
///
/// `options.effort` is repurposed as JPEG quality (1..=100) directly,
/// *not* run through [`crate::effort_to_preset`] like PNG's optimization
/// level: quality is a continuous size/fidelity trade-off, not an
/// encoder-speed knob, so mapping it onto a small preset range would
/// throw away precision the caller (or the eventual target-size
/// convergence loop) actually needs.
#[derive(Debug, Default)]
pub struct JpegCodec;

#[async_trait]
impl ImageCodec for JpegCodec {
    fn name(&self) -> &'static str {
        "jpeg"
    }

    fn is_lossless(&self) -> bool {
        false
    }

    async fn recompress(&self, input: Vec<u8>, options: &CompressOptions) -> Result<Vec<u8>> {
        let quality = options.effort.clamp(1, 100);

        run_blocking(move || {
            let decoded = image::load_from_memory(&input)
                .map_err(|e| Error::UnsupportedFormat(format!("decoding image: {e}")))?
                .to_rgb8();
            let (width, height) = decoded.dimensions();
            let (width, height) = (
                u16::try_from(width).map_err(|_| {
                    Error::UnsupportedFormat("image too wide for JPEG (max 65535px)".into())
                })?,
                u16::try_from(height).map_err(|_| {
                    Error::UnsupportedFormat("image too tall for JPEG (max 65535px)".into())
                })?,
            );

            let mut out = Vec::new();
            Encoder::new(&mut out, quality)
                .encode(decoded.as_raw(), width, height, ColorType::Rgb)
                .map_err(|e| Error::UnsupportedFormat(format!("jpeg-encoder: {e}")))?;
            Ok(out)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare_pixels;

    fn tiny_source_png() -> Vec<u8> {
        let mut img = image::RgbImage::new(32, 32);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 8) as u8, (y * 8) as u8, 100]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn high_quality_reencode_stays_visually_close() {
        let input = tiny_source_png();
        let output = JpegCodec
            .recompress(
                input.clone(),
                &CompressOptions {
                    effort: 95,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let report = compare_pixels(&input, &output).unwrap();
        assert_eq!((report.width, report.height), (32, 32));
        // Lossy by definition, but quality 95 shouldn't wreck the image.
        assert!(
            report.mean_channel_delta < 15.0,
            "mean channel delta too high for quality=95: {}",
            report.mean_channel_delta
        );
    }

    #[tokio::test]
    async fn lower_quality_produces_smaller_output() {
        let input = tiny_source_png();
        let high = JpegCodec
            .recompress(
                input.clone(),
                &CompressOptions {
                    effort: 95,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let low = JpegCodec
            .recompress(
                input,
                &CompressOptions {
                    effort: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(low.len() < high.len());
    }
}
