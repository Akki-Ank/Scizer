use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use sizer_core::{Codec, CompressOptions, NullProgress};
use sizer_registry::codec_by_name;

const ITERATIONS: usize = 5;

/// Compares sizer's in-process codecs against the system `gzip`/`zstd`
/// binaries (when present on PATH) on a real file, both on speed and
/// output size — the "Benchmark against 7-Zip / zstd / ..." requirement
/// from the product spec, scoped to what M1 actually ships (gzip, zstd).
pub async fn run(input: &Path, effort: u8) -> anyhow::Result<()> {
    // Loaded into memory once so repeated in-process iterations don't
    // re-read from disk each time and so timing isolates codec work from
    // filesystem I/O. This is a benchmarking-harness exception to the
    // engine's own streaming rule (crates/*/src -- the codecs themselves
    // never buffer a whole file); it's fine here because the input is a
    // bounded sample the caller chose to benchmark with, not production
    // compression traffic.
    let bytes = std::fs::read(input)?;
    println!(
        "Benchmarking {} ({:.2} MB) at effort={effort}, {ITERATIONS} iterations for in-process codecs\n",
        input.display(),
        bytes.len() as f64 / 1_000_000.0
    );

    for name in ["gzip", "zstd"] {
        let codec = codec_by_name(name)?;
        let (mean, min, out_len) = bench_in_process(codec.as_ref(), &bytes, effort).await?;
        println!(
            "sizer/{name:<5} mean={:>7.2}ms  min={:>7.2}ms  output={out_len} bytes  ratio={:.2}x",
            mean.as_secs_f64() * 1000.0,
            min.as_secs_f64() * 1000.0,
            bytes.len() as f64 / out_len as f64,
        );

        if let Some((duration, size)) = bench_system_tool(name, input) {
            println!(
                "system/{name:<4} wall={:>7.2}ms  output={size} bytes  ratio={:.2}x",
                duration.as_secs_f64() * 1000.0,
                bytes.len() as f64 / size as f64,
            );
        } else {
            println!("system/{name:<4} not found on PATH, skipping comparison");
        }
        println!();
    }

    Ok(())
}

async fn bench_in_process(
    codec: &dyn Codec,
    input: &[u8],
    effort: u8,
) -> anyhow::Result<(Duration, Duration, usize)> {
    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let mut durations = Vec::with_capacity(ITERATIONS);
    let mut last_len = 0;

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let mut out = Vec::new();
        codec
            .compress(&mut Cursor::new(input), &mut out, &options, &NullProgress)
            .await?;
        durations.push(start.elapsed());
        last_len = out.len();
    }

    let total: Duration = durations.iter().sum();
    let mean = total / ITERATIONS as u32;
    let min = *durations.iter().min().unwrap();
    Ok((mean, min, last_len))
}

/// Runs the system binary once (process-spawn overhead dwarfs the
/// in-process iteration count anyway) and returns (wall time, output
/// size), or `None` if the tool isn't installed.
fn bench_system_tool(name: &str, input: &Path) -> Option<(Duration, u64)> {
    let start = Instant::now();
    let output = Command::new(name).arg("-c").arg(input).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some((start.elapsed(), output.stdout.len() as u64))
}
