use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use sizer_core::{CompressOptions, Error, Progress, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use crate::probe::{missing_tool_err, probe_duration_ms};
use crate::{effort_to_crf, VideoCodec};

/// Shells out to `ffmpeg` with libx264 and a CRF derived from
/// `options.effort` (see `effort_to_crf`). Audio is copied through
/// unmodified (`-c:a copy`) -- video is the size driver for almost every
/// real file, and silently re-encoding audio too would be a second,
/// separate lossy decision this codec shouldn't make on the caller's
/// behalf.
///
/// Requires `libx264` in the invoking user's `ffmpeg` build. This crate
/// doesn't control or vendor that build (see the crate-level doc
/// comment) -- if it's missing, `ffmpeg` itself reports the error and
/// this codec surfaces it rather than silently falling back to a
/// different encoder the caller didn't ask for.
#[derive(Debug, Default)]
pub struct FfmpegCodec;

#[async_trait]
impl VideoCodec for FfmpegCodec {
    fn name(&self) -> &'static str {
        "ffmpeg"
    }

    async fn recompress(
        &self,
        input: &Path,
        output: &Path,
        options: &CompressOptions,
        progress: &dyn Progress,
    ) -> Result<()> {
        let crf = effort_to_crf(options.effort);
        // Best-effort: an unknown duration just means progress reports
        // as "time processed" without a percentage, not a hard failure.
        let total_ms = probe_duration_ms(input).await.unwrap_or(None);

        let mut child = Command::new("ffmpeg")
            .arg("-y")
            .arg("-i")
            .arg(input)
            .args([
                "-c:v",
                "libx264",
                "-crf",
                &crf.to_string(),
                "-preset",
                "medium",
                "-c:a",
                "copy",
                "-progress",
                "pipe:1",
                "-nostats",
                "-loglevel",
                "error",
            ])
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| missing_tool_err("ffmpeg", e))?;

        let stdout = child.stdout.take().expect("stdout was piped at spawn");
        let stderr = child.stderr.take().expect("stderr was piped at spawn");

        // Drained concurrently with stdout, not after: if ffmpeg writes
        // enough to stderr (e.g. a wall of warnings) to fill the OS pipe
        // buffer while nothing is reading it, ffmpeg blocks on that
        // write and the stdout progress loop below hangs forever waiting
        // for a process that's stuck, not finished.
        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut buf).await;
            buf
        });

        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await.map_err(Error::Io)? {
            if progress.is_cancelled() {
                let _ = child.kill().await;
                return Err(Error::Cancelled);
            }
            // ffmpeg's `-progress` output has a well-known naming quirk:
            // the `out_time_ms` key actually reports **microseconds**,
            // not milliseconds -- `out_time_us` is the (also-microsecond)
            // field with the honest name. Divide by 1000 so this stays
            // in milliseconds, matching probe_duration_ms and this
            // trait's documented progress units.
            if let Some(us_str) = line.strip_prefix("out_time_us=") {
                if let Ok(processed_us) = us_str.parse::<u64>() {
                    progress.on_progress(processed_us / 1000, total_ms);
                }
            }
        }

        let status = child.wait().await.map_err(Error::Io)?;
        let stderr_output = stderr_task.await.unwrap_or_default();

        if !status.success() {
            return Err(Error::UnsupportedFormat(format!(
                "ffmpeg exited with {status}: {}",
                stderr_output.trim()
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// Skips (rather than fails) when ffmpeg/ffprobe aren't on PATH --
    /// this crate deliberately doesn't bundle them (see the crate-level
    /// doc comment), so a dev machine or CI runner without FFmpeg
    /// installed is an environment gap, not a code bug. CI installs
    /// ffmpeg specifically so these tests do run there; see
    /// .github/workflows/ci.yml.
    macro_rules! require_ffmpeg {
        () => {
            if std::process::Command::new("ffmpeg")
                .arg("-version")
                .output()
                .is_err()
            {
                eprintln!("skipping: ffmpeg not found on PATH");
                return;
            }
        };
    }

    /// Generates a tiny synthetic test video via ffmpeg's `lavfi`
    /// test-source filter -- no external test asset needed.
    fn make_test_video(path: &std::path::Path) {
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-nostats",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=64x64:rate=10",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(path)
            .status()
            .expect("failed to run ffmpeg to generate test fixture");
        assert!(status.success(), "ffmpeg fixture generation failed");
    }

    #[tokio::test]
    async fn recompresses_a_real_video_and_reports_progress() {
        require_ffmpeg!();

        let dir = std::env::temp_dir().join(format!("sizer-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.mp4");
        let output = dir.join("output.mp4");
        make_test_video(&input);

        struct CountingProgress {
            calls: AtomicU64,
        }
        impl Progress for CountingProgress {
            fn on_progress(&self, _processed: u64, _total: Option<u64>) {
                self.calls.fetch_add(1, Ordering::Relaxed);
            }
        }
        let progress = Arc::new(CountingProgress {
            calls: AtomicU64::new(0),
        });
        let progress_ref: Arc<dyn Progress> = progress.clone();

        FfmpegCodec
            .recompress(
                &input,
                &output,
                &CompressOptions {
                    effort: 80,
                    ..Default::default()
                },
                progress_ref.as_ref(),
            )
            .await
            .unwrap();

        assert!(output.exists());
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
        assert!(
            progress.calls.load(Ordering::Relaxed) > 0,
            "expected at least one progress callback"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_tool_error_names_the_tool_and_points_at_path() {
        // Doesn't need require_ffmpeg! -- this tests missing_tool_err's
        // message construction directly rather than the full spawn path
        // (FfmpegCodec always spawns literally "ffmpeg", so genuinely
        // exercising "binary not found" end-to-end would require ffmpeg
        // to be absent, which contradicts the other test in this file).
        let err = missing_tool_err("ffmpeg", std::io::Error::from(std::io::ErrorKind::NotFound));
        let message = err.to_string();
        assert!(message.contains("ffmpeg"));
        assert!(message.contains("PATH"));
    }
}
