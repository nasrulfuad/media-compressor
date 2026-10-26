//! Removal of unnecessary PDF objects that consume space without adding
//! value for most end-users (thumbnails, embedded JavaScript, non-link
//! annotations, output intents, piece info, structure trees, AcroForms,
//! and embedded files).
//!
//! All functions operate directly on the lopdf [`Document`] and return an
//! estimate of the bytes freed.

use lopdf::{Document, Object, ObjectId};
use tracing::debug;

use crate::compressor::level::CompressionLevel;
use crate::compressor::pdf_utils::estimate_object_size;
use crate::error::AppResult;

/// Remove unnecessary objects from `doc` according to the requested
/// compression `level`.
///
/// Returns an estimate of the bytes freed.
pub fn remove_unnecessary_objects(doc: &mut Document, level: CompressionLevel) -> AppResult<u64> {
    let mut total_freed: u64 = 0;

    total_freed += remove_thumbnails(doc);
    total_freed += remove_javascript(doc);
    total_freed += remove_non_link_annotations(doc);
    total_freed += remove_output_intents(doc);
    total_freed += remove_piece_info(doc);

    if level.remove_struct_tree() {
        total_freed += remove_struct_tree(doc);
    }

    if level.remove_embedded_files() {
        total_freed += remove_embedded_files(doc);
    }

    if level.remove_acroform() {
        total_freed += remove_acroform(doc);
    }

    // Remove named destinations (they are not needed for rendering and
    // can be large when the PDF was produced with bookmark software).
    total_freed += remove_named_destinations(doc);

    debug!(
        bytes_freed = total_freed,
        "Unnecessary object removal complete"
    );
    Ok(total_freed)
}

// ── Individual removal passes ─────────────────────────────────────────────────

/// Remove embedded page thumbnail images (`/Thumb` entries on page objects).
fn remove_thumbnails(doc: &mut Document) -> u64 {
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let mut freed: u64 = 0;

    for page_id in page_ids {
        // Retrieve the thumb reference before mutating.
        let thumb_ref = doc
            .get_dictionary(page_id)
            .ok()
            .and_then(|d| d.get(b"Thumb").ok())
            .and_then(|o| o.as_reference().ok());

        if let Some(thumb_id) = thumb_ref {
            // Remove the /Thumb entry from the page dictionary.
            if let Ok(page_dict) = doc.get_dictionary_mut(page_id) {
                page_dict.remove(b"Thumb");
            }
            // Remove the thumbnail object itself.
            if let Some(obj) = doc.objects.remove(&thumb_id) {
                freed += estimate_object_size(&obj);
                debug!(page_id = ?page_id, thumb_id = ?thumb_id, "Removed thumbnail");
            }
        }
    }

    freed
}

/// Remove embedded JavaScript actions from the document's `/Names` tree and
/// any `/JS` or `/JavaScript` objects found at the top level.
fn remove_javascript(doc: &mut Document) -> u64 {
    let mut freed: u64 = 0;

    // Collect IDs of objects that look like JavaScript objects.
    let js_ids: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(&id, obj)| {
            if is_javascript_object(obj) {
                Some(id)
            } else {
                None
            }
        })
        .collect();

    for id in js_ids {
        // Use delete_object which also removes references from arrays/dicts.
        if let Some(obj) = doc.delete_object(id) {
            freed += estimate_object_size(&obj);
            debug!(object_id = ?id, "Removed JavaScript object");
        }
    }

    // Also clean up the /JavaScript subtree in the catalog /Names dictionary.
    freed += remove_catalog_names_entry(doc, b"JavaScript");

    freed
}

