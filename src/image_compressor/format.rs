use crate::compressor::level::CompressionLevel;
use crate::strategy::{ImageFormat, ImageOutputFormat};

/// Detect the image format from the first bytes of a buffer (no file I/O).
///
/// Returns `None` when the bytes do not match any known magic sequence.
pub fn detect_format_from_bytes(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return Some(ImageFormat::Jpeg);
    }
    if bytes.len() >= 8 && bytes[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some(ImageFormat::Png);
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return Some(ImageFormat::Gif);
    }
    if bytes.len() >= 4
        && (bytes[..4] == [0x49, 0x49, 0x2A, 0x00] || bytes[..4] == [0x4D, 0x4D, 0x00, 0x2A])
    {
        return Some(ImageFormat::Tiff);
    }
    if bytes.len() >= 2 && bytes[..2] == [0x42, 0x4D] {
        return Some(ImageFormat::Bmp);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(ImageFormat::WebP);
    }
    None
}

/// Resolve the actual output `ImageFormat` from the requested output format and the input format.
///
/// Formats that cannot be written directly (TIFF, BMP, GIF, WebP — no encoder yet) are
/// automatically converted to JPEG when `output_fmt` is `Original`.
pub fn resolve_output_format(
    output_fmt: &ImageOutputFormat,
    input_fmt: &ImageFormat,
) -> ImageFormat {
    match output_fmt {
        ImageOutputFormat::Original => match input_fmt {
            ImageFormat::Jpeg => ImageFormat::Jpeg,
            ImageFormat::Png => ImageFormat::Png,
            // No encoder for WebP (Phase E), TIFF, BMP, GIF → fall back to JPEG.
            ImageFormat::WebP | ImageFormat::Tiff | ImageFormat::Bmp | ImageFormat::Gif => {
                ImageFormat::Jpeg
            }
        },
        ImageOutputFormat::Jpeg => ImageFormat::Jpeg,
        ImageOutputFormat::Png => ImageFormat::Png,
        ImageOutputFormat::Webp => ImageFormat::WebP,
    }
}

/// Map a `CompressionLevel` to an oxipng preset effort value (0–6).
///
/// Higher effort = more optimization passes and better compression, but slower.
pub fn png_effort_for_level(level: CompressionLevel) -> u8 {
    match level {
        CompressionLevel::Low => 2,
        CompressionLevel::Medium => 4,
        CompressionLevel::High => 5,
        CompressionLevel::Extreme => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_jpeg_from_bytes() {
        let bytes = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_format_from_bytes(&bytes), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn detect_png_from_bytes() {
        let bytes = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(detect_format_from_bytes(&bytes), Some(ImageFormat::Png));
    }

    #[test]
    fn detect_unknown_returns_none() {
        let bytes = [0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(detect_format_from_bytes(&bytes), None);
    }

    #[test]
    fn resolve_original_jpeg_stays_jpeg() {
        let result = resolve_output_format(&ImageOutputFormat::Original, &ImageFormat::Jpeg);
        assert_eq!(result, ImageFormat::Jpeg);
    }

    #[test]
    fn resolve_original_png_stays_png() {
        let result = resolve_output_format(&ImageOutputFormat::Original, &ImageFormat::Png);
        assert_eq!(result, ImageFormat::Png);
    }

    #[test]
    fn resolve_original_tiff_defaults_to_jpeg() {
        let result = resolve_output_format(&ImageOutputFormat::Original, &ImageFormat::Tiff);
        assert_eq!(result, ImageFormat::Jpeg);
    }

    #[test]
    fn resolve_original_bmp_defaults_to_jpeg() {
        let result = resolve_output_format(&ImageOutputFormat::Original, &ImageFormat::Bmp);
        assert_eq!(result, ImageFormat::Jpeg);
    }

    #[test]
    fn resolve_original_gif_defaults_to_jpeg() {
        let result = resolve_output_format(&ImageOutputFormat::Original, &ImageFormat::Gif);
        assert_eq!(result, ImageFormat::Jpeg);
    }

    #[test]
    fn resolve_explicit_jpeg_always_jpeg() {
        let result = resolve_output_format(&ImageOutputFormat::Jpeg, &ImageFormat::Png);
        assert_eq!(result, ImageFormat::Jpeg);
    }

    #[test]
    fn png_effort_maps_correctly() {
        assert_eq!(png_effort_for_level(CompressionLevel::Low), 2);
        assert_eq!(png_effort_for_level(CompressionLevel::Medium), 4);
        assert_eq!(png_effort_for_level(CompressionLevel::High), 5);
        assert_eq!(png_effort_for_level(CompressionLevel::Extreme), 6);
    }
}
