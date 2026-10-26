use std::collections::BTreeSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

const MAX_REFERENCE_HOPS: usize = 32;

pub fn resolve<'a>(document: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    for _ in 0..MAX_REFERENCE_HOPS {
        match current {
            Object::Reference(id) => match document.objects.get(id) {
                Some(next) => current = next,
                None => return current,
            },
            _ => return current,
        }
    }
    current
}

pub fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object) {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

pub fn resolve_array<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Vec<Object>> {
    match resolve(document, object) {
        Object::Array(items) => Some(items),
        _ => None,
    }
}

pub fn name_equals(object: &Object, expected: &[u8]) -> bool {
    matches!(object, Object::Name(name) if name == expected)
}

pub fn dict_name_is(dict: &Dictionary, key: &[u8], expected: &[u8]) -> bool {
    dict.get(key)
        .is_ok_and(|value| name_equals(value, expected))
}

pub fn is_image_stream_dict(dict: &Dictionary) -> bool {
    dict_name_is(dict, b"Subtype", b"Image")
}

pub fn is_form_stream_dict(dict: &Dictionary) -> bool {
    dict_name_is(dict, b"Subtype", b"Form")
}

pub fn page_content_stream_ids(document: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let Ok(page) = document.get_dictionary(page_id) else {
        return Vec::new();
    };
    let Ok(contents) = page.get(b"Contents") else {
        return Vec::new();
    };
    match contents {
        Object::Reference(id) => match document.objects.get(id) {
            Some(Object::Array(items)) => reference_ids(items),
            Some(Object::Stream(_)) => vec![*id],
            _ => Vec::new(),
        },
        Object::Array(items) => reference_ids(items),
        _ => Vec::new(),
    }
}

fn reference_ids(items: &[Object]) -> Vec<ObjectId> {
    items
        .iter()
        .filter_map(|item| item.as_reference().ok())
        .collect()
}

pub fn content_stream_ids(document: &Document) -> BTreeSet<ObjectId> {
    let mut ids: BTreeSet<ObjectId> = document
        .page_iter()
        .flat_map(|page_id| page_content_stream_ids(document, page_id))
        .collect();
    for (id, object) in &document.objects {
        if let Object::Stream(stream) = object {
            if is_form_stream_dict(&stream.dict) {
                ids.insert(*id);
            }
        }
    }
    ids
}

pub enum DictSlot {
    Direct,
    Indirect(ObjectId),
}

pub fn child_dict_slot(document: &Document, owner: ObjectId, key: &[u8]) -> Option<DictSlot> {
    match document.get_dictionary(owner).ok()?.get(key).ok()? {
        Object::Dictionary(_) => Some(DictSlot::Direct),
        Object::Reference(id) => Some(DictSlot::Indirect(*id)),
        _ => None,
    }
}

pub fn child_dict_mut<'a>(
    document: &'a mut Document,
    owner: ObjectId,
    key: &[u8],
) -> Option<&'a mut Dictionary> {
    match child_dict_slot(document, owner, key)? {
        DictSlot::Indirect(id) => document.get_dictionary_mut(id).ok(),
        DictSlot::Direct => document
            .get_dictionary_mut(owner)
            .ok()?
            .get_mut(key)
            .ok()?
            .as_dict_mut()
            .ok(),
    }
}

pub fn catalog_id(document: &Document) -> Option<ObjectId> {
    document.trailer.get(b"Root").ok()?.as_reference().ok()
}

