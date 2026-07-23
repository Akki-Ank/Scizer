mod bench;
mod progress;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use sizer_core::{Detector, HashingSink, MagicByteDetector};
use tokio::io::AsyncReadExt;

use crate::progress::CliProgress;

#[derive(Parser)]
#[command(name = "sizer", version, about = "Sizer compression engine CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Identify a file's format from its magic bytes.
    Detect { path: PathBuf },

    /// Compress a file with an explicitly chosen codec.
    ///
    /// There's no auto-selection yet — that's later "Smart Engine"
    /// milestone work. For now the caller picks gzip or zstd.
    Compress {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "zstd")]
        codec: String,
        /// 0 (fastest/largest) .. 100 (slowest/smallest).
        #[arg(long, default_value_t = 50)]
        effort: u8,
        /// Decompress the output again after compressing and compare its
        /// hash against the original input — the "Verify integrity" step
        /// of the automatic engine pipeline (detect -> analyze ->
        /// benchmark -> choose -> compress -> verify).
        #[arg(long)]
        verify: bool,
    },

    /// Decompress a file. The codec is auto-detected from magic bytes —
    /// there is nothing to choose, unlike compress.
    Decompress {
        input: PathBuf,
        output: PathBuf,
        /// Refuses to write more than this many decompressed bytes,
        /// guarding against decompression bombs. Defaults to 10x the
        /// compressed input size, which is generous for gzip/zstd on real
        /// data but still bounded — pass --max-decompressed-bytes
        /// explicitly for untrusted input where even that ratio is
        /// unacceptable.
        #[arg(long)]
        max_decompressed_bytes: Option<u64>,
    },

    /// Compare sizer's codecs against system gzip/zstd (when installed)
    /// on a real file, for speed and output size.
    Bench {
        input: PathBuf,
        #[arg(long, default_value_t = 50)]
        effort: u8,
    },

    /// Recompress an image (PNG lossless re-optimization, or re-encode as
    /// JPEG at a target quality).
    ///
    /// This is a separate command from `compress`, not a `--codec` value
    /// on it: image recompression doesn't round-trip to the original
    /// bytes the way archive compression does, so it needs its own
    /// pixel-fidelity check instead of `compress --verify`'s SHA-256
    /// comparison. See docs/ARCHITECTURE.md ("Two codec shapes, not one").
    ImageCompress {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "png")]
        codec: String,
        /// For PNG: optimization effort 0..=100 (higher = tries harder,
        /// slower, still lossless). For JPEG: target quality 1..=100
        /// directly (higher = larger, closer to the original) -- see
        /// JpegCodec's doc comment for why these two are different axes.
        #[arg(long, default_value_t = 80)]
        effort: u8,
        /// Decode both the original and the recompressed output and
        /// report how close the pixels are. Errors out if a lossless
        /// codec (PNG) doesn't reproduce exact pixels -- that would be a
        /// correctness bug, not an expected trade-off.
        #[arg(long)]
        check_fidelity: bool,
    },

    /// Recompress a video with ffmpeg (must be installed and on PATH --
    /// Sizer shells out to it rather than bundling it, see
    /// docs/ARCHITECTURE.md "Video (M5): shell out, don't bundle").
    VideoCompress {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "ffmpeg")]
        codec: String,
        /// 0 (minimal compression, near-original quality) .. 100
        /// (maximum compression, lowest quality) -- same direction as
        /// the archive codecs' effort, unlike image-compress's JPEG
        /// quality (which is inverted). See VideoCodec's doc comment.
        #[arg(long, default_value_t = 50)]
        effort: u8,
        /// Probe the output with ffprobe and confirm it's a valid video
        /// with a plausible duration (within 5% of the input's). Not a
        /// pixel-fidelity check like image-compress's --check-fidelity --
        /// that would mean decoding every frame of both videos, which is
        /// expensive enough not to run by default. This just catches
        /// "ffmpeg exited 0 but wrote garbage."
        #[arg(long)]
        check: bool,
    },

    /// Recompress a document's embedded images in place.
    ///
    /// Scope today: PDF only, and only its embedded JPEG (DCTDecode)
    /// image streams -- no page rasterization, no font subsetting, no
    /// Office formats yet. See docs/ARCHITECTURE.md.
    DocumentCompress {
        input: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "pdf")]
        codec: String,
        /// JPEG quality 1..=100 applied to each embedded image found --
        /// same convention as image-compress's JPEG codec.
        #[arg(long, default_value_t = 60)]
        effort: u8,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Command::Detect { path } => detect(&path).await?,
        Command::Compress {
            input,
            output,
            codec,
            effort,
            verify,
        } => compress(&input, &output, &codec, effort, verify).await?,
        Command::Decompress {
            input,
            output,
            max_decompressed_bytes,
        } => decompress(&input, &output, max_decompressed_bytes).await?,
        Command::Bench { input, effort } => bench::run(&input, effort).await?,
        Command::ImageCompress {
            input,
            output,
            codec,
            effort,
            check_fidelity,
        } => image_compress(&input, &output, &codec, effort, check_fidelity).await?,
        Command::VideoCompress {
            input,
            output,
            codec,
            effort,
            check,
        } => video_compress(&input, &output, &codec, effort, check).await?,
        Command::DocumentCompress {
            input,
            output,
            codec,
            effort,
        } => document_compress(&input, &output, &codec, effort).await?,
    }

    Ok(())
}

