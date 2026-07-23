//! Bridges `sizer_core::Progress` (a synchronous trait -- codecs call
//! `on_progress` inline from their hot loop, see `sizer-core/src/progress.rs`)
//! to Redis (an async I/O call). `on_progress` hands events off to an
//! unbounded channel and returns immediately; a background task drains the
//! channel and writes to Redis, so a slow/stalled Redis connection can
//! never stall a codec's compress loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use uuid::Uuid;

use sizer_core::Progress;

use crate::queue::Queue;

pub struct RedisProgress {
    tx: mpsc::UnboundedSender<(u64, Option<u64>)>,
    cancelled: Arc<AtomicBool>,
}

impl RedisProgress {
    /// Spawns the background writer task and returns a handle. `cancelled`
    /// is shared with a separate poller (see `worker.rs`) that watches
    /// Redis for a cancel request and flips the flag this reads from.
    pub fn spawn(queue: Queue, job_id: Uuid, cancelled: Arc<AtomicBool>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<(u64, Option<u64>)>();
        tokio::spawn(async move {
            while let Some((processed, total)) = rx.recv().await {
                if let Err(err) = queue.set_progress(job_id, processed, total).await {
                    tracing::warn!(%job_id, error = %err, "failed to write job progress to redis");
                }
            }
        });
        Self { tx, cancelled }
    }
}

impl Progress for RedisProgress {
    fn on_progress(&self, processed: u64, total: Option<u64>) {
        // Unbounded + best-effort: a dropped progress update just means a
        // slightly stale status for one poll, never a stalled codec.
        let _ = self.tx.send((processed, total));
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}
