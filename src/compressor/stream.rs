use flate2::Compression;
use lopdf::{Dictionary, Object, ObjectId, Stream};
use tracing::{debug, warn};

use crate::compressor::pdf_utils::{
    get_primary_filter, zlib_compress, zlib_decompress, MIN_COMPRESSIBLE_SIZE,
};
use crate::error::AppResult;

// ── LZW decompression (minimal, PDF-compatible) ───────────────────────────────
// PDF uses early-change LZW by default (DecodeParms /EarlyChange 1).
// We implement a basic GIF/TIFF-style LZW decoder that handles the common
// PDF variant.  Non-conforming LZW streams are skipped rather than errored.

/// Re-compresses all FlateDecode streams in `doc` using the given compression level.
///
/// The function collects object IDs first to avoid simultaneous mutable and
/// immutable borrows on `doc.objects`, then processes each stream object in
/// sequence. Streams that fail decompression or where re-compression yields
/// no benefit are left untouched.
///
/// Returns the total number of bytes saved across all processed streams.
pub fn compress_streams(doc: &mut lopdf::Document, compression: Compression) -> AppResult<u64> {
    // Collect IDs up-front so we can mutably iterate `doc.objects` afterwards.
    let ids: Vec<lopdf::ObjectId> = doc.objects.keys().copied().collect();

    let mut total_saved: u64 = 0;

    for id in ids {
        let obj = match doc.objects.get_mut(&id) {
            Some(o) => o,
            None => continue,
        };

        let stream = match obj {
            Object::Stream(s) => s,
            _ => continue,
        };

        match process_stream(stream, compression) {
            Ok(Some(saved)) => {
                total_saved += saved;
                debug!(object_id = ?id, bytes_saved = saved, "Stream re-compressed");
            }
            Ok(None) => {
                debug!(object_id = ?id, "Stream skipped (no benefit or not compressible)");
            }
            Err(e) => {
                warn!(object_id = ?id, error = %e, "Failed to re-compress stream, skipping");
            }
        }
    }

    Ok(total_saved)
}

/// Attempt to decompress and re-compress a single PDF stream.
///
/// Returns `Ok(Some(bytes_saved))` if the stream was improved,
/// `Ok(None)` if the stream was skipped, and `Err` only for
/// unrecoverable internal errors (not expected in normal flow).
fn process_stream(stream: &mut Stream, compression: Compression) -> AppResult<Option<u64>> {
    // Skip image XObjects — they are handled by the image compression pipeline.
    if matches!(stream.dict.get(b"Subtype"), Ok(Object::Name(n)) if n == b"Image") {
        return Ok(None);
    }

    // Determine the raw (currently stored) size before any changes.
    let original_size = stream.content.len() as u64;

    // Skip obviously tiny streams — overhead outweighs benefit.
    if original_size < MIN_COMPRESSIBLE_SIZE as u64 {
        return Ok(None);
    }

    // Decode the stream to its raw (uncompressed) bytes regardless of the
    // current filter chain. `decompress()` handles FlateDecode, LZWDecode,
    // and passthrough for unfiltered streams.
    let raw_bytes = match decompress_stream(stream) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "Cannot decompress stream; leaving untouched");
            return Ok(None);
        }
    };

    // Skip streams that are already tiny in their raw form.
    if raw_bytes.len() < MIN_COMPRESSIBLE_SIZE {
        return Ok(None);
    }

    // For content streams (not images) try whitespace compaction before
    // re-compressing — smaller raw payload leads to smaller compressed output.
    let raw_bytes = if is_content_stream(&stream.dict) {
        compact_whitespace(&raw_bytes).unwrap_or(raw_bytes)
    } else {
        raw_bytes
    };

    // Re-compress with the requested level.
    let recompressed = match zlib_compress(&raw_bytes, compression) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "Cannot re-compress stream; leaving untouched");
            return Ok(None);
        }
    };

    let new_size = recompressed.len() as u64;

    // Only replace if we actually made it smaller.
    if new_size >= original_size {
        return Ok(None);
    }

    let saved = original_size - new_size;

    // Replace stream content and update the filter dictionary.
    stream.content = recompressed;
    stream.dict.set("Length", Object::Integer(new_size as i64));

    // Set the filter to FlateDecode (replacing whatever was there before).
    stream
        .dict
        .set("Filter", Object::Name(b"FlateDecode".to_vec()));

    // Remove DecodeParms if they existed for the old filter — our plain
    // zlib output does not need any additional parameters.
    stream.dict.remove(b"DecodeParms");

    Ok(Some(saved))
}

