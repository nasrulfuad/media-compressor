//! Inline image detection and conversion for PDF content streams (Phase 2.5.8).
//!
//! Inline images use operators `BI` (begin image dict), `ID` (begin data),
//! and `EI` (end image) directly inside content streams.  This module:
//!
//! 1. Detects inline images in decompressed page content streams.
//! 2. Converts them to XObject Image references: extracts the image data,
//!    creates a new Image XObject, and replaces the `BI`…`EI` block with
//!    a `/Do` operator that references the new XObject.
//!
//! Converting inline images to XObjects enables the image compression
//! pipeline (mozjpeg, oxipng, DPI downsampling) to process them and also
//! allows stream deduplication to catch repeated inline images.

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashMap;
use tracing::debug;

use crate::compressor::level::CompressionLevel;
use crate::compressor::pdf_utils::{
    get_primary_filter, zlib_compress_best, zlib_decompress_or_raw,
};
use crate::error::AppResult;

// ── Inline image key abbreviations (PDF spec Table 93) ───────────────────────

fn expand_inline_key(abbrev: &[u8]) -> Vec<u8> {
    match abbrev {
        b"BPC" => b"BitsPerComponent".to_vec(),
        b"CS" => b"ColorSpace".to_vec(),
        b"D" => b"Decode".to_vec(),
        b"DP" => b"DecodeParms".to_vec(),
        b"F" => b"Filter".to_vec(),
        b"H" => b"Height".to_vec(),
        b"IM" => b"ImageMask".to_vec(),
        b"I" => b"Interpolate".to_vec(),
        b"W" => b"Width".to_vec(),
        other => other.to_vec(),
    }
}

fn expand_inline_value(name: &[u8]) -> Vec<u8> {
    match name {
        b"G" => b"DeviceGray".to_vec(),
        b"RGB" => b"DeviceRGB".to_vec(),
        b"CMYK" => b"DeviceCMYK".to_vec(),
        b"AHx" => b"ASCIIHexDecode".to_vec(),
        b"A85" => b"ASCII85Decode".to_vec(),
        b"LZW" => b"LZWDecode".to_vec(),
        b"Fl" => b"FlateDecode".to_vec(),
        b"RL" => b"RunLengthDecode".to_vec(),
        b"CCF" => b"CCITTFaxDecode".to_vec(),
        b"DCT" => b"DCTDecode".to_vec(),
        other => other.to_vec(),
    }
}

// ── Core conversion ──────────────────────────────────────────────────────────

/// Convert inline images in page content streams to XObject references.
///
/// Returns the number of inline images converted.
#[allow(unused_variables)]
pub fn convert_inline_images(
    doc: &mut Document,
    quality: u8,
    level: CompressionLevel,
) -> AppResult<u64> {
    let pages: Vec<ObjectId> = doc.page_iter().collect();
    let mut total_converted: u64 = 0;

    for page_id in pages {
        let converted = convert_inline_images_on_page(doc, page_id)?;
        total_converted += converted;
    }

    if total_converted == 0 {
        debug!("No inline images found in document");
    } else {
        debug!(
            total = total_converted,
            "Inline images converted to XObjects"
        );
    }

    Ok(total_converted)
}

