//! Font subsetting for embedded PDF fonts.
//!
//! ## Subsetting
//!
//! For TrueType / OpenType fonts (FontFile2 / FontFile3 with CFF), the
//! `subsetter` crate is used to strip unused glyphs.  Used glyph IDs are
//! collected by **parsing all page content streams** (tracking `Tf`, `Tj`,
//! `TJ`, `'`, `"` operators) to find exactly which character codes each font
//! actually renders.  This is far more reliable than the old approach of
//! inferring glyph usage from `/W` or `/Widths` arrays, which miss glyphs
//! that use the default width (`/DW`).
//!
//! Only fonts with an embedded font file are processed; Standard 14 fonts are
//! skipped.

use std::collections::{HashMap, HashSet};

use lopdf::content::Operation;
use lopdf::{Document, Object, ObjectId};
use tracing::{debug, info};

use crate::error::AppResult;

// Keys used to locate the embedded font file stream inside a FontDescriptor.
const FONT_FILE_KEYS: &[&[u8]] = &[b"FontFile", b"FontFile2", b"FontFile3"];

// ── Font Subsetting ──────────────────────────────────────────────────────────

/// Subset all embedded TrueType/OpenType fonts so only used glyphs remain.
///
/// For each font with an embedded font file (`/FontFile2` for TrueType,
/// `/FontFile3` for CFF/OpenType), the module:
///
/// 1. Parses all page content streams to find exactly which character codes
///    each font renders (via `Tf`, `Tj`, `TJ`, `'`, `"` operators).
/// 2. Maps character codes to glyph IDs (CID = glyph ID for Identity mapping;
///    char code ≈ glyph ID for simple fonts).
/// 3. Runs the `subsetter` crate to produce a minimal font containing only
///    those glyphs (plus `.notdef`).
/// 4. Replaces the embedded font file stream with the smaller version.
///
/// Returns the total bytes saved.  Fonts that are already subsetted (name
/// contains '+' prefix like `ABCDEF+Arial`) are skipped.
pub fn subset_fonts(doc: &mut Document) -> AppResult<u64> {
    // ── Phase 0: Parse all content streams to find used chars per font ────
    let used_chars = collect_used_chars_from_content_streams(doc);
    debug!(
        fonts_with_text = used_chars.len(),
        "Content stream character collection complete"
    );

    let all_ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut total_saved: u64 = 0;

    for id in &all_ids {
        // Look for /Font objects.
        let (base_font, descriptor_id, font_subtype) = {
            let obj = match doc.objects.get(id) {
                Some(o) => o,
                None => continue,
            };
            let dict = match obj {
                Object::Dictionary(d) => d,
                _ => continue,
            };
            let is_font = matches!(
                dict.get(b"Type"),
                Ok(Object::Name(n)) if n == b"Font"
            );
            if !is_font {
                continue;
            }
            let base_font: Vec<u8> = match dict.get(b"BaseFont") {
                Ok(Object::Name(n)) => n.clone(),
                _ => continue,
            };
            let descriptor_id: ObjectId = match dict.get(b"FontDescriptor") {
                Ok(Object::Reference(r)) => *r,
                _ => continue,
            };
            let subtype: Vec<u8> = match dict.get(b"Subtype") {
                Ok(Object::Name(n)) => n.clone(),
                _ => continue,
            };
            (base_font, descriptor_id, subtype)
        };

        // Skip already-subsetted fonts (e.g. "ABCDEF+Arial").
        let base_font_str = String::from_utf8_lossy(&base_font);
        if is_already_subsetted(&base_font_str) {
            debug!(font = %base_font_str, "Font already subsetted, skipping");
            continue;
        }

        // Find the embedded font file stream.
        let (font_file_key, font_file_id) = match find_font_file(doc, descriptor_id) {
            Some(f) => f,
            None => continue,
        };

        // Only handle FontFile2 (TrueType) and FontFile3 (CFF/OpenType).
        if font_file_key == b"FontFile" {
            debug!(font = %base_font_str, "Type1 FontFile — subsetter cannot handle, skipping");
            continue;
        }

        // For CIDFont (Type0), verify CIDToGIDMap is Identity — otherwise we
        // cannot reliably map character codes to glyph IDs.
        if font_subtype == b"Type0" && !has_identity_cid_to_gid_map(doc, *id) {
            debug!(
                font = %base_font_str,
                "CIDToGIDMap is not Identity, skipping subsetting"
            );
            continue;
        }

        // Use content-stream-derived character codes.
        let glyph_ids = match used_chars.get(id) {
            Some(chars) if !chars.is_empty() => {
                let mut glyphs = chars.clone();
                glyphs.insert(0); // always include .notdef
                glyphs
            }
            _ => {
                debug!(
                    font = %base_font_str,
                    "Font not referenced in content streams, skipping subsetting"
                );
                continue;
            }
        };

        // Get the font file data.
        let original_data = match doc.objects.get(&font_file_id) {
            Some(Object::Stream(s)) => s.content.clone(),
            _ => continue,
        };
        let original_size = original_data.len();

        // Run subsetter.
        let glyph_vec: Vec<u16> = glyph_ids.into_iter().collect();
        let mapper = subsetter::GlyphRemapper::new_from_glyphs(&glyph_vec);

        match subsetter::subset(&original_data, 0, &mapper) {
            Ok(subsetted) => {
                if subsetted.len() < original_size {
                    let saved = original_size - subsetted.len();
                    debug!(
                        font = %base_font_str,
                        original_bytes = original_size,
                        subsetted_bytes = subsetted.len(),
                        glyph_count = glyph_vec.len(),
                        saved,
                        "Font subsetted successfully"
                    );

                    // Replace the font file stream content.
                    if let Some(Object::Stream(s)) = doc.objects.get_mut(&font_file_id) {
                        s.content = subsetted;
                        s.dict
                            .set("Length", Object::Integer(s.content.len() as i64));
                        // Remove Length1/Length2/Length3 if present — they refer to
                        // the old uncompressed/compressed lengths.
                        s.dict.remove(b"Length1");
                        s.dict.remove(b"Length2");
                        s.dict.remove(b"Length3");
                    }
                    total_saved += saved as u64;
                } else {
                    debug!(
                        font = %base_font_str,
                        "Subsetted font not smaller, keeping original"
                    );
                }
            }
            Err(e) => {
                debug!(
                    font = %base_font_str,
                    error = %e,
                    "Subsetter failed, keeping original font"
                );
            }
        }
    }

    if total_saved > 0 {
        info!(bytes_saved = total_saved, "Font subsetting complete");
    }

    Ok(total_saved)
}

