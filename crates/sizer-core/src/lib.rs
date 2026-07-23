//! Sizer's compression engine, shared verbatim by every surface (CLI, Tauri
//! desktop, WASM/browser, cloud API/workers). This crate defines *what a
//! codec is* and *how a file gets identified*; it deliberately implements no
//! codecs itself — see the `sizer-codecs-*` crates for concrete algorithms.
//!
//! No surface-specific IO (sockets, browser File objects, S3 clients) is
//! assumed here: everything speaks in terms of `tokio::io::{AsyncRead,
//! AsyncWrite}` so the same engine runs unmodified behind a CLI, a Tauri
//! command, wasm-bindgen bindings, or an Axum handler.

mod codec;
mod detect;
mod error;
mod guard;
mod integrity;
mod progress;

pub use codec::{Codec, CodecId, CompressOptions};
pub use detect::{Detector, FileKind, Format, MagicByteDetector};
pub use error::{Error, Result};
pub use guard::LimitedWriter;
pub use integrity::{hash, verify, HashingSink, Sha256Digest};
pub use progress::{NullProgress, Progress};
