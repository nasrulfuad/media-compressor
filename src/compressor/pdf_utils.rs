//! Shared PDF utility functions used across multiple compressor modules.
//!
//! Consolidates duplicated helpers for zlib compression/decompression,
//! PDF dictionary inspection, and object size estimation.

use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use lopdf::{Dictionary, Object};
use std::io::{Read, Write};

// ── Size thresholds ──────────────────────────────────────────────────────────

/// Minimum stream byte length worth re-compressing or deduplicating.
pub const MIN_COMPRESSIBLE_SIZE: usize = 64;

/// Minimum image stream byte length worth processing.
pub const MIN_IMAGE_SIZE: usize = 128;

/// Minimum pixel dimension for orphan image downsampling.
pub const ORPHAN_IMAGE_MIN_DIM: u32 = 300;

// ── Image quality adjustment constants ───────────────────────────────────────

/// Stddev below this threshold indicates a flat/uniform image.
pub const FLAT_IMAGE_STDDEV: f64 = 20.0;

/// Stddev above this threshold indicates a complex/photographic image.
pub const COMPLEX_IMAGE_STDDEV: f64 = 60.0;

/// Quality reduction for flat images.
pub const FLAT_QUALITY_REDUCTION: u8 = 15;

/// Quality boost for complex images.
pub const COMPLEX_QUALITY_BOOST: u8 = 5;

/// Maximum JPEG quality cap.
pub const MAX_QUALITY: u8 = 95;

// ── Zlib compression/decompression ───────────────────────────────────────────

/// Decompress zlib/deflate-encoded bytes.
pub fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib decompress failed: {}", e))?;
    Ok(out)
}

/// Compress bytes using zlib at the given compression level.
pub fn zlib_compress(data: &[u8], level: Compression) -> Result<Vec<u8>, String> {
    let mut encoder = ZlibEncoder::new(Vec::new(), level);
    encoder
        .write_all(data)
        .map_err(|e| format!("zlib compress write failed: {}", e))?;
    encoder
        .finish()
        .map_err(|e| format!("zlib compress finish failed: {}", e))
}

/// Compress bytes using zlib at the best compression level.
pub fn zlib_compress_best(data: &[u8]) -> Result<Vec<u8>, String> {
    zlib_compress(data, Compression::best())
}

/// Decompress a FlateDecode stream, returning original data on failure.
///
/// This is a lenient variant used by inline image processing where
/// decompression failure should fall back to the raw content.
pub fn zlib_decompress_or_raw(data: &[u8]) -> Vec<u8> {
    zlib_decompress(data).unwrap_or_else(|_| data.to_vec())
}

// ── PDF dictionary helpers ───────────────────────────────────────────────────

/// Extract the primary (first) filter name from a stream dictionary.
pub fn get_primary_filter(dict: &Dictionary) -> Option<String> {
    match dict.get(b"Filter") {
        Ok(Object::Name(name)) => String::from_utf8(name.clone()).ok(),
        Ok(Object::Array(arr)) => {
            if let Some(Object::Name(name)) = arr.first() {
                String::from_utf8(name.clone()).ok()
            } else {
                None
            }
        }
        _ => None,
    }
}

// ── Object size estimation ───────────────────────────────────────────────────

/// Rough byte-size estimate for a PDF object (used for reporting only).
pub fn estimate_object_size(obj: &Object) -> u64 {
    match obj {
        Object::Stream(s) => s.content.len() as u64,
        Object::Dictionary(d) => d.len() as u64 * 20,
        Object::Array(a) => a.len() as u64 * 16,
        Object::String(s, _) => s.len() as u64,
        _ => 16,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::Stream;

    #[test]
    fn zlib_round_trip() {
        let original = b"Hello, world! This is a test of zlib compression.";
        let compressed = zlib_compress(original, Compression::best()).unwrap();
        let decompressed = zlib_decompress(&compressed).unwrap();
        assert_eq!(decompressed, original);
    }

    #[test]
    fn zlib_compress_best_works() {
        let data = b"some data to compress with best level";
        let compressed = zlib_compress_best(data).unwrap();
        let decompressed = zlib_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn zlib_decompress_or_raw_returns_raw_on_invalid() {
        let invalid = b"not zlib data";
        let result = zlib_decompress_or_raw(invalid);
        assert_eq!(result, invalid);
    }

    #[test]
    fn get_primary_filter_returns_name() {
        let mut dict = Dictionary::new();
        dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        assert_eq!(get_primary_filter(&dict).as_deref(), Some("FlateDecode"));
    }

    #[test]
    fn get_primary_filter_returns_first_from_array() {
        let mut dict = Dictionary::new();
        dict.set(
            "Filter",
            Object::Array(vec![
                Object::Name(b"FlateDecode".to_vec()),
                Object::Name(b"LZWDecode".to_vec()),
            ]),
        );
        assert_eq!(get_primary_filter(&dict).as_deref(), Some("FlateDecode"));
    }

    #[test]
    fn get_primary_filter_returns_none_when_absent() {
        let dict = Dictionary::new();
        assert_eq!(get_primary_filter(&dict), None);
    }

    #[test]
    fn estimate_object_size_stream() {
        let mut dict = Dictionary::new();
        dict.set("Length", Object::Integer(11));
        let obj = Object::Stream(Stream {
            dict,
            content: b"hello world".to_vec(),
            allows_compression: true,
            start_position: None,
        });
        assert_eq!(estimate_object_size(&obj), 11);
    }

    #[test]
    fn estimate_object_size_integer() {
        assert_eq!(estimate_object_size(&Object::Integer(42)), 16);
    }
}