/// Check if a font name indicates it was already subsetted.
///
/// Subsetted fonts have a prefix like `ABCDEF+` (6 uppercase letters + plus sign).
fn is_already_subsetted(name: &str) -> bool {
    if name.len() < 8 {
        return false;
    }
    let bytes = name.as_bytes();
    // Check first 6 chars are uppercase ASCII, 7th is '+'.
    bytes[..6].iter().all(|b| b.is_ascii_uppercase()) && bytes[6] == b'+'
}

/// Find the font file stream ID from a FontDescriptor.
fn find_font_file(doc: &Document, descriptor_id: ObjectId) -> Option<(&'static [u8], ObjectId)> {
    let descriptor_dict = match doc.objects.get(&descriptor_id) {
        Some(Object::Dictionary(d)) => d,
        _ => return None,
    };
    for &key in FONT_FILE_KEYS {
        if let Ok(Object::Reference(fid)) = descriptor_dict.get(key) {
            return Some((key, *fid));
        }
    }
    None
}

/// Check if a Type0 font's descendant CIDFont uses Identity CIDToGIDMap.
fn has_identity_cid_to_gid_map(doc: &Document, font_id: ObjectId) -> bool {
    let font_dict = match doc.objects.get(&font_id) {
        Some(Object::Dictionary(d)) => d,
        _ => return false,
    };

    let descendants = match font_dict.get(b"DescendantFonts") {
        Ok(Object::Array(arr)) => arr,
        Ok(Object::Reference(r)) => match doc.objects.get(r) {
            Some(Object::Array(arr)) => arr,
            _ => return false,
        },
        _ => return false,
    };

    for desc_ref in descendants {
        let cid_font_id = match desc_ref {
            Object::Reference(r) => *r,
            _ => continue,
        };
        let cid_dict = match doc.objects.get(&cid_font_id) {
            Some(Object::Dictionary(d)) => d,
            _ => continue,
        };
        match cid_dict.get(b"CIDToGIDMap") {
            Ok(Object::Name(n)) if n == b"Identity" => return true,
            // A stream CIDToGIDMap is a full mapping table — we can't easily
            // invert it, so treat as non-Identity and skip subsetting.
            _ => return false,
        }
    }

    false
}

