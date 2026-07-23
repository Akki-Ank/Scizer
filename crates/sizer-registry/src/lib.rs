//! Codec name/format -> instance lookup. Pulled out of `sizer-cli` once
//! `sizer-desktop` needed the exact same mapping: this crate exists so
//! "what does the string `\"zstd\"` mean" is answered in exactly one
//! place, not reimplemented per surface.

use sizer_codecs_archive::{GzipCodec, ZstdCodec};
use sizer_codecs_document::{DocumentCodec, PdfCodec};
use sizer_codecs_image::{ImageCodec, JpegCodec, PngCodec};
use sizer_codecs_video::{FfmpegCodec, VideoCodec};
use sizer_core::{Codec, Error, Format, Result};

/// Explicit name -> codec lookup for archive compression. There's no
/// auto-selection yet (that's later "Smart Engine" scope): picking the
/// best archive codec for arbitrary input isn't implemented, and forcing
/// that choice on the caller now would just be unimplemented magic.
pub fn codec_by_name(name: &str) -> Result<Box<dyn Codec>> {
    match name {
        "gzip" => Ok(Box::new(GzipCodec)),
        "zstd" => Ok(Box::new(ZstdCodec)),
        other => Err(Error::UnsupportedFormat(format!(
            "unknown codec {other:?}; supported codecs: gzip, zstd"
        ))),
    }
}

/// Format -> codec lookup for archive decompression, which *does*
/// auto-detect: the compressed file's magic bytes unambiguously say which
/// codec produced it, so there's nothing for the caller to choose.
pub fn codec_for_format(format: Format) -> Result<Box<dyn Codec>> {
    match format {
        Format::Gzip => Ok(Box::new(GzipCodec)),
        Format::Zstd => Ok(Box::new(ZstdCodec)),
        other => Err(Error::UnsupportedFormat(format!(
            "no codec registered to decompress {other:?} yet (supported: gzip, zstd)"
        ))),
    }
}

/// Explicit name -> image codec lookup. Unlike archive decompress, there's
/// no format-based auto-detection here either: recompressing *as* PNG vs
/// *as* JPEG is a caller choice (a lossless PNG in, JPEG out is a valid,
/// useful thing to ask for), not something the input format alone
/// determines.
pub fn image_codec_by_name(name: &str) -> Result<Box<dyn ImageCodec>> {
    match name {
        "png" => Ok(Box::new(PngCodec)),
        "jpeg" | "jpg" => Ok(Box::new(JpegCodec)),
        other => Err(Error::UnsupportedFormat(format!(
            "unknown image codec {other:?}; supported: png, jpeg"
        ))),
    }
}

/// Explicit name -> video codec lookup. Only one implementation exists
/// today (`ffmpeg`, shelling out to a system install -- see
/// `sizer-codecs-video`'s crate-level doc comment), but this still goes
/// through the same by-name lookup as the other domains rather than the
/// caller constructing `FfmpegCodec` directly, so a second video codec
/// later doesn't require touching every surface that calls this one.
pub fn video_codec_by_name(name: &str) -> Result<Box<dyn VideoCodec>> {
    match name {
        "ffmpeg" => Ok(Box::new(FfmpegCodec)),
        other => Err(Error::UnsupportedFormat(format!(
            "unknown video codec {other:?}; supported: ffmpeg"
        ))),
    }
}

/// Explicit name -> document codec lookup. Only "pdf" exists today
/// (embedded-JPEG recompression only -- see `sizer-codecs-document`'s
/// crate-level doc comment); Office formats are follow-up scope.
pub fn document_codec_by_name(name: &str) -> Result<Box<dyn DocumentCodec>> {
    match name {
        "pdf" => Ok(Box::new(PdfCodec)),
        other => Err(Error::UnsupportedFormat(format!(
            "unknown document codec {other:?}; supported: pdf"
        ))),
    }
}
