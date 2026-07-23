//! Cloud API + worker (M6): the fourth surface over the same `sizer-core` /
//! `sizer-codecs-*` engine the CLI, desktop, and browser builds use -- see
//! `docs/ARCHITECTURE.md` "Cloud mode is opt-in, not core" for why this is
//! a separate, off-by-default product rather than a core dependency.
//!
//! Two binaries share this lib: `sizer-cloud-api` (Axum HTTP surface --
//! auth, job bookkeeping, presigned upload/download URLs) and
//! `sizer-cloud-worker` (dequeues jobs from Redis, runs the actual codec,
//! uploads the result). They never share a process: the API never touches
//! file bytes directly (uploads/downloads go straight between the caller
//! and S3-compatible storage via presigned URLs), and the worker never
//! terminates an HTTP connection.

pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod progress;
pub mod queue;
pub mod routes;
pub mod storage;

pub use app::AppState;
pub use config::Config;
pub use error::ApiError;
