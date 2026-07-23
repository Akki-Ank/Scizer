use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Progress, Result};

/// Stable identifier for a codec implementation, used for registry lookup
/// and for the plugin system (`sizer-codecs-*` crates each register one or
/// more of these). Not the same thing as [`crate::Format`]: several codecs
/// can target the same format (e.g. two JPEG re-encoders), and the engine
/// picks between them by benchmarking, not by format alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CodecId(pub &'static str);

/// Knobs a caller can set before compression runs. Codecs interpret only
/// the fields relevant to them and ignore the rest — e.g. `target_bytes`
/// only means something to codecs that support iterative convergence
/// (image/video), while an archive codec ignores it.
#[derive(Debug, Clone, Default)]
pub struct CompressOptions {
    /// Best-effort target output size in bytes. Codecs that can't converge
    /// on a size (e.g. lossless archive formats) ignore this.
    pub target_bytes: Option<u64>,
    /// 0 = fastest/largest, 100 = slowest/smallest. Codec-specific mapping.
    pub effort: u8,
    /// Upper bound on decompressed output size, enforced by every codec
    /// that implements `decompress`. Required to defend against
    /// decompression-bomb attacks on untrusted input (see `Error::ExpansionLimitExceeded`).
    pub max_decompressed_bytes: u64,
}

/// A streaming, bidirectional compressor/decompressor. Implementations
/// live in `sizer-codecs-*` crates, never in `sizer-core` itself.
///
/// Both directions are streaming: implementations must not buffer the
/// entire input in memory, so that a 100GB file costs the same peak RAM as
/// a 10MB one. `reader`/`writer` are plain async IO traits so the same
/// implementation runs behind a CLI file handle, a Tauri stream, a
/// wasm-bindgen `ReadableStream` bridge, or an S3 multipart upload.
#[async_trait]
pub trait Codec: Send + Sync {
    fn id(&self) -> CodecId;

    /// Formats this codec knows how to compress into.
    fn supported_formats(&self) -> &'static [crate::Format];

    async fn compress(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()>;

    async fn decompress(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        writer: &mut (dyn AsyncWrite + Send + Unpin),
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()>;
}