// ── Content-stream-based character code collection ──────────────────────────

/// Build a map of font ObjectId → set of character codes actually used across
/// all page content streams.
///
/// Parses every page's content stream(s), tracking the current font set by
/// `Tf` operators, and collecting character codes from `Tj`, `TJ`, `'`, and
/// `"` text-showing operators.  Also recurses into Form XObjects referenced
/// via `Do` operators.
fn collect_used_chars_from_content_streams(doc: &Document) -> HashMap<ObjectId, HashSet<u16>> {
    let mut font_chars: HashMap<ObjectId, HashSet<u16>> = HashMap::new();

    let pages: Vec<ObjectId> = doc.page_iter().collect();

    for page_id in pages {
        // Build font resource name → (font_object_id, is_cid_font) for this page.
        let font_map = build_page_font_map(doc, page_id);
        if font_map.is_empty() {
            continue;
        }

        // Get XObject dictionary for Form XObject recursion.
        let xobject_map = build_page_xobject_map(doc, page_id);

        // Decode all content streams for this page (handles decompression).
        let content = match doc.get_and_decode_page_content(page_id) {
            Ok(c) => c,
            Err(e) => {
                debug!(page_id = ?page_id, error = %e, "Failed to decode page content");
                continue;
            }
        };

        collect_chars_from_operations(
            doc,
            &content.operations,
            &font_map,
            &xobject_map,
            &mut font_chars,
            0,
        );
    }

    font_chars
}

/// Build a map from font resource names (e.g. `b"F1"`) to `(font ObjectId, is_cid)`.
fn build_page_font_map(doc: &Document, page_id: ObjectId) -> HashMap<Vec<u8>, (ObjectId, bool)> {
    let mut map = HashMap::new();

    // Use lopdf's get_page_resources which handles inherited resources.
    let (resource_dict, resource_ids) = match doc.get_page_resources(page_id) {
        Ok(r) => r,
        Err(_) => return map,
    };

    let mut collect = |resources: &lopdf::Dictionary| {
        let font_obj = match resources.get(b"Font") {
            Ok(o) => o,
            _ => return,
        };
        let font_dict = match font_obj {
            Object::Dictionary(d) => d,
            Object::Reference(r) => match doc.objects.get(r) {
                Some(Object::Dictionary(d)) => d,
                _ => return,
            },
            _ => return,
        };

        for (name, value) in font_dict.iter() {
            let font_id = match value {
                Object::Reference(r) => *r,
                _ => continue,
            };
            if map.contains_key(name) {
                continue;
            }
            let is_cid = is_cid_font(doc, font_id);
            map.insert(name.clone(), (font_id, is_cid));
        }
    };

    if let Some(resources) = resource_dict {
        collect(resources);
    }
    for rid in resource_ids {
        if let Ok(resources) = doc.get_dictionary(rid) {
            collect(resources);
        }
    }

    map
}

