use crate::domain::media::{ImageFormat, MediaType};

pub const MAGIC_BYTES_TO_READ: usize = 16;

const PDF_MAGIC: &[u8] = b"%PDF";
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const GIF87A_MAGIC: &[u8] = b"GIF87a";
const GIF89A_MAGIC: &[u8] = b"GIF89a";
const TIFF_LITTLE_ENDIAN_MAGIC: &[u8] = &[0x49, 0x49, 0x2A, 0x00];
const TIFF_BIG_ENDIAN_MAGIC: &[u8] = &[0x4D, 0x4D, 0x00, 0x2A];
const BMP_MAGIC: &[u8] = &[0x42, 0x4D];
const RIFF_MAGIC: &[u8] = b"RIFF";
const WEBP_MAGIC: &[u8] = b"WEBP";
const WEBP_MARKER_OFFSET: usize = 8;

const IMAGE_SIGNATURES: &[(&[u8], ImageFormat)] = &[
    (JPEG_MAGIC, ImageFormat::Jpeg),
    (PNG_MAGIC, ImageFormat::Png),
    (GIF87A_MAGIC, ImageFormat::Gif),
    (GIF89A_MAGIC, ImageFormat::Gif),
    (TIFF_LITTLE_ENDIAN_MAGIC, ImageFormat::Tiff),
    (TIFF_BIG_ENDIAN_MAGIC, ImageFormat::Tiff),
    (BMP_MAGIC, ImageFormat::Bmp),
];

fn is_webp(leading_bytes: &[u8]) -> bool {
    leading_bytes.starts_with(RIFF_MAGIC)
        && leading_bytes
            .get(WEBP_MARKER_OFFSET..WEBP_MARKER_OFFSET + WEBP_MAGIC.len())
            .is_some_and(|marker| marker == WEBP_MAGIC)
}

pub fn detect_media_type(leading_bytes: &[u8]) -> MediaType {
    let leading = &leading_bytes[..leading_bytes.len().min(MAGIC_BYTES_TO_READ)];
    if leading.starts_with(PDF_MAGIC) {
        return MediaType::Pdf;
    }
    if let Some((_, format)) = IMAGE_SIGNATURES
        .iter()
        .find(|(signature, _)| leading.starts_with(signature))
    {
        return MediaType::Image(*format);
    }
    if is_webp(leading) {
        return MediaType::Image(ImageFormat::WebP);
    }
    MediaType::Unknown {
        leading_bytes: leading.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_image(bytes: &[u8], format: ImageFormat) {
        assert_eq!(detect_media_type(bytes), MediaType::Image(format));
    }

    #[test]
    fn detect_pdf() {
        assert_eq!(detect_media_type(b"%PDF-1.4 some content"), MediaType::Pdf);
    }

    #[test]
    fn detect_jpeg() {
        assert_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10], ImageFormat::Jpeg);
    }

    #[test]
    fn detect_png() {
        assert_image(
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00],
            ImageFormat::Png,
        );
    }

    #[test]
    fn detect_gif87a_and_gif89a() {
        assert_image(b"GIF87a\x01\x00\x01\x00", ImageFormat::Gif);
        assert_image(b"GIF89a\x01\x00\x01\x00", ImageFormat::Gif);
    }

    #[test]
    fn detect_tiff_little_and_big_endian() {
        assert_image(&[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00], ImageFormat::Tiff);
        assert_image(&[0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00], ImageFormat::Tiff);
    }

    #[test]
    fn detect_bmp() {
        assert_image(&[0x42, 0x4D, 0x36, 0x00, 0x00, 0x00], ImageFormat::Bmp);
    }

    #[test]
    fn detect_webp() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"WEBP");
        assert_image(&bytes, ImageFormat::WebP);
    }

    #[test]
    fn detect_riff_without_webp_marker_is_unknown() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        bytes.extend_from_slice(b"WAVE");
        assert!(matches!(
            detect_media_type(&bytes),
            MediaType::Unknown { .. }
        ));
    }

    #[test]
    fn detect_unknown_returns_leading_bytes() {
        let bytes = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        assert_eq!(
            detect_media_type(&bytes),
            MediaType::Unknown {
                leading_bytes: bytes.to_vec()
            }
        );
    }

    #[test]
    fn detect_unknown_truncates_leading_bytes_to_sixteen() {
        let bytes = [0xAAu8; 40];
        match detect_media_type(&bytes) {
            MediaType::Unknown { leading_bytes } => {
                assert_eq!(leading_bytes.len(), MAGIC_BYTES_TO_READ)
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn detect_empty_input_is_unknown() {
        assert_eq!(
            detect_media_type(&[]),
            MediaType::Unknown {
                leading_bytes: Vec::new()
            }
        );
    }

    #[test]
    fn detect_truncated_pdf_magic_is_unknown() {
        assert!(matches!(
            detect_media_type(b"%PD"),
            MediaType::Unknown { .. }
        ));
    }
}
