use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use tracing::debug;

use crate::error::AppResult;

use super::MIN_COMPRESSIBLE_SIZE;

fn content_hash(content: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn dictionary_without_length(dict: &Dictionary) -> Dictionary {
    let mut cleaned = dict.clone();
    cleaned.remove(b"Length");
    cleaned
}

fn stream_of(document: &Document, id: ObjectId) -> Option<&Stream> {
    match document.objects.get(&id) {
        Some(Object::Stream(stream)) => Some(stream),
        _ => None,
    }
}

fn are_interchangeable(left: &Stream, right: &Stream) -> bool {
    left.content == right.content
        && dictionary_without_length(&left.dict) == dictionary_without_length(&right.dict)
}

fn candidate_hashes(document: &Document) -> BTreeMap<ObjectId, u64> {
    document
        .objects
        .iter()
        .filter_map(|(id, object)| match object {
            Object::Stream(stream) if stream.content.len() >= MIN_COMPRESSIBLE_SIZE => {
                Some((*id, content_hash(&stream.content)))
            }
            _ => None,
        })
        .collect()
}

fn find_replacements(document: &Document) -> HashMap<ObjectId, ObjectId> {
    let mut canonical_by_hash: HashMap<u64, Vec<ObjectId>> = HashMap::new();
    let mut replacements = HashMap::new();
    for (id, hash) in candidate_hashes(document) {
        let Some(stream) = stream_of(document, id) else {
            continue;
        };
        let canonicals = canonical_by_hash.entry(hash).or_default();
        let duplicate_of = canonicals.iter().copied().find(|canonical_id| {
            stream_of(document, *canonical_id)
                .is_some_and(|canonical| are_interchangeable(canonical, stream))
        });
        match duplicate_of {
            Some(canonical_id) => {
                replacements.insert(id, canonical_id);
            }
            None => canonicals.push(id),
        }
    }
    replacements
}

pub fn deduplicate_streams(document: &mut Document) -> AppResult<i64> {
    let replacements = find_replacements(document);
    if replacements.is_empty() {
        debug!("No duplicate streams found");
        return Ok(0);
    }
    let bytes_saved: usize = replacements
        .keys()
        .filter_map(|id| stream_of(document, *id))
        .map(|stream| stream.content.len())
        .sum();
    debug!(
        count = replacements.len(),
        bytes_saved, "Rewriting references to duplicate streams"
    );
    document.traverse_objects(|object| {
        if let Object::Reference(reference) = object {
            if let Some(canonical_id) = replacements.get(reference) {
                *reference = *canonical_id;
            }
        }
    });
    Ok(bytes_saved as i64)
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::test_support::{assert_no_dangling_references, FixtureImage, PdfFixture};

    fn add_stream(document: &mut Document, dict: Dictionary, content: &[u8]) -> ObjectId {
        document.add_object(Stream::new(dict, content.to_vec()))
    }

    fn document_referencing(ids: &[ObjectId]) -> Document {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        let references: Vec<Object> = ids.iter().map(|id| Object::Reference(*id)).collect();
        document
            .get_dictionary_mut(catalog_id)
            .unwrap()
            .set("Shared", Object::Array(references));
        document
    }

    fn shared_array(document: &Document) -> Vec<ObjectId> {
        let catalog = document.catalog().unwrap();
        catalog
            .get(b"Shared")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_reference().unwrap())
            .collect()
    }

    #[test]
    fn dedup_merges_streams_with_identical_dict_and_content() {
        let payload = vec![5u8; 200];
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let first = add_stream(&mut document, dictionary! { "Kind" => 1 }, &payload);
        let second = add_stream(&mut document, dictionary! { "Kind" => 1 }, &payload);
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        document.get_dictionary_mut(catalog_id).unwrap().set(
            "Shared",
            Object::Array(vec![Object::Reference(first), Object::Reference(second)]),
        );
        let saved = deduplicate_streams(&mut document).unwrap();
        assert_eq!(saved, 200);
        let ids = shared_array(&document);
        assert_eq!(ids[0], ids[1]);
        assert_eq!(ids[0], first);
    }

    #[test]
    fn dedup_ignores_length_entry_when_comparing_dictionaries() {
        let payload = vec![6u8; 100];
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let first = add_stream(
            &mut document,
            dictionary! { "Kind" => 1, "Length" => 100 },
            &payload,
        );
        let second = add_stream(
            &mut document,
            dictionary! { "Kind" => 1, "Length" => 999 },
            &payload,
        );
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        document.get_dictionary_mut(catalog_id).unwrap().set(
            "Shared",
            Object::Array(vec![Object::Reference(first), Object::Reference(second)]),
        );
        assert_eq!(deduplicate_streams(&mut document).unwrap(), 100);
    }

    #[test]
    fn dedup_keeps_streams_with_same_content_but_different_dict() {
        let payload = vec![5u8; 200];
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let first = add_stream(&mut document, dictionary! { "Width" => 10 }, &payload);
        let second = add_stream(&mut document, dictionary! { "Width" => 20 }, &payload);
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        document.get_dictionary_mut(catalog_id).unwrap().set(
            "Shared",
            Object::Array(vec![Object::Reference(first), Object::Reference(second)]),
        );
        assert_eq!(deduplicate_streams(&mut document).unwrap(), 0);
        assert_eq!(shared_array(&document), vec![first, second]);
    }

    #[test]
    fn dedup_rewrites_references_and_leaves_no_dangling_references() {
        let payload = vec![7u8; 300];
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let first = add_stream(&mut document, Dictionary::new(), &payload);
        let second = add_stream(&mut document, Dictionary::new(), &payload);
        let third = add_stream(&mut document, Dictionary::new(), &payload);
        let catalog_id = document
            .trailer
            .get(b"Root")
            .unwrap()
            .as_reference()
            .unwrap();
        document.get_dictionary_mut(catalog_id).unwrap().set(
            "Shared",
            Object::Array(vec![
                Object::Reference(first),
                Object::Reference(second),
                Object::Reference(third),
            ]),
        );
        let saved = deduplicate_streams(&mut document).unwrap();
        assert_eq!(saved, 600);
        document.prune_objects();
        assert_no_dangling_references(&document);
        assert_eq!(shared_array(&document), vec![first, first, first]);
        assert!(!document.objects.contains_key(&second));
        assert!(!document.objects.contains_key(&third));
    }

    #[test]
    fn dedup_ignores_small_streams() {
        let mut document = PdfFixture::new().page_with_content(b"q Q").build();
        let first = add_stream(&mut document, Dictionary::new(), b"tiny");
        let second = add_stream(&mut document, Dictionary::new(), b"tiny");
        let mut document = {
            let catalog_id = document
                .trailer
                .get(b"Root")
                .unwrap()
                .as_reference()
                .unwrap();
            document.get_dictionary_mut(catalog_id).unwrap().set(
                "Shared",
                Object::Array(vec![Object::Reference(first), Object::Reference(second)]),
            );
            document
        };
        assert_eq!(deduplicate_streams(&mut document).unwrap(), 0);
    }

    #[test]
    fn dedup_merges_identical_image_streams_with_identical_dictionaries() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 16,
                height: 16,
            })
            .with_image(FixtureImage::FlateRgb {
                width: 16,
                height: 16,
            })
            .build();
        assert!(deduplicate_streams(&mut document).unwrap() > 0);
        let page_id = *document.get_pages().get(&1).unwrap();
        let resources = document
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap();
        let xobjects = resources.get(b"XObject").unwrap().as_dict().unwrap();
        let first = xobjects.get(b"Im1").unwrap().as_reference().unwrap();
        let second = xobjects.get(b"Im2").unwrap().as_reference().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn dedup_without_duplicates_changes_nothing() {
        let mut document = document_referencing(&[]);
        assert_eq!(deduplicate_streams(&mut document).unwrap(), 0);
    }
}