/// Build a map from XObject resource names to their ObjectIds.
fn build_page_xobject_map(doc: &Document, page_id: ObjectId) -> HashMap<Vec<u8>, ObjectId> {
    let mut map = HashMap::new();

    let (resource_dict, resource_ids) = match doc.get_page_resources(page_id) {
        Ok(r) => r,
        Err(_) => return map,
    };

    let mut collect = |resources: &lopdf::Dictionary| {
        let xobj = match resources.get(b"XObject") {
            Ok(o) => o,
            _ => return,
        };
        let xobj_dict = match xobj {
            Object::Dictionary(d) => d,
            Object::Reference(r) => match doc.objects.get(r) {
                Some(Object::Dictionary(d)) => d,
                _ => return,
            },
            _ => return,
        };

        for (name, value) in xobj_dict.iter() {
            if let Object::Reference(r) = value {
                map.entry(name.clone()).or_insert(*r);
            }
        }
    };

    if let Some(resources) = resource_dict {
        collect(resources);
    }
    for rid in resource_ids {
        if let Ok(resources) = doc.get_dictionary(rid) {
            collect(resources);
        }
    }

    map
}

/// Determine if a font ObjectId refers to a CIDFont (Type0 / composite font).
fn is_cid_font(doc: &Document, font_id: ObjectId) -> bool {
    match doc.objects.get(&font_id) {
        Some(Object::Dictionary(d)) => {
            matches!(d.get(b"Subtype"), Ok(Object::Name(n)) if n == b"Type0")
        }
        _ => false,
    }
}