async fn detect(path: &PathBuf) -> anyhow::Result<()> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut header = [0u8; 64];
    let n = file.read(&mut header).await?;

    let format = MagicByteDetector.sniff(&header[..n]);
    println!("{}: {:?} ({:?})", path.display(), format, format.kind());
    Ok(())
}

async fn compress(
    input: &PathBuf,
    output: &PathBuf,
    codec_name: &str,
    effort: u8,
    verify: bool,
) -> anyhow::Result<()> {
    let codec = sizer_registry::codec_by_name(codec_name)?;
    let options = sizer_core::CompressOptions {
        effort,
        ..Default::default()
    };
    let progress = CliProgress::default();

    let mut reader = tokio::fs::File::open(input).await?;
    let mut writer = tokio::fs::File::create(output).await?;
    codec
        .compress(&mut reader, &mut writer, &options, &progress)
        .await?;
    progress.finish();

    let input_len = tokio::fs::metadata(input).await?.len();
    let output_len = tokio::fs::metadata(output).await?.len();
    println!(
        "{} -> {} ({} -> {} bytes, {:.2}x)",
        input.display(),
        output.display(),
        input_len,
        output_len,
        input_len as f64 / output_len.max(1) as f64
    );

    if verify {
        print!("verifying... ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let original_digest = sizer_core::hash(tokio::fs::File::open(input).await?).await?;

        let format = sniff_file(output).await?;
        let decode_codec = sizer_registry::codec_for_format(format)?;
        let mut compressed = tokio::fs::File::open(output).await?;
        let mut sink = HashingSink::new();
        decode_codec
            .decompress(
                &mut compressed,
                &mut sink,
                &sizer_core::CompressOptions {
                    max_decompressed_bytes: input_len.saturating_mul(1000).max(1_000_000),
                    ..Default::default()
                },
                &sizer_core::NullProgress,
            )
            .await?;

        if sink.finalize() == original_digest {
            println!("ok (sha256 matches)");
        } else {
            anyhow::bail!(
                "integrity check FAILED: decompressed output does not match original input"
            );
        }
    }

    Ok(())
}

async fn decompress(
    input: &PathBuf,
    output: &PathBuf,
    max_decompressed_bytes: Option<u64>,
) -> anyhow::Result<()> {
    let format = sniff_file(input).await?;
    let codec = sizer_registry::codec_for_format(format)?;

    let compressed_len = tokio::fs::metadata(input).await?.len();
    let limit =
        max_decompressed_bytes.unwrap_or_else(|| compressed_len.saturating_mul(10).max(1_000_000));

    let options = sizer_core::CompressOptions {
        max_decompressed_bytes: limit,
        ..Default::default()
    };
    let progress = CliProgress::default();

    let mut reader = tokio::fs::File::open(input).await?;
    let mut writer = tokio::fs::File::create(output).await?;
    codec
        .decompress(&mut reader, &mut writer, &options, &progress)
        .await?;
    progress.finish();

    let output_len = tokio::fs::metadata(output).await?.len();
    println!(
        "{} -> {} ({compressed_len} -> {output_len} bytes)",
        input.display(),
        output.display()
    );
    Ok(())
}

async fn image_compress(
    input: &PathBuf,
    output: &PathBuf,
    codec_name: &str,
    effort: u8,
    check_fidelity: bool,
) -> anyhow::Result<()> {
    let codec = sizer_registry::image_codec_by_name(codec_name)?;
    let input_bytes = tokio::fs::read(input).await?;
    let input_len = input_bytes.len() as u64;

    let options = sizer_core::CompressOptions {
        effort,
        ..Default::default()
    };
    let output_bytes = codec.recompress(input_bytes.clone(), &options).await?;
    let output_len = output_bytes.len() as u64;
    tokio::fs::write(output, &output_bytes).await?;

    println!(
        "{} -> {} ({} -> {} bytes, {:.2}x, codec={})",
        input.display(),
        output.display(),
        input_len,
        output_len,
        input_len as f64 / output_len.max(1) as f64,
        codec.name(),
    );

    if check_fidelity {
        print!("checking pixel fidelity... ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let report = sizer_codecs_image::compare_pixels(&input_bytes, &output_bytes)?;
        if codec.is_lossless() {
            if report.is_exact_match() {
                println!("ok (exact pixel match, {}x{})", report.width, report.height);
            } else {
                anyhow::bail!(
                    "lossless codec {} produced non-identical pixels (max channel delta {}) \
                     -- this is a codec bug, not an expected trade-off",
                    codec.name(),
                    report.max_channel_delta
                );
            }
        } else {
            println!(
                "ok ({}x{}, max channel delta {}, mean {:.2})",
                report.width, report.height, report.max_channel_delta, report.mean_channel_delta
            );
        }
    }

    Ok(())
}

async fn video_compress(
    input: &PathBuf,
    output: &PathBuf,
    codec_name: &str,
    effort: u8,
    check: bool,
) -> anyhow::Result<()> {
    let codec = sizer_registry::video_codec_by_name(codec_name)?;
    let options = sizer_core::CompressOptions {
        effort,
        ..Default::default()
    };
    let progress = CliProgress::new("ms");

    codec.recompress(input, output, &options, &progress).await?;
    progress.finish();

    let input_len = tokio::fs::metadata(input).await?.len();
    let output_len = tokio::fs::metadata(output).await?.len();
    println!(
        "{} -> {} ({} -> {} bytes, {:.2}x, codec={})",
        input.display(),
        output.display(),
        input_len,
        output_len,
        input_len as f64 / output_len.max(1) as f64,
        codec.name(),
    );

    if check {
        print!("checking output validity... ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let input_ms = sizer_codecs_video::probe_duration_ms(input).await?;
        let output_ms = sizer_codecs_video::probe_duration_ms(output).await?;

        match (input_ms, output_ms) {
            (Some(input_ms), Some(output_ms)) => {
                let diff = input_ms.abs_diff(output_ms);
                let tolerance = input_ms / 20; // 5%
                if diff <= tolerance {
                    println!("ok (duration {input_ms}ms -> {output_ms}ms)");
                } else {
                    anyhow::bail!(
                        "output duration {output_ms}ms differs from input {input_ms}ms by more \
                         than 5% -- ffmpeg likely produced a truncated or corrupt file"
                    );
                }
            }
            (None, _) => anyhow::bail!("could not determine input duration to compare against"),
            (_, None) => anyhow::bail!(
                "output does not probe as a valid video with a known duration -- ffmpeg likely \
                 produced a corrupt file"
            ),
        }
    }

    Ok(())
}

async fn document_compress(
    input: &PathBuf,
    output: &PathBuf,
    codec_name: &str,
    effort: u8,
) -> anyhow::Result<()> {
    let codec = sizer_registry::document_codec_by_name(codec_name)?;
    let input_bytes = tokio::fs::read(input).await?;
    let input_len = input_bytes.len() as u64;

    let options = sizer_core::CompressOptions {
        effort,
        ..Default::default()
    };
    let report = codec.recompress(input_bytes, &options).await?;
    let output_len = report.output.len() as u64;
    tokio::fs::write(output, &report.output).await?;

    println!(
        "{} -> {} ({} -> {} bytes, {:.2}x, codec={}, images recompressed={} skipped={})",
        input.display(),
        output.display(),
        input_len,
        output_len,
        input_len as f64 / output_len.max(1) as f64,
        codec.name(),
        report.images_recompressed,
        report.images_skipped,
    );

    Ok(())
}

async fn sniff_file(path: &PathBuf) -> anyhow::Result<sizer_core::Format> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut header = [0u8; 64];
    let n = file.read(&mut header).await?;
    Ok(MagicByteDetector.sniff(&header[..n]))
}
