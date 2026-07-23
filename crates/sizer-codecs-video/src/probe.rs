use std::path::Path;

use sizer_core::{Error, Result};
use tokio::process::Command;

/// Returns the input's duration in milliseconds via `ffprobe`, or `None`
/// if it can't be determined -- callers fall back to a spinner-style
/// progress display (bytes/time processed, no percentage) in that case
/// rather than treating it as a hard error.
pub async fn probe_duration_ms(input: &Path) -> Result<Option<u64>> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(input)
        .output()
        .await
        .map_err(|e| missing_tool_err("ffprobe", e))?;

    if !output.status.success() {
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    match text.trim().parse::<f64>() {
        Ok(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Ok(Some((seconds * 1000.0).round() as u64))
        }
        _ => Ok(None),
    }
}

/// A `NotFound` io::Error (the tool isn't on PATH) gets a message
/// pointing at the actual fix; anything else (permissions, etc.) passes
/// through as a plain `Error::Io`.
pub(crate) fn missing_tool_err(tool: &str, source: std::io::Error) -> Error {
    if source.kind() == std::io::ErrorKind::NotFound {
        Error::UnsupportedFormat(format!(
            "`{tool}` not found on PATH -- sizer-codecs-video shells out to a system-installed \
             ffmpeg/ffprobe rather than bundling one; install FFmpeg and make sure {tool} is on \
             PATH"
        ))
    } else {
        Error::Io(source)
    }
}
