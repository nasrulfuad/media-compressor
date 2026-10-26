use std::io::{Read, Write};

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::error::{AppError, AppResult};

pub const MAX_DEFLATE_LEVEL: u32 = 9;

pub fn deflate(data: &[u8], level: u32) -> AppResult<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level.min(MAX_DEFLATE_LEVEL)));
    encoder
        .write_all(data)
        .map_err(|error| AppError::Encode(format!("zlib write failed: {error}")))?;
    encoder
        .finish()
        .map_err(|error| AppError::Encode(format!("zlib finish failed: {error}")))
}

pub fn inflate_bounded(data: &[u8], max_output_bytes: usize) -> AppResult<Vec<u8>> {
    let mut output = Vec::new();
    let read_limit = (max_output_bytes as u64).saturating_add(1);
    ZlibDecoder::new(data)
        .take(read_limit)
        .read_to_end(&mut output)
        .map_err(|error| AppError::Decompress(error.to_string()))?;
    if output.len() > max_output_bytes {
        return Err(AppError::DecompressionLimitExceeded {
            limit: max_output_bytes,
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_then_inflate_round_trips() {
        let original = b"Hello, world! This is a test of zlib compression.".repeat(10);
        let compressed = deflate(&original, 9).unwrap();
        let restored = inflate_bounded(&compressed, original.len()).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn deflate_clamps_level_above_maximum() {
        let compressed = deflate(b"abc", 99).unwrap();
        assert_eq!(inflate_bounded(&compressed, 3).unwrap(), b"abc");
    }

    #[test]
    fn inflate_bounded_rejects_invalid_data() {
        let error = inflate_bounded(b"not zlib data", 100).unwrap_err();
        assert!(matches!(error, AppError::Decompress(_)));
    }

    #[test]
    fn inflate_bounded_accepts_data_at_limit() {
        let original = vec![7u8; 1024];
        let compressed = deflate(&original, 6).unwrap();
        assert_eq!(inflate_bounded(&compressed, 1024).unwrap(), original);
    }

    #[test]
    fn inflate_bounded_rejects_flate_bomb() {
        let bomb = deflate(&vec![0u8; 64 * 1024 * 1024], 9).unwrap();
        let error = inflate_bounded(&bomb, 1024 * 1024).unwrap_err();
        assert!(matches!(
            error,
            AppError::DecompressionLimitExceeded { limit } if limit == 1024 * 1024
        ));
    }
}