/// Convert inline images on a single page.
fn convert_inline_images_on_page(doc: &mut Document, page_id: ObjectId) -> AppResult<u64> {
    // Get content stream IDs for this page.
    let content_ids: Vec<ObjectId> = match doc.get_dictionary(page_id) {
        Ok(d) => match d.get(b"Contents") {
            Ok(Object::Reference(id)) => vec![*id],
            Ok(Object::Array(arr)) => arr.iter().filter_map(|o| o.as_reference().ok()).collect(),
            _ => return Ok(0),
        },
        Err(_) => return Ok(0),
    };

    let mut converted: u64 = 0;
    // XObject name counter for generating unique names per page.
    let mut xobj_counter = find_max_inline_xobj_index(doc, page_id);

    for sid in content_ids {
        let stream = match doc.objects.get(&sid) {
            Some(Object::Stream(s)) => s.clone(),
            _ => continue,
        };

        let raw = decompress_content(&stream);
        let (new_content, xobjects) = extract_inline_images(&raw, &mut xobj_counter);

        if xobjects.is_empty() {
            continue;
        }

        converted += xobjects.len() as u64;

        // Create XObject Image entries in the document and register them.
        let mut xobj_refs: HashMap<String, ObjectId> = HashMap::new();
        for (name, img_stream) in &xobjects {
            let obj_id = doc.add_object(Object::Stream(img_stream.clone()));
            xobj_refs.insert(name.clone(), obj_id);
        }

        // Re-compress the modified content stream.
        let compressed = compress_zlib(&new_content);
        if let Some(Object::Stream(s)) = doc.objects.get_mut(&sid) {
            s.content = compressed;
            s.dict
                .set("Length", Object::Integer(s.content.len() as i64));
            s.dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
            s.dict.remove(b"DecodeParms");
        }

        // Add XObject references to the page's Resources.
        add_xobject_refs_to_page(doc, page_id, &xobj_refs);
    }

    Ok(converted)
}

/// Find the highest existing /ImInl<N> index in the page's XObject dictionary.
fn find_max_inline_xobj_index(doc: &Document, page_id: ObjectId) -> u32 {
    let resources = match doc.get_page_resources(page_id) {
        Ok((Some(d), _)) => d.clone(),
        _ => return 0,
    };
    let xobj_dict = match resources.get(b"XObject") {
        Ok(Object::Dictionary(d)) => d,
        Ok(Object::Reference(r)) => match doc.objects.get(r) {
            Some(Object::Dictionary(d)) => d,
            _ => return 0,
        },
        _ => return 0,
    };

    let mut max_idx: u32 = 0;
    for (key, _) in xobj_dict.iter() {
        let name = String::from_utf8_lossy(key);
        if let Some(rest) = name.strip_prefix("ImInl") {
            if let Ok(n) = rest.parse::<u32>() {
                max_idx = max_idx.max(n);
            }
        }
    }
    max_idx
}

/// Parse inline images from a decompressed content stream.
///
/// Returns the modified content stream (with inline images replaced by `Do`
/// operators) and a list of `(name, Stream)` pairs for the extracted images.
fn extract_inline_images(data: &[u8], counter: &mut u32) -> (Vec<u8>, Vec<(String, Stream)>) {
    let mut result = Vec::with_capacity(data.len());
    let mut xobjects: Vec<(String, Stream)> = Vec::new();
    let mut i = 0;

    while i < data.len() {
        // Look for "BI" operator preceded by whitespace/start.
        if i + 2 <= data.len()
            && data[i] == b'B'
            && data[i + 1] == b'I'
            && is_whitespace_or_start(data, i)
            && (i + 2 >= data.len() || is_pdf_whitespace(data[i + 2]))
        {
            // Try to parse the inline image.
            match parse_inline_image(data, i) {
                Some((dict, img_data, end_pos)) => {
                    *counter += 1;
                    let name = format!("ImInl{}", counter);

                    // Build an Image XObject stream.
                    let stream = build_image_xobject(dict, img_data);
                    xobjects.push((name.clone(), stream));

                    // Replace BI...EI with: /<name> Do
                    result.extend_from_slice(b"/");
                    result.extend_from_slice(name.as_bytes());
                    result.extend_from_slice(b" Do");

                    i = end_pos;
                    continue;
                }
                None => {
                    // Could not parse — leave byte as-is.
                    result.push(data[i]);
                    i += 1;
                }
            }
        } else {
            result.push(data[i]);
            i += 1;
        }
    }

    (result, xobjects)
}

