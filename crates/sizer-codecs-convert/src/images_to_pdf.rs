use printpdf::{
    ColorBits, ColorSpace, Image, ImageTransform, ImageXObject, Mm, PdfDocument,
    PdfDocumentReference, Px,
};
use sizer_core::{Error, Result};

/// Pixels are treated as this many dots per inch when computing each
/// page's physical size, so a page comes out exactly image-sized (no
/// cropping or letterboxing) -- 96 is the common "CSS pixel" / screen-DPI
/// convention, not a print-industry standard; it doesn't need to be, since
/// nothing here is measuring a real-world print size.
const DPI: f32 = 96.0;

/// Composes `images` (any format `image` can decode) into a new PDF, one
/// image per page, in the order given.
///
/// Only ever holds a single `PdfDocumentReference` (never clones it):
/// `PdfDocumentReference::save_to_bytes` calls `Rc::try_unwrap` internally
/// and panics if any other clone of the handle is still alive, so this
/// function is written to make that structurally impossible rather than
/// "probably fine" -- see printpdf 0.7's `pdf_document.rs`.
pub async fn images_to_pdf(images: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    if images.is_empty() {
        return Err(Error::UnsupportedFormat(
            "no images provided to convert to PDF".into(),
        ));
    }

    crate::run_blocking(move || {
        let mut doc_ref: Option<PdfDocumentReference> = None;

        for (index, bytes) in images.into_iter().enumerate() {
            let decoded = image::load_from_memory(&bytes)
                .map_err(|e| Error::UnsupportedFormat(format!("decoding image {index}: {e}")))?;
            let rgb = decoded.to_rgb8();
            let (width, height) = rgb.dimensions();
            let page_width = Mm(width as f32 / DPI * 25.4);
            let page_height = Mm(height as f32 / DPI * 25.4);

            let (page_index, layer_index) = match &doc_ref {
                None => {
                    let (doc, page_index, layer_index) =
                        PdfDocument::new("Sizer conversion", page_width, page_height, "Layer 1");
                    doc_ref = Some(doc);
                    (page_index, layer_index)
                }
                Some(doc) => doc.add_page(page_width, page_height, "Layer 1"),
            };

            let xobject = ImageXObject {
                width: Px(width as usize),
                height: Px(height as usize),
                color_space: ColorSpace::Rgb,
                bits_per_component: ColorBits::Bit8,
                interpolate: true,
                image_data: rgb.into_raw(),
                image_filter: None,
                smask: None,
                clipping_bbox: None,
            };

            // Borrow, don't clone: see the function-level doc comment.
            let doc = doc_ref.as_ref().expect("just set above");
            let layer = doc.get_page(page_index).get_layer(layer_index);
            Image::from(xobject).add_to_layer(
                layer,
                ImageTransform {
                    dpi: Some(DPI),
                    ..Default::default()
                },
            );
        }

        doc_ref
            .expect("checked non-empty at function entry")
            .save_to_bytes()
            .map_err(|e| Error::UnsupportedFormat(format!("writing pdf: {e}")))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x * 8) as u8, (y * 8) as u8, 100])
        });
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::Png,
            )
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn composes_multiple_images_into_a_multi_page_pdf() {
        let images = vec![tiny_png(32, 32), tiny_png(64, 48), tiny_png(16, 16)];
        let pdf_bytes = images_to_pdf(images).await.unwrap();

        assert!(pdf_bytes.starts_with(b"%PDF"));
        let doc = lopdf::Document::load_mem(&pdf_bytes).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
    }

    #[tokio::test]
    async fn rejects_empty_input() {
        let err = images_to_pdf(vec![]).await.unwrap_err();
        assert!(matches!(err, Error::UnsupportedFormat(_)));
    }
}
