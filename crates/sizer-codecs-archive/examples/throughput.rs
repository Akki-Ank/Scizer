//! In-process throughput micro-benchmark: how fast is each codec alone,
//! with process-spawn overhead excluded (the CLI's `bench` subcommand
//! covers wall-clock comparisons against system `gzip`/`zstd` instead —
//! this one isolates the codec).
//!
//! Deliberately dependency-free (no criterion/zerocopy): a plain
//! `std::time::Instant` loop, run in release mode for a meaningful number.
//!
//! Run with: `cargo run --release -p sizer-codecs-archive --example throughput`
//!
//! Native-only below `main`: compares against `ZstdCodec`, which doesn't
//! exist on wasm32 (see lib.rs), and uses the multi-thread
//! `tokio::runtime::Runtime` builder, which isn't available there either.
//! `main` itself is a no-op on wasm32 purely so this target still compiles
//! for `cargo check`/clippy there -- it was never meant to run in a
//! browser regardless.

#[cfg(not(target_arch = "wasm32"))]
use sizer_codecs_archive::{GzipCodec, ZstdCodec};
#[cfg(not(target_arch = "wasm32"))]
use sizer_core::{Codec, CompressOptions, NullProgress};
#[cfg(not(target_arch = "wasm32"))]
use std::io::Cursor;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Runtime;

#[cfg(not(target_arch = "wasm32"))]
const ITERATIONS: usize = 10;
#[cfg(not(target_arch = "wasm32"))]
const INPUT_SIZE: usize = 8_000_000;

#[cfg(not(target_arch = "wasm32"))]
fn payload() -> Vec<u8> {
    // Mixed-compressibility: repetitive enough to compress well, varied
    // enough not to degenerate into a single RLE run.
    (0..INPUT_SIZE as u32).map(|i| (i % 251) as u8).collect()
}

#[cfg(not(target_arch = "wasm32"))]
async fn bench_codec(name: &str, codec: &dyn Codec, input: &[u8], effort: u8) {
    let options = CompressOptions {
        effort,
        ..Default::default()
    };

    let mut sizes = Vec::with_capacity(ITERATIONS);
    let mut durations = Vec::with_capacity(ITERATIONS);

    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let mut out = Vec::new();
        codec
            .compress(&mut Cursor::new(input), &mut out, &options, &NullProgress)
            .await
            .unwrap();
        durations.push(start.elapsed());
        sizes.push(out.len());
    }

    let total_nanos: u128 = durations.iter().map(|d| d.as_nanos()).sum();
    let mean = total_nanos / ITERATIONS as u128;
    let min = durations.iter().min().unwrap();
    let avg_size = sizes.iter().sum::<usize>() / sizes.len();
    let ratio = input.len() as f64 / avg_size as f64;
    let mb_per_sec = (input.len() as f64 / (mean as f64 / 1_000_000_000.0)) / 1_000_000.0;

    println!(
        "{name:<8} effort={effort:<4} mean={mean_ms:>7.2}ms  min={min_ms:>7.2}ms  \
         throughput={mb_per_sec:>7.1} MB/s  ratio={ratio:>5.2}x  ({avg_size} bytes)",
        mean_ms = mean as f64 / 1_000_000.0,
        min_ms = min.as_nanos() as f64 / 1_000_000.0,
    );
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let rt = Runtime::new().unwrap();
    let input = payload();

    println!(
        "Comparing sizer-codecs-archive codecs on an {:.1}MB mixed-compressibility payload, \
         {ITERATIONS} iterations per row (release build recommended: \
         `cargo run --release ...`).\n",
        input.len() as f64 / 1_000_000.0
    );

    rt.block_on(async {
        for effort in [10, 50, 90] {
            bench_codec("gzip", &GzipCodec, &input, effort).await;
            bench_codec("zstd", &ZstdCodec, &input, effort).await;
            println!();
        }
    });
}
