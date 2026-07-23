//! Image recompression codecs.
//!
//! These do **not** implement `sizer_core::Codec`. That trait models
//! reversible compression (`decompress(compress(x))` reconstructs `x`
//! exactly, verified by SHA-256) — the right shape for archive formats,
//! wrong for this domain. Recompressing a JPEG produces a *different but
//! valid* JPEG that decodes to similar (not identical) pixels; there is no
//! "decompress it back to the original bytes" operation to speak of. See
//! `docs/ARCHITECTURE.md` ("Two codec shapes, not one") for the full
//! reasoning.
//!
//! Success here is measured by [`fidelity::compare_pixels`] — do the
//! decoded pixels match (lossless) or stay within an acceptable delta
//! (lossy) — not by a byte-exact round-trip.
//!
//! No heavyweight native encoders: `jpeg-encoder` and `image`'s own
//! PNG/JPEG codecs are pure Rust, and `oxipng` pulls in `libdeflate`
//! (a small, well-audited C library) purely as a fast DEFLATE backend —
//! a very different build/toolchain footprint from `mozjpeg-sys` (full
//! libjpeg-turbo fork) or `libwebp-sys`/`libaom` (autotools/cmake, much
//! larger surface). Those heavier encoders are deliberately deferred
//! until they're actually needed (better JPEG ratios, WebP/AVIF encode) —
//! see the crate-level roadmap note in `docs/ARCHITECTURE.md`.
//!
//! `PngCodec` is native-only: `oxipng`'s `libdeflate` dependency above is
//! *unconditional* (not feature-gated), so there is no wasm32 build of it
//! without a wasm-capable C toolchain. `JpegCodec` has no such dependency
//! and builds for `wasm32-unknown-unknown` unmodified — see
//! `docs/ARCHITECTURE.md` for the full reasoning and what would need to
//! change to bring PNG to the browser.

mod fidelity;
mod jpeg;
#[cfg(not(target_arch = "wasm32"))]
mod png;

pub use fidelity::{compare_pixels, FidelityReport};
pub use jpeg::JpegCodec;
#[cfg(not(target_arch = "wasm32"))]
pub use png::PngCodec;

use async_trait::async_trait;
use sizer_core::{CompressOptions, Result};

/// A single-direction image recompressor: bytes in, smaller (or at least
/// re-optimized) bytes of the same logical format out. `options.effort` is
/// repurposed per codec — see each implementation's doc comment — since
/// "encoder speed effort" and "target quality" aren't the same axis for a
/// single-pass image re-encode the way they're related for streaming
/// archive codecs.
#[async_trait]
pub trait ImageCodec: Send + Sync {
    fn name(&self) -> &'static str;

    /// Whether decoding `recompress(x)` reproduces the exact same pixels
    /// as decoding `x`. `true` for PNG (lossless re-optimization), `false`
    /// for JPEG (lossy re-encode) — callers use this to decide whether an
    /// exact or delta-tolerant fidelity check is the right one to run.
    fn is_lossless(&self) -> bool;

    async fn recompress(&self, input: Vec<u8>, options: &CompressOptions) -> Result<Vec<u8>>;
}

/// Runs `f` (a synchronous, CPU-bound recompress call) without blocking
/// whatever async runtime is driving the caller.
///
/// On native, that means `tokio::task::spawn_blocking` -- oxipng and
/// jpeg-encoder are synchronous and would otherwise stall the runtime.
/// On `wasm32` there is no OS thread pool to spawn onto (`spawn_blocking`
/// doesn't exist there), and there doesn't need to be one: in the
/// browser, the whole wasm module instance already runs inside a
/// dedicated Web Worker (see `sizer-wasm`), so "off the main/UI thread"
/// is already true at the JS level before any Rust code runs -- `f` just
/// runs inline.
#[cfg(not(target_arch = "wasm32"))]
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|e| {
        sizer_core::Error::UnsupportedFormat(format!("recompress task panicked: {e}"))
    })?
}

#[cfg(target_arch = "wasm32")]
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    f()
}

/// Maps `effort` (0..=100, "how hard should the encoder try") onto a
/// small integer preset range. Used by `PngCodec` (native-only, hence the
/// cfg) -- `JpegCodec` uses `effort` as a continuous quality value
/// directly instead, see its own doc comment.
#[cfg(not(target_arch = "wasm32"))]
fn effort_to_preset(effort: u8, max_preset: u8) -> u8 {
    ((effort.min(100) as u16 * max_preset as u16) / 100) as u8
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod preset_tests {
    use super::*;

    #[test]
    fn maps_full_range() {
        assert_eq!(effort_to_preset(0, 6), 0);
        assert_eq!(effort_to_preset(100, 6), 6);
        assert_eq!(effort_to_preset(50, 6), 3);
    }
}