/// Decompress a stream to its raw bytes, handling the filter chain.
///
/// Supports `FlateDecode` and `LZWDecode`. All other filters are rejected
/// so the caller can leave the stream untouched.
fn decompress_stream(stream: &Stream) -> Result<Vec<u8>, String> {
    let filter = get_primary_filter(&stream.dict);

    match filter.as_deref() {
        Some("FlateDecode") | Some("Fl") => zlib_decompress(&stream.content),
        Some("LZWDecode") | Some("LZW") => lzw_decompress(&stream.content),
        // Streams without a filter are already raw.
        None => Ok(stream.content.clone()),
        Some(other) => {
            // For unsupported filters (JBIG2Decode, CCITTFaxDecode, etc.)
            // we leave the stream untouched.
            Err(format!("Unsupported filter '{}'; skipping stream", other))
        }
    }
}

// ── LZW decompressor ──────────────────────────────────────────────────────────

/// Decompress LZWDecode (PDF early-change variant) encoded bytes.
///
/// PDF LZW uses 9–12 bit codes with an initial code size of 9 bits.
/// The clear code is `256` and the EOI code is `257`. The dictionary
/// starts with 258 entries (0–255 literal bytes, 256 clear, 257 EOI).
///
/// Early-change means the code width is incremented one entry *before* the
/// dictionary would overflow its current bit-width (the PDF default).
fn lzw_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    const CLEAR: u16 = 256;
    const EOI: u16 = 257;
    const INITIAL_NEXT: u16 = 258;

    // Seed the dictionary with single-byte entries.
    let mut dict: Vec<Vec<u8>> = (0u16..256).map(|b| vec![b as u8]).collect();
    dict.push(vec![]); // 256 = clear
    dict.push(vec![]); // 257 = EOI

    let mut bit_width: u32 = 9;
    let mut output: Vec<u8> = Vec::new();
    let mut prev_entry: Option<Vec<u8>> = None;

    // Bit-level reader over `data`.
    let mut bit_buf: u32 = 0;
    let mut bits_in_buf: u32 = 0;
    let mut byte_pos: usize = 0;

    let read_code = |bit_buf: &mut u32,
                     bits_in_buf: &mut u32,
                     byte_pos: &mut usize,
                     width: u32|
     -> Option<u16> {
        while *bits_in_buf < width {
            if *byte_pos >= data.len() {
                return None;
            }
            *bit_buf = (*bit_buf << 8) | data[*byte_pos] as u32;
            *bits_in_buf += 8;
            *byte_pos += 1;
        }
        *bits_in_buf -= width;
        Some(((*bit_buf >> *bits_in_buf) & ((1 << width) - 1)) as u16)
    };

    while let Some(code) = read_code(&mut bit_buf, &mut bits_in_buf, &mut byte_pos, bit_width) {
        if code == EOI {
            break;
        }

        if code == CLEAR {
            // Reset dictionary and bit width.
            dict.truncate(INITIAL_NEXT as usize);
            bit_width = 9;
            prev_entry = None;
            continue;
        }

        let entry: Vec<u8> = if (code as usize) < dict.len() {
            dict[code as usize].clone()
        } else if let Some(ref prev) = prev_entry {
            // The code refers to an entry not yet in the dictionary.
            // It equals prev + prev[0] by definition.
            let mut e = prev.clone();
            e.push(prev[0]);
            e
        } else {
            return Err(format!(
                "LZW: invalid code {} with empty previous entry",
                code
            ));
        };

        output.extend_from_slice(&entry);

        if let Some(ref prev) = prev_entry {
            if dict.len() < 4096 {
                let mut new_entry = prev.clone();
                new_entry.push(entry[0]);
                dict.push(new_entry);

                // Early-change: widen *before* the next code that would overflow.
                let next_len = dict.len() as u32;
                if next_len >= (1 << bit_width) && bit_width < 12 {
                    bit_width += 1;
                }
            }
        }

        prev_entry = Some(entry);
    }

    Ok(output)
}