/// Remove all annotations from pages whose `/Subtype` is not `/Link`.
fn remove_non_link_annotations(doc: &mut Document) -> u64 {
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    let mut freed: u64 = 0;

    for page_id in page_ids {
        // Collect annotation IDs that are non-link.
        let annot_refs: Vec<ObjectId> = collect_non_link_annot_ids(doc, page_id);

        if annot_refs.is_empty() {
            continue;
        }

        // Remove the annotation objects and their references from /Annots array.
        for annot_id in &annot_refs {
            if let Some(obj) = doc.objects.remove(annot_id) {
                freed += estimate_object_size(&obj);
            }
        }

        // Prune the /Annots array in-place, keeping only link annotations that
        // still exist in the object table.
        if let Ok(page_dict) = doc.get_dictionary_mut(page_id) {
            if let Ok(Object::Array(ref mut annots)) = page_dict.get_mut(b"Annots") {
                annots.retain(|item| match item {
                    Object::Reference(ref_id) => !annot_refs.contains(ref_id),
                    _ => true,
                });
            }
        }

        debug!(
            page_id = ?page_id,
            removed = annot_refs.len(),
            "Removed non-link annotations"
        );
    }

    freed
}

/// Remove the `/OutputIntents` array from the document catalog.
fn remove_output_intents(doc: &mut Document) -> u64 {
    remove_catalog_array_and_objects(doc, b"OutputIntents")
}

/// Remove `/PieceInfo` from the catalog and all page dictionaries.
fn remove_piece_info(doc: &mut Document) -> u64 {
    let mut freed: u64 = 0;

    // Remove from catalog.
    freed += remove_catalog_key_and_object(doc, b"PieceInfo");

    // Remove from all page dictionaries.
    let page_ids: Vec<ObjectId> = doc.page_iter().collect();
    for page_id in page_ids {
        freed += remove_page_key_and_object(doc, page_id, b"PieceInfo");
    }

    freed
}

/// Remove the document structure tree (`/StructTreeRoot` and `/MarkInfo`).
fn remove_struct_tree(doc: &mut Document) -> u64 {
    let mut freed: u64 = 0;

    freed += remove_catalog_key_and_object(doc, b"StructTreeRoot");
    freed += remove_catalog_key_and_object(doc, b"MarkInfo");

    if freed > 0 {
        debug!(bytes_freed = freed, "Removed structure tree");
    }

    freed
}

/// Remove the `/AcroForm` entry from the catalog.
fn remove_acroform(doc: &mut Document) -> u64 {
    let freed = remove_catalog_key_and_object(doc, b"AcroForm");
    if freed > 0 {
        debug!(bytes_freed = freed, "Removed AcroForm");
    }
    freed
}

/// Remove the `/EmbeddedFiles` subtree from the catalog's `/Names` tree.
fn remove_embedded_files(doc: &mut Document) -> u64 {
    let freed = remove_catalog_names_entry(doc, b"EmbeddedFiles");
    if freed > 0 {
        debug!(bytes_freed = freed, "Removed embedded files");
    }
    freed
}

