use image::GenericImageView;
use sizer_core::{Error, Result};

/// Result of comparing decoded pixels between an original image and its
/// recompressed output. This is this crate's substitute for the
/// byte-exact SHA-256 verification archive codecs use — see the
/// crate-level doc comment for why the two domains need different
/// verification strategies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FidelityReport {
    pub width: u32,
    pub height: u32,
    /// Largest single-channel (R/G/B/A) absolute difference found across
    /// every pixel. 0 means byte-for-byte identical pixels.
    pub max_channel_delta: u8,
    /// Mean absolute per-channel difference — a coarse stand-in for
    /// perceptual similarity. Cheap and dependency-free; swap for
    /// SSIM/PSNR if a later milestone needs a tighter quality budget.
    pub mean_channel_delta: f64,
}

impl FidelityReport {
    pub fn is_exact_match(&self) -> bool {
        self.max_channel_delta == 0
    }
}

/// Decodes both byte slices as images and compares their pixels.
/// Dimension mismatches are always an error — that's a correctness bug in
/// the codec, not a fidelity trade-off to report a number for.
pub fn compare_pixels(original: &[u8], recompressed: &[u8]) -> Result<FidelityReport> {
    let original = image::load_from_memory(original)
        .map_err(|e| Error::UnsupportedFormat(format!("decoding original: {e}")))?;
    let recompressed = image::load_from_memory(recompressed)
        .map_err(|e| Error::UnsupportedFormat(format!("decoding recompressed output: {e}")))?;

    if original.dimensions() != recompressed.dimensions() {
        return Err(Error::UnsupportedFormat(format!(
            "dimension mismatch: original {:?} vs recompressed {:?}",
            original.dimensions(),
            recompressed.dimensions()
        )));
    }

    let a = original.to_rgba8();
    let b = recompressed.to_rgba8();

    let mut max_delta: u8 = 0;
    let mut total_delta: u64 = 0;
    let mut channel_count: u64 = 0;

    for (pa, pb) in a.pixels().zip(b.pixels()) {
        for (ca, cb) in pa.0.iter().zip(pb.0.iter()) {
            let delta = ca.abs_diff(*cb);
            max_delta = max_delta.max(delta);
            total_delta += delta as u64;
            channel_count += 1;
        }
    }

    Ok(FidelityReport {
        width: a.width(),
        height: a.height(),
        max_channel_delta: max_delta,
        mean_channel_delta: total_delta as f64 / channel_count.max(1) as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png(color: [u8; 3]) -> Vec<u8> {
        let mut img = image::RgbImage::new(4, 4);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb(color);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    #[test]
    fn identical_images_report_zero_delta() {
        let png = tiny_png([10, 20, 30]);
        let report = compare_pixels(&png, &png).unwrap();
        assert!(report.is_exact_match());
        assert_eq!(report.width, 4);
        assert_eq!(report.height, 4);
    }

    #[test]
    fn different_images_report_nonzero_delta() {
        let a = tiny_png([0, 0, 0]);
        let b = tiny_png([255, 255, 255]);
        let report = compare_pixels(&a, &b).unwrap();
        assert!(!report.is_exact_match());
        assert_eq!(report.max_channel_delta, 255);
    }
}