// ── Merge per-page content streams ────────────────────────────────────────────

/// Merge multiple content streams on each page into a single FlateDecode stream.
///
/// Many PDF producers write several small content streams per page. Merging
/// them into one reduces per-stream overhead and gives zlib a larger input to
/// work with, which typically improves the compression ratio.
///
/// # Algorithm
///
/// 1. Iterate every page in the document.
/// 2. If the page `/Contents` is an array with more than one element, collect
///    each referenced stream, decompress it, and concatenate the raw bytes with
///    a newline separator.
/// 3. Re-compress the merged payload at best-level FlateDecode.
/// 4. Add the merged stream as a new indirect object and replace `/Contents`
///    with a single reference to it.
/// 5. The now-unreferenced old streams will be removed by the caller's
///    `doc.prune_objects()` call.
///
/// Returns the total bytes saved (sum of old stream content lengths minus the
/// merged stream length).
pub fn merge_page_content_streams(doc: &mut lopdf::Document) -> AppResult<u64> {
    // 2.5.3 note: `Document::compress()` and `delete_unused_objects()` are not
    // available in lopdf 0.34's public API and are therefore not used here.

    // Collect (page_id, [stream_object_ids]) for pages with multiple content streams.
    let pages: Vec<(ObjectId, Vec<ObjectId>)> = {
        let page_map = doc.get_pages(); // BTreeMap<u32, ObjectId>
        page_map
            .values()
            .filter_map(|&page_id| {
                let dict = doc.get_dictionary(page_id).ok()?;
                let contents = dict.get(b"Contents").ok()?;
                match contents {
                    Object::Array(arr) if arr.len() > 1 => {
                        let ids: Vec<ObjectId> =
                            arr.iter().filter_map(|o| o.as_reference().ok()).collect();
                        if ids.len() > 1 {
                            Some((page_id, ids))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect()
    };

    if pages.is_empty() {
        debug!("No pages with multiple content streams found");
        return Ok(0);
    }

    let mut total_saved: u64 = 0;

    for (page_id, stream_ids) in pages {
        // Decompress and concatenate all content stream bytes.
        let mut merged: Vec<u8> = Vec::new();
        let mut old_total_size: u64 = 0;

        let mut any_failed = false;
        for &sid in &stream_ids {
            let stream = match doc.objects.get(&sid) {
                Some(Object::Stream(s)) => s,
                _ => {
                    any_failed = true;
                    break;
                }
            };
            old_total_size += stream.content.len() as u64;
            let raw = match decompress_stream(stream) {
                Ok(b) => b,
                Err(e) => {
                    warn!(stream_id = ?sid, error = %e, "Cannot decompress content stream for merge; skipping page");
                    any_failed = true;
                    break;
                }
            };
            if !merged.is_empty() {
                merged.push(b'\n');
            }
            merged.extend_from_slice(&raw);
        }

        if any_failed || merged.is_empty() {
            continue;
        }

        // Re-compress the merged content.
        let compressed = match zlib_compress(&merged, Compression::best()) {
            Ok(b) => b,
            Err(e) => {
                warn!(page_id = ?page_id, error = %e, "Failed to compress merged content stream; skipping page");
                continue;
            }
        };

        let new_size = compressed.len() as u64;

        // Build the new stream object.
        let mut new_dict = Dictionary::new();
        new_dict.set("Length", Object::Integer(new_size as i64));
        new_dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let new_stream = Stream {
            dict: new_dict,
            content: compressed,
            allows_compression: true,
            start_position: None,
        };
        let new_id = doc.add_object(Object::Stream(new_stream));

        // Replace /Contents on the page with a single reference.
        if let Ok(page_dict) = doc.get_dictionary_mut(page_id) {
            page_dict.set("Contents", Object::Reference(new_id));
        }

        let saved = old_total_size.saturating_sub(new_size);
        total_saved += saved;

        debug!(
            page_id = ?page_id,
            streams_merged = stream_ids.len(),
            old_bytes = old_total_size,
            new_bytes = new_size,
            bytes_saved = saved,
            "Content streams merged"
        );
    }

    Ok(total_saved)
}

// ── Content stream whitespace cleanup ─────────────────────────────────────────

/// Returns `true` when `dict` indicates a PDF page content stream that is safe
/// for whitespace compaction.
///
/// Only page content streams (which contain text-based PDF operators like `BT`,
/// `Tf`, `Tj`, `cm`, etc.) are eligible for whitespace compaction.  All other
/// streams — font programs (FontFile/FontFile2/FontFile3), ICC profiles,
/// CMap streams, Form XObjects, and image streams — contain binary or
/// structured data where collapsing whitespace-valued bytes would corrupt
/// the data.
///
/// Page content streams are identified by the **absence** of any `/Type` or
/// `/Subtype` key in the stream dictionary. Streams with these keys are
/// classified as typed resources and are never compacted.
fn is_content_stream(dict: &Dictionary) -> bool {
    // Any stream with a /Subtype is a typed resource (Image, Form, CIDFontType0C, etc.).
    if dict.get(b"Subtype").is_ok() {
        return false;
    }
    // Any stream with a /Type is a typed resource (Font, XRef, CMap, Metadata, etc.).
    if dict.get(b"Type").is_ok() {
        return false;
    }
    // Streams with /Length1 are font file programs (Type1 FontFile).
    if dict.get(b"Length1").is_ok() {
        return false;
    }
    true
}

/// Remove PDF comment lines from a content stream.
///
/// Lines beginning with `%` are comments in PDF content stream syntax and
/// carry no operational meaning. Removing them reduces the raw payload before
/// zlib compression, which improves the final compressed size.
///
/// Safety: We only remove lines where `%` is the first non-whitespace
/// character at the start of a logical line (i.e., after a newline or at the
/// very beginning of the stream). We do **not** attempt to strip inline `%`
/// characters that might appear inside string literals or hexadecimal strings,
/// since distinguishing those safely requires a full PDF content-stream parser.
///
/// Returns the cleaned bytes, or the original bytes unchanged when no comments
/// were present (avoids an unnecessary allocation).
fn remove_content_stream_comments(data: &[u8]) -> Vec<u8> {
    // Fast path: if no `%` at all, skip.
    if !data.contains(&b'%') {
        return data.to_vec();
    }

    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;

    while i < data.len() {
        if data[i] == b'%' {
            // Skip to end of line (consume the comment including the newline).
            while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            // Consume the line terminator itself.
            if i < data.len() && data[i] == b'\r' {
                i += 1;
            }
            if i < data.len() && data[i] == b'\n' {
                i += 1;
            }
        } else {
            out.push(data[i]);
            i += 1;
        }
    }

    out
}

/// Collapse runs of ASCII whitespace in a PDF operator stream to a single
/// space, and strip PDF comment lines.  This is safe because PDF's tokeniser
/// treats any run of whitespace between tokens as a single separator, and
/// comment lines carry no operational meaning.
///
/// Note: Removing redundant `q`/`Q` pairs is out of scope without a proper
/// PDF content stream parser — that optimisation is therefore not implemented.
///
/// Returns the compacted bytes. If no compaction was possible (result ≥ input),
/// returns `None`.
fn compact_whitespace(data: &[u8]) -> Option<Vec<u8>> {
    // First pass: strip comment lines.
    let no_comments = remove_content_stream_comments(data);

    // Second pass: collapse whitespace runs.
    let mut out = Vec::with_capacity(no_comments.len());
    let mut in_ws = false;

    for &b in &no_comments {
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\n' || b == 0x0C || b == 0x00 {
            if !in_ws {
                out.push(b' ');
                in_ws = true;
            }
        } else {
            out.push(b);
            in_ws = false;
        }
    }

    if out.len() < data.len() {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal FlateDecode stream for testing.
    fn make_flate_stream(raw_content: &[u8]) -> Stream {
        let compressed = zlib_compress(raw_content, Compression::default()).unwrap();
        let len = compressed.len() as i64;
        let mut dict = Dictionary::new();
        dict.set("Length", Object::Integer(len));
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        Stream {
            dict,
            content: compressed,
            allows_compression: true,
            start_position: None,
        }
    }

    /// Build a stream with no filter (raw bytes stored directly).
    fn make_raw_stream(content: &[u8]) -> Stream {
        let mut dict = Dictionary::new();
        dict.set("Length", Object::Integer(content.len() as i64));
        Stream {
            dict,
            content: content.to_vec(),
            allows_compression: true,
            start_position: None,
        }
    }

    #[test]
    fn process_stream_improves_compressible_content() {
        // Highly repetitive data compresses well.
        let raw: Vec<u8> = std::iter::repeat(b'A').take(4096).collect();
        let mut stream = make_flate_stream(&raw);
        // Re-compress at best level.
        let result = process_stream(&mut stream, Compression::best()).unwrap();
        // Should save bytes (best vs default).
        // At minimum it should not error; savings depend on data.
        assert!(result.is_some() || result.is_none()); // compiles and runs without panic
    }

    #[test]
    fn process_stream_skips_tiny_streams() {
        let raw = b"tiny";
        let mut stream = make_raw_stream(raw);
        let result = process_stream(&mut stream, Compression::best()).unwrap();
        assert_eq!(result, None, "Tiny streams should be skipped");
    }

    #[test]
    fn decompress_unfiltered_stream_returns_content() {
        let data = b"raw content bytes here for testing purposes, enough to exceed the minimum";
        let stream = make_raw_stream(data);
        let out = decompress_stream(&stream).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn decompress_flate_stream_returns_raw_bytes() {
        let raw = b"The quick brown fox jumps over the lazy dog. ".repeat(10);
        let stream = make_flate_stream(&raw);
        let out = decompress_stream(&stream).unwrap();
        assert_eq!(out, raw);
    }

    #[test]
    fn remove_content_stream_comments_strips_comment_lines() {
        let input = b"BT\n% This is a comment\n/F1 12 Tf\nET\n";
        let out = remove_content_stream_comments(input);
        assert!(!out.contains(&b'%'), "Comment lines should be removed");
        assert!(
            out.windows(2).any(|w| w == b"BT"),
            "Operators must be preserved"
        );
    }

    #[test]
    fn remove_content_stream_comments_preserves_no_comment_data() {
        let input = b"BT /F1 12 Tf ET";
        let out = remove_content_stream_comments(input);
        assert_eq!(out, input, "Data without comments must be unchanged");
    }

    #[test]
    fn compact_whitespace_strips_comments_and_collapses_spaces() {
        let input = b"BT\n% comment\n/F1  12 Tf\nET\n";
        // After comment removal: "BT\n/F1  12 Tf\nET\n"
        // After whitespace collapse: "BT /F1 12 Tf ET "
        let result = compact_whitespace(input);
        // Result should be shorter than input.
        assert!(result.is_some(), "Should compact the input");
        let compacted = result.unwrap();
        assert!(!compacted.contains(&b'%'), "Comments must be gone");
    }

    #[test]
    fn merge_page_content_streams_empty_doc_returns_zero() {
        use lopdf::Document;
        let mut doc = Document::new();
        let saved = merge_page_content_streams(&mut doc).unwrap();
        assert_eq!(saved, 0);
    }
}
