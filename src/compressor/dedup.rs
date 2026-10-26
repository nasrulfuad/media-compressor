//! Stream deduplication: find identical stream objects and collapse them to a
//! single canonical object, updating all references throughout the document.
//!
//! Only `Object::Stream` objects are considered — dictionaries, strings, and
//! integers are left untouched because they are generally small and reference
//! semantics do not apply in the same way.

use std::collections::HashMap;

use lopdf::{Document, Object, ObjectId};
use tracing::{debug, warn};

use crate::compressor::pdf_utils::MIN_COMPRESSIBLE_SIZE;
use crate::error::AppResult;

/// Deduplicate identical stream objects within `doc`.
///
/// For every pair of streams with byte-for-byte identical content, the
/// duplicate is removed and all `Object::Reference` values that pointed to it
/// are rewritten to point to the canonical copy instead.
///
/// Returns an estimate of the bytes saved (sum of the content lengths of all
/// removed duplicates).
pub fn deduplicate_streams(doc: &mut Document) -> AppResult<u64> {
    // ── Phase 1: build content → canonical-id map ─────────────────────────
    // We store content as the key and the first-seen ObjectId as the value.
    // Because HashMap keys must implement Hash + Eq and Vec<u8> satisfies
    // both, we use the raw bytes directly as the key.
    let mut canonical: HashMap<Vec<u8>, ObjectId> = HashMap::new();

    // Maps duplicate-id → canonical-id.  Collected separately so we can
    // iterate objects immutably while building the map.
    let mut replacements: HashMap<ObjectId, ObjectId> = HashMap::new();
    let mut bytes_saved: u64 = 0;

    // Collect all stream object IDs and their content first (immutable pass).
    let stream_entries: Vec<(ObjectId, Vec<u8>)> = doc
        .objects
        .iter()
        .filter_map(|(&id, obj)| {
            if let Object::Stream(s) = obj {
                // Only deduplicate streams with non-trivial content.
                if s.content.len() >= MIN_COMPRESSIBLE_SIZE {
                    return Some((id, s.content.clone()));
                }
            }
            None
        })
        .collect();

    for (id, content) in stream_entries {
        match canonical.get(&content) {
            Some(&canonical_id) => {
                // Duplicate found — schedule replacement.
                replacements.insert(id, canonical_id);
                bytes_saved += content.len() as u64;
                debug!(
                    duplicate_id = ?id,
                    canonical_id = ?canonical_id,
                    stream_bytes = content.len(),
                    "Duplicate stream found"
                );
            }
            None => {
                canonical.insert(content, id);
            }
        }
    }

    if replacements.is_empty() {
        debug!("No duplicate streams found");
        return Ok(0);
    }

    debug!(
        count = replacements.len(),
        bytes_saved, "Rewriting references to duplicate streams"
    );

    // ── Phase 2: rewrite all references throughout the document ───────────
    // `traverse_objects` visits every Object in the document (including
    // nested arrays and dictionaries) and lets us mutate them in-place.
    let replacements_clone = replacements.clone();
    doc.traverse_objects(|obj| {
        if let Object::Reference(ref mut ref_id) = *obj {
            if let Some(&canonical_id) = replacements_clone.get(ref_id) {
                *ref_id = canonical_id;
            }
        }
    });

    // ── Phase 3: remove the duplicate objects ─────────────────────────────
    // At this point no reference in the document points to the duplicate IDs
    // any more, so we can safely remove them from the object table.
    for dup_id in replacements.keys() {
        if doc.objects.remove(dup_id).is_none() {
            warn!(object_id = ?dup_id, "Duplicate object was already absent when removing");
        }
    }

    Ok(bytes_saved)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Object, Stream};

    fn make_stream(content: &[u8]) -> Object {
        let mut dict = Dictionary::new();
        dict.set("Length", Object::Integer(content.len() as i64));
        Object::Stream(Stream {
            dict,
            content: content.to_vec(),
            allows_compression: true,
            start_position: None,
        })
    }

    /// Create a document with two identical streams; after deduplication only
    /// one should remain.  References to the removed duplicate inside
    /// document-structure-reachable objects are rewritten to the canonical id
    /// by `traverse_objects` (which starts from the trailer).  This test
    /// verifies the removal and savings accounting — the reference-rewrite
    /// path is exercised in integration with a real PDF.
    #[test]
    fn deduplicate_streams_removes_identical_copies() {
        let mut doc = Document::new();

        let payload: Vec<u8> = std::iter::repeat(b'X').take(256).collect();
        let id_a = doc.add_object(make_stream(&payload));
        let id_b = doc.add_object(make_stream(&payload));

        let saved = deduplicate_streams(&mut doc).unwrap();

        // Exactly one of the pair should have been removed.
        let a_present = doc.objects.contains_key(&id_a);
        let b_present = doc.objects.contains_key(&id_b);
        assert!(
            a_present ^ b_present,
            "Exactly one of the pair must survive; a={a_present}, b={b_present}"
        );

        assert!(
            saved >= 256,
            "Should report at least the stream byte length as savings"
        );
    }

    #[test]
    fn deduplicate_streams_ignores_unique_streams() {
        let mut doc = Document::new();
        doc.add_object(make_stream(b"aaaaa_unique_content_one_xxxxxxxxxxx_yyy"));
        doc.add_object(make_stream(b"bbbbb_unique_content_two_xxxxxxxxxxx_zzz"));

        let saved = deduplicate_streams(&mut doc).unwrap();
        assert_eq!(saved, 0, "No savings for unique streams");
    }

    #[test]
    fn deduplicate_streams_skips_tiny_streams() {
        let mut doc = Document::new();
        // Two identical but very short streams (below the 64-byte threshold).
        let small: Vec<u8> = b"small".to_vec();
        doc.add_object(make_stream(&small));
        doc.add_object(make_stream(&small));

        let saved = deduplicate_streams(&mut doc).unwrap();
        assert_eq!(saved, 0, "Tiny streams below threshold must not be deduped");
    }
}
