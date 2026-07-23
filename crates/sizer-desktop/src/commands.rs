use serde::Serialize;
use sizer_codecs_image::ImageCodec;
use sizer_core::{CompressOptions, Detector, HashingSink, MagicByteDetector, NullProgress};
use tauri::AppHandle;
use tokio::io::AsyncReadExt;

use crate::progress::TauriProgress;

#[derive(Serialize)]
pub struct DetectResult {
    format: String,
    kind: String,
}

#[tauri::command]
pub async fn detect_format(path: String) -> Result<DetectResult, String> {
    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| e.to_string())?;
    let mut header = [0u8; 64];
    let n = file.read(&mut header).await.map_err(|e| e.to_string())?;
    let format = MagicByteDetector.sniff(&header[..n]);
    Ok(DetectResult {
        format: format!("{format:?}"),
        kind: format!("{:?}", format.kind()),
    })
}

#[derive(Serialize)]
pub struct CompressResult {
    input_bytes: u64,
    output_bytes: u64,
    ratio: f64,
    verified: Option<bool>,
}

#[tauri::command]
pub async fn compress_archive(
    app: AppHandle,
    input: String,
    output: String,
    codec: String,
    effort: u8,
    verify: bool,
) -> Result<CompressResult, String> {
    let codec_impl = sizer_registry::codec_by_name(&codec).map_err(|e| e.to_string())?;
    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let progress = TauriProgress::new(app, "compress-progress");

    let mut reader = tokio::fs::File::open(&input)
        .await
        .map_err(|e| e.to_string())?;
    let mut writer = tokio::fs::File::create(&output)
        .await
        .map_err(|e| e.to_string())?;
    codec_impl
        .compress(&mut reader, &mut writer, &options, &progress)
        .await
        .map_err(|e| e.to_string())?;

    let input_bytes = tokio::fs::metadata(&input)
        .await
        .map_err(|e| e.to_string())?
        .len();
    let output_bytes = tokio::fs::metadata(&output)
        .await
        .map_err(|e| e.to_string())?
        .len();

    let verified = if verify {
        let original_digest = sizer_core::hash(
            tokio::fs::File::open(&input)
                .await
                .map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;

        let format = detect_format_bytes(&output).await?;
        let decode_codec = sizer_registry::codec_for_format(format).map_err(|e| e.to_string())?;
        let mut compressed = tokio::fs::File::open(&output)
            .await
            .map_err(|e| e.to_string())?;
        let mut sink = HashingSink::new();
        decode_codec
            .decompress(
                &mut compressed,
                &mut sink,
                &CompressOptions {
                    max_decompressed_bytes: input_bytes.saturating_mul(1000).max(1_000_000),
                    ..Default::default()
                },
                &NullProgress,
            )
            .await
            .map_err(|e| e.to_string())?;

        Some(sink.finalize() == original_digest)
    } else {
        None
    };

    Ok(CompressResult {
        input_bytes,
        output_bytes,
        ratio: input_bytes as f64 / output_bytes.max(1) as f64,
        verified,
    })
}

#[derive(Serialize)]
pub struct DecompressResult {
    input_bytes: u64,
    output_bytes: u64,
}

#[tauri::command]
pub async fn decompress_archive(
    app: AppHandle,
    input: String,
    output: String,
    max_decompressed_bytes: Option<u64>,
) -> Result<DecompressResult, String> {
    let format = detect_format_bytes(&input).await?;
    let codec = sizer_registry::codec_for_format(format).map_err(|e| e.to_string())?;

    let input_bytes = tokio::fs::metadata(&input)
        .await
        .map_err(|e| e.to_string())?
        .len();
    let limit =
        max_decompressed_bytes.unwrap_or_else(|| input_bytes.saturating_mul(10).max(1_000_000));

    let options = CompressOptions {
        max_decompressed_bytes: limit,
        ..Default::default()
    };
    let progress = TauriProgress::new(app, "decompress-progress");

    let mut reader = tokio::fs::File::open(&input)
        .await
        .map_err(|e| e.to_string())?;
    let mut writer = tokio::fs::File::create(&output)
        .await
        .map_err(|e| e.to_string())?;
    codec
        .decompress(&mut reader, &mut writer, &options, &progress)
        .await
        .map_err(|e| e.to_string())?;

    let output_bytes = tokio::fs::metadata(&output)
        .await
        .map_err(|e| e.to_string())?
        .len();

    Ok(DecompressResult {
        input_bytes,
        output_bytes,
    })
}

#[derive(Serialize)]
pub struct ImageCompressResult {
    input_bytes: u64,
    output_bytes: u64,
    ratio: f64,
    is_lossless: bool,
    fidelity: Option<FidelityInfo>,
}

#[derive(Serialize)]
pub struct FidelityInfo {
    width: u32,
    height: u32,
    max_channel_delta: u8,
    mean_channel_delta: f64,
    exact_match: bool,
}

#[tauri::command]
pub async fn compress_image(
    input: String,
    output: String,
    codec: String,
    effort: u8,
    check_fidelity: bool,
) -> Result<ImageCompressResult, String> {
    let codec_impl = sizer_registry::image_codec_by_name(&codec).map_err(|e| e.to_string())?;
    let input_bytes_data = tokio::fs::read(&input).await.map_err(|e| e.to_string())?;
    let input_bytes = input_bytes_data.len() as u64;

    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let output_bytes_data = codec_impl
        .recompress(input_bytes_data.clone(), &options)
        .await
        .map_err(|e| e.to_string())?;
    let output_bytes = output_bytes_data.len() as u64;
    tokio::fs::write(&output, &output_bytes_data)
        .await
        .map_err(|e| e.to_string())?;

    let fidelity = if check_fidelity {
        let report = sizer_codecs_image::compare_pixels(&input_bytes_data, &output_bytes_data)
            .map_err(|e| e.to_string())?;
        Some(FidelityInfo {
            width: report.width,
            height: report.height,
            max_channel_delta: report.max_channel_delta,
            mean_channel_delta: report.mean_channel_delta,
            exact_match: report.is_exact_match(),
        })
    } else {
        None
    };

    Ok(ImageCompressResult {
        input_bytes,
        output_bytes,
        ratio: input_bytes as f64 / output_bytes.max(1) as f64,
        is_lossless: codec_impl.is_lossless(),
        fidelity,
    })
}

#[derive(Serialize)]
pub struct VideoCompressResult {
    input_bytes: u64,
    output_bytes: u64,
    ratio: f64,
}

#[tauri::command]
pub async fn compress_video(
    app: AppHandle,
    input: String,
    output: String,
    codec: String,
    effort: u8,
) -> Result<VideoCompressResult, String> {
    let codec_impl = sizer_registry::video_codec_by_name(&codec).map_err(|e| e.to_string())?;
    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let progress = TauriProgress::new(app, "video-compress-progress");

    let input_path = std::path::Path::new(&input);
    let output_path = std::path::Path::new(&output);
    codec_impl
        .recompress(input_path, output_path, &options, &progress)
        .await
        .map_err(|e| e.to_string())?;

    let input_bytes = tokio::fs::metadata(&input)
        .await
        .map_err(|e| e.to_string())?
        .len();
    let output_bytes = tokio::fs::metadata(&output)
        .await
        .map_err(|e| e.to_string())?
        .len();

    Ok(VideoCompressResult {
        input_bytes,
        output_bytes,
        ratio: input_bytes as f64 / output_bytes.max(1) as f64,
    })
}

#[derive(Serialize)]
pub struct DocumentCompressResult {
    input_bytes: u64,
    output_bytes: u64,
    ratio: f64,
    images_recompressed: usize,
    images_skipped: usize,
}

#[tauri::command]
pub async fn compress_document(
    input: String,
    output: String,
    codec: String,
    effort: u8,
) -> Result<DocumentCompressResult, String> {
    let codec_impl = sizer_registry::document_codec_by_name(&codec).map_err(|e| e.to_string())?;
    let input_bytes_data = tokio::fs::read(&input).await.map_err(|e| e.to_string())?;
    let input_bytes = input_bytes_data.len() as u64;

    let options = CompressOptions {
        effort,
        ..Default::default()
    };
    let report = codec_impl
        .recompress(input_bytes_data, &options)
        .await
        .map_err(|e| e.to_string())?;
    let output_bytes = report.output.len() as u64;
    tokio::fs::write(&output, &report.output)
        .await
        .map_err(|e| e.to_string())?;

    Ok(DocumentCompressResult {
        input_bytes,
        output_bytes,
        ratio: input_bytes as f64 / output_bytes.max(1) as f64,
        images_recompressed: report.images_recompressed,
        images_skipped: report.images_skipped,
    })
}

#[derive(Serialize)]
pub struct TargetSizeResult {
    input_bytes: u64,
    output_bytes: u64,
    ratio: f64,
    achieved_quality: u8,
    /// Whether `output_bytes` actually came in at or under the requested
    /// target -- JPEG quality 1 (the lowest this codec goes) still might
    /// not be small enough for an unreasonably low target, and the caller
    /// should know that rather than silently getting an oversized file.
    hit_target: bool,
    iterations: u32,
}

/// Binary-searches JPEG quality (1..=100) for the encode closest to
/// `target_bytes` without exceeding it. Deliberately JPEG-only: PNG is
/// lossless (its output size is whatever the entropy actually is, not a
/// dial you can turn), and archive/video/PDF have no per-call quality
/// knob this codebase currently converges on a byte target with either --
/// see `crates/sizer-cloud/README.md`'s "target size" note in
/// `docs/ARCHITECTURE.md` for the same JPEG-only scoping reasoning applied
/// to the cloud surface.
#[tauri::command]
pub async fn compress_image_to_target_size(
    input: String,
    output: String,
    target_bytes: u64,
) -> Result<TargetSizeResult, String> {
    let input_bytes_data = tokio::fs::read(&input).await.map_err(|e| e.to_string())?;
    let input_bytes = input_bytes_data.len() as u64;
    let codec = sizer_codecs_image::JpegCodec;

    let mut low: u8 = 1;
    let mut high: u8 = 100;
    // Largest candidate seen that's still <= target (the answer we want).
    let mut best_under: Option<(u8, Vec<u8>)> = None;
    // Smallest candidate seen overall, in case even quality=1 can't get
    // under target -- better to return the smallest we could make than
    // to fail outright.
    let mut smallest: Option<(u8, Vec<u8>)> = None;
    let mut iterations = 0u32;

    while low <= high {
        iterations += 1;
        let mid = low + (high - low) / 2;
        let options = CompressOptions {
            effort: mid,
            ..Default::default()
        };
        let candidate = codec
            .recompress(input_bytes_data.clone(), &options)
            .await
            .map_err(|e| e.to_string())?;

        if smallest
            .as_ref()
            .is_none_or(|(_, s)| candidate.len() < s.len())
        {
            smallest = Some((mid, candidate.clone()));
        }

        let candidate_len = candidate.len() as u64;
        if candidate_len <= target_bytes {
            if best_under
                .as_ref()
                .is_none_or(|(_, s)| candidate.len() > s.len())
            {
                best_under = Some((mid, candidate));
            }
            if mid == 100 {
                break;
            }
            low = mid + 1; // try for closer-to-target (larger, higher quality)
        } else {
            if mid == 1 {
                break;
            }
            high = mid - 1; // still too big, need lower quality
        }
    }

    let hit_target = best_under.is_some();
    let (achieved_quality, output_bytes_data) = best_under
        .or(smallest)
        .ok_or_else(|| "target-size search produced no candidate".to_string())?;
    let output_bytes = output_bytes_data.len() as u64;
    tokio::fs::write(&output, &output_bytes_data)
        .await
        .map_err(|e| e.to_string())?;

    Ok(TargetSizeResult {
        input_bytes,
        output_bytes,
        ratio: input_bytes as f64 / output_bytes.max(1) as f64,
        achieved_quality,
        hit_target,
        iterations,
    })
}

#[derive(Serialize)]
pub struct ConvertImageResult {
    input_bytes: u64,
    output_bytes: u64,
}

#[tauri::command]
pub async fn convert_image(
    input: String,
    output: String,
    target_format: String,
) -> Result<ConvertImageResult, String> {
    let input_bytes_data = tokio::fs::read(&input).await.map_err(|e| e.to_string())?;
    let input_bytes = input_bytes_data.len() as u64;

    let output_bytes_data = sizer_codecs_convert::convert_image(input_bytes_data, &target_format)
        .await
        .map_err(|e| e.to_string())?;
    let output_bytes = output_bytes_data.len() as u64;
    tokio::fs::write(&output, &output_bytes_data)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ConvertImageResult {
        input_bytes,
        output_bytes,
    })
}

