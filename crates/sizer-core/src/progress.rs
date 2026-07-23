/// Sink for progress events during a compress/decompress call. Implemented
/// differently per surface: the CLI renders a terminal bar, Tauri emits an
/// event to the frontend, the web build posts a message from the worker,
/// and the cloud worker writes job progress to Redis for the API to poll.
/// `sizer-core` only ever calls this trait — it never assumes how progress
/// is displayed.
pub trait Progress: Send + Sync {
    /// `processed`/`total` are in bytes of the *input* stream. `total` is
    /// `None` when the source length isn't known upfront (e.g. a piped
    /// stream), in which case implementations should fall back to a
    /// spinner rather than a percentage.
    fn on_progress(&self, processed: u64, total: Option<u64>);

    /// Called if the caller requests cancellation mid-stream. Codecs must
    /// poll this between chunks and return `Error::Cancelled` promptly —
    /// large files must be abortable without waiting for completion.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// No-op implementation for callers (tests, benchmarks) that don't care
/// about progress reporting.
#[derive(Debug, Default)]
pub struct NullProgress;

impl Progress for NullProgress {
    fn on_progress(&self, _processed: u64, _total: Option<u64>) {}
}