/// Remove named destinations from the document catalog (Phase 2.5.6).
///
/// Named destinations can appear in two places in a PDF:
///
/// 1. **`/Names` → `/Dests`** — a name tree inside the catalog's `/Names`
///    dictionary (PDF 1.2+).
/// 2. **`/Dests`** — a flat dictionary directly on the catalog (PDF 1.1).
///
/// Both are removed when found. The `/Names` dictionary itself is also removed
/// from the catalog if it becomes empty after the `/Dests` entry is stripped.
///
/// Named destinations are not needed for rendering; they are only used for
/// cross-document or cross-application navigation links. Removing them is safe
/// for documents whose internal hyperlinks use explicit destination arrays
/// rather than named references.
///
/// Returns an estimate of the bytes freed.
fn remove_named_destinations(doc: &mut Document) -> u64 {
    let mut freed: u64 = 0;

    // ── 1. /Dests inside /Names ────────────────────────────────────────────

    // Locate the /Names reference from the catalog.
    let names_ref_opt = doc
        .catalog()
        .ok()
        .and_then(|d| d.get(b"Names").ok())
        .and_then(|o| o.as_reference().ok());

    if let Some(names_ref) = names_ref_opt {
        // Does the /Names dict contain a /Dests entry?
        let dests_ref_opt = doc
            .objects
            .get(&names_ref)
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Dests").ok())
            .and_then(|o| o.as_reference().ok());

        // Remove the /Dests subtree object if it was an indirect reference.
        if let Some(dests_id) = dests_ref_opt {
            if let Some(obj) = doc.objects.remove(&dests_id) {
                freed += estimate_object_size(&obj);
            }
        }

        // Remove the /Dests key from the /Names dictionary.
        let names_is_now_empty = if let Some(obj) = doc.objects.get_mut(&names_ref) {
            if let Ok(d) = obj.as_dict_mut() {
                d.remove(b"Dests");
                debug!("Removed /Dests from /Names dictionary");
                d.is_empty()
            } else {
                false
            }
        } else {
            false
        };

        // If /Names is now empty, remove it from the catalog too.
        if names_is_now_empty {
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.remove(b"Names");
            }
            if let Some(obj) = doc.objects.remove(&names_ref) {
                freed += estimate_object_size(&obj);
                debug!("Removed now-empty /Names dictionary from catalog");
            }
        }
    }

    // ── 2. /Dests directly on the catalog (PDF 1.1 format) ────────────────
    // clone the ref before we call catalog_mut to avoid double borrow
    let direct_dests_ref = doc
        .catalog()
        .ok()
        .and_then(|d| d.get(b"Dests").ok())
        .and_then(|o| o.as_reference().ok());

    if let Some(dests_id) = direct_dests_ref {
        if let Ok(catalog) = doc.catalog_mut() {
            catalog.remove(b"Dests");
        }
        if let Some(obj) = doc.objects.remove(&dests_id) {
            freed += estimate_object_size(&obj);
            debug!("Removed /Dests from document catalog");
        }
    } else {
        // Inline /Dests (not a reference — just remove the key).
        let has_inline = doc
            .catalog()
            .ok()
            .map(|d| d.get(b"Dests").is_ok())
            .unwrap_or(false);
        if has_inline {
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.remove(b"Dests");
                freed += 32; // nominal estimate for an inline entry
                debug!("Removed inline /Dests from document catalog");
            }
        }
    }

    if freed > 0 {
        debug!(bytes_freed = freed, "Removed named destinations");
    }

    freed
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Determine whether an object is a JavaScript action or JavaScript object.
fn is_javascript_object(obj: &Object) -> bool {
    match obj {
        Object::Dictionary(d) => {
            // /Type /JavaScript  or  /S /JavaScript  or  /JS key present
            let type_is_js = matches!(
                d.get(b"Type"),
                Ok(Object::Name(n)) if n == b"JavaScript"
            );
            let subtype_is_js = matches!(
                d.get(b"S"),
                Ok(Object::Name(n)) if n == b"JavaScript"
            );
            let has_js_key = d.get(b"JS").is_ok();
            type_is_js || subtype_is_js || has_js_key
        }
        Object::Stream(s) => {
            let type_is_js = matches!(
                s.dict.get(b"Type"),
                Ok(Object::Name(n)) if n == b"JavaScript"
            );
            let subtype_is_js = matches!(
                s.dict.get(b"S"),
                Ok(Object::Name(n)) if n == b"JavaScript"
            );
            let has_js_key = s.dict.get(b"JS").is_ok();
            type_is_js || subtype_is_js || has_js_key
        }
        _ => false,
    }
}