/// Walk content stream operations, tracking font state and collecting char codes.
///
/// `depth` limits Form XObject recursion to prevent cycles.
fn collect_chars_from_operations(
    doc: &Document,
    operations: &[Operation],
    font_map: &HashMap<Vec<u8>, (ObjectId, bool)>,
    xobject_map: &HashMap<Vec<u8>, ObjectId>,
    font_chars: &mut HashMap<ObjectId, HashSet<u16>>,
    depth: u32,
) {
    let mut current_font: Option<(ObjectId, bool)> = None;

    for op in operations {
        match op.operator.as_str() {
            "Tf" => {
                // /FontName size Tf
                if let Some(Object::Name(name)) = op.operands.first() {
                    if let Some(&font_info) = font_map.get(name.as_slice()) {
                        current_font = Some(font_info);
                    }
                }
            }
            "Tj" | "'" => {
                // (string) Tj  or  (string) '
                if let Some((font_id, is_cid)) = current_font {
                    if let Some(Object::String(bytes, _)) = op.operands.first() {
                        extract_char_codes(bytes, is_cid, font_chars.entry(font_id).or_default());
                    }
                }
            }
            "\"" => {
                // aw ac (string) "
                if let Some((font_id, is_cid)) = current_font {
                    if let Some(Object::String(bytes, _)) = op.operands.get(2) {
                        extract_char_codes(bytes, is_cid, font_chars.entry(font_id).or_default());
                    }
                }
            }
            "TJ" => {
                // [(string) num (string) ...] TJ
                if let Some((font_id, is_cid)) = current_font {
                    if let Some(Object::Array(arr)) = op.operands.first() {
                        let chars = font_chars.entry(font_id).or_default();
                        for item in arr {
                            if let Object::String(bytes, _) = item {
                                extract_char_codes(bytes, is_cid, chars);
                            }
                        }
                    }
                }
            }
            "Do" => {
                // Form XObject — may contain its own text operators.
                if depth < 4 {
                    if let Some(Object::Name(xobj_name)) = op.operands.first() {
                        if let Some(&xobj_id) = xobject_map.get(xobj_name.as_slice()) {
                            process_form_xobject_chars(
                                doc,
                                xobj_id,
                                font_map,
                                font_chars,
                                depth + 1,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recurse into a Form XObject's content stream to collect character codes.
fn process_form_xobject_chars(
    doc: &Document,
    xobj_id: ObjectId,
    parent_font_map: &HashMap<Vec<u8>, (ObjectId, bool)>,
    font_chars: &mut HashMap<ObjectId, HashSet<u16>>,
    depth: u32,
) {
    let stream = match doc.objects.get(&xobj_id) {
        Some(Object::Stream(s)) => s,
        _ => return,
    };

    // Verify it's a Form XObject.
    let is_form = matches!(
        stream.dict.get(b"Subtype"),
        Ok(Object::Name(n)) if n == b"Form"
    );
    if !is_form {
        return;
    }

    // Build font map from the Form's own /Resources, falling back to parent.
    let mut font_map = parent_font_map.clone();
    if let Ok(Object::Dictionary(resources)) = stream.dict.get(b"Resources") {
        if let Ok(font_obj) = resources.get(b"Font") {
            let font_dict = match font_obj {
                Object::Dictionary(d) => Some(d),
                Object::Reference(r) => match doc.objects.get(r) {
                    Some(Object::Dictionary(d)) => Some(d),
                    _ => None,
                },
                _ => None,
            };
            if let Some(fd) = font_dict {
                for (name, value) in fd.iter() {
                    if let Object::Reference(r) = value {
                        let is_cid = is_cid_font(doc, *r);
                        font_map.insert(name.clone(), (*r, is_cid));
                    }
                }
            }
        }
    }

    // Build XObject map from Form's own resources.
    let mut xobject_map: HashMap<Vec<u8>, ObjectId> = HashMap::new();
    if let Ok(Object::Dictionary(resources)) = stream.dict.get(b"Resources") {
        if let Ok(xobj) = resources.get(b"XObject") {
            let xobj_dict = match xobj {
                Object::Dictionary(d) => Some(d),
                Object::Reference(r) => match doc.objects.get(r) {
                    Some(Object::Dictionary(d)) => Some(d),
                    _ => None,
                },
                _ => None,
            };
            if let Some(xd) = xobj_dict {
                for (name, value) in xd.iter() {
                    if let Object::Reference(r) = value {
                        xobject_map.insert(name.clone(), *r);
                    }
                }
            }
        }
    }

    // Decode the Form XObject's content stream.
    let content = match stream.decode_content() {
        Ok(c) => c,
        Err(e) => {
            debug!(xobj_id = ?xobj_id, error = %e, "Failed to decode Form XObject content");
            return;
        }
    };

    collect_chars_from_operations(
        doc,
        &content.operations,
        &font_map,
        &xobject_map,
        font_chars,
        depth,
    );
}

/// Extract character codes from a PDF string and insert them as glyph IDs.
///
/// For CIDFont (Type0): 2-byte big-endian encoding, each pair = one CID.
/// For simple fonts: 1-byte encoding, each byte = one character code.
fn extract_char_codes(bytes: &[u8], is_cid: bool, chars: &mut HashSet<u16>) {
    if is_cid {
        for chunk in bytes.chunks(2) {
            if chunk.len() == 2 {
                let cid = u16::from_be_bytes([chunk[0], chunk[1]]);
                chars.insert(cid);
            }
        }
    } else {
        for &b in bytes {
            chars.insert(b as u16);
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_already_subsetted_detects_prefix() {
        assert!(is_already_subsetted("ABCDEF+Arial"));
        assert!(!is_already_subsetted("Arial"));
        assert!(!is_already_subsetted("abc+Arial")); // lowercase
        assert!(!is_already_subsetted("AB+Ar")); // too short prefix
    }

    #[test]
    fn subset_fonts_empty_doc_returns_zero() {
        let mut doc = Document::new();
        let saved = subset_fonts(&mut doc).unwrap();
        assert_eq!(saved, 0);
    }
}
