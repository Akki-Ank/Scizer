//! File-format conversion -- not compression. See `docs/ARCHITECTURE.md`'s
//! "Multiple codec shapes, not one" for why this doesn't reuse
//! `Codec`/`ImageCodec`/`DocumentCodec`: conversion has no reversibility
//! or fidelity contract to check against, it's a different kind of
//! operation entirely, so it gets its own small set of free functions
//! rather than being forced into one of those traits.
//!
//! Scope is deliberately bounded, the same "single highest-value slice"
//! reasoning as `sizer-codecs-document`'s embedded-JPEG-only PDF handling
//! -- not a general "any format to any format" converter:
//! - `convert_image`: re-encoding a raster image across the formats
//!   `image` both decodes and encodes (PNG, JPEG, BMP, GIF, TIFF, ICO).
//! - `images_to_pdf`: composing one or more images into a new PDF, one
//!   image per page.
//! - `merge_pdfs`: concatenating multiple existing PDFs' pages into one.
//!
//! Native-only (no wasm32 target): `printpdf`'s PDF writer and `lopdf`'s
//! merge path have no reason to run in a browser sandbox for this
//! project's current surfaces (the desktop app is the only caller today).

mod image_convert;
mod images_to_pdf;
mod pdf_merge;

pub use image_convert::{convert_image, image_format_by_name, SUPPORTED_IMAGE_FORMATS};
pub use images_to_pdf::images_to_pdf;
pub use pdf_merge::merge_pdfs;

use sizer_core::{Error, Result};

/// Runs a synchronous, CPU-bound closure on tokio's blocking thread pool.
/// Same rationale as `sizer_codecs_image::run_blocking`: image/PDF
/// encoding is synchronous and would otherwise stall the async runtime.
/// Unlike that sibling crate, there's no wasm32 fallback branch to
/// maintain here -- this crate only ever runs natively.
pub(crate) async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::UnsupportedFormat(format!("background task panicked: {e}")))?
}
