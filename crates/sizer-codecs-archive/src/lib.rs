//! Streaming gzip and zstd codecs. Both wrap `async-compression`, which
//! wraps the audited C libraries (`zlib`/`libzstd`) — no format parsing is
//! reimplemented here, per the "don't hand-roll parsers" rule in
//! `docs/ARCHITECTURE.md`.
//!
//! `ZstdCodec` is native-only: it wraps `zstd-sys`, which needs a
//! wasm32-targeting C compiler that isn't set up here (see this crate's
//! `Cargo.toml` and `docs/ARCHITECTURE.md`). `GzipCodec` builds for both
//! native and `wasm32-unknown-unknown` since `flate2`'s default backend
//! (`miniz_oxide`) is pure Rust.

mod gzip;
#[cfg(not(target_arch = "wasm32"))]
mod zstd;

pub use gzip::GzipCodec;
#[cfg(not(target_arch = "wasm32"))]
pub use zstd::ZstdCodec;

use sizer_core::{Error, Progress, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CHUNK_SIZE: usize = 64 * 1024;

/// Copies `reader` into `writer` in fixed-size chunks, reporting progress
/// and honoring cancellation between chunks — unlike `tokio::io::copy`,
/// which offers neither hook. Every codec's compress/decompress path
/// funnels through this so both behaviors are consistent across codecs
/// rather than reimplemented per format.
///
/// Does **not** shut the writer down — when `writer` is a compression
/// encoder/decoder, finishing the underlying stream (writing the final
/// block/trailer) is a distinct step from flushing, and callers must
/// `.shutdown().await` it themselves once copying is done.
async fn copy_with_progress(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    total: Option<u64>,
    progress: &dyn Progress,
) -> Result<()> {
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut processed: u64 = 0;
    loop {
        if progress.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buf).await.map_err(Error::Io)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await.map_err(Error::Io)?;
        processed += n as u64;
        progress.on_progress(processed, total);
    }
    Ok(())
}

/// Effort 0..=100 maps onto each codec's native compression-level range.
/// Centralized here so gzip (0..=9) and zstd (1..=22) agree on what "half
/// effort" means from the caller's point of view.
fn effort_to_level(effort: u8, max_level: i32) -> i32 {
    let effort = effort.min(100) as i32;
    (effort * max_level / 100).max(1)
}

#[cfg(test)]
mod effort_tests {
    use super::*;

    #[test]
    fn zero_effort_still_produces_a_valid_level() {
        assert_eq!(effort_to_level(0, 9), 1);
    }

    #[test]
    fn full_effort_hits_max_level() {
        assert_eq!(effort_to_level(100, 9), 9);
        assert_eq!(effort_to_level(100, 22), 22);
    }

    #[test]
    fn effort_is_clamped_above_100() {
        assert_eq!(effort_to_level(255, 9), 9);
    }
}