/// Return the IDs of all non-link annotation objects referenced from a page.
fn collect_non_link_annot_ids(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let annots_array = match doc.get_dictionary(page_id) {
        Ok(d) => match d.get(b"Annots") {
            Ok(Object::Array(arr)) => arr.clone(),
            Ok(Object::Reference(ref_id)) => {
                // /Annots may itself be an indirect reference to an array.
                match doc.objects.get(ref_id) {
                    Some(Object::Array(arr)) => arr.clone(),
                    _ => return vec![],
                }
            }
            _ => return vec![],
        },
        Err(_) => return vec![],
    };

    annots_array
        .iter()
        .filter_map(|item| {
            let ref_id = match item {
                Object::Reference(id) => *id,
                _ => return None,
            };

            // Dereference the annotation object.
            let annot_dict = match doc.objects.get(&ref_id) {
                Some(Object::Dictionary(d)) => d,
                Some(Object::Stream(s)) => &s.dict,
                _ => return None,
            };

            let subtype_is_link = matches!(
                annot_dict.get(b"Subtype"),
                Ok(Object::Name(n)) if n == b"Link"
            );

            if subtype_is_link {
                None
            } else {
                Some(ref_id)
            }
        })
        .collect()
}

/// Remove a key from the catalog dictionary and delete the referenced object.
/// Returns estimated bytes freed.
fn remove_catalog_key_and_object(doc: &mut Document, key: &[u8]) -> u64 {
    let ref_id = match doc.catalog().ok().and_then(|d| d.get(key).ok()) {
        Some(Object::Reference(id)) => *id,
        _ => {
            // Remove inline entry (non-reference) if it exists.
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.remove(key);
            }
            return 0;
        }
    };

    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(key);
    }

    match doc.objects.remove(&ref_id) {
        Some(obj) => {
            let size = estimate_object_size(&obj);
            debug!(key = ?String::from_utf8_lossy(key), object_id = ?ref_id, bytes = size, "Removed catalog key");
            size
        }
        None => 0,
    }
}

/// Remove a key from the given page dictionary and delete the referenced object.
fn remove_page_key_and_object(doc: &mut Document, page_id: ObjectId, key: &[u8]) -> u64 {
    let ref_id = match doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|d| d.get(key).ok())
    {
        Some(Object::Reference(id)) => *id,
        _ => {
            if let Ok(d) = doc.get_dictionary_mut(page_id) {
                d.remove(key);
            }
            return 0;
        }
    };

    if let Ok(d) = doc.get_dictionary_mut(page_id) {
        d.remove(key);
    }

    match doc.objects.remove(&ref_id) {
        Some(obj) => estimate_object_size(&obj),
        None => 0,
    }
}

/// Remove an array-type entry from the catalog and delete all objects it
/// directly references.
fn remove_catalog_array_and_objects(doc: &mut Document, key: &[u8]) -> u64 {
    let mut freed: u64 = 0;

    // Clone the array value so we can drop the borrow before mutating.
    let array = match doc.catalog().ok().and_then(|d| d.get(key).ok()) {
        Some(Object::Array(arr)) => arr.clone(),
        Some(Object::Reference(ref_id)) => {
            let id = *ref_id;
            // The catalog entry is itself an indirect object holding the array.
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.remove(key);
            }
            if let Some(obj) = doc.objects.remove(&id) {
                freed += estimate_object_size(&obj);
            }
            return freed;
        }
        _ => {
            if let Ok(catalog) = doc.catalog_mut() {
                catalog.remove(key);
            }
            return 0;
        }
    };

    // Remove the key from the catalog first.
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(key);
    }

    // Delete each referenced object.
    for item in &array {
        if let Object::Reference(ref_id) = item {
            if let Some(obj) = doc.objects.remove(ref_id) {
                freed += estimate_object_size(&obj);
                debug!(
                    key = ?String::from_utf8_lossy(key),
                    object_id = ?ref_id,
                    "Removed array-referenced object"
                );
            }
        }
    }

    freed
}

