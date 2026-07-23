use std::collections::BTreeMap;

use lopdf::{Document, Object, ObjectId};
use sizer_core::{Error, Result};

/// Concatenates multiple PDFs' pages into one new PDF, in the order given.
///
/// Adapted from lopdf's own bundled `examples/merge.rs` (the reference
/// pattern for this -- lopdf has no built-in `Document::merge`), with the
/// bookmark/table-of-contents machinery stripped out: that example builds
/// a navigable outline tree per merged document, which is more than this
/// crate's "just concatenate the pages" scope needs.
pub async fn merge_pdfs(inputs: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    if inputs.len() < 2 {
        return Err(Error::UnsupportedFormat(
            "merging needs at least two PDFs".into(),
        ));
    }

    crate::run_blocking(move || {
        let mut max_id = 1u32;
        let mut documents_pages: BTreeMap<ObjectId, Object> = BTreeMap::new();
        let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
        let mut document = Document::with_version("1.5");

        for (index, bytes) in inputs.iter().enumerate() {
            let mut doc = Document::load_mem(bytes)
                .map_err(|e| Error::UnsupportedFormat(format!("reading PDF {index}: {e}")))?;

            doc.renumber_objects_with(max_id);
            max_id = doc.max_id + 1;

            documents_pages.extend(
                doc.get_pages()
                    .into_values()
                    .map(|object_id| (object_id, doc.get_object(object_id).unwrap().to_owned())),
            );
            documents_objects.extend(doc.objects);
        }

        let mut catalog_object: Option<(ObjectId, Object)> = None;
        let mut pages_object: Option<(ObjectId, Object)> = None;

        // Every object except "Page" (handled separately below via
        // documents_pages, so each page's parent can be repointed at the
        // single merged "Pages" tree) goes into the merged document as-is.
        // "Catalog" and "Pages" are collected rather than copied directly
        // since every source PDF has its own; only the first of each
        // survives, updated to describe the merged document.
        for (object_id, object) in documents_objects {
            match object.type_name().unwrap_or(b"") {
                b"Catalog" => {
                    catalog_object.get_or_insert((object_id, object));
                }
                b"Pages" => {
                    if let Ok(dictionary) = object.as_dict() {
                        let mut dictionary = dictionary.clone();
                        if let Some((_, ref existing)) = pages_object {
                            if let Ok(existing_dict) = existing.as_dict() {
                                dictionary.extend(existing_dict);
                            }
                        }
                        let id = pages_object.as_ref().map_or(object_id, |(id, _)| *id);
                        pages_object = Some((id, Object::Dictionary(dictionary)));
                    }
                }
                b"Page" => {} // handled via documents_pages below
                b"Outlines" | b"Outline" => {} // not carried over -- see doc comment
                _ => {
                    document.objects.insert(object_id, object);
                }
            }
        }

        let (pages_id, pages_object) = pages_object
            .ok_or_else(|| Error::UnsupportedFormat("no Pages root found in input PDFs".into()))?;
        let (catalog_id, catalog_object) = catalog_object.ok_or_else(|| {
            Error::UnsupportedFormat("no Catalog root found in input PDFs".into())
        })?;

        for (object_id, object) in &documents_pages {
            if let Ok(dictionary) = object.as_dict() {
                let mut dictionary = dictionary.clone();
                dictionary.set("Parent", pages_id);
                document.objects.insert(*object_id, Object::Dictionary(dictionary));
            }
        }

        if let Ok(dictionary) = pages_object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Count", documents_pages.len() as u32);
            dictionary.set(
                "Kids",
                documents_pages
                    .keys()
                    .map(|id| Object::Reference(*id))
                    .collect::<Vec<_>>(),
            );
            document.objects.insert(pages_id, Object::Dictionary(dictionary));
        }

        if let Ok(dictionary) = catalog_object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Pages", pages_id);
            dictionary.remove(b"Outlines");
            document.objects.insert(catalog_id, Object::Dictionary(dictionary));
        }

        document.trailer.set("Root", catalog_id);
        document.max_id = document.objects.len() as u32;
        document.renumber_objects();
        document.adjust_zero_pages();
        document.compress();

        let mut out = Vec::new();
        document
            .save_to(&mut out)
            .map_err(|e| Error::UnsupportedFormat(format!("writing merged pdf: {e}")))?;
        Ok(out)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{content::Content, content::Operation, dictionary, Stream};

    fn fake_pdf(page_count: u32) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });

        let mut kids = Vec::new();
        for _ in 0..page_count {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![50.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal("hello")]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            });
            kids.push(page_id.into());
        }

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => page_count,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);

        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[tokio::test]
    async fn merges_page_counts_correctly() {
        let a = fake_pdf(2);
        let b = fake_pdf(3);
        let merged = merge_pdfs(vec![a, b]).await.unwrap();

        let doc = Document::load_mem(&merged).unwrap();
        assert_eq!(doc.get_pages().len(), 5);
    }

    #[tokio::test]
    async fn rejects_fewer_than_two_inputs() {
        let err = merge_pdfs(vec![fake_pdf(1)]).await.unwrap_err();
        assert!(matches!(err, Error::UnsupportedFormat(_)));
    }
}