pub fn inherited_page_resources(document: &Document, page_id: ObjectId) -> Option<Dictionary> {
    let mut current = page_id;
    for _ in 0..MAX_REFERENCE_HOPS {
        let node = document.get_dictionary(current).ok()?;
        if let Ok(resources) = node.get(b"Resources") {
            return resolve_dict(document, resources).cloned();
        }
        current = node.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use lopdf::{dictionary, Stream};

    use super::*;
    use crate::test_support::{FixtureImage, PdfFixture};

    #[test]
    fn resolve_follows_reference_chain_to_value() {
        let mut document = Document::new();
        let target = document.add_object(Object::Integer(7));
        let hop = document.add_object(Object::Reference(target));
        let start = Object::Reference(hop);
        assert_eq!(resolve(&document, &start), &Object::Integer(7));
    }

    #[test]
    fn resolve_returns_dangling_reference_unchanged() {
        let document = Document::new();
        let start = Object::Reference((99, 0));
        assert_eq!(resolve(&document, &start), &start);
    }

    #[test]
    fn resolve_stops_on_reference_cycle() {
        let mut document = Document::new();
        let first = document.new_object_id();
        let second = document.add_object(Object::Reference(first));
        document.objects.insert(first, Object::Reference(second));
        let start = Object::Reference(first);
        assert!(matches!(resolve(&document, &start), Object::Reference(_)));
    }

    #[test]
    fn resolve_dict_reads_dictionary_and_stream_dictionary() {
        let mut document = Document::new();
        let dict_id = document.add_object(dictionary! { "A" => 1 });
        let stream_id = document.add_object(Stream::new(dictionary! { "B" => 2 }, vec![]));
        assert!(resolve_dict(&document, &Object::Reference(dict_id))
            .unwrap()
            .has(b"A"));
        assert!(resolve_dict(&document, &Object::Reference(stream_id))
            .unwrap()
            .has(b"B"));
        assert!(resolve_dict(&document, &Object::Integer(1)).is_none());
    }

    #[test]
    fn content_stream_ids_include_page_contents_and_form_xobjects_only() {
        let document = PdfFixture::new()
            .page_with_content_streams(&[b"q Q", b"BT ET"])
            .with_form_xobject(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 2,
                height: 2,
            })
            .with_icc_color_space_stream()
            .build();
        let ids = content_stream_ids(&document);
        let page_id = *document.get_pages().get(&1).unwrap();
        let mut expected: BTreeSet<ObjectId> =
            document.get_page_contents(page_id).into_iter().collect();
        let form_id = document
            .objects
            .iter()
            .find_map(|(id, object)| match object {
                Object::Stream(stream) if is_form_stream_dict(&stream.dict) => Some(*id),
                _ => None,
            })
            .unwrap();
        expected.insert(form_id);
        assert_eq!(ids, expected);
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn page_content_stream_ids_handles_reference_to_array() {
        let mut fixture_document = PdfFixture::new()
            .page_with_content_streams(&[b"q", b"Q"])
            .build();
        let page_id = *fixture_document.get_pages().get(&1).unwrap();
        let original = fixture_document.get_page_contents(page_id);
        let array_id = fixture_document.add_object(Object::Array(
            original.iter().map(|id| Object::Reference(*id)).collect(),
        ));
        fixture_document
            .get_dictionary_mut(page_id)
            .unwrap()
            .set("Contents", Object::Reference(array_id));
        assert_eq!(
            page_content_stream_ids(&fixture_document, page_id),
            original
        );
    }

    #[test]
    fn inherited_page_resources_reads_resources_from_pages_node() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_inherited_font_resource()
            .build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let resources = inherited_page_resources(&document, page_id).unwrap();
        assert!(resources.has(b"Font"));
    }

    #[test]
    fn inherited_page_resources_prefers_page_own_resources() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_inherited_font_resource()
            .with_image(FixtureImage::FlateRgb {
                width: 2,
                height: 2,
            })
            .build();
        let page_id = *document.get_pages().get(&1).unwrap();
        let resources = inherited_page_resources(&document, page_id).unwrap();
        assert!(resources.has(b"XObject"));
        assert!(!resources.has(b"Font"));
    }

    #[test]
    fn inherited_page_resources_is_none_when_absent() {
        let document = PdfFixture::new().page_with_content(b"q Q").build();
        let page_id = *document.get_pages().get(&1).unwrap();
        assert!(inherited_page_resources(&document, page_id).is_none());
    }

    #[test]
    fn child_dict_mut_reaches_direct_and_indirect_child_dictionaries() {
        let mut document = Document::new();
        let child_id = document.add_object(dictionary! { "K" => 1 });
        let owner = document.add_object(dictionary! {
            "Direct" => dictionary! { "D" => 2 },
            "Indirect" => child_id,
            "Scalar" => 3,
        });
        assert!(child_dict_mut(&mut document, owner, b"Direct")
            .unwrap()
            .has(b"D"));
        assert!(child_dict_mut(&mut document, owner, b"Indirect")
            .unwrap()
            .has(b"K"));
        assert!(child_dict_mut(&mut document, owner, b"Scalar").is_none());
        assert!(child_dict_mut(&mut document, owner, b"Missing").is_none());
    }

    #[test]
    fn catalog_id_reads_trailer_root() {
        let document = PdfFixture::new().page_with_content(b"q Q").build();
        let id = catalog_id(&document).unwrap();
        assert!(document.get_dictionary(id).unwrap().type_is(b"Catalog"));
    }

    #[test]
    fn dict_name_is_compares_name_values() {
        let dict = dictionary! { "Subtype" => "Image" };
        assert!(is_image_stream_dict(&dict));
        assert!(!is_form_stream_dict(&dict));
    }
}
