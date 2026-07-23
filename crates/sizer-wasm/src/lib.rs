//! wasm-bindgen surface exposing `sizer-core` to the browser.
//!
//! **Scope: gzip (archive) and JPEG (image) only**, not the full set
//! `sizer-cli`/`sizer-desktop` support. Both `zstd` (`sizer-codecs-archive`)
//! and PNG-via-oxipng (`sizer-codecs-image`) wrap C libraries
//! (`zstd-sys`, `libdeflate-sys`) that need a wasm32-targeting C compiler
//! to build, which isn't set up in this project's toolchain yet -- see
//! those crates' `Cargo.toml` comments and `docs/ARCHITECTURE.md`. Gzip
//! and JPEG have no such dependency and compile for `wasm32-unknown-unknown`
//! unmodified.
//!
//! This crate holds no compression logic of its own -- every function
//! here is a thin wasm-bindgen wrapper around `sizer-codecs-archive`/
//! `sizer-codecs-image`, the same crates the CLI and desktop app use.
//!
//! Threading: this module is meant to be instantiated inside a dedicated
//! Web Worker (see `web/worker.js`), not on the page's main thread --
//! `recompress`/`compress`/`decompress` here are synchronous-ish CPU work
//! wrapped in `async fn` only so wasm-bindgen can return a `Promise`, not
//! because there's any real concurrency inside the wasm module itself
//! (wasm32-unknown-unknown is single-threaded without the separate
//! SharedArrayBuffer/atomics setup this milestone doesn't include yet).
//! Running inside a Worker is what keeps the page responsive; see
//! `sizer_codecs_image::run_blocking`'s doc comment for the same point
//! from the Rust side.

mod progress;

use serde::Serialize;
use sizer_codecs_archive::GzipCodec;
use sizer_codecs_image::{ImageCodec as _, JpegCodec};
use sizer_core::{Codec, CompressOptions, Detector, MagicByteDetector};
use wasm_bindgen::prelude::*;

use crate::progress::JsProgress;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[derive(Serialize)]
struct DetectResult {
    format: String,
    kind: String,
}

/// Identifies a byte buffer's format from its magic bytes.
#[wasm_bindgen(js_name = detectFormat)]
pub fn detect_format(bytes: &[u8]) -> Result<JsValue, JsValue> {
    let header = &bytes[..bytes.len().min(64)];
    let format = MagicByteDetector.sniff(header);
    let result = DetectResult {
        format: format!("{format:?}"),
        kind: format!("{:?}", format.kind()),
    };
    serde_wasm_bindgen::to_value(&result).map_err(js_err)
}

/// Compresses `input` with gzip. `on_progress`, if provided, is called as
/// `(processedBytes, totalBytesOrNull)`. `totalBytes` is currently always
/// `null`: `sizer_codecs_archive::copy_with_progress` doesn't thread a
/// known total through to the progress callback even when the caller
/// (like this one) does know it upfront from an in-memory buffer -- fine
/// for a byte-counter/spinner UI, but a percentage/ETA display needs that
/// wired through first.
#[wasm_bindgen(js_name = compressGzip)]
pub async fn compress_gzip(
    input: Vec<u8>,
    effort: u8,
    on_progress: Option<js_sys::Function>,
) -> Result<Vec<u8>, JsValue> {
    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let progress = JsProgress::new(on_progress);
    let mut output = Vec::new();
    GzipCodec
        .compress(
            &mut std::io::Cursor::new(&input),
            &mut output,
            &options,
            &progress,
        )
        .await
        .map_err(js_err)?;
    Ok(output)
}

/// Decompresses a gzip buffer. `max_decompressed_bytes` guards against
/// decompression bombs exactly as it does natively -- see
/// `sizer_core::LimitedWriter`; the browser has no OS-level memory limit
/// enforcement to fall back on, so this matters here at least as much as
/// it does server-side.
#[wasm_bindgen(js_name = decompressGzip)]
pub async fn decompress_gzip(
    input: Vec<u8>,
    max_decompressed_bytes: f64,
    on_progress: Option<js_sys::Function>,
) -> Result<Vec<u8>, JsValue> {
    let options = CompressOptions {
        max_decompressed_bytes: max_decompressed_bytes as u64,
        ..Default::default()
    };
    let progress = JsProgress::new(on_progress);
    let mut output = Vec::new();
    GzipCodec
        .decompress(
            &mut std::io::Cursor::new(&input),
            &mut output,
            &options,
            &progress,
        )
        .await
        .map_err(js_err)?;
    Ok(output)
}

/// Re-encodes `input` as JPEG at `quality` (1..=100). See
/// `sizer_codecs_image::JpegCodec`'s doc comment for why `quality` is used
/// directly rather than mapped through an effort preset.
#[wasm_bindgen(js_name = recompressJpeg)]
pub async fn recompress_jpeg(input: Vec<u8>, quality: u8) -> Result<Vec<u8>, JsValue> {
    let options = CompressOptions {
        effort: quality,
        ..Default::default()
    };
    JpegCodec.recompress(input, &options).await.map_err(js_err)
}

#[derive(Serialize)]
struct FidelityInfo {
    width: u32,
    height: u32,
    max_channel_delta: u8,
    mean_channel_delta: f64,
    exact_match: bool,
}

/// Decodes both buffers and reports how close their pixels are -- the
/// browser-side equivalent of the CLI's `--check-fidelity` /
/// `sizer-desktop`'s fidelity report. Synchronous: decoding two images is
/// comparatively cheap, and there's no advantage to a Promise here.
#[wasm_bindgen(js_name = comparePixels)]
pub fn compare_pixels(original: &[u8], recompressed: &[u8]) -> Result<JsValue, JsValue> {
    let report = sizer_codecs_image::compare_pixels(original, recompressed).map_err(js_err)?;
    let info = FidelityInfo {
        width: report.width,
        height: report.height,
        max_channel_delta: report.max_channel_delta,
        mean_channel_delta: report.mean_channel_delta,
        exact_match: report.is_exact_match(),
    };
    serde_wasm_bindgen::to_value(&info).map_err(js_err)
}

fn js_err(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}
