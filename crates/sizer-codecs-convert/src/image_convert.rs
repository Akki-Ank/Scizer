use image::ImageFormat;
use sizer_core::{Error, Result};

pub const SUPPORTED_IMAGE_FORMATS: &[&str] = &["png", "jpeg", "bmp", "gif", "tiff", "ico"];

pub fn image_format_by_name(name: &str) -> Result<ImageFormat> {
    match name.to_ascii_lowercase().as_str() {
        "png" => Ok(ImageFormat::Png),
        "jpeg" | "jpg" => Ok(ImageFormat::Jpeg),
        "bmp" => Ok(ImageFormat::Bmp),
        "gif" => Ok(ImageFormat::Gif),
        "tiff" | "tif" => Ok(ImageFormat::Tiff),
        "ico" => Ok(ImageFormat::Ico),
        other => Err(Error::UnsupportedFormat(format!(
            "unknown image format {other:?}; supported: {}",
            SUPPORTED_IMAGE_FORMATS.join(", ")
        ))),
    }
}

/// Re-encodes `input` (any format `image` can decode) as `target_format`.
/// This is re-encoding, not recompression -- there's no quality/effort
/// knob here; see `sizer_codecs_image::JpegCodec`/`PngCodec` for that.
pub async fn convert_image(input: Vec<u8>, target_format: &str) -> Result<Vec<u8>> {
    let format = image_format_by_name(target_format)?;
    crate::run_blocking(move || {
        let decoded = image::load_from_memory(&input)
            .map_err(|e| Error::UnsupportedFormat(format!("decoding image: {e}")))?;

        // JPEG has no alpha channel; drop it explicitly rather than let
        // the encoder fail (or silently mishandle it) on an RGBA source.
        let decoded = if format == ImageFormat::Jpeg {
            image::DynamicImage::ImageRgb8(decoded.to_rgb8())
        } else {
            decoded
        };

        let mut out = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut out, format)
            .map_err(|e| Error::UnsupportedFormat(format!("encoding image: {e}")))?;
        Ok(out.into_inner())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_rgba_png() -> Vec<u8> {
        let mut img = image::RgbaImage::new(16, 16);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 16) as u8, (y * 16) as u8, 100, 200]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn converts_png_to_jpeg_and_back() {
        let png = tiny_rgba_png();
        let jpeg = convert_image(png, "jpeg").await.unwrap();
        assert_eq!(
            image::guess_format(&jpeg).unwrap(),
            ImageFormat::Jpeg
        );

        let png_again = convert_image(jpeg, "png").await.unwrap();
        assert_eq!(image::guess_format(&png_again).unwrap(), ImageFormat::Png);
    }

    #[tokio::test]
    async fn converts_to_bmp_gif_tiff_ico() {
        let png = tiny_rgba_png();
        for format in ["bmp", "gif", "tiff", "ico"] {
            let converted = convert_image(png.clone(), format).await.unwrap();
            let expected = image_format_by_name(format).unwrap();
            assert_eq!(
                image::guess_format(&converted).unwrap(),
                expected,
                "converting to {format}"
            );
        }
    }

    #[tokio::test]
    async fn rejects_unknown_target_format() {
        let err = convert_image(tiny_rgba_png(), "webp").await.unwrap_err();
        assert!(matches!(err, Error::UnsupportedFormat(_)));
    }
}
