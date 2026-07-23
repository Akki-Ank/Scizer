use std::fmt;
use std::pin::Pin;
use std::task::{Context, Poll};

use sha2::{Digest, Sha256};
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite};

use crate::{Error, Result};

/// A SHA-256 digest, hex-encoded on display. Every compress/decompress
/// round-trip in Sizer is expected to be verified against one of these —
/// "no data loss" from the product spec is enforced here, not assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Hashes `reader` to completion in fixed-size chunks — never buffers the
/// whole stream, so this is safe to run on a 100GB file with constant
/// memory, matching the project's memory-usage requirement.
pub async fn hash(mut reader: impl AsyncRead + Unpin) -> Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).await.map_err(Error::Io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Sha256Digest(hasher.finalize().into()))
}

/// Verifies `reader`'s content hashes to `expected`, returning
/// [`Error::IntegrityMismatch`] otherwise.
pub async fn verify(reader: impl AsyncRead + Unpin, expected: &Sha256Digest) -> Result<()> {
    let actual = hash(reader).await?;
    if actual == *expected {
        Ok(())
    } else {
        Err(Error::IntegrityMismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        })
    }
}

/// An `AsyncWrite` sink that discards written bytes but hashes them as
/// they pass through — lets a caller verify a decompressed round-trip
/// against a source digest without buffering the decompressed output
/// anywhere, matching the "constant memory regardless of file size" rule
/// that a `hash(reader)` + `Vec<u8>` buffer would violate for large files.
#[derive(Debug, Default)]
pub struct HashingSink {
    hasher: Sha256,
    written: u64,
}

impl HashingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes_written(&self) -> u64 {
        self.written
    }

    pub fn finalize(self) -> Sha256Digest {
        Sha256Digest(self.hasher.finalize().into())
    }
}

impl AsyncWrite for HashingSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.hasher.update(buf);
        self.written += buf.len() as u64;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn known_vector_matches() {
        // sha256("") — the standard empty-input test vector.
        let digest = hash(std::io::Cursor::new(b"" as &[u8])).await.unwrap();
        assert_eq!(
            digest.to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[tokio::test]
    async fn verify_detects_mismatch() {
        let expected = hash(std::io::Cursor::new(b"hello" as &[u8])).await.unwrap();
        let result = verify(std::io::Cursor::new(b"goodbye" as &[u8]), &expected).await;
        assert!(matches!(result, Err(Error::IntegrityMismatch { .. })));
    }

    #[tokio::test]
    async fn verify_accepts_match() {
        let expected = hash(std::io::Cursor::new(b"hello" as &[u8])).await.unwrap();
        let result = verify(std::io::Cursor::new(b"hello" as &[u8]), &expected).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn hashing_sink_matches_hash_of_same_bytes() {
        use tokio::io::AsyncWriteExt;

        let expected = hash(std::io::Cursor::new(b"hello world" as &[u8]))
            .await
            .unwrap();

        let mut sink = HashingSink::new();
        sink.write_all(b"hello world").await.unwrap();
        assert_eq!(sink.bytes_written(), 11);
        assert_eq!(sink.finalize(), expected);
    }
}
