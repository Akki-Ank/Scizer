use serde::Serialize;
use sizer_core::Progress;
use tauri::{AppHandle, Emitter};

/// Emits progress as a Tauri event to the webview instead of printing to a
/// terminal (the CLI's `CliProgress`) or writing to a job-status field in
/// Redis (the future cloud worker). Same `sizer_core::Progress` trait,
/// three different sinks -- exactly the point of the trait existing in
/// `sizer-core` in the first place.
pub struct TauriProgress {
    app: AppHandle,
    event_name: &'static str,
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    processed: u64,
    total: Option<u64>,
}

impl TauriProgress {
    pub fn new(app: AppHandle, event_name: &'static str) -> Self {
        Self { app, event_name }
    }
}

impl Progress for TauriProgress {
    fn on_progress(&self, processed: u64, total: Option<u64>) {
        // Best-effort: a dropped progress event isn't worth failing the
        // whole compression job over.
        let _ = self
            .app
            .emit(self.event_name, ProgressPayload { processed, total });
    }
}