#[derive(Serialize)]
pub struct ImagesToPdfResult {
    output_bytes: u64,
    page_count: usize,
}

#[tauri::command]
pub async fn images_to_pdf(
    inputs: Vec<String>,
    output: String,
) -> Result<ImagesToPdfResult, String> {
    let mut images = Vec::with_capacity(inputs.len());
    for path in &inputs {
        images.push(tokio::fs::read(path).await.map_err(|e| e.to_string())?);
    }
    let page_count = images.len();

    let pdf_bytes = sizer_codecs_convert::images_to_pdf(images)
        .await
        .map_err(|e| e.to_string())?;
    let output_bytes = pdf_bytes.len() as u64;
    tokio::fs::write(&output, &pdf_bytes)
        .await
        .map_err(|e| e.to_string())?;

    Ok(ImagesToPdfResult {
        output_bytes,
        page_count,
    })
}

#[derive(Serialize)]
pub struct MergePdfsResult {
    output_bytes: u64,
    input_count: usize,
}

#[tauri::command]
pub async fn merge_pdfs(inputs: Vec<String>, output: String) -> Result<MergePdfsResult, String> {
    let mut documents = Vec::with_capacity(inputs.len());
    for path in &inputs {
        documents.push(tokio::fs::read(path).await.map_err(|e| e.to_string())?);
    }
    let input_count = documents.len();

    let merged = sizer_codecs_convert::merge_pdfs(documents)
        .await
        .map_err(|e| e.to_string())?;
    let output_bytes = merged.len() as u64;
    tokio::fs::write(&output, &merged)
        .await
        .map_err(|e| e.to_string())?;

    Ok(MergePdfsResult {
        output_bytes,
        input_count,
    })
}

async fn detect_format_bytes(path: &str) -> Result<sizer_core::Format, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?;
    let mut header = [0u8; 64];
    let n = file.read(&mut header).await.map_err(|e| e.to_string())?;
    Ok(MagicByteDetector.sniff(&header[..n]))
}
