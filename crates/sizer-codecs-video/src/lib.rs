//! Video recompression by shelling out to a system-installed `ffmpeg`/
//! `ffprobe` -- **not** a linked/vendored/bundled FFmpeg. Sizer never
//! ships FFmpeg binaries or links against `libav*`, so FFmpeg's own
//! GPL/LGPL licensing terms are the invoking user's concern (their own
//! install, their own build config), not something this project has to
//! solve by policing which codecs are compiled in. See
//! `docs/ARCHITECTURE.md` ("Video (M5): shell out, don't bundle") for the
//! full reasoning, including why this is a deliberate change from the
//! "FFmpeg must be built LGPL-only" note written during M0 planning
//! (which assumed we'd be the ones distributing FFmpeg).
//!
//! Native-only, no `wasm32` build: `tokio::process` (spawning a real OS
//! subprocess) has no wasm32-unknown-unknown equivalent, and shelling out
//! to a binary makes no sense in a browser sandbox regardless.
//!
//! This is a **third** codec shape, alongside `sizer_core::Codec`
//! (reversible, byte-stream) and `sizer_codecs_image::ImageCodec`
//! (lossy, in-memory buffer): `VideoCodec` operates on **file paths**,
//! not byte streams or buffers, because video files are exactly the case
//! where "never load the whole file into RAM" matters most (the product
//! spec's own examples go up to 100GB) and because `ffmpeg` itself reads
//! and writes files (or pipes) directly -- there's no benefit to routing
//! bytes through Rust-side buffers just to hand them to a subprocess.

mod ffmpeg;
mod probe;

pub use ffmpeg::FfmpegCodec;
pub use probe::probe_duration_ms;

use std::path::Path;

use async_trait::async_trait;
use sizer_core::{CompressOptions, Progress, Result};

/// A single-direction video recompressor, file path in, file path out.
/// Like `ImageCodec`, there's no `decompress`/inverse operation -- a
/// re-encoded video is a different but valid file, not a byte-exact
/// round-trip. Unlike `ImageCodec`, correctness verification (does the
/// output still look right) isn't provided by this crate: full frame-by-
/// frame fidelity checking is expensive enough (decode every frame of
/// both videos) that it doesn't belong in every call the way
/// `compare_pixels` does for still images. `sizer-cli`'s
/// `video-compress --check` instead does a cheap sanity check (does the
/// output probe as a valid video with a plausible duration), not a
/// pixel-level one.
#[async_trait]
pub trait VideoCodec: Send + Sync {
    fn name(&self) -> &'static str;

    /// `options.effort` here means "how aggressively to shrink" (0 =
    /// minimal compression/near-original quality, 100 = maximum
    /// compression/lowest quality) -- the same direction as the archive
    /// codecs' effort, unlike `JpegCodec`'s inverted "effort = quality"
    /// convention. See `FfmpegCodec`'s doc comment for the concrete
    /// CRF mapping.
    ///
    /// Progress is reported as **milliseconds of output produced**
    /// against **milliseconds of total input duration** -- not bytes.
    /// `sizer_core::Progress::on_progress`'s parameters are named
    /// `processed`/`total` generically for exactly this reason: what
    /// they count is domain-specific, documented per codec, not fixed by
    /// the trait.
    async fn recompress(
        &self,
        input: &Path,
        output: &Path,
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()>;
}

/// Maps `effort` (0..=100) onto x264/x265-style CRF (0..=51, lower =
/// higher quality/larger output). 0 effort -> crf 18 (visually
/// near-lossless, still a real reduction from raw); 100 effort -> crf 51
/// (maximum compression, the codec's own defined floor for quality).
fn effort_to_crf(effort: u8) -> u32 {
    const CRF_MIN: u32 = 18;
    const CRF_MAX: u32 = 51;
    CRF_MIN + (effort.min(100) as u32 * (CRF_MAX - CRF_MIN)) / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crf_spans_full_range() {
        assert_eq!(effort_to_crf(0), 18);
        assert_eq!(effort_to_crf(100), 51);
    }

    #[test]
    fn crf_is_clamped_above_100() {
        assert_eq!(effort_to_crf(255), 51);
    }
}
