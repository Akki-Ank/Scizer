use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{self, AsyncWrite};

/// Wraps an `AsyncWrite` and fails closed once more than `limit` bytes have
/// been written through it.
///
/// This is the decompression-bomb guard referenced throughout
/// `docs/ARCHITECTURE.md`: every codec that decompresses attacker-controlled
/// input wraps its output writer in one of these, sized from
/// `CompressOptions::max_decompressed_bytes`, instead of trusting the
/// input's claimed uncompressed size (which is attacker-controlled and easy
/// to lie about).
pub struct LimitedWriter<W> {
    inner: W,
    limit: u64,
    written: u64,
}

impl<W> LimitedWriter<W> {
    pub fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            limit,
            written: 0,
        }
    }

    pub fn bytes_written(&self) -> u64 {
        self.written
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for LimitedWriter<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.written.saturating_add(buf.len() as u64) > self.limit {
            return Poll::Ready(Err(io::Error::other(
                crate::Error::ExpansionLimitExceeded { limit: self.limit },
            )));
        }
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &poll {
            self.written += *n as u64;
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn allows_writes_under_limit() {
        let mut buf = Vec::new();
        let mut w = LimitedWriter::new(&mut buf, 10);
        w.write_all(b"hello").await.unwrap();
        assert_eq!(w.bytes_written(), 5);
    }

    #[tokio::test]
    async fn rejects_writes_over_limit() {
        let mut buf = Vec::new();
        let mut w = LimitedWriter::new(&mut buf, 4);
        let result = w.write_all(b"hello").await;
        assert!(result.is_err());
    }
}