/// Parsed inline image: dict entries, raw image data, end position.
type InlineImageParse = (Vec<(Vec<u8>, Object)>, Vec<u8>, usize);

/// Parse an inline image starting at position `bi_pos` (the 'B' of "BI").
///
/// Returns `(dict_entries, image_data_bytes, position_after_EI)` on success.
fn parse_inline_image(data: &[u8], bi_pos: usize) -> Option<InlineImageParse> {
    // Skip "BI" and subsequent whitespace.
    let mut pos = bi_pos + 2;
    pos = skip_whitespace(data, pos);

    // Parse key-value pairs until we find "ID".
    let mut dict_entries: Vec<(Vec<u8>, Object)> = Vec::new();

    loop {
        if pos >= data.len() {
            return None;
        }

        // Check for "ID" operator (marks start of image data).
        if pos + 1 < data.len()
            && data[pos] == b'I'
            && data[pos + 1] == b'D'
            && (pos + 2 >= data.len() || is_pdf_whitespace(data[pos + 2]))
        {
            // "ID" found — image data starts after exactly one whitespace byte.
            pos += 2;
            if pos < data.len() && is_pdf_whitespace(data[pos]) {
                pos += 1; // skip the single whitespace after ID
            }
            break;
        }

        // Parse a key (must be a name starting with '/').
        if data[pos] != b'/' {
            return None;
        }
        let (key, next) = parse_name(data, pos)?;
        pos = skip_whitespace(data, next);

        // Parse the value.
        let (value, next) = parse_inline_value(data, pos)?;
        pos = skip_whitespace(data, next);

        // Expand abbreviations.
        let expanded_key = expand_inline_key(&key);
        let expanded_value = match &value {
            Object::Name(n) => Object::Name(expand_inline_value(n)),
            other => other.clone(),
        };

        dict_entries.push((expanded_key, expanded_value));
    }

    // Now at start of image data. Find "EI" marker.
    // The EI must be preceded by whitespace and followed by whitespace or EOF.
    let data_start = pos;
    let mut data_end = None;

    // Scan for EI preceded by whitespace.
    let mut scan = data_start;
    while scan + 2 <= data.len() {
        if is_pdf_whitespace(data[scan])
            && scan + 1 < data.len()
            && data[scan + 1] == b'E'
            && scan + 2 < data.len()
            && data[scan + 2] == b'I'
            && (scan + 3 >= data.len() || is_pdf_whitespace(data[scan + 3]))
        {
            data_end = Some(scan);
            break;
        }
        scan += 1;
    }

    let data_end = data_end?;
    let img_data = data[data_start..data_end].to_vec();

    // Position after "EI" + whitespace.
    let after_ei = data_end + 3; // skip whitespace + 'E' + 'I'

    Some((dict_entries, img_data, after_ei))
}

/// Build an Image XObject Stream from inline image dict entries and raw data.
fn build_image_xobject(entries: Vec<(Vec<u8>, Object)>, img_data: Vec<u8>) -> Stream {
    let mut dict = Dictionary::new();
    dict.set("Type", Object::Name(b"XObject".to_vec()));
    dict.set("Subtype", Object::Name(b"Image".to_vec()));

    for (key, value) in entries {
        dict.set(key, value);
    }

    dict.set("Length", Object::Integer(img_data.len() as i64));

    Stream {
        dict,
        content: img_data,
        allows_compression: true,
        start_position: None,
    }
}

