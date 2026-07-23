//! Redis-backed job queue + per-job progress/cancel state -- the "cloud
//! worker's Redis-backed [`sizer_core::Progress`]" that
//! docs/ARCHITECTURE.md's M3 section already anticipated as the fourth
//! `Progress` sink alongside the CLI's terminal bar, Tauri's event, and
//! the browser's `postMessage`.
//!
//! Memurai (Redis-protocol-compatible, native Windows) locally; any real
//! Redis-compatible service (Upstash in prod) elsewhere -- this only ever
//! calls the standard `redis` crate against a connection URL, never
//! anything provider-specific.

use std::collections::HashMap;
use std::time::Duration;

use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use uuid::Uuid;

const PENDING_LIST: &str = "sizer:jobs:pending";

#[derive(Clone)]
pub struct Queue {
    manager: ConnectionManager,
}

impl Queue {
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(url)?;
        let manager = client.get_connection_manager().await?;
        Ok(Self { manager })
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        let _: String = redis::cmd("PING").query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn enqueue(&self, job_id: Uuid) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        conn.lpush::<_, _, ()>(PENDING_LIST, job_id.to_string())
            .await?;
        Ok(())
    }

    /// Blocks (server-side, via `BRPOP`) for up to `timeout` waiting for a
    /// job. Returns `None` on timeout so the worker's loop can use the
    /// idle tick to run its retention sweep instead of busy-polling.
    ///
    /// Issued as a raw command with a `u64` argument rather than through
    /// `ConnectionLike::brpop` (whose signature hardcodes an `f64`
    /// timeout): `redis`'s float formatting emits a decimal (e.g. `"5"` vs.
    /// a form Redis <6 parses as non-integer), and Redis only accepts
    /// fractional `BRPOP` timeouts from version 6 on. A plain integer
    /// argument is valid on every version, including the Redis 5.0.14
    /// build (`redis-windows`) used for local dev here.
    pub async fn dequeue_blocking(&self, timeout: Duration) -> anyhow::Result<Option<Uuid>> {
        let mut conn = self.manager.clone();
        let result: Option<(String, String)> = redis::cmd("BRPOP")
            .arg(PENDING_LIST)
            .arg(timeout.as_secs().max(1))
            .query_async(&mut conn)
            .await?;
        match result {
            Some((_, id_str)) => Ok(Some(Uuid::parse_str(&id_str)?)),
            None => Ok(None),
        }
    }

    pub async fn set_progress(
        &self,
        job_id: Uuid,
        processed: u64,
        total: Option<u64>,
    ) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        let key = format!("sizer:job:{job_id}:progress");
        let total_val = total.map(|t| t.to_string()).unwrap_or_default();
        conn.hset_multiple::<_, _, _, ()>(
            &key,
            &[("processed", processed.to_string()), ("total", total_val)],
        )
        .await?;
        // Progress is transient job-run state, not a durable record --
        // let it expire on its own if a worker crashes mid-job rather than
        // leaving stale keys around forever.
        conn.expire::<_, ()>(&key, 3600).await?;
        Ok(())
    }

    pub async fn get_progress(&self, job_id: Uuid) -> anyhow::Result<Option<(u64, Option<u64>)>> {
        let mut conn = self.manager.clone();
        let key = format!("sizer:job:{job_id}:progress");
        let map: HashMap<String, String> = conn.hgetall(&key).await?;
        if map.is_empty() {
            return Ok(None);
        }
        let processed = map.get("processed").and_then(|s| s.parse().ok()).unwrap_or(0);
        let total = map
            .get("total")
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok());
        Ok(Some((processed, total)))
    }

    pub async fn request_cancel(&self, job_id: Uuid) -> anyhow::Result<()> {
        let mut conn = self.manager.clone();
        let key = format!("sizer:job:{job_id}:cancel");
        conn.set_ex::<_, _, ()>(&key, 1, 3600).await?;
        Ok(())
    }

    pub async fn is_cancel_requested(&self, job_id: Uuid) -> anyhow::Result<bool> {
        let mut conn = self.manager.clone();
        let key = format!("sizer:job:{job_id}:cancel");
        let exists: bool = conn.exists(&key).await?;
        Ok(exists)
    }
}
