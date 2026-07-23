use async_trait::async_trait;
use sizer_core::{CompressOptions, Error, Result};

use crate::{effort_to_preset, run_blocking, ImageCodec};

/// Lossless PNG re-optimization via `oxipng`: re-picks filter/zlib
/// settings and strips non-essential metadata, producing a smaller PNG
/// that decodes to byte-identical pixels. `options.effort` maps onto
/// oxipng's 0..=6 optimization preset (higher = tries harder, slower).
#[derive(Debug, Default)]
pub struct PngCodec;

const OXIPNG_MAX_PRESET: u8 = 6;

#[async_trait]
impl ImageCodec for PngCodec {
    fn name(&self) -> &'static str {
        "png"
    }

    fn is_lossless(&self) -> bool {
        true
    }

    async fn recompress(&self, input: Vec<u8>, options: &CompressOptions) -> Result<Vec<u8>> {
        let preset = effort_to_preset(options.effort, OXIPNG_MAX_PRESET);

        // oxipng's optimizer is synchronous and CPU-bound (it tries
        // several filter/compression strategies internally) -- see
        // run_blocking's doc comment for why this isn't always
        // spawn_blocking.
        run_blocking(move || {
            let opts = oxipng::Options::from_preset(preset);
            oxipng::optimize_from_memory(&input, &opts)
                .map_err(|e| Error::UnsupportedFormat(format!("oxipng: {e}")))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare_pixels;

    fn tiny_png() -> Vec<u8> {
        let mut img = image::RgbImage::new(16, 16);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 16) as u8, (y * 16) as u8, 128]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn recompressed_png_decodes_to_identical_pixels() {
        let input = tiny_png();
        let output = PngCodec
            .recompress(
                input.clone(),
                &CompressOptions {
                    effort: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let report = compare_pixels(&input, &output).unwrap();
        assert!(
            report.is_exact_match(),
            "PNG recompression must be lossless, got max delta {}",
            report.max_channel_delta
        );
    }
}
