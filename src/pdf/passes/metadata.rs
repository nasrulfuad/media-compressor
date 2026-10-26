use lopdf::{Document, Object, ObjectId};
use tracing::debug;

use crate::pdf::object_graph::{dict_name_is, resolve};

const METADATA_KEY: &[u8] = b"Metadata";

fn is_metadata_stream(document: &Document, value: &Object) -> bool {
    matches!(
        resolve(document, value),
        Object::Stream(stream) if dict_name_is(&stream.dict, b"Type", b"Metadata")
    )
}

fn owners_with_metadata_stream(document: &Document) -> Vec<ObjectId> {
    document
        .objects
        .iter()
        .filter(|(_, object)| {
            let dict = match object {
                Object::Dictionary(dict) => dict,
                Object::Stream(stream) => &stream.dict,
                _ => return false,
            };
            dict.get(METADATA_KEY)
                .is_ok_and(|value| is_metadata_stream(document, value))
        })
        .map(|(id, _)| *id)
        .collect()
}

fn remove_metadata_key(document: &mut Document, owner_id: ObjectId) {
    match document.objects.get_mut(&owner_id) {
        Some(Object::Dictionary(dict)) => {
            dict.remove(METADATA_KEY);
        }
        Some(Object::Stream(stream)) => {
            stream.dict.remove(METADATA_KEY);
        }
        _ => {}
    }
}

pub fn strip_metadata(document: &mut Document) {
    document.trailer.remove(b"Info");
    for owner_id in owners_with_metadata_stream(document) {
        remove_metadata_key(document, owner_id);
    }
    debug!("Metadata references removed");
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::test_support::{assert_no_dangling_references, PdfFixture};

    #[test]
    fn metadata_strip_removes_info_and_catalog_metadata_reference() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_info_and_xmp_metadata()
            .build();
        strip_metadata(&mut document);
        assert!(!document.trailer.has(b"Info"));
        assert!(!document.catalog().unwrap().has(b"Metadata"));
    }

    #[test]
    fn metadata_strip_removes_page_level_metadata_stream_reference() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let xmp = document.add_object(lopdf::Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x/>".to_vec(),
        ));
        document
            .get_dictionary_mut(page_id)
            .unwrap()
            .set("Metadata", Object::Reference(xmp));
        strip_metadata(&mut document);
        assert!(!document.get_dictionary(page_id).unwrap().has(b"Metadata"));
    }

    #[test]
    fn metadata_strip_keeps_metadata_keys_that_are_not_metadata_streams() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        document
            .get_dictionary_mut(page_id)
            .unwrap()
            .set("Metadata", Object::Integer(5));
        strip_metadata(&mut document);
        assert!(document.get_dictionary(page_id).unwrap().has(b"Metadata"));
    }

    #[test]
    fn metadata_strip_leaves_no_dangling_references() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_info_and_xmp_metadata()
            .build();
        strip_metadata(&mut document);
        document.prune_objects();
        assert_no_dangling_references(&document);
        assert!(!document.objects.values().any(
            |object| matches!(object, Object::Stream(stream) if stream.content == b"<x:xmpmeta/>")
        ));
    }

    #[test]
    fn metadata_strip_on_document_without_metadata_changes_nothing() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let before = document.objects.len();
        strip_metadata(&mut document);
        assert_eq!(document.objects.len(), before);
    }
}
