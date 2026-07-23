use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use sizer_core::Progress;

/// Prints a throttled `\r`-updating progress line to stderr. Throttled to
/// roughly once per percentage point (or once per 1000 units when the
/// total is unknown) so a fast in-memory codec doesn't spend more time
/// printing than compressing.
///
/// `unit` labels whatever `processed`/`total` actually count for the
/// codec driving this -- "bytes" for archive/image codecs, "ms" for
/// video (`VideoCodec::recompress`'s doc comment). `Progress::on_progress`
/// itself has no fixed unit; this is where that gets made honest for a
/// human reading the terminal.
pub struct CliProgress {
    last_reported: AtomicU64,
    unit: &'static str,
}

impl Default for CliProgress {
    fn default() -> Self {
        Self::new("bytes")
    }
}

impl Progress for CliProgress {
    fn on_progress(&self, processed: u64, total: Option<u64>) {
        match total {
            Some(total) if total > 0 => {
                let percent = (processed * 100 / total).min(100);
                if percent != self.last_reported.swap(percent, Ordering::Relaxed) {
                    eprint!("\r  {percent:>3}%  ({processed} / {total} {})", self.unit);
                    let _ = std::io::stderr().flush();
                }
            }
            _ if self.unit == "bytes" => {
                let mb = processed / 1_000_000;
                if mb != self.last_reported.swap(mb, Ordering::Relaxed) {
                    eprint!("\r  {mb} MB processed");
                    let _ = std::io::stderr().flush();
                }
            }
            _ => {
                let thousands = processed / 1000;
                if thousands != self.last_reported.swap(thousands, Ordering::Relaxed) {
                    eprint!("\r  {processed} {} processed", self.unit);
                    let _ = std::io::stderr().flush();
                }
            }
        }
    }
}

impl CliProgress {
    pub fn new(unit: &'static str) -> Self {
        Self {
            last_reported: AtomicU64::new(0),
            unit,
        }
    }

    pub fn finish(&self) {
        eprintln!();
    }
}
