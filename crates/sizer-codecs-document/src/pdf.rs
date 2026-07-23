use async_trait::async_trait;
use lopdf::{Document, Object};
use sizer_codecs_image::{ImageCodec, JpegCodec};
use sizer_core::{CompressOptions, Error, Result};

use crate::{DocumentCodec, RecompressReport};

/// Recompresses embedded JPEG (`DCTDecode`) image streams in a PDF via
/// `sizer_codecs_image::JpegCodec`, in place. Does not rasterize pages,
/// touch fonts, or recompress non-JPEG image filters (`FlateDecode`-only
/// raster images, JBIG2, CCITT fax, JPX) -- see this crate's doc comment
/// for why that's the deliberately scoped v1 target.
#[derive(Debug, Default)]
pub struct PdfCodec;

#[async_trait]
impl DocumentCodec for PdfCodec {
    fn name(&self) -> &'static str {
        "pdf"
    }

    async fn recompress(
        &self,
        input: Vec<u8>,
        options: &CompressOptions,
    ) -> Result<RecompressReport> {
        let mut document = Document::load_mem(&input)
            .map_err(|e| Error::UnsupportedFormat(format!("parsing PDF: {e}")))?;

        let jpeg_codec = JpegCodec;
        let mut recompressed = 0usize;
        let mut skipped = 0usize;

        let object_ids: Vec<_> = document.objects.keys().copied().collect();
        for id in object_ids {
            let Some(Object::Stream(stream)) = document.objects.get(&id) else {
                continue;
            };
            if !is_dct_image_stream(stream) {
                continue;
            }

            let original = stream.content.clone();
            match jpeg_codec.recompress(original.clone(), options).await {
                Ok(new_content) if new_content.len() < original.len() => {
                    if let Some(Object::Stream(stream)) = document.objects.get_mut(&id) {
                        stream.set_content(new_content);
                    }
                    recompressed += 1;
                }
                Ok(_) => {
                    // Recompression didn't actually shrink this one (already
                    // near-optimal, or effort too low to beat it) -- leave
                    // the original bytes rather than swap in a same-or-larger
                    // stream.
                    skipped += 1;
                }
                Err(_) => {
                    // Not every DCTDecode stream is a JPEG `image` can
                    // decode (e.g. CMYK JPEGs are rare but real). Skip
                    // rather than fail the whole document over one
                    // unreadable image.
                    skipped += 1;
                }
            }
        }

        let mut output = Vec::new();
        document
            .save_to(&mut output)
            .map_err(|e| Error::UnsupportedFormat(format!("writing PDF: {e}")))?;

        Ok(RecompressReport {
            output,
            images_recompressed: recompressed,
            images_skipped: skipped,
        })
    }
}

fn is_dct_image_stream(stream: &lopdf::Stream) -> bool {
    let is_image = stream
        .dict
        .get(b"Subtype")
        .ok()
        .and_then(|o| o.as_name().ok())
        == Some(b"Image");
    if !is_image {
        return false;
    }

    match stream.dict.get(b"Filter") {
        Ok(Object::Name(name)) => name.as_slice() == b"DCTDecode",
        Ok(Object::Array(filters)) => filters
            .iter()
            .any(|f| f.as_name().ok() == Some(b"DCTDecode")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Stream};

    fn tiny_jpeg() -> Vec<u8> {
        // A real, decodable JPEG -- generated via sizer-codecs-image's own
        // encoder so this test has no external asset dependency.
        let mut img = image::RgbImage::new(64, 64);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 4) as u8, (y * 4) as u8, 128]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();

        // Re-encode that PNG as a (deliberately low-quality, so there's
        // real room to shrink further) JPEG via jpeg-encoder directly,
        // to build a minimal one-page PDF fixture around it.
        let decoded = image::load_from_memory(&buf).unwrap().to_rgb8();
        let mut jpeg_bytes = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg_bytes, 95)
            .encode(decoded.as_raw(), 64, 64, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        jpeg_bytes
    }

    fn make_test_pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let image_stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 64,
                "Height" => 64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            tiny_jpeg(),
        );
        let image_id = doc.add_object(image_stream);

        let content = "q 64 0 0 64 0 0 cm /Im0 Do Q";
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));

        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => image_id },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 64.into(), 64.into()],
        });

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    #[tokio::test]
    async fn recompresses_embedded_jpeg_and_stays_a_valid_pdf() {
        let input = make_test_pdf();

        let report = PdfCodec
            .recompress(
                input.clone(),
                &CompressOptions {
                    effort: 20, // low JPEG quality -> real shrink from the quality-95 fixture
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(report.images_recompressed, 1);
        assert_eq!(report.images_skipped, 0);
        assert!(
            report.output.len() < input.len(),
            "expected smaller output: {} -> {}",
            input.len(),
            report.output.len()
        );

        // Structural sanity: the output must still parse as a PDF with
        // the same page count.
        let reopened = Document::load_mem(&report.output).unwrap();
        assert_eq!(reopened.get_pages().len(), 1);
    }

    #[tokio::test]
    async fn text_only_pdf_recompresses_zero_images_without_erroring() {
        let mut doc = Document::with_version("1.5");
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf (hello) Tj ET".to_vec(),
        ));
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut input = Vec::new();
        doc.save_to(&mut input).unwrap();

        let report = PdfCodec
            .recompress(
                input,
                &CompressOptions {
                    effort: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(report.images_recompressed, 0);
        assert_eq!(report.images_skipped, 0);
        Document::load_mem(&report.output).unwrap();
    }
}
