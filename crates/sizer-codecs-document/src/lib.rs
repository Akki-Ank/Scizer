//! Document structural recompression.
//!
//! Like `sizer-codecs-image`, this does **not** implement
//! `sizer_core::Codec` — recompressing a PDF's embedded images in place
//! produces a different but valid PDF, not a byte-exact round-trip, so
//! there's no meaningful `decompress` operation. `DocumentCodec` has the
//! same shape as `ImageCodec` (single-direction `recompress`, in-memory
//! buffer) rather than reusing that trait directly: the domains are
//! structurally identical today but conceptually distinct (a PDF isn't an
//! image), and keeping them separate means they can diverge later without
//! one trait having to serve two unrelated purposes. See
//! `docs/ARCHITECTURE.md` for the fuller "N codec shapes" reasoning.
//!
//! **Scope of `PdfCodec`**: recompresses embedded JPEG (`DCTDecode`)
//! image streams smaller, in place, via `sizer_codecs_image::JpegCodec`.
//! It does **not** rasterize or re-render pages, subset fonts, or touch
//! non-image content — this is deliberately the single highest-value
//! slice (photo-heavy/scanned PDFs are usually image-dominated in file
//! size) rather than a general-purpose PDF optimizer. Font subsetting,
//! XML/content-stream minification, and Office formats (DOCX/XLSX/PPTX --
//! themselves zip archives, so this crate's second target) are follow-up
//! scope, not implemented yet.

mod pdf;

pub use pdf::PdfCodec;

use async_trait::async_trait;
use sizer_core::{CompressOptions, Result};

/// A single-direction document recompressor. Same shape as
/// `sizer_codecs_image::ImageCodec` and for the same reason: no
/// `decompress` counterpart, because recompressing a document's internal
/// structure isn't a reversible operation. Verification is the caller's
/// job (`sizer-cli document-compress --check` does a structural sanity
/// check, not full rendering fidelity, which would need a PDF rasterizer
/// this project doesn't have) -- this trait only recompresses.
#[async_trait]
pub trait DocumentCodec: Send + Sync {
    fn name(&self) -> &'static str;

    /// Recompresses `input`, returning the new document bytes plus how
    /// many embedded images were actually recompressed (0 is valid --
    /// e.g. a text-only PDF with no images) so callers can report
    /// something more useful than "0 bytes saved" with no explanation.
    async fn recompress(
        &self,
        input: Vec<u8>,
        options: &CompressOptions,
    ) -> Result<RecompressReport>;
}

#[derive(Debug, Clone)]
pub struct RecompressReport {
    pub output: Vec<u8>,
    pub images_recompressed: usize,
    pub images_skipped: usize,
}
