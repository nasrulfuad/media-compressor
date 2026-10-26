use std::io::Read;
use std::path::Path;

use crate::error::AppResult;

use super::{ImageFormat, MediaType};

/// Detect the media type of a file by reading its magic bytes.
///
/// The first 16 bytes are read from the file and matched against known
/// signatures. The file extension is intentionally ignored — magic bytes are
/// the authoritative source of truth.
pub struct FileTypeResolver;

impl FileTypeResolver {
    /// Read up to 16 bytes from `file_path` and return the detected [`MediaType`].
    pub fn detect(file_path: &Path) -> AppResult<MediaType> {
        let mut file = std::fs::File::open(file_path)?;
        let mut header = [0u8; 16];
        let n = file.read(&mut header)?;
        let header = &header[..n];

        // ── PDF (%PDF) ─────────────────────────────────────────────────────
        if header.len() >= 4 && &header[..4] == b"%PDF" {
            return Ok(MediaType::Pdf);
        }

        // ── JPEG (FF D8 FF) ────────────────────────────────────────────────
        if header.len() >= 3 && header[..3] == [0xFF, 0xD8, 0xFF] {
            return Ok(MediaType::Image(ImageFormat::Jpeg));
        }

        // ── PNG (89 50 4E 47 0D 0A 1A 0A) ─────────────────────────────────
        if header.len() >= 8 && header[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
            return Ok(MediaType::Image(ImageFormat::Png));
        }

        // ── GIF (GIF87a / GIF89a) ──────────────────────────────────────────
        if header.len() >= 6 && (&header[..6] == b"GIF87a" || &header[..6] == b"GIF89a") {
            return Ok(MediaType::Image(ImageFormat::Gif));
        }

        // ── TIFF (little-endian: 49 49 2A 00 / big-endian: 4D 4D 00 2A) ───
        if header.len() >= 4
            && (header[..4] == [0x49, 0x49, 0x2A, 0x00] || header[..4] == [0x4D, 0x4D, 0x00, 0x2A])
        {
            return Ok(MediaType::Image(ImageFormat::Tiff));
        }

        // ── BMP (42 4D) ────────────────────────────────────────────────────
        if header.len() >= 2 && header[..2] == [0x42, 0x4D] {
            return Ok(MediaType::Image(ImageFormat::Bmp));
        }

        // ── WebP (RIFF????WEBP) ────────────────────────────────────────────
        if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
            return Ok(MediaType::Image(ImageFormat::WebP));
        }

        Ok(MediaType::Unknown(header.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_file(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    #[test]
    fn detect_pdf() {
        let f = write_file(b"%PDF-1.4 some content");
        assert_eq!(FileTypeResolver::detect(f.path()).unwrap(), MediaType::Pdf);
    }

    #[test]
    fn detect_jpeg() {
        let f = write_file(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Jpeg)
        );
    }

    #[test]
    fn detect_png() {
        let f = write_file(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00]);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Png)
        );
    }

    #[test]
    fn detect_gif87a() {
        let f = write_file(b"GIF87a\x01\x00\x01\x00");
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Gif)
        );
    }

    #[test]
    fn detect_gif89a() {
        let f = write_file(b"GIF89a\x01\x00\x01\x00");
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Gif)
        );
    }

    #[test]
    fn detect_tiff_little_endian() {
        let f = write_file(&[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Tiff)
        );
    }

    #[test]
    fn detect_tiff_big_endian() {
        let f = write_file(&[0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Tiff)
        );
    }

    #[test]
    fn detect_bmp() {
        let f = write_file(&[0x42, 0x4D, 0x36, 0x00, 0x00, 0x00]);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::Bmp)
        );
    }

    #[test]
    fn detect_webp() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]); // file size (arbitrary)
        bytes.extend_from_slice(b"WEBP");
        let f = write_file(&bytes);
        assert_eq!(
            FileTypeResolver::detect(f.path()).unwrap(),
            MediaType::Image(ImageFormat::WebP)
        );
    }

    #[test]
    fn detect_unknown_returns_magic_bytes() {
        let data = &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        let f = write_file(data);
        match FileTypeResolver::detect(f.path()).unwrap() {
            MediaType::Unknown(bytes) => assert_eq!(bytes, data),
            other => panic!("Expected Unknown, got {:?}", other),
        }
    }

    #[test]
    fn detect_ignores_extension_uses_magic_bytes() {
        // File named .txt but contains PDF magic bytes.
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        f.write_all(b"%PDF-1.4 content").unwrap();
        assert_eq!(FileTypeResolver::detect(f.path()).unwrap(), MediaType::Pdf);
    }
}