/// Remove a named entry from the catalog's `/Names` dictionary tree.
/// This only removes the reference from the Names node — the underlying
/// named objects are left to `prune_objects` to collect.
fn remove_catalog_names_entry(doc: &mut Document, entry_key: &[u8]) -> u64 {
    // Locate the /Names dictionary referenced from the catalog.
    let names_ref = match doc.catalog().ok().and_then(|d| d.get(b"Names").ok()) {
        Some(Object::Reference(id)) => *id,
        _ => return 0,
    };

    let has_entry = match doc.objects.get(&names_ref) {
        Some(Object::Dictionary(d)) => d.get(entry_key).is_ok(),
        _ => false,
    };

    if !has_entry {
        return 0;
    }

    // Get the ref of the sub-tree node before mutating.
    let sub_ref = match doc.objects.get(&names_ref) {
        Some(Object::Dictionary(d)) => match d.get(entry_key) {
            Ok(Object::Reference(id)) => Some(*id),
            _ => None,
        },
        _ => None,
    };

    // Remove the entry from the Names dictionary.
    if let Some(Object::Dictionary(d)) = doc.objects.get_mut(&names_ref) {
        d.remove(entry_key);
        debug!(
            entry = ?String::from_utf8_lossy(entry_key),
            "Removed Names tree entry"
        );
    }

    // Remove the sub-tree object if it was an indirect reference.
    let mut freed: u64 = 0;
    if let Some(sub_id) = sub_ref {
        if let Some(obj) = doc.objects.remove(&sub_id) {
            freed += estimate_object_size(&obj);
        }
    }

    freed
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Object, Stream};

    fn make_stream_obj(content: &[u8]) -> Object {
        let mut dict = Dictionary::new();
        dict.set("Length", Object::Integer(content.len() as i64));
        Object::Stream(Stream {
            dict,
            content: content.to_vec(),
            allows_compression: true,
            start_position: None,
        })
    }

    #[test]
    fn is_javascript_object_detects_js_dict() {
        let mut d = Dictionary::new();
        d.set("S", Object::Name(b"JavaScript".to_vec()));
        assert!(is_javascript_object(&Object::Dictionary(d)));
    }

    #[test]
    fn is_javascript_object_rejects_regular_dict() {
        let mut d = Dictionary::new();
        d.set("Type", Object::Name(b"Page".to_vec()));
        assert!(!is_javascript_object(&Object::Dictionary(d)));
    }

    #[test]
    fn estimate_size_stream_returns_content_length() {
        let obj = make_stream_obj(b"hello world");
        assert_eq!(estimate_object_size(&obj), 11);
    }

    #[test]
    fn remove_unnecessary_objects_runs_without_panic() {
        // Smoke test: a minimal document should not panic at any level.
        for level in [
            CompressionLevel::Low,
            CompressionLevel::Medium,
            CompressionLevel::High,
            CompressionLevel::Extreme,
        ] {
            let mut doc = Document::new();
            let result = remove_unnecessary_objects(&mut doc, level);
            assert!(result.is_ok(), "Should not error on empty document");
        }
    }

    #[test]
    fn remove_named_destinations_empty_doc_is_noop() {
        let mut doc = Document::new();
        let freed = remove_named_destinations(&mut doc);
        assert_eq!(freed, 0, "Empty document should free zero bytes");
    }

    #[test]
    fn remove_named_destinations_removes_dests_from_catalog() {
        use lopdf::{Document, Object};
        let mut doc = Document::new();

        // Build a minimal catalog with a /Dests dictionary.
        let dests_content = b"named destination data that is long enough".as_ref();
        let dests_id = doc.add_object(Object::Dictionary({
            let mut d = Dictionary::new();
            d.set("SomeDest", Object::Array(vec![]));
            d
        }));

        // Build catalog referencing /Dests.
        let catalog_dict = {
            let mut d = Dictionary::new();
            d.set("Type", Object::Name(b"Catalog".to_vec()));
            d.set("Dests", Object::Reference(dests_id));
            d
        };
        let catalog_id = doc.add_object(Object::Dictionary(catalog_dict));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        let freed = remove_named_destinations(&mut doc);

        // /Dests object should be gone.
        assert!(
            !doc.objects.contains_key(&dests_id),
            "/Dests object must be removed"
        );
        assert!(freed > 0, "Should report freed bytes");
        let _ = dests_content; // suppress unused warning
    }
}