/// Add XObject references to a page's Resources dictionary.
fn add_xobject_refs_to_page(
    doc: &mut Document,
    page_id: ObjectId,
    xobj_refs: &HashMap<String, ObjectId>,
) {
    // Get or create the Resources dictionary for this page.
    let page_dict = match doc.objects.get_mut(&page_id) {
        Some(Object::Dictionary(d)) => d as *mut Dictionary,
        _ => return,
    };

    // Safety: we need mutable access to page dict while also adding objects.
    // The page_dict pointer is valid for the duration of this function.
    let page_dict = unsafe { &mut *page_dict };

    // Get or create XObject sub-dictionary.
    let xobj_dict_id: Option<ObjectId> = match page_dict.get(b"Resources") {
        Ok(Object::Dictionary(res)) => match res.get(b"XObject") {
            Ok(Object::Reference(r)) => Some(*r),
            _ => None,
        },
        Ok(Object::Reference(res_ref)) => {
            let res_ref = *res_ref;
            match doc.objects.get(&res_ref) {
                Some(Object::Dictionary(res)) => match res.get(b"XObject") {
                    Ok(Object::Reference(r)) => Some(*r),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    };

    if let Some(xobj_id) = xobj_dict_id {
        // Add to existing XObject dictionary (indirect).
        if let Some(Object::Dictionary(xobj_dict)) = doc.objects.get_mut(&xobj_id) {
            for (name, obj_id) in xobj_refs {
                xobj_dict.set(name.as_bytes(), Object::Reference(*obj_id));
            }
        }
    } else {
        // Try to add to inline Resources/XObject dictionary.
        // Get or create Resources dict.
        let has_resources = page_dict.get(b"Resources").is_ok();
        if !has_resources {
            page_dict.set("Resources", Object::Dictionary(Dictionary::new()));
        }

        if let Ok(Object::Dictionary(res)) = page_dict.get_mut(b"Resources") {
            let has_xobj = res.get(b"XObject").is_ok();
            if !has_xobj {
                res.set("XObject", Object::Dictionary(Dictionary::new()));
            }
            if let Ok(Object::Dictionary(xobj_dict)) = res.get_mut(b"XObject") {
                for (name, obj_id) in xobj_refs {
                    xobj_dict.set(name.as_bytes(), Object::Reference(*obj_id));
                }
            }
        }
    }
}

// ── Tokenizer helpers ────────────────────────────────────────────────────────

fn is_pdf_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 0x0C | 0x00)
}

fn is_whitespace_or_start(data: &[u8], pos: usize) -> bool {
    pos == 0 || is_pdf_whitespace(data[pos - 1])
}

fn skip_whitespace(data: &[u8], mut pos: usize) -> usize {
    while pos < data.len() && is_pdf_whitespace(data[pos]) {
        pos += 1;
    }
    pos
}

/// Parse a PDF name token starting with '/'.
fn parse_name(data: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
    if pos >= data.len() || data[pos] != b'/' {
        return None;
    }
    let mut end = pos + 1;
    while end < data.len()
        && !is_pdf_whitespace(data[end])
        && data[end] != b'/'
        && data[end] != b'['
        && data[end] != b'('
        && data[end] != b'<'
    {
        end += 1;
    }
    Some((data[pos + 1..end].to_vec(), end))
}

/// Parse an inline image dict value (integer, boolean, name, or array).
fn parse_inline_value(data: &[u8], pos: usize) -> Option<(Object, usize)> {
    if pos >= data.len() {
        return None;
    }

    match data[pos] {
        b'/' => {
            // Name value.
            let (name, end) = parse_name(data, pos)?;
            Some((Object::Name(name), end))
        }
        b'[' => {
            // Array value — find matching ']'.
            let mut depth = 1;
            let mut end = pos + 1;
            let mut items = Vec::new();
            while end < data.len() && depth > 0 {
                if data[end] == b'[' {
                    depth += 1;
                } else if data[end] == b']' {
                    depth -= 1;
                }
                end += 1;
            }
            // Simple: just store as a string for now.
            let inner = &data[pos + 1..end.saturating_sub(1)];
            // Try to parse numbers from the array.
            let parts: Vec<&[u8]> = inner
                .split(|b| is_pdf_whitespace(*b))
                .filter(|s| !s.is_empty())
                .collect();
            for part in parts {
                let s = String::from_utf8_lossy(part);
                if let Ok(n) = s.parse::<i64>() {
                    items.push(Object::Integer(n));
                } else if let Ok(f) = s.parse::<f32>() {
                    items.push(Object::Real(f));
                }
            }
            Some((Object::Array(items), end))
        }
        b't' if data[pos..].starts_with(b"true") => Some((Object::Boolean(true), pos + 4)),
        b'f' if data[pos..].starts_with(b"false") => Some((Object::Boolean(false), pos + 5)),
        _ => {
            // Try to parse as number.
            let mut end = pos;
            while end < data.len()
                && (data[end].is_ascii_digit() || data[end] == b'.' || data[end] == b'-')
            {
                end += 1;
            }
            if end == pos {
                return None;
            }
            let s = String::from_utf8_lossy(&data[pos..end]);
            if let Ok(n) = s.parse::<i64>() {
                Some((Object::Integer(n), end))
            } else if let Ok(f) = s.parse::<f32>() {
                Some((Object::Real(f), end))
            } else {
                None
            }
        }
    }
}

// ── Compression helpers ──────────────────────────────────────────────────────

fn decompress_content(stream: &lopdf::Stream) -> Vec<u8> {
    let filter = get_primary_filter(&stream.dict);
    match filter.as_deref() {
        Some("FlateDecode") | Some("Fl") => zlib_decompress_or_raw(&stream.content),
        _ => stream.content.clone(),
    }
}

fn compress_zlib(data: &[u8]) -> Vec<u8> {
    zlib_compress_best(data).unwrap_or_else(|_| data.to_vec())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inline_image_basic() {
        let data = b"q\nBI\n/W 10 /H 5 /BPC 8 /CS /G\nID\nXXXXX\nEI\nQ\n";
        let (new_content, xobjects) = extract_inline_images(data, &mut 0);
        assert_eq!(xobjects.len(), 1, "Should extract 1 inline image");
        assert!(
            new_content.windows(2).any(|w| w == b"Do"),
            "Content should contain Do operator"
        );
        // Check the XObject has correct dict entries.
        let (name, stream) = &xobjects[0];
        assert_eq!(name, "ImInl1");
        assert_eq!(stream.dict.get(b"Width").unwrap(), &Object::Integer(10));
        assert_eq!(stream.dict.get(b"Height").unwrap(), &Object::Integer(5));
    }

    #[test]
    fn parse_inline_image_no_images() {
        let data = b"q 1 0 0 1 0 0 cm /Img1 Do Q";
        let (new_content, xobjects) = extract_inline_images(data, &mut 0);
        assert!(xobjects.is_empty());
        assert_eq!(new_content, data);
    }

    #[test]
    fn parse_inline_image_multiple() {
        let data =
            b"BI /W 1 /H 1 /BPC 8 /CS /G\nID\nA\nEI BI /W 2 /H 2 /BPC 8 /CS /G\nID\nBB\nEI\n";
        let (_, xobjects) = extract_inline_images(data, &mut 0);
        assert_eq!(xobjects.len(), 2, "Should extract 2 inline images");
    }

    #[test]
    fn expand_inline_abbreviations() {
        assert_eq!(expand_inline_key(b"W"), b"Width");
        assert_eq!(expand_inline_key(b"H"), b"Height");
        assert_eq!(expand_inline_key(b"BPC"), b"BitsPerComponent");
        assert_eq!(expand_inline_value(b"G"), b"DeviceGray");
        assert_eq!(expand_inline_value(b"RGB"), b"DeviceRGB");
        assert_eq!(expand_inline_value(b"DCT"), b"DCTDecode");
    }

    #[test]
    fn convert_inline_images_empty_doc_returns_zero() {
        let mut doc = Document::new();
        let saved = convert_inline_images(&mut doc, 75, CompressionLevel::Medium).unwrap();
        assert_eq!(saved, 0);
    }
}
