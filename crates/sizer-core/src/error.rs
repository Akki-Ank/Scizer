use std::io;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("no codec registered for {0:?}")]
    NoCodec(crate::CodecId),

    #[error(
        "refusing to decompress: output would exceed {limit} bytes (ratio guard against decompression bombs)"
    )]
    ExpansionLimitExceeded { limit: u64 },

    #[error("integrity check failed: expected {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },

    #[error("operation cancelled")]
    Cancelled,
}

impl Error {
    /// Codecs write decompressed output through a [`crate::LimitedWriter`],
    /// whose `poll_write` can only fail with `io::Error` (that's the
    /// `AsyncWrite` contract) — so it stuffs the real `Error` in as the
    /// io::Error's source via `io::Error::other`. This unwraps that back
    /// to the original variant, so callers see `Error::ExpansionLimitExceeded`
    /// directly instead of a generic `Error::Io` after it round-trips
    /// through a write call.
    pub fn unwrap_io(self) -> Self {
        match self {
            Error::Io(io_err) => {
                let kind = io_err.kind();
                match io_err.into_inner() {
                    Some(inner) => match inner.downcast::<Error>() {
                        Ok(boxed) => *boxed,
                        Err(inner) => Error::Io(io::Error::new(kind, inner)),
                    },
                    None => Error::Io(io::Error::from(kind)),
                }
            }
            other => other,
        }
    }
}
